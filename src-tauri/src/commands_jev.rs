//! Comandos do Jev — a camada de decisão barata na frente dos modelos locais.
//!
//! O que passa por aqui: a configuração (um setting só, como o gateway), o
//! estado para a tela, um teste, a decisão que o chat pede antes de mandar a
//! conversa ao llama-server, e o ciclo de vida do **decisor local** — um
//! segundo `llama-server` (do fork `parallel-decision`) com um modelo pequeno
//! só para decidir, que sobe junto do motor principal.
//!
//! A decisão vem de uma cadeia: o decisor local primeiro, o Jev remoto via
//! OpenRouter como reserva (quando há chave e a reserva está ligada), e o
//! padrão da pessoa quando nenhum responde.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use lr_providers::jev_esforco::{
    LIMIAR_CONFIANCA_MAX, LIMIAR_CONFIANCA_MIN, LIMIAR_CONFIANCA_PADRAO, aplicar_no_chat,
    decidir_esforco, montar_estado, perguntas,
};
use lr_providers::{
    CapacidadeModelo, ClienteDecisaoLocal, ClienteJev, ContextoDecisao, DecisaoEsforco, Decisores,
    JEV_MODELO_PADRAO, MensagemResumida, Origem, ResumoContadores, Superficie,
};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};

use crate::state::AppState;

type CmdResult<T> = Result<T, String>;

fn err_str<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Chave do setting. Separada de `providers.config` pelo mesmo motivo do
/// gateway: é um recurso opcional por cima dos provedores, não um deles.
pub const JEV_SETTING: &str = "jev.config";

/// O modelo padrão do decisor local: Qwen2.5-1.5B-Instruct em Q8_0 — atenção
/// pura (o fork rende melhor nela), 1,9 GB, Apache-2.0, e a base do
/// experimento original que provou a ideia.
pub const DECISOR_MODELO_PADRAO_REPO: &str = "Qwen/Qwen2.5-1.5B-Instruct-GGUF";
pub const DECISOR_MODELO_PADRAO: &str = "qwen2.5-1.5b-instruct-q8_0.gguf";
pub const DECISOR_MODELO_BYTES: u64 = 1_894_532_128;

/// Abaixo disto o decisor local não liga sozinho: são ~2,6 GB de VRAM ao lado
/// do modelo do chat, e as medições de "cabe?" não descontam isso. A pessoa
/// pode ligar à mão, com o custo escrito na tela.
pub const VRAM_MINIMA_AUTO: u64 = 12 << 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JevConfig {
    pub enabled: bool,
    pub model: String,
    /// Decidir o esforço das conversas do chat do app.
    pub gate_chat: bool,
    /// Decidir o esforço das requisições dos harnesses (via proxy).
    pub gate_harness: bool,
    pub min_confidence: f32,
    /// O decisor local. `None` = a regra da VRAM decide (ver
    /// [`VRAM_MINIMA_AUTO`]); `Some` é a escolha explícita da pessoa.
    pub local_enabled: Option<bool>,
    /// O GGUF da biblioteca que o decisor local carrega.
    pub local_model: String,
    /// Cair no Jev remoto (OpenRouter) quando o local não responde.
    pub remote_fallback: bool,
}

impl Default for JevConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: JEV_MODELO_PADRAO.to_string(),
            gate_chat: true,
            gate_harness: true,
            min_confidence: LIMIAR_CONFIANCA_PADRAO,
            local_enabled: None,
            local_model: DECISOR_MODELO_PADRAO.to_string(),
            remote_fallback: true,
        }
    }
}

impl JevConfig {
    /// Deixa a configuração num estado que o resto do código pode assumir:
    /// modelos não-vazios e limiar dentro da faixa.
    fn normalizada(mut self) -> Self {
        if self.model.trim().is_empty() {
            self.model = JEV_MODELO_PADRAO.to_string();
        }
        if self.local_model.trim().is_empty() {
            self.local_model = DECISOR_MODELO_PADRAO.to_string();
        }
        if !self.min_confidence.is_finite() {
            self.min_confidence = LIMIAR_CONFIANCA_PADRAO;
        }
        self.min_confidence = self
            .min_confidence
            .clamp(LIMIAR_CONFIANCA_MIN, LIMIAR_CONFIANCA_MAX);
        self
    }
}

