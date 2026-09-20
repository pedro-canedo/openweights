//! Comandos do Jev — a camada de decisão barata na frente dos modelos locais.
//!
//! O que passa por aqui: a configuração (um setting só, como o gateway), o
//! estado para a tela, um teste de chave, e a decisão que o chat pede antes
//! de mandar a conversa ao llama-server. O streaming continua no webview; o
//! que o Jev devolve é um `effort` novo para o `params` da requisição.
//!
//! A chave é a do OpenRouter (`providers.config`): o Jev não tem credencial
//! própria neste app, e é isso que faz o recurso "exigir" o OpenRouter.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use lr_providers::jev_esforco::{
    LIMIAR_CONFIANCA_MAX, LIMIAR_CONFIANCA_MIN, LIMIAR_CONFIANCA_PADRAO, aplicar_no_chat,
    decidir_esforco,
};
use lr_providers::{
    CapacidadeModelo, ClienteJev, ContextoDecisao, DecisaoEsforco, JEV_MODELO_PADRAO,
    MensagemResumida, Origem, ResumoContadores, Superficie,
};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::AppState;

type CmdResult<T> = Result<T, String>;

fn err_str<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Chave do setting. Separada de `providers.config` pelo mesmo motivo do
/// gateway: é um recurso opcional por cima dos provedores, não um deles.
pub const JEV_SETTING: &str = "jev.config";

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
}

impl Default for JevConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: JEV_MODELO_PADRAO.to_string(),
            gate_chat: true,
            gate_harness: true,
            min_confidence: LIMIAR_CONFIANCA_PADRAO,
        }
    }
}

impl JevConfig {
    /// Deixa a configuração num estado que o resto do código pode assumir:
    /// modelo não-vazio e limiar dentro da faixa.
    fn normalizada(mut self) -> Self {
        if self.model.trim().is_empty() {
            self.model = JEV_MODELO_PADRAO.to_string();
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

/// O cliente pronto, ou o motivo de não haver um.
pub(crate) fn cliente_jev(state: &AppState) -> Result<ClienteJev, &'static str> {
    let cfg = jev_config(state);
    if !cfg.enabled {
        return Err("Jev desligado");
    }
    let chave = chave_openrouter(state).ok_or("sem chave do OpenRouter")?;
    Ok(ClienteJev::openrouter(chave).with_modelo(cfg.model))
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JevStatus {
    pub enabled: bool,
    pub key_present: bool,
    pub gate_chat: bool,
    pub gate_harness: bool,
    pub min_confidence: f32,
    pub model: String,
    pub proxy_running: bool,
    pub proxy_port: Option<u16>,
    pub proxy_base_url: Option<String>,
    pub contadores: ResumoContadores,
}

pub(crate) async fn montar_status(state: &AppState) -> JevStatus {
    let cfg = jev_config(state);
    let proxy = state.decisor.lock().await;
    JevStatus {
        enabled: cfg.enabled,
        key_present: chave_openrouter(state).is_some(),
        gate_chat: cfg.gate_chat,
        gate_harness: cfg.gate_harness,
        min_confidence: cfg.min_confidence,
        model: cfg.model,
        proxy_running: proxy.is_some(),
        proxy_port: proxy.as_ref().map(|p| p.porta()),
        proxy_base_url: proxy.as_ref().map(|p| p.base_url()),
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
    sincronizar_shim(&app, &state).await;
    Ok(montar_status(&state).await)
}

#[tauri::command]
pub async fn jev_status(state: State<'_, AppState>) -> CmdResult<JevStatus> {
    Ok(montar_status(&state).await)
}

/// Uma decisão de amostra, para a tela provar que a chave e o endpoint
/// funcionam antes de a pessoa depender disso numa conversa.
#[tauri::command]
pub async fn jev_testar(state: State<'_, AppState>) -> CmdResult<DecisaoEsforco> {
    let cfg = jev_config(&state);
    let chave = chave_openrouter(&state).ok_or("configure a chave do OpenRouter primeiro")?;
    let cliente = ClienteJev::openrouter(chave).with_modelo(cfg.model);
    let mensagens = [MensagemResumida::nova(
        "user",
        "Prove that the square root of 2 is irrational.",
    )];
    let decisao = decidir_esforco(
        &cliente,
        &mensagens,
        &ContextoDecisao::default(),
        cfg.min_confidence,
        Superficie::Chat,
        Some(&state.jev_contadores),
    )
    .await;
    // Um teste que "passa" com fail-open não provaria nada: aqui o erro de
    // rede/chave vira erro de verdade, com a mensagem do cliente.
    if decisao.origem == Origem::Padrao && decisao.custo.is_none() {
        return Err(decisao
            .motivo
            .unwrap_or_else(|| "o Jev não respondeu".to_string()));
    }
    Ok(decisao)
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
    let cliente = match cliente_jev(&state) {
        Ok(c) => c,
        Err(motivo) => return Ok(DecisaoChat::padrao(&default_effort, motivo)),
    };
    if !capacidade_do_modelo(&state, &model).pode_raciocinar() {
        return Ok(DecisaoChat::padrao(
            &default_effort,
            "o modelo não tem interruptor de raciocínio",
        ));
    }
    let decisao = decidir_esforco(
        &cliente,
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
            source: Origem::Jev,
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
pub(crate) async fn sincronizar_shim(_app: &tauri::AppHandle, state: &AppState) {
    let cfg = jev_config(state);
    let cliente = cliente_jev(state).ok();
    let upstream = {
        let guard = state.server.lock().await;
        guard
            .as_ref()
            .filter(|s| s.is_spawned())
            .map(|s| s.config().connect_url())
    };
    let deve_rodar = cfg.enabled && cfg.gate_harness && cliente.is_some() && upstream.is_some();

    let mut slot = state.decisor.lock().await;
    match (deve_rodar, slot.as_ref()) {
        (true, Some(atual)) => {
            // Já no ar: só atualiza cliente e limiar, sem derrubar conexões.
            atual.atualizar(cliente, cfg.min_confidence).await;
        }
        (true, None) => {
            let (models_dir, cache) = (state.models_dir.clone(), state.jev_capacidades.clone());
            let config = lr_decisor::ConfigShim {
                upstream: upstream.unwrap_or_default(),
                porta_preferida: lr_decisor::PORTA_PADRAO,
                jev: cliente,
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
