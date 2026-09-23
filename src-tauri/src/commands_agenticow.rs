//! O AgenticOw no app: o fork do DeepSeek Harness (pedro-canedo/agenticow)
//! rodando como runtime pré-compilado e aparecendo DENTRO da janela principal.
//!
//! - Instalação: o pacote do alvo desta máquina, com sha256 e tamanho
//!   embutidos no binário (`lr_agenticow::pins`), sem npm install nenhum.
//! - Processo: o Node portátil do app roda o Host; o protocolo de controle
//!   (stdin/stdout) leva o idioma e o catálogo de modelos e traz o `ready`.
//! - Interface: uma webview FILHA da janela principal navega para a URL
//!   autenticada — primeira parte, então o cookie `SameSite=Strict` do Host
//!   vale (um iframe cross-site nunca o receberia). A webview não tem IPC do
//!   Tauri: a URL é remota e nenhuma capability libera `remote`.
//! - Catálogo: os modelos do Servidor Local (pelo proxy do Jev quando ele está
//!   no ar), os favoritos do OpenRouter e o 9router, empurrados de novo sempre
//!   que um deles muda — sem reiniciar nada. As chaves vão só na memória do Host.
//!
//! A URL com o token de lançamento nunca sai daqui: nem para evento, nem para log.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use lr_agenticow::catalog::{
    Modelo, NINEROUTER_KEY_ENV, OPENROUTER_KEY_ENV, OPENWEIGHTS_KEY_ENV, Rota,
};
use lr_agenticow::{AgenticowHost, Comando, EventoDoHost, Layout};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::state::AppState;

type CmdResult<T> = Result<T, String>;

/// Canal de eventos da tela do AgenticOw.
const EVENTO: &str = "agenticow";

/// Rótulo da webview filha que mostra a UI do AgenticOw.
pub const WEBVIEW: &str = "agenticow";

/// Onde a porta estável fica guardada.
const SETTING: &str = "agenticow.config";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Configuracao {
    porta: Option<u16>,
}

fn carregar_configuracao(state: &AppState) -> Configuracao {
    state
        .store
        .get_setting(SETTING)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn salvar_configuracao(state: &AppState, cfg: &Configuracao) {
    if let Ok(texto) = serde_json::to_string(cfg) {
        let _ = state.store.set_setting(SETTING, &texto);
    }
}

pub(crate) fn layout(state: &AppState) -> Layout {
    Layout::new(&state.data_dir)
}

/// O que a tela mostra.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgenticowStatus {
    /// Há pacote do AgenticOw para esta máquina.
    pub supported: bool,
    pub installed: bool,
    pub running: bool,
    pub ready: bool,
    pub port: Option<u16>,
    /// Tag da release do runtime que este app fixa.
    pub tag: String,
    /// Revisão do fork que este app fixa.
    pub revision: String,
    /// Tag do upstream (DeepSeek Harness) em que a revisão se baseia, quando o
    /// Host já saudou.
    pub upstream_tag: Option<String>,
    pub last_error: Option<String>,
}

/// Eventos da tela.
#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum EventoAgenticow {
    /// Fase da preparação: `node`, `download`, `verifying`, `extracting`, `migrating`, `starting`.
    Phase {
        phase: String,
    },
    Progress {
        received_bytes: u64,
        total_bytes: u64,
    },
    Ready,
    Failed {
        message: String,
    },
    Stopped,
    Log {
        line: String,
    },
    Catalog {
        revision: Option<u64>,
        ok: bool,
        message: Option<String>,
    },
}

fn emitir(app: &AppHandle, ev: EventoAgenticow) {
    let _ = app.emit(EVENTO, &ev);
}

fn registrar_erro(state: &AppState, erro: Option<String>) {
    *state
        .agenticow_erro
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = erro;
}