pub(crate) fn jev_config(state: &AppState) -> JevConfig {
    state
        .store
        .get_setting(JEV_SETTING)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<JevConfig>(&raw).ok())
        .unwrap_or_default()
        .normalizada()
}

/// A chave do OpenRouter, se há uma e o provedor está ligado.
pub(crate) fn chave_openrouter(state: &AppState) -> Option<String> {
    let cfg = crate::commands_providers::load_config(state);
    let chave = cfg.open_router.api_key.trim();
    (cfg.open_router.enabled && !chave.is_empty()).then(|| chave.to_string())
}

/// A GPU tem VRAM de sobra para um segundo processo?
pub(crate) fn vram_ok(state: &AppState) -> bool {
    state
        .profile
        .best_gpu()
        .is_some_and(|g| g.vram_total_bytes >= VRAM_MINIMA_AUTO)
}

/// O decisor local está ligado: pela pessoa, ou pela regra da VRAM.
pub(crate) fn local_ligado(cfg: &JevConfig, state: &AppState) -> bool {
    cfg.local_enabled.unwrap_or_else(|| vram_ok(state))
}

/// A URL do decisor local, se ele está no ar (pronto ou ainda carregando —
/// carregando responde 503, e a cadeia cai no remoto sozinha).
pub(crate) async fn base_url_decisor_local(state: &AppState) -> Option<String> {
    state
        .decisor_local
        .lock()
        .await
        .as_ref()
        .filter(|d| d.is_spawned())
        .map(|d| d.base_url())
}

/// A cadeia de decisores pronta, ou o motivo de não haver nenhum.
pub(crate) async fn decisores(state: &AppState) -> Result<Decisores, &'static str> {
    let cfg = jev_config(state);
    if !cfg.enabled {
        return Err("Jev desligado");
    }
    let local = if local_ligado(&cfg, state) {
        base_url_decisor_local(state)
            .await
            .map(|url| ClienteDecisaoLocal::novo(&url))
    } else {
        None
    };
    let remoto = if cfg.remote_fallback {
        chave_openrouter(state).map(|chave| ClienteJev::openrouter(chave).with_modelo(cfg.model))
    } else {
        None
    };
    let d = Decisores { local, remoto };
    if d.vazio() {
        return Err("nenhum decisor: instale o decisor local ou configure a chave do OpenRouter");
    }
    Ok(d)
}

// ----------------------------------------------------------- capacidades ---

/// O que cada modelo local sabe fazer com raciocínio, lido do cabeçalho do
/// GGUF. Cache por nome pedido: a leitura é barata, mas não a ponto de valer
/// a cada mensagem.
pub(crate) fn capacidade_do_modelo(state: &AppState, id: &str) -> CapacidadeModelo {
    capacidade_cacheada(&state.models_dir, &state.jev_capacidades, id)
}

type CacheCapacidades = Arc<std::sync::Mutex<HashMap<String, CapacidadeModelo>>>;

fn capacidade_cacheada(models_dir: &Path, cache: &CacheCapacidades, id: &str) -> CapacidadeModelo {
    if let Some(c) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(id) {
        return c.clone();
    }
    let capacidade = ler_capacidade(models_dir, id).unwrap_or_default();
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.to_string(), capacidade.clone());
    capacidade
}

/// Esquece o cache — quando o motor sobe, o conjunto de modelos pode ter
/// mudado.
pub(crate) fn esquecer_capacidades(state: &AppState) {
    state
        .jev_capacidades
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
}

fn sem_gguf(s: &str) -> &str {
    s.strip_suffix(".gguf")
        .or_else(|| s.strip_suffix(".GGUF"))
        .unwrap_or(s)
}

fn ler_capacidade(models_dir: &Path, id: &str) -> Option<CapacidadeModelo> {
    let artefatos = lr_models::scan_local(models_dir);
    let a = artefatos
        .iter()
        .find(|a| a.name == id || sem_gguf(&a.name) == sem_gguf(id))?;
    let meta = lr_models::read_local_meta(&a.primary_path);
    Some(CapacidadeModelo {
        thinking_toggle: meta.thinking_toggle,
        efforts: meta.reasoning_efforts,
    })
}

// -------------------------------------------------------------- comandos ---

