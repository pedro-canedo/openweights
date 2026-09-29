//! O catálogo de modelos que os agentes de código enxergam.
//!
//! O AgenticOw e o OwCLI pensam com os modelos do app — nunca com provedor
//! próprio. As fontes são as mesmas para os dois: o Servidor Local (o Router
//! do llama.cpp), os favoritos do OpenRouter e o 9router. Aqui elas saem num
//! formato neutro; cada agente converte para o seu (a seção `llm-pi-ai` do
//! AgenticOw, o `openweights.json` do OwCLI).
//!
//! Quem muda o que os agentes enxergam (motor subindo ou caindo, modelo
//! baixado ou apagado, Jev, OpenRouter, 9router) chama [`agendar`]: rajadas
//! viram um envio só, e o último vence.

use std::sync::atomic::Ordering;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::state::AppState;

/// Id da fonte do Servidor Local.
pub const LOCAL: &str = "openweights";
pub const OPENROUTER: &str = "openrouter";
pub const NINEROUTER: &str = "ninerouter";

/// Uma fonte de modelos, com o que um agente precisa para falar com ela.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fonte {
    pub id: &'static str,
    pub nome: &'static str,
    /// Base OpenAI-compatível, COM `/v1`.
    pub base_url: String,
    /// Chave da fonte; `None` quando ela não autentica.
    pub chave: Option<String>,
    pub modelos: Vec<ModeloDaFonte>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeloDaFonte {
    pub id: String,
    /// Janela de contexto; `None` quando a fonte não diz.
    pub janela: Option<u32>,
    /// Níveis de esforço que o chat template aceita. Vazio = só liga e desliga
    /// (ou modelo remoto, cujo raciocínio é do provedor).
    pub esforcos: Vec<String>,
    /// O raciocínio pode ser ligado e desligado por quem chama.
    pub raciocinio: bool,
}

/// O que cada fonte tem agora, sem ir à rede: é o que as telas dos agentes
/// mostram para a pessoa escolher o cérebro quando não há modelo nenhum.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EstadoDasFontes {
    /// Modelos GGUF na biblioteca.
    pub local_models: usize,
    /// O Servidor Local está no ar.
    pub server_running: bool,
    /// OpenRouter ligado e com chave.
    pub openrouter_key: bool,
    /// Favoritos do OpenRouter (só eles entram no catálogo).
    pub openrouter_favorites: usize,
    pub ninerouter_installed: bool,
    pub ninerouter_running: bool,
}

pub fn estado_das_fontes(state: &AppState) -> EstadoDasFontes {
    let cfg = crate::commands_providers::load_config(state);
    // PIDs, não os mutexes: uma partida do motor segura o dele enquanto espera
    // o /health, e o status das telas não pode ficar preso atrás disso.
    EstadoDasFontes {
        local_models: lr_models::scan_local(&state.models_dir).len(),
        server_running: state.server_pid.load(Ordering::SeqCst) != 0,
        openrouter_key: cfg.open_router.enabled && !cfg.open_router.api_key.trim().is_empty(),
        openrouter_favorites: cfg.open_router.favorites.len(),
        ninerouter_installed: cfg.nine_router.installed,
        ninerouter_running: state.ninerouter_pid.load(Ordering::SeqCst) != 0,
    }
}

