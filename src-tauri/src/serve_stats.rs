//! Estatísticas de serviço: coleta dos counters do `GET /metrics` do
//! llama-server e os dois acumuladores que a tela mostra.
//!
//! Semântica dos números: eles cobrem TODO o tráfego atendido pelo servidor —
//! chat interno e apps externos (DeepSeek Harness, Claude Code, Cursor…) —
//! porque vêm do próprio motor. "Sessão" = desde que o APP abriu (não desde o
//! boot do servidor); "desde sempre" mora na tabela `serve_totals` do SQLite.
//!
//! Como os counters vivem no processo child e morrem com ele, unload/stop
//! apaga até ~30 s de tráfego não coletado. O scrape final best-effort em
//! stop/restart iniciados pelo app encolhe essa janela; o que sobrar é perda
//! aceita e registrada no design.
//!
//! Vizinhança de Prometheus: o app usa SÓ counters e é imune a scrapes de
//! terceiros — mas cada scrape NOSSO zera o bucket dos gauges de janela
//! (`prompt_tokens_seconds`/`predicted_tokens_seconds`) para qualquer outro
//! consumidor. Quem monitora com Prometheus próprio deve usar `rate()` sobre
//! os `*_total` (nota também em `lr_engine::metrics`).

use crate::state::AppState;
use lr_engine::metrics::{DeltaTracker, MetricCounters, parse_metrics};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};
use tauri::{AppHandle, Manager, State};

type CmdResult<T> = Result<T, String>;

/// De quanto em quanto tempo o laço periódico coleta.
const TICK: std::time::Duration = std::time::Duration::from_secs(30);

/// Scrape é HTTP local e rápido; um servidor pendurado não pode segurar nem o
/// laço nem o scrape final do stop.
const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Estado do coletor. Um único mutex cobre tracker + sessão: além de proteger
/// os dados, ele SERIALIZA os scrapes — dois scrapes concorrentes (laço +
/// comando) leriam o mesmo counter duas vezes e dobrariam o delta.
#[derive(Default)]
struct Inner {
    tracker: DeltaTracker,
    /// Acumulado da sessão por modelo (zera no Clear ou ao fechar o app).
    session: HashMap<String, MetricCounters>,
}

pub struct ServeStatsCollector {
    inner: tokio::sync::Mutex<Inner>,
    http: reqwest::Client,
}

impl Default for ServeStatsCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Foto do servidor no ar: URL conectável, chave e a política de sono.
struct ServerSnapshot {
    base: String,
    api_key: Option<String>,
    sleep_idle: bool,
}

async fn server_snapshot(state: &AppState) -> Option<ServerSnapshot> {
    let guard = state.server.lock().await;
    let srv = guard.as_ref().filter(|s| s.is_spawned())?;
    Some(ServerSnapshot {
        base: srv.config().connect_url(),
        api_key: srv.config().api_key.clone(),
        sleep_idle: srv.config().sleep_idle_seconds > 0,
    })
}

impl ServeStatsCollector {
    pub fn new() -> Self {
        Self {
            inner: tokio::sync::Mutex::new(Inner::default()),
            http: reqwest::Client::builder()
                .timeout(HTTP_TIMEOUT)
                .build()
                .unwrap_or_default(),
        }
    }

    /// Passada do laço periódico de 30 s.
    ///
    /// **Política de sono**: com `sleep_idle_seconds > 0` o laço NÃO coleta —
    /// cada scrape reseta o timer de idle do child (o `GET /metrics` não usa
    /// bypass de sono) e o modelo nunca dormiria. Nesse modo a coleta só
    /// acontece nas chamadas do comando [`serve_stats`], ou seja, com a tela
    /// de estatísticas aberta — aí manter o modelo acordado é o esperado.
    async fn periodic_pass(&self, state: &AppState) {
        let Some(snap) = server_snapshot(state).await else {
            return;
        };
        if snap.sleep_idle {
            return;
        }
        let mut inner = self.inner.lock().await;
        self.scrape_into(&mut inner, state, &snap).await;
    }