/// O decisor local, como a tela o vê.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JevLocalStatus {
    /// Esta máquina pode rodá-lo (Windows/Linux x86_64 com CUDA 13).
    pub supported: bool,
    /// Ligado: pela pessoa ou pela regra da VRAM.
    pub enabled: bool,
    /// A escolha explícita da pessoa, se houver.
    pub enabled_by_user: Option<bool>,
    pub vram_ok: bool,
    pub runtime_installed: bool,
    pub runtime_tag: String,
    pub runtime_approx_bytes: u64,
    pub model: String,
    pub model_present: bool,
    pub model_repo: String,
    pub model_bytes: u64,
    /// O processo está no ar (pode ainda estar carregando o modelo).
    pub running: bool,
    /// Respondeu ao `/health` e foi aquecido.
    pub ready: bool,
    pub base_url: Option<String>,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JevStatus {
    pub enabled: bool,
    pub key_present: bool,
    pub remote_fallback: bool,
    pub gate_chat: bool,
    pub gate_harness: bool,
    pub min_confidence: f32,
    pub model: String,
    pub proxy_running: bool,
    pub proxy_port: Option<u16>,
    pub proxy_base_url: Option<String>,
    pub local: JevLocalStatus,
    pub contadores: ResumoContadores,
}

pub(crate) async fn montar_status_local(state: &AppState) -> JevLocalStatus {
    let cfg = jev_config(state);
    let runtime = state.runtime_mgr.decision_state();
    let processo = state.decisor_local.lock().await;
    let no_ar = processo.as_ref().filter(|d| d.is_spawned());
    JevLocalStatus {
        supported: lr_runtime::decision::supported(&state.profile),
        enabled: local_ligado(&cfg, state),
        enabled_by_user: cfg.local_enabled,
        vram_ok: vram_ok(state),
        runtime_installed: runtime.installed,
        runtime_tag: lr_runtime::decision::TAG.to_string(),
        runtime_approx_bytes: lr_runtime::decision::TAMANHO_APROXIMADO_BYTES,
        model_present: crate::commands::caminho_do_modelo(state, &cfg.local_model).is_some(),
        model: cfg.local_model,
        model_repo: DECISOR_MODELO_PADRAO_REPO.to_string(),
        model_bytes: DECISOR_MODELO_BYTES,
        running: no_ar.is_some(),
        ready: no_ar.is_some() && state.decisor_local_pronto.load(Ordering::SeqCst),
        base_url: no_ar.map(|d| d.base_url()),
        port: no_ar.map(|d| d.config().port),
    }
}

pub(crate) async fn montar_status(state: &AppState) -> JevStatus {
    let cfg = jev_config(state);
    let local = montar_status_local(state).await;
    let proxy = state.decisor.lock().await;
    JevStatus {
        enabled: cfg.enabled,
        key_present: chave_openrouter(state).is_some(),
        remote_fallback: cfg.remote_fallback,
        gate_chat: cfg.gate_chat,
        gate_harness: cfg.gate_harness,
        min_confidence: cfg.min_confidence,
        model: cfg.model,
        proxy_running: proxy.is_some(),
        proxy_port: proxy.as_ref().map(|p| p.porta()),
        proxy_base_url: proxy.as_ref().map(|p| p.base_url()),
        local,
        contadores: state.jev_contadores.resumo(),
    }
}

#[tauri::command]
pub fn jev_config_get(state: State<'_, AppState>) -> CmdResult<JevConfig> {
    Ok(jev_config(&state))
}

#[tauri::command]
pub async fn jev_config_set(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    config: JevConfig,
) -> CmdResult<JevStatus> {
    let cfg = config.normalizada();
    let json = serde_json::to_string(&cfg).map_err(err_str)?;
    state
        .store
        .set_setting(JEV_SETTING, &json)
        .map_err(err_str)?;
    sincronizar_decisor_local(&app, &state).await;
    sincronizar_shim(&app, &state).await;
    Ok(montar_status(&state).await)
}

#[tauri::command]
pub async fn jev_status(state: State<'_, AppState>) -> CmdResult<JevStatus> {
    Ok(montar_status(&state).await)
}

/// Uma decisão de amostra, para a tela provar que a cadeia funciona antes
/// de a pessoa depender disso numa conversa. Devolve também quem respondeu
/// e quanto tempo levou.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TesteJev {
    #[serde(flatten)]
    pub decisao: DecisaoEsforco,
    pub ms: u64,
}