async fn status_atual(state: &AppState) -> AgenticowStatus {
    let l = layout(state);
    let mut guard = state.agenticow.lock().await;
    if guard.as_mut().is_some_and(|h| h.morreu()) {
        guard.take();
        state.agenticow_pid.store(0, Ordering::SeqCst);
    }
    let pins = lr_agenticow::pins::pins();
    let saudacao = state
        .agenticow_upstream
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    AgenticowStatus {
        supported: lr_agenticow::pins::asset_atual().is_some(),
        installed: l.instalado(),
        running: guard.is_some(),
        ready: guard.as_ref().is_some_and(|h| h.url().is_some()),
        port: guard.as_ref().and_then(|h| h.porta()),
        tag: pins.tag.clone(),
        revision: pins.revision.clone(),
        upstream_tag: saudacao,
        last_error: state
            .agenticow_erro
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone(),
    }
}

#[tauri::command]
pub async fn agenticow_status(state: State<'_, AppState>) -> CmdResult<AgenticowStatus> {
    Ok(status_atual(&state).await)
}

// ------------------------------------------------------------- catálogo ---

/// Modelos do Router local: todos os que o servidor atende agora, sem as
/// entradas internas de visão (mesmo filtro do seletor do chat).
async fn modelos_locais(state: &AppState) -> Result<(String, Option<String>, Vec<Modelo>), String> {
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
            Modelo {
                name: id.clone(),
                id,
                context_window: Some(ctx),
                max_tokens: Some(lr_agenticow::catalog::teto_de_saida(ctx)),
                efforts: meta
                    .as_ref()
                    .map(|m| m.reasoning_efforts.clone())
                    .unwrap_or_default(),
                thinking: meta.is_some_and(|m| m.thinking_toggle),
            }
        })
        .collect();
    Ok((base, chave, modelos))
}

/// As rotas e as chaves do catálogo.
///
/// `openweights` sempre que o Router tem modelos; `openrouter` só com o
/// provedor ligado, chave e favoritos; `ninerouter` só instalado E rodando
/// com catálogo. Rota sem modelos não entra. Catálogo VAZIO é válido: o
/// AgenticOw abre do mesmo jeito e mostra como configurar um provedor.
async fn montar_catalogo(state: &AppState) -> (Vec<(String, Rota)>, BTreeMap<String, String>) {
    let mut rotas = Vec::new();
    let mut chaves = BTreeMap::new();

    match modelos_locais(state).await {
        Ok((base, chave, modelos)) if !modelos.is_empty() => {
            rotas.push((
                "openweights".to_string(),
                Rota {
                    display_name: "OpenWeights (local)".to_string(),
                    base_url: format!("{base}/v1"),
                    api_key_env: OPENWEIGHTS_KEY_ENV.to_string(),
                    models: modelos,
                },
            ));
            // O adaptador exige credencial mesmo de endpoint que não
            // autentica: sem chave real, o dummy consagrado `local`.
            chaves.insert(
                OPENWEIGHTS_KEY_ENV.to_string(),
                chave.unwrap_or_else(|| "local".to_string()),
            );
        }
        Ok(_) => {}
        Err(e) => log::info!("AgenticOw sem a rota local ({e}); seguindo com as remotas"),
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
            .map(|id| Modelo {
                id: id.clone(),
                name: id.clone(),
                context_window: ctx_de(id),
                // O teto de saída e o formato de raciocínio de um modelo
                // remoto são do provedor.
                max_tokens: None,
                efforts: Vec::new(),
                thinking: false,
            })
            .collect();
        drop(cache);
        rotas.push((
            "openrouter".to_string(),
            Rota {
                display_name: "OpenRouter".to_string(),
                base_url: lr_providers::OPENROUTER_BASE_URL.to_string(),
                api_key_env: OPENROUTER_KEY_ENV.to_string(),
                models: modelos,
            },
        ));
        chaves.insert(OPENROUTER_KEY_ENV.to_string(), chave_or);
    }

    let nove_rodando = state.ninerouter.lock().await.is_some();
    if cfg.nine_router.installed && nove_rodando {
        let porta = cfg.nine_router.port;
        let modelos9 = lr_ninerouter::listar_modelos(porta)
            .await
            .unwrap_or_default();
        if !modelos9.is_empty() {
            rotas.push((
                "ninerouter".to_string(),
                Rota {
                    display_name: "9Router".to_string(),
                    base_url: format!("http://127.0.0.1:{porta}/v1"),
                    api_key_env: NINEROUTER_KEY_ENV.to_string(),
                    models: modelos9
                        .into_iter()
                        .map(|m| Modelo {
                            name: m.id.clone(),
                            id: m.id,
                            context_window: m.context_length,
                            max_tokens: None,
                            efforts: Vec::new(),
                            thinking: false,
                        })
                        .collect(),
                },
            ));
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
            if !chave9.is_empty() {
                chaves.insert(NINEROUTER_KEY_ENV.to_string(), chave9);
            }
        }
    }
    (rotas, chaves)
}