    /// Coleta agora, best-effort (erros só param o modelo da vez). É o que o
    /// comando chama antes de responder e o que stop/restart chamam antes de
    /// derrubar o processo.
    pub async fn scrape_now(&self, state: &AppState) {
        let Some(snap) = server_snapshot(state).await else {
            return;
        };
        let mut inner = self.inner.lock().await;
        self.scrape_into(&mut inner, state, &snap).await;
    }

    /// O scrape em si, já com o mutex do coletor na mão.
    async fn scrape_into(&self, inner: &mut Inner, state: &AppState, snap: &ServerSnapshot) {
        // Só modelos `loaded`: perguntar por um `sleeping` o ACORDARIA, e os
        // demais estados nem têm counters para dar.
        let modelos = match self.loaded_models(snap).await {
            Some(m) => m,
            None => return,
        };
        for modelo in modelos {
            let Some(atual) = self.scrape_model(snap, &modelo).await else {
                continue;
            };
            let delta = inner.tracker.delta(&modelo, atual);
            if delta.is_zero() {
                // Sem tráfego novo: nada a somar, nada a escrever no banco.
                continue;
            }
            // Tráfego novo: a máquina está sendo usada. A bateria de
            // especulação espera por este relógio — e ela mesma gera tokens,
            // então precisa não se marcar como "uso" ou nunca começaria.
            if !crate::commands_tuning::medindo() {
                state.last_engine_use.store(
                    crate::commands::now_ms(),
                    std::sync::atomic::Ordering::SeqCst,
                );
            }
            inner.session.entry(modelo.clone()).or_default().add(&delta);
            // Tokens são contagens; o arredondamento desfaz o ruído da
            // serialização em 6 dígitos significativos do upstream.
            if let Err(e) = state.store.serve_totals_add(
                &modelo,
                delta.prompt_tokens.round() as i64,
                delta.cached_tokens.round() as i64,
                delta.prompt_seconds,
                delta.predicted_tokens.round() as i64,
                delta.predicted_seconds,
            ) {
                log::warn!("estatísticas de serviço: não gravei {modelo}: {e}");
            }
        }
    }

    /// `GET /models` com o cliente COM timeout deste coletor (o scrape final
    /// do stop não pode pendurar), filtrado ao estado `loaded`.
    async fn loaded_models(&self, snap: &ServerSnapshot) -> Option<Vec<String>> {
        let mut req = self.http.get(format!("{}/models", snap.base));
        if let Some(key) = &snap.api_key {
            req = req.bearer_auth(key);
        }
        let resp = req.send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let corpo: serde_json::Value = resp.json().await.ok()?;
        Some(
            lr_engine::parse_models_status(&corpo)?
                .into_iter()
                .filter(|m| m.state == "loaded")
                .map(|m| m.id)
                .collect(),
        )
    }

    /// `GET /metrics?model=…&autoload=false` de um modelo.
    ///
    /// `autoload=false` é OBRIGATÓRIO: sem ele o proxy GET do router CARREGA
    /// o modelo se ele foi descarregado entre a listagem e o scrape
    /// (`models_autoload = true` no common.h do b10441) — e com models_max=1
    /// isso despejaria o modelo em uso. O query param também resolve o
    /// URL-encoding de ids com `/` e `:`. HTTP 400 ("model is not loaded") ou
    /// qualquer outro erro = pular o modelo nesta rodada.
    async fn scrape_model(&self, snap: &ServerSnapshot, modelo: &str) -> Option<MetricCounters> {
        let mut req = self
            .http
            .get(format!("{}/metrics", snap.base))
            .query(&[("model", modelo), ("autoload", "false")]);
        if let Some(key) = &snap.api_key {
            req = req.bearer_auth(key);
        }
        let resp = req.send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        Some(parse_metrics(&resp.text().await.ok()?))
    }
}

/// Laço do coletor: só pode nascer DEPOIS de `app.manage(state)` — cada
/// tick pega o estado pelo handle.
pub fn spawn_loop(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let state = app.state::<AppState>();
            state.serve_stats.periodic_pass(&state).await;
        }
    });
}

// ---------------------------------------------------------------- DTOs ---