#[tauri::command]
pub async fn jev_testar(state: State<'_, AppState>) -> CmdResult<TesteJev> {
    let cfg = jev_config(&state);
    let d = decisores(&state).await?;
    let mensagens = [MensagemResumida::nova(
        "user",
        "Prove that the square root of 2 is irrational.",
    )];
    let inicio = std::time::Instant::now();
    let decisao = decidir_esforco(
        &d,
        &mensagens,
        &ContextoDecisao::default(),
        cfg.min_confidence,
        Superficie::Chat,
        Some(&state.jev_contadores),
    )
    .await;
    let ms = inicio.elapsed().as_millis() as u64;
    // Um teste que "passa" com fail-open não provaria nada: quando NENHUM
    // decisor respondeu, o motivo vira erro de verdade. Resposta com
    // confiança baixa é resultado (mostra a fonte e o número), não erro.
    if decisao.origem == Origem::Padrao && decisao.fonte.is_none() {
        return Err(decisao
            .motivo
            .unwrap_or_else(|| "nenhum decisor respondeu".to_string()));
    }
    Ok(TesteJev { decisao, ms })
}

/// O que o chat recebe: o `effort` a usar (o dele mesmo quando nada foi
/// decidido) e de onde veio.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisaoChat {
    pub effort: String,
    pub source: Origem,
    pub confidence: Option<f32>,
    pub cost: Option<f64>,
    pub reason: Option<String>,
}

impl DecisaoChat {
    fn padrao(effort: &str, motivo: &str) -> Self {
        Self {
            effort: effort.to_string(),
            source: Origem::Padrao,
            confidence: None,
            cost: None,
            reason: Some(motivo.to_string()),
        }
    }
}

/// Decide o esforço de UMA requisição do chat. Nunca falha: o pior caso é
/// devolver `default_effort` com o motivo.
#[tauri::command]
pub async fn jev_decidir_esforco(
    state: State<'_, AppState>,
    model: String,
    messages: Vec<MensagemResumida>,
    default_effort: String,
    contexto: Option<ContextoDecisao>,
) -> CmdResult<DecisaoChat> {
    let cfg = jev_config(&state);
    if !cfg.enabled || !cfg.gate_chat {
        return Ok(DecisaoChat::padrao(
            &default_effort,
            "Jev desligado para o chat",
        ));
    }
    let d = match decisores(&state).await {
        Ok(d) => d,
        Err(motivo) => return Ok(DecisaoChat::padrao(&default_effort, motivo)),
    };
    if !capacidade_do_modelo(&state, &model).pode_raciocinar() {
        return Ok(DecisaoChat::padrao(
            &default_effort,
            "o modelo não tem interruptor de raciocínio",
        ));
    }
    let decisao = decidir_esforco(
        &d,
        &messages,
        &contexto.unwrap_or_default(),
        cfg.min_confidence,
        Superficie::Chat,
        Some(&state.jev_contadores),
    )
    .await;
    Ok(match decisao.nivel {
        Some(nivel) if decisao.aplicada() => DecisaoChat {
            effort: aplicar_no_chat(nivel, &default_effort),
            source: decisao.origem,
            confidence: decisao.confianca,
            cost: decisao.custo,
            reason: None,
        },
        _ => DecisaoChat {
            effort: default_effort,
            source: Origem::Padrao,
            confidence: decisao.confianca,
            cost: decisao.custo,
            reason: decisao.motivo,
        },
    })
}

// ----------------------------------------------------------------- proxy ---