/// Monta e manda o catálogo ao Host, se ele estiver de pé.
async fn empurrar_catalogo(state: &AppState) {
    let (rotas, chaves) = montar_catalogo(state).await;
    let revisao = state.agenticow_catalogo.fetch_add(1, Ordering::SeqCst) + 1;
    let comando = Comando::Catalog {
        revision: revisao,
        pi_ai: lr_agenticow::catalog::secao_llm_pi_ai(&rotas),
        env: chaves,
    };
    let guard = state.agenticow.lock().await;
    if let Some(host) = guard.as_ref()
        && let Err(e) = host.enviar(&comando).await
    {
        log::warn!("catálogo não chegou ao AgenticOw: {e}");
    }
}

/// Pede um novo catálogo. Chamado por quem muda o que o AgenticOw enxerga:
/// motor subindo ou caindo, modelo baixado ou apagado, Jev, OpenRouter,
/// 9router. Rajadas viram UM envio (a última vence).
pub(crate) fn agendar_catalogo(app: &AppHandle) {
    let state = app.state::<AppState>();
    let geracao = state.agenticow_agendado.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let state = app.state::<AppState>();
        if state.agenticow_agendado.load(Ordering::SeqCst) != geracao {
            return;
        }
        let de_pe = state
            .agenticow
            .lock()
            .await
            .as_ref()
            .is_some_and(|h| h.url().is_some());
        if de_pe {
            empurrar_catalogo(&state).await;
        }
    });
}

#[tauri::command]
pub async fn agenticow_refresh_catalog(state: State<'_, AppState>) -> CmdResult<()> {
    empurrar_catalogo(&state).await;
    Ok(())
}