/// Agregado de um recorte (sessão ou desde sempre; todos os modelos ou um).
#[derive(Serialize, Clone, Copy, Default)]
#[serde(rename_all = "camelCase")]
pub struct ServeAgg {
    pub prompt_tokens: f64,
    pub cached_tokens: f64,
    pub predicted_tokens: f64,
    /// prompt + cached + predicted.
    pub total_tokens: f64,
    /// cached / (prompt + cached); `null` sem dados.
    pub cache_efficiency: Option<f64>,
    /// promptTokens / promptSeconds — EXCLUI cache, o recorte do upstream.
    pub avg_prompt_tps: Option<f64>,
    /// predictedTokens / predictedSeconds.
    pub avg_gen_tps: Option<f64>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ServeStatsDto {
    /// Servidor de pé agora (senão os números são só históricos).
    pub running: bool,
    /// Modelos com dados (união sessão ∪ desde sempre), sem filtro.
    pub models: Vec<String>,
    pub session: ServeAgg,
    pub all_time: ServeAgg,
}

/// Fecha um agregado a partir das somas cruas: médias por divisão (imunes ao
/// efeito de janela dos gauges) e `null` onde não há denominador.
fn close_agg(sum: &MetricCounters) -> ServeAgg {
    let processed = sum.prompt_tokens + sum.cached_tokens;
    ServeAgg {
        prompt_tokens: sum.prompt_tokens,
        cached_tokens: sum.cached_tokens,
        predicted_tokens: sum.predicted_tokens,
        total_tokens: processed + sum.predicted_tokens,
        cache_efficiency: (processed > 0.0).then(|| sum.cached_tokens / processed),
        avg_prompt_tps: (sum.prompt_seconds > 0.0).then(|| sum.prompt_tokens / sum.prompt_seconds),
        avg_gen_tps: (sum.predicted_seconds > 0.0)
            .then(|| sum.predicted_tokens / sum.predicted_seconds),
    }
}

// ------------------------------------------------------------ comandos ---

/// As estatísticas para a tela. Com `model`, os agregados cobrem só aquele
/// modelo; sem, cobrem tudo. A lista `models` vem sempre inteira.
///
/// A chamada também dispara uma coleta imediata (com o servidor de pé): é o
/// que mantém os números frescos no modo de sono, em que o laço periódico
/// fica desligado de propósito.
#[tauri::command]
pub async fn serve_stats(
    state: State<'_, AppState>,
    model: Option<String>,
) -> CmdResult<ServeStatsDto> {
    let running = server_snapshot(&state).await.is_some();
    if running {
        state.serve_stats.scrape_now(&state).await;
    }

    let filtro = model.as_deref().map(str::trim).filter(|m| !m.is_empty());

    let mut nomes: BTreeSet<String> = BTreeSet::new();
    let mut session_sum = MetricCounters::default();
    {
        let inner = state.serve_stats.inner.lock().await;
        for (modelo, acc) in &inner.session {
            nomes.insert(modelo.clone());
            if filtro.is_none_or(|f| f == modelo.as_str()) {
                session_sum.add(acc);
            }
        }
    }

    let mut all_time_sum = MetricCounters::default();
    for row in state.store.serve_totals().map_err(|e| e.to_string())? {
        nomes.insert(row.model_id.clone());
        if filtro.is_none_or(|f| f == row.model_id) {
            all_time_sum.add(&MetricCounters {
                prompt_tokens: row.prompt_tokens as f64,
                cached_tokens: row.cached_tokens as f64,
                prompt_seconds: row.prompt_seconds,
                predicted_tokens: row.predicted_tokens as f64,
                predicted_seconds: row.predicted_seconds,
            });
        }
    }

    Ok(ServeStatsDto {
        running,
        models: nomes.into_iter().collect(),
        session: close_agg(&session_sum),
        all_time: close_agg(&all_time_sum),
    })
}

/// O "Limpar": zera sessão E desde sempre. A última leitura do coletor fica —
/// apagá-la faria o próximo scrape recontar o counter inteiro como delta.
#[tauri::command]
pub async fn serve_stats_clear(state: State<'_, AppState>) -> CmdResult<()> {
    let mut inner = state.serve_stats.inner.lock().await;
    inner.session.clear();
    state.store.serve_totals_clear().map_err(|e| e.to_string())
}