/// Sobe ou derruba o proxy dos harnesses conforme a configuração e o motor.
///
/// Chamado quando qualquer uma das três condições muda: o motor subiu ou
/// desceu, a configuração do Jev mudou, a chave do OpenRouter mudou. Fazer
/// isso no backend (e não na tela) garante que vale também quando quem sobe
/// o motor é o agendador ou o próprio dsh.
pub(crate) async fn sincronizar_shim(app: &tauri::AppHandle, state: &AppState) {
    let cfg = jev_config(state);
    let decisores = decisores(state).await.unwrap_or_default();
    let decisor_local_url = if cfg.enabled {
        base_url_decisor_local(state).await
    } else {
        None
    };
    let upstream = {
        let guard = state.server.lock().await;
        guard
            .as_ref()
            .filter(|s| s.is_spawned())
            .map(|s| s.config().connect_url())
    };
    // O proxy serve duas coisas: o portão dos harnesses (só com decisores
    // para decidir) e o repasse do `/v1/decision` ao decisor local. Sem o
    // portão ele não decide — passa direto e só repassa decisões.
    let deve_rodar = cfg.enabled
        && upstream.is_some()
        && ((cfg.gate_harness && !decisores.vazio()) || decisor_local_url.is_some());
    let politica = if cfg.gate_harness {
        decisores
    } else {
        Decisores::default()
    };

    let mut slot = state.decisor.lock().await;
    match (deve_rodar, slot.as_ref()) {
        (true, Some(atual)) => {
            // Já no ar: só atualiza decisores e limiar, sem derrubar conexões.
            atual
                .atualizar(politica, decisor_local_url, cfg.min_confidence)
                .await;
        }
        (true, None) => {
            let (models_dir, cache) = (state.models_dir.clone(), state.jev_capacidades.clone());
            let config = lr_decisor::ConfigShim {
                upstream: upstream.unwrap_or_default(),
                porta_preferida: lr_decisor::PORTA_PADRAO,
                decisores: politica,
                decisor_local_url,
                min_confianca: cfg.min_confidence,
                capacidade: Arc::new(move |id: &str| capacidade_cacheada(&models_dir, &cache, id)),
                contadores: state.jev_contadores.clone(),
                tempo_decisao: lr_decisor::TEMPO_DECISAO_PADRAO,
            };
            match lr_decisor::Shim::iniciar(config).await {
                Ok(shim) => {
                    log::info!("proxy do Jev no ar em {}", shim.base_url());
                    *slot = Some(shim);
                }
                Err(e) => log::warn!("não foi possível subir o proxy do Jev: {e}"),
            }
        }
        (false, Some(_)) => {
            if let Some(shim) = slot.take() {
                shim.parar().await;
                log::info!("proxy do Jev parado");
            }
        }
        (false, None) => {}
    }
    // O proxy, o motor ou as chaves mudaram: o catálogo do AgenticOw também.
    crate::commands_agenticow::agendar_catalogo(app);
}

// --------------------------------------------------------- decisor local ---

/// Este evento de download é o modelo do decisor local ficando pronto?
pub(crate) fn e_download_do_decisor(ev: &lr_models::DownloadEvent) -> bool {
    match ev {
        lr_models::DownloadEvent::Update { status } => {
            status.state == lr_models::DownloadState::Done
                && status.id
                    == lr_models::download_id(DECISOR_MODELO_PADRAO_REPO, DECISOR_MODELO_PADRAO)
        }
        lr_models::DownloadEvent::Removed { .. } => false,
    }
}