/// Modelos do Router local: todos os que o servidor atende agora, sem as
/// entradas internas de visão (mesmo filtro do seletor do chat). Devolve a
/// raiz (sem `/v1`), a chave e os modelos.
async fn modelos_locais(
    state: &AppState,
) -> Result<(String, Option<String>, Vec<ModeloDaFonte>), String> {
    let cfg = {
        let guard = state.server.lock().await;
        match guard.as_ref() {
            Some(srv) if srv.is_spawned() => srv.config().clone(),
            _ => return Err("servidor não está rodando".to_string()),
        }
    };
    // Com o portão do Jev ligado para harnesses, a rota local passa pelo
    // proxy — é ele que decide o esforço por requisição. A chave é a mesma:
    // o proxy repassa o `Authorization`.
    let base = crate::commands_jev::base_local_para_harness(state)
        .await
        .unwrap_or_else(|| cfg.connect_url());
    let chave = cfg.api_key.clone();
    let ids: Vec<String> = lr_engine::LlamaServer::new(cfg)
        .models_status()
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|m| m.id)
        .filter(|id| !id.ends_with(crate::commands::VISION_SUFFIX))
        .collect();

    // Janela de contexto: perfil gravado > cabeçalho do GGUF > 32768. O
    // cabeçalho de cada modelo é lido UMA vez — dele saem a janela de treino e
    // o interruptor de raciocínio.
    let artefatos = lr_models::scan_local(&state.models_dir);
    let sem_gguf = |s: &str| {
        s.strip_suffix(".gguf")
            .or_else(|| s.strip_suffix(".GGUF"))
            .map(str::to_string)
            .unwrap_or_else(|| s.to_string())
    };
    let cabecalho = |id: &str| {
        artefatos
            .iter()
            .find(|a| a.name == id || sem_gguf(&a.name) == sem_gguf(id))
            .map(|a| lr_models::read_local_meta(&a.primary_path))
    };
    let modelos = ids
        .into_iter()
        .map(|id| {
            let meta = cabecalho(&id);
            let ctx = crate::commands::profile_for(state, &id)
                .and_then(|p| p.ctx)
                .or_else(|| meta.as_ref().and_then(|m| m.context_length))
                .unwrap_or(32_768);
            ModeloDaFonte {
                id,
                janela: Some(ctx),
                esforcos: meta
                    .as_ref()
                    .map(|m| m.reasoning_efforts.clone())
                    .unwrap_or_default(),
                raciocinio: meta.is_some_and(|m| m.thinking_toggle),
            }
        })
        .collect();
    Ok((base, chave, modelos))
}

/// As fontes que têm modelos agora.
///
/// A local sempre que o Router tem modelos; o OpenRouter só com o provedor
/// ligado, chave e favoritos; o 9router só instalado E rodando com catálogo.
/// Fonte sem modelos não entra. Lista vazia é válida: o agente abre do mesmo
/// jeito e mostra como configurar uma fonte.
pub async fn fontes(state: &AppState) -> Vec<Fonte> {
    let mut fontes = Vec::new();

    match modelos_locais(state).await {
        Ok((base, chave, modelos)) if !modelos.is_empty() => fontes.push(Fonte {
            id: LOCAL,
            nome: "OpenWeights (local)",
            base_url: format!("{base}/v1"),
            chave,
            modelos,
        }),
        Ok(_) => {}
        Err(e) => log::info!("catálogo sem a fonte local ({e}); seguindo com as remotas"),
    }

    let cfg = crate::commands_providers::load_config(state);

    let chave_or = cfg.open_router.api_key.trim().to_string();
    if cfg.open_router.enabled && !chave_or.is_empty() && !cfg.open_router.favorites.is_empty() {
        // Janela do catálogo em cache — sem ir à rede aqui.
        let cache = state.openrouter_cache.lock().await;
        let ctx_de = |id: &str| {
            cache
                .as_ref()
                .and_then(|(_, ms)| ms.iter().find(|m| m.id == id))
                .and_then(|m| m.context_length)
        };
        let modelos = cfg
            .open_router
            .favorites
            .iter()
            .map(|id| ModeloDaFonte {
                id: id.clone(),
                janela: ctx_de(id),
                esforcos: Vec::new(),
                raciocinio: false,
            })
            .collect();
        drop(cache);
        fontes.push(Fonte {
            id: OPENROUTER,
            nome: "OpenRouter",
            base_url: lr_providers::OPENROUTER_BASE_URL.to_string(),
            chave: Some(chave_or),
            modelos,
        });
    }

    let nove_rodando = state.ninerouter.lock().await.is_some();
    state.ninerouter_sem_chave.store(false, Ordering::SeqCst);
    if cfg.nine_router.installed && nove_rodando {
        let porta = cfg.nine_router.port;
        let modelos9 = lr_ninerouter::listar_modelos(porta)
            .await
            .unwrap_or_default();
        if !modelos9.is_empty() {
            let mut chave9 = cfg.nine_router.api_key.trim().to_string();
            if chave9.is_empty() {
                let l9 = lr_ninerouter::Layout::new(&state.data_dir.join("providers"));
                match lr_ninerouter::garantir_api_key(&l9, porta).await {
                    Ok(chave) => {
                        let mut cfg2 = crate::commands_providers::load_config(state);
                        cfg2.nine_router.api_key = chave.clone();
                        let _ = state
                            .store
                            .set_setting(crate::commands_providers::SETTING, &cfg2.to_json());
                        chave9 = chave;
                    }
                    Err(e) => log::warn!("9router sem chave de API utilizável: {e}"),
                }
            }
            // Sem chave o 9router responde 401 ("Missing API key") a todo
            // pedido, e uma fonte que só falha é pior do que fonte nenhuma. A
            // chave nasce com o primeiro boot do 9router, então esta é uma
            // espera: o catálogo é refeito daqui a pouco (`agendar`).
            if chave9.is_empty() {
                state.ninerouter_sem_chave.store(true, Ordering::SeqCst);
                return fontes;
            }
            fontes.push(Fonte {
                id: NINEROUTER,
                nome: "9Router",
                base_url: format!("http://127.0.0.1:{porta}/v1"),
                chave: Some(chave9),
                modelos: modelos9
                    .into_iter()
                    .map(|m| ModeloDaFonte {
                        id: m.id,
                        janela: m.context_length,
                        esforcos: Vec::new(),
                        raciocinio: false,
                    })
                    .collect(),
            });
        }
    }
    fontes
}