#[tauri::command]
pub async fn agenticow_set_locale(state: State<'_, AppState>, locale: String) -> CmdResult<()> {
    let locale = normalizar_idioma(&locale);
    *state
        .agenticow_idioma
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(locale.clone());
    let guard = state.agenticow.lock().await;
    if let Some(host) = guard.as_ref() {
        host.enviar(&Comando::Locale(locale))
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// O app fala pt-BR e en; o AgenticOw também.
fn normalizar_idioma(locale: &str) -> String {
    if locale.to_ascii_lowercase().starts_with("en") {
        "en".into()
    } else {
        "pt-BR".into()
    }
}

// ------------------------------------------------------------ ciclo ---

fn ouvinte(app: &AppHandle) -> lr_agenticow::OuvinteDoHost {
    let app = app.clone();
    Arc::new(move |ev| match ev {
        EventoDoHost::Log { linha } => {
            log::debug!("[agenticow] {linha}");
            emitir(&app, EventoAgenticow::Log { line: linha });
        }
        EventoDoHost::Saudou {
            revisao,
            upstream_tag,
            dsh,
        } => {
            log::info!("AgenticOw {revisao} (upstream {upstream_tag}, dsh {dsh})");
            let state = app.state::<AppState>();
            *state
                .agenticow_upstream
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(upstream_tag);
        }
        EventoDoHost::Pronto { .. } => {}
        EventoDoHost::Fatal { mensagem } => {
            log::warn!("AgenticOw: {mensagem}");
        }
        EventoDoHost::CatalogoAplicado { revisao } => {
            emitir(
                &app,
                EventoAgenticow::Catalog {
                    revision: revisao,
                    ok: true,
                    message: None,
                },
            );
        }
        EventoDoHost::CatalogoRecusado { revisao, mensagem } => {
            log::warn!("AgenticOw recusou o catálogo {revisao:?}: {mensagem}");
            emitir(
                &app,
                EventoAgenticow::Catalog {
                    revision: revisao,
                    ok: false,
                    message: Some(mensagem),
                },
            );
        }
    })
}

/// Sobe o AgenticOw de ponta a ponta: Node portátil, migração do home da era
/// do DeepSeek Harness, runtime verificado, Host, idioma e catálogo.
pub(crate) async fn iniciar(
    app: &AppHandle,
    state: &AppState,
    locale: Option<String>,
) -> CmdResult<AgenticowStatus> {
    let _op = state.agenticow_operacao.lock().await;
    if let Some(l) = locale.as_deref() {
        *state
            .agenticow_idioma
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(normalizar_idioma(l));
    }
    if state
        .agenticow
        .lock()
        .await
        .as_ref()
        .is_some_and(|h| h.url().is_some())
    {
        return Ok(status_atual(state).await);
    }
    registrar_erro(state, None);
    match subir(app, state).await {
        Ok(()) => {
            emitir(app, EventoAgenticow::Ready);
            Ok(status_atual(state).await)
        }
        Err(e) => {
            registrar_erro(state, Some(e.clone()));
            emitir(app, EventoAgenticow::Failed { message: e.clone() });
            Err(e)
        }
    }
}

async fn subir(app: &AppHandle, state: &AppState) -> Result<(), String> {
    if lr_agenticow::pins::asset_atual().is_none() {
        return Err("agenticow-unsupported".into());
    }

    emitir(
        app,
        EventoAgenticow::Phase {
            phase: "node".into(),
        },
    );
    // Os eventos do Node portátil viram os da tela: o `ready` dele é "Node
    // instalado", não "AgenticOw pronto", e a falha já volta pelo `?`.
    let app_node = app.clone();
    state
        .node
        .ensure(move |ev| match ev {
            lr_nodejs::NodeEvent::Progress {
                received_bytes,
                total_bytes,
                ..
            } => emitir(
                &app_node,
                EventoAgenticow::Progress {
                    received_bytes,
                    total_bytes,
                },
            ),
            lr_nodejs::NodeEvent::Extracting { .. } => emitir(
                &app_node,
                EventoAgenticow::Phase {
                    phase: "node".into(),
                },
            ),
            lr_nodejs::NodeEvent::Ready | lr_nodejs::NodeEvent::Failed { .. } => {}
        })
        .await
        .map_err(|e| e.to_string())?;
    let node_exe = state
        .node
        .node_exe()
        .ok_or("o Node portátil não está instalado")?;

    // O home da era do DeepSeek Harness vem por cópia, uma vez.
    let home = lr_agenticow::home(&state.data_dir);
    if !home.exists() {
        emitir(
            app,
            EventoAgenticow::Phase {
                phase: "migrating".into(),
            },
        );
        let origem = state.data_dir.join("dsh-home");
        let destino = home.clone();
        match tokio::task::spawn_blocking(move || lr_agenticow::migrate::migrar(&origem, &destino))
            .await
        {
            Ok(Ok(m)) => log::info!("AgenticOw: migração do home antigo: {m:?}"),
            Ok(Err(e)) => {
                log::warn!("AgenticOw: o home antigo não foi copiado ({e}); seguindo com um novo")
            }
            Err(e) => log::warn!("AgenticOw: a migração do home não terminou ({e})"),
        }
    }

    let l = layout(state);
    if !l.instalado() {
        let app_ev = app.clone();
        let on_event = move |ev: lr_agenticow::EventoDeInstalacao| {
            use lr_agenticow::EventoDeInstalacao as E;
            let ev = match ev {
                E::Progress {
                    received_bytes,
                    total_bytes,
                } => EventoAgenticow::Progress {
                    received_bytes,
                    total_bytes,
                },
                E::Verifying => EventoAgenticow::Phase {
                    phase: "verifying".into(),
                },
                E::Extracting => EventoAgenticow::Phase {
                    phase: "extracting".into(),
                },
                E::Installed => EventoAgenticow::Phase {
                    phase: "installed".into(),
                },
            };
            emitir(&app_ev, ev);
        };
        emitir(
            app,
            EventoAgenticow::Phase {
                phase: "download".into(),
            },
        );
        lr_agenticow::install::instalar(&l, &on_event)
            .await
            .map_err(|e| e.to_string())?;
    }

    emitir(
        app,
        EventoAgenticow::Phase {
            phase: "starting".into(),
        },
    );
    let mut cfg = carregar_configuracao(state);
    let preferida = cfg.porta.unwrap_or(lr_agenticow::PORTA_PREFERIDA);
    // Duas tentativas: a porta escolhida pode ser tomada entre a checagem e o
    // bind, e `EADDRINUSE` derruba o Host.
    let mut ultima = String::new();
    for tentativa in 0..2 {
        let porta = if tentativa == 0 {
            lr_proc::free_port(preferida)
        } else {
            lr_proc::free_port(0)
        };
        let config = lr_agenticow::Config {
            node_exe: node_exe.clone(),
            node_env: state.node.env_isolado(&l.atual()),
            runtime_dir: l.atual(),
            home: home.clone(),
            porta,
        };
        let mut host = AgenticowHost::spawn(&config, ouvinte(app)).map_err(|e| e.to_string())?;
        if let Some(pid) = host.pid() {
            state.agenticow_pid.store(pid, Ordering::SeqCst);
        }
        match host.aguardar_pronto(lr_agenticow::PRAZO_PARA_SUBIR).await {
            Ok((_, porta_real)) => {
                if cfg.porta != Some(porta_real) {
                    cfg.porta = Some(porta_real);
                    salvar_configuracao(state, &cfg);
                }
                let idioma = state
                    .agenticow_idioma
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
                    .unwrap_or_else(|| "pt-BR".into());
                if let Err(e) = host.enviar(&Comando::Locale(idioma)).await {
                    log::warn!("idioma não chegou ao AgenticOw: {e}");
                }
                *state.agenticow.lock().await = Some(host);
                empurrar_catalogo(state).await;
                // Um boot bom: agora sim as versões antigas podem sair.
                let l2 = l.clone();
                tokio::task::spawn_blocking(move || lr_agenticow::install::podar(&l2));
                return Ok(());
            }
            Err(e) => {
                ultima = e.to_string();
                host.parar().await;
                state.agenticow_pid.store(0, Ordering::SeqCst);
                log::warn!("AgenticOw não subiu na porta {porta}: {ultima}");
            }
        }
    }
    Err(ultima)
}

#[tauri::command]
pub async fn agenticow_start(
    app: AppHandle,
    state: State<'_, AppState>,
    locale: Option<String>,
) -> CmdResult<AgenticowStatus> {
    iniciar(&app, &state, locale).await
}

async fn parar(app: &AppHandle, state: &AppState) {
    fechar_webview(app);
    let host = state.agenticow.lock().await.take();
    if let Some(mut host) = host {
        host.parar().await;
    }
    state.agenticow_pid.store(0, Ordering::SeqCst);
    emitir(app, EventoAgenticow::Stopped);
}

#[tauri::command]
pub async fn agenticow_stop(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<AgenticowStatus> {
    let _op = state.agenticow_operacao.lock().await;
    parar(&app, &state).await;
    Ok(status_atual(&state).await)
}

/// Remove o runtime (e, com `remove_data`, o home: sessões e configurações).
/// O home da era do DeepSeek Harness não é tocado aqui.
#[tauri::command]
pub async fn agenticow_uninstall(
    app: AppHandle,
    state: State<'_, AppState>,
    remove_data: bool,
) -> CmdResult<AgenticowStatus> {
    let _op = state.agenticow_operacao.lock().await;
    parar(&app, &state).await;
    let raiz = layout(&state).raiz().to_path_buf();
    let home = lr_agenticow::home(&state.data_dir);
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        if raiz.exists() {
            lr_fetch::remove_dir_all_retrying(&raiz)?;
        }
        if remove_data && home.exists() {
            lr_fetch::remove_dir_all_retrying(&home)?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    Ok(status_atual(&state).await)
}

// ------------------------------------------------------------ webview ---

/// Retângulo da área de conteúdo, em pixels lógicos (os do CSS).
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Area {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

fn aplicar_area(webview: &tauri::Webview, area: Area) -> tauri::Result<()> {
    crate::janela::posicionar(webview, area.x, area.y, area.width, area.height)
}

/// Mostra a UI do AgenticOw sobre a área de conteúdo. A webview nasce na
/// primeira vez e depois só é mostrada e escondida — nunca recriada ao trocar
/// de tela, então a sessão aberta continua viva.
#[tauri::command]
pub async fn agenticow_show(
    app: AppHandle,
    state: State<'_, AppState>,
    area: Area,
) -> CmdResult<()> {
    let url = {
        let guard = state.agenticow.lock().await;
        guard
            .as_ref()
            .and_then(|h| h.url())
            .ok_or("o AgenticOw ainda não está pronto")?
    };
    let destino = tauri::Url::parse(&url).map_err(|e| e.to_string())?;
    if let Some(webview) = app.get_webview(WEBVIEW) {
        // Host reiniciado = token novo: a mesma webview navega para a URL nova.
        let atual = webview.url().ok();
        let mesma_origem = atual
            .as_ref()
            .is_some_and(|u| u.port() == destino.port() && u.host_str() == destino.host_str());
        if !mesma_origem {
            webview.navigate(destino).map_err(|e| e.to_string())?;
        }
        aplicar_area(&webview, area).map_err(|e| e.to_string())?;
        webview.show().map_err(|e| e.to_string())?;
        return Ok(());
    }
    let janela = app
        .get_window(crate::janela::JANELA)
        .ok_or("janela principal ausente")?;
    let webview = janela
        .add_child(
            tauri::webview::WebviewBuilder::new(WEBVIEW, tauri::WebviewUrl::External(destino)),
            tauri::LogicalPosition::new(area.x, area.y),
            tauri::LogicalSize::new(area.width.max(1.0), area.height.max(1.0)),
        )
        .map_err(|e| e.to_string())?;
    // No Linux é aqui que ela sai da caixa da janela para a sobreposição.
    aplicar_area(&webview, area).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn agenticow_hide(app: AppHandle) -> CmdResult<()> {
    if let Some(webview) = app.get_webview(WEBVIEW) {
        webview.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn agenticow_set_bounds(app: AppHandle, area: Area) -> CmdResult<()> {
    if let Some(webview) = app.get_webview(WEBVIEW) {
        aplicar_area(&webview, area).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(crate) fn fechar_webview(app: &AppHandle) {
    if let Some(webview) = app.get_webview(WEBVIEW) {
        let _ = webview.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idioma_do_app_vira_idioma_do_agenticow() {
        assert_eq!(normalizar_idioma("pt-BR"), "pt-BR");
        assert_eq!(normalizar_idioma("en"), "en");
        assert_eq!(normalizar_idioma("en-US"), "en");
        assert_eq!(
            normalizar_idioma("es"),
            "pt-BR",
            "o que não é inglês cai no padrão do app"
        );
    }

    #[test]
    fn os_eventos_chegam_a_tela_em_camel_case() {
        let v = serde_json::to_value(EventoAgenticow::Progress {
            received_bytes: 1,
            total_bytes: 2,
        })
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({ "kind": "progress", "receivedBytes": 1, "totalBytes": 2 })
        );
        let v = serde_json::to_value(EventoAgenticow::Catalog {
            revision: Some(3),
            ok: false,
            message: Some("x".into()),
        })
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({ "kind": "catalog", "revision": 3, "ok": false, "message": "x" })
        );
    }

    #[test]
    fn a_area_vem_do_css() {
        let a: Area =
            serde_json::from_str(r#"{"x":240,"y":0,"width":1040.5,"height":820}"#).unwrap();
        assert_eq!((a.x, a.y, a.width, a.height), (240.0, 0.0, 1040.5, 820.0));
    }
}