/// Sobe ou derruba o decisor local conforme a configuração, o que está
/// instalado e o motor principal.
///
/// Sobe quando: Jev ligado, decisor local ligado (pela pessoa ou pela VRAM),
/// motor de decisão instalado, modelo na biblioteca, motor principal no ar,
/// GPU não reservada pelo Studio e nenhuma medição em curso. Qualquer outra
/// combinação derruba. Não espera o `/health`: o modelo leva segundos para
/// carregar, e enquanto isso o servidor responde 503 — a cadeia cai no
/// remoto sozinha. A espera e o aquecimento correm numa task.
pub(crate) async fn sincronizar_decisor_local(app: &tauri::AppHandle, state: &AppState) {
    let cfg = jev_config(state);
    let runtime = state.runtime_mgr.decision_state();
    let modelo = crate::commands::caminho_do_modelo(state, &cfg.local_model);
    let motor_no_ar = state
        .server
        .lock()
        .await
        .as_ref()
        .is_some_and(|s| s.is_spawned());
    let deve_rodar = cfg.enabled
        && local_ligado(&cfg, state)
        && runtime.installed
        && modelo.is_some()
        && motor_no_ar
        && !crate::studio::GPU_RESERVED.load(Ordering::SeqCst)
        && !crate::comparison::active();

    let mut slot = state.decisor_local.lock().await;
    let mesmo_modelo = slot
        .as_ref()
        .filter(|d| d.is_spawned())
        .is_some_and(|d| Some(&d.config().model_path) == modelo.as_ref());
    match (deve_rodar, mesmo_modelo) {
        (true, true) => {}
        (true, false) => {
            if let Some(mut antigo) = slot.take() {
                antigo.stop().await;
                crate::gpu_lease::release("decisor");
                state.decisor_local_pronto.store(false, Ordering::SeqCst);
            }
            let (Some(exe), Some(modelo)) = (runtime.server_exe, modelo) else {
                return;
            };
            if let Err(e) = crate::gpu_lease::acquire("decisor") {
                log::warn!("decisor local não subiu: {e}");
                return;
            }
            let porta = lr_proc::free_port(lr_engine::DECISION_PORTA_PADRAO);
            let mut servidor = lr_engine::DecisionServer::new(
                lr_engine::DecisionServerConfig::new(exe, modelo, porta),
            );
            if let Err(e) = servidor.spawn() {
                log::warn!("decisor local não subiu: {e}");
                crate::gpu_lease::release("decisor");
                return;
            }
            state
                .decisor_local_pid
                .store(servidor.pid().unwrap_or(0), Ordering::SeqCst);
            let (stdout, stderr) = servidor.take_output();
            for saida in [stdout.map(Saida::Out), stderr.map(Saida::Err)]
                .into_iter()
                .flatten()
            {
                let app2 = app.clone();
                tauri::async_runtime::spawn(async move {
                    use tokio::io::AsyncBufReadExt;
                    match saida {
                        Saida::Out(o) => {
                            let mut lines = tokio::io::BufReader::new(o).lines();
                            while let Ok(Some(line)) = lines.next_line().await {
                                let _ = app2.emit("server-log", &format!("[decisor] {line}"));
                            }
                        }
                        Saida::Err(e) => {
                            let mut lines = tokio::io::BufReader::new(e).lines();
                            while let Ok(Some(line)) = lines.next_line().await {
                                let _ = app2.emit("server-log", &format!("[decisor] {line}"));
                            }
                        }
                    }
                });
            }
            let base_url = servidor.base_url();
            *slot = Some(servidor);
            log::info!("decisor local subindo em {base_url}");
            let app2 = app.clone();
            tauri::async_runtime::spawn(async move {
                aguardar_decisor_local(app2, base_url).await;
            });
        }
        (false, _) => {
            if let Some(mut antigo) = slot.take() {
                antigo.stop().await;
                crate::gpu_lease::release("decisor");
                state.decisor_local_pid.store(0, Ordering::SeqCst);
                state.decisor_local_pronto.store(false, Ordering::SeqCst);
                log::info!("decisor local parado");
            }
        }
    }
}

enum Saida {
    Out(tokio::process::ChildStdout),
    Err(tokio::process::ChildStderr),
}