/// Pede um catálogo novo aos agentes. Rajadas viram UM envio (a última
/// vence), 400 ms depois do último pedido.
pub fn agendar(app: &AppHandle) {
    let state = app.state::<AppState>();
    let geracao = state.agenticow_agendado.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let state = app.state::<AppState>();
        if state.agenticow_agendado.load(Ordering::SeqCst) != geracao {
            return;
        }
        let agenticow_de_pe = state
            .agenticow
            .lock()
            .await
            .as_ref()
            .is_some_and(|h| h.url().is_some());
        if agenticow_de_pe {
            crate::commands_agenticow::empurrar_catalogo(&state).await;
        }
        // O OwCLI: rotas do gateway e o openweights.json (só se já foi usado).
        crate::commands_owcli::sincronizar(&app).await;

        // O 9router acabou de subir e ainda não tem a chave que o app usa: tenta
        // de novo daqui a pouco, algumas vezes, em vez de deixar a fonte de fora
        // até a próxima mudança.
        if state.ninerouter_sem_chave.load(Ordering::SeqCst) {
            let tentativas = state
                .ninerouter_espera_da_chave
                .fetch_add(1, Ordering::SeqCst)
                + 1;
            if let Some(espera) = espera_pela_chave(tentativas) {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(espera).await;
                    agendar(&app);
                });
            }
        } else {
            state.ninerouter_espera_da_chave.store(0, Ordering::SeqCst);
        }
    });
}

/// Quanto esperar para tentar de novo a chave do 9router, dada a quantidade de
/// tentativas sem ela. Zero tentativas: a chave veio (ou não há 9router). Depois
/// de `TENTATIVAS_DA_CHAVE` o app desiste até a próxima mudança de fontes.
const TENTATIVAS_DA_CHAVE: u32 = 8;

fn espera_pela_chave(tentativas: u32) -> Option<std::time::Duration> {
    (1..=TENTATIVAS_DA_CHAVE)
        .contains(&tentativas)
        .then(|| std::time::Duration::from_secs(u64::from(tentativas.min(4)) * 3))
}

#[cfg(test)]
mod testes_da_chave {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_chave_do_9router_e_esperada_com_pausa_crescente_e_por_pouco_tempo() {
        assert_eq!(espera_pela_chave(0), None, "sem espera: a chave veio");
        assert_eq!(espera_pela_chave(1), Some(Duration::from_secs(3)));
        assert_eq!(espera_pela_chave(4), Some(Duration::from_secs(12)));
        assert_eq!(
            espera_pela_chave(8),
            Some(Duration::from_secs(12)),
            "a pausa para de crescer"
        );
        assert_eq!(espera_pela_chave(9), None, "depois de oito, desiste");
    }
}