/// Espera o `/health`, aquece com uma decisão de amostra (paga o prefill do
/// prefixo agora, não na primeira mensagem da pessoa) e avisa a tela.
async fn aguardar_decisor_local(app: tauri::AppHandle, base_url: String) {
    let state = app.state::<AppState>();
    let prazo = tokio::time::Instant::now() + std::time::Duration::from_secs(90);
    let cliente =
        ClienteDecisaoLocal::novo(&base_url).with_timeout(std::time::Duration::from_secs(20));
    loop {
        let atual = state
            .decisor_local
            .lock()
            .await
            .as_ref()
            .filter(|d| d.is_spawned() && d.base_url() == base_url)
            .map(|d| d.config().clone());
        let Some(_cfg) = atual else {
            return; // parado (ou trocado) no meio da subida
        };
        let saude = {
            let guard = state.decisor_local.lock().await;
            match guard.as_ref() {
                Some(d) => d.health().await,
                None => return,
            }
        };
        if saude == lr_engine::Health::Ready {
            break;
        }
        if tokio::time::Instant::now() >= prazo {
            log::warn!("decisor local não respondeu ao /health em 90 s; derrubando");
            let mut guard = state.decisor_local.lock().await;
            if let Some(mut d) = guard.take() {
                d.stop().await;
                crate::gpu_lease::release("decisor");
                state.decisor_local_pid.store(0, Ordering::SeqCst);
            }
            let _ = app.emit("jev-status", &montar_status(&state).await);
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    let estado = montar_estado(
        &[MensagemResumida::nova("user", "hello")],
        &ContextoDecisao::default(),
    )
    .unwrap_or_default();
    let inicio = std::time::Instant::now();
    match cliente.decidir(&estado, &perguntas()).await {
        Ok(_) => log::info!(
            "decisor local pronto em {base_url} (aquecimento: {} ms)",
            inicio.elapsed().as_millis()
        ),
        Err(e) => log::warn!("decisor local respondeu ao /health mas não decidiu: {e}"),
    }
    state.decisor_local_pronto.store(true, Ordering::SeqCst);
    sincronizar_shim(&app, &state).await;
    let _ = app.emit("jev-status", &montar_status(&state).await);
}

/// Instala o decisor local: o motor (release nossa, ~200 MB) e o modelo
/// padrão (Hub, 1,9 GB), os dois em segundo plano e com progresso no painel
/// de downloads. Quando cada parte termina, o processo sobe sozinho se o
/// motor principal estiver no ar.
#[tauri::command]
pub async fn jev_local_install(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<JevLocalStatus> {
    if !lr_runtime::decision::supported(&state.profile) {
        return Err("decision-unsupported".into());
    }
    let cfg = jev_config(&state);
    if crate::commands::caminho_do_modelo(&state, &cfg.local_model).is_none() {
        if cfg.local_model != DECISOR_MODELO_PADRAO {
            return Err(format!(
                "o modelo escolhido para o decisor ({}) não está na biblioteca",
                cfg.local_model
            ));
        }
        crate::commands::enfileirar_download(
            &state,
            DECISOR_MODELO_PADRAO_REPO,
            DECISOR_MODELO_PADRAO,
        )
        .await?;
    }
    if state.runtime_mgr.decision_state().installed {
        sincronizar_decisor_local(&app, &state).await;
        sincronizar_shim(&app, &state).await;
    } else {
        let mgr = state.runtime_mgr.clone();
        let profile = state.profile.clone();
        let app2 = app.clone();
        tauri::async_runtime::spawn(async move {
            let app3 = app2.clone();
            let r = mgr
                .ensure_decision(&profile, move |ev| {
                    let _ = app3.emit("runtime-decision", &ev);
                })
                .await;
            match r {
                Ok(_) => {
                    let st = app2.state::<AppState>();
                    sincronizar_decisor_local(&app2, &st).await;
                    sincronizar_shim(&app2, &st).await;
                    let _ = app2.emit("jev-status", &montar_status(&st).await);
                }
                Err(e) => log::warn!("motor de decisão não instalou: {e}"),
            }
        });
    }
    Ok(montar_status_local(&state).await)
}

/// A URL local que os harnesses devem usar: o proxy quando está no ar,
/// senão nada (quem chama cai na URL do motor).
pub(crate) async fn base_local_para_harness(state: &AppState) -> Option<String> {
    state.decisor.lock().await.as_ref().map(|p| p.base_url())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_broken_config_falls_back_to_the_defaults() {
        let cfg: JevConfig = serde_json::from_str("{}").unwrap();
        assert!(!cfg.enabled);
        assert_eq!(cfg.model, JEV_MODELO_PADRAO);
        assert!(cfg.gate_chat && cfg.gate_harness);
        assert_eq!(cfg.min_confidence, LIMIAR_CONFIANCA_PADRAO);
    }

    #[test]
    fn normalisation_clamps_the_threshold_and_restores_an_empty_model() {
        let cfg = JevConfig {
            model: "   ".into(),
            min_confidence: 3.0,
            ..Default::default()
        }
        .normalizada();
        assert_eq!(cfg.model, JEV_MODELO_PADRAO);
        assert_eq!(cfg.min_confidence, LIMIAR_CONFIANCA_MAX);
        let cfg = JevConfig {
            min_confidence: f32::NAN,
            ..Default::default()
        }
        .normalizada();
        assert_eq!(cfg.min_confidence, LIMIAR_CONFIANCA_PADRAO);
    }

    #[test]
    fn the_config_round_trips_in_camel_case() {
        let json = serde_json::to_string(&JevConfig::default()).unwrap();
        assert!(json.contains("\"gateChat\""));
        assert!(json.contains("\"minConfidence\""));
        let de: JevConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(de.model, JEV_MODELO_PADRAO);
    }

    #[test]
    fn the_gguf_suffix_is_ignored_when_matching_a_model_name() {
        assert_eq!(sem_gguf("qwen3.gguf"), "qwen3");
        assert_eq!(sem_gguf("qwen3.GGUF"), "qwen3");
        assert_eq!(sem_gguf("qwen3"), "qwen3");
    }
}
