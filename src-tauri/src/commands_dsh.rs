//! Ciclo de vida gerenciado do DeepSeek Harness (dsh), no molde do 9router.
//!
//! "Abrir o agente" vira UM clique: garante Node portátil + pacote pinado
//! (instalando com progresso na primeira vez), sobe o servidor local se
//! preciso, escreve o `settings.yaml` multi-provider (cirurgicamente — ver
//! `lr_dshhost::settings`), spawna `dsh web --port 0 --no-open`, lê a porta
//! real do stdout e abre o painel numa janela do app.
//!
//! A chave de API NUNCA entra em arquivo: o yaml referencia nomes de envs
//! (`apiKeyEnv`) e os valores vão só no ambiente do processo filho. Quando o
//! servidor local não tem chave, entra o dummy `local` — o adapter do dsh
//! exige credencial mesmo para um endpoint que não autentica.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::state::AppState;
use lr_dshhost::settings::{
    ModeloDsh, NINEROUTER_KEY_ENV, OPENROUTER_KEY_ENV, OPENWEIGHTS_KEY_ENV, ProvedorDsh,
};
use lr_dshhost::{DshEvent, DshHost, Layout, RunEnv};

type CmdResult<T> = Result<T, String>;

fn err_str<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Mesmo canal de progresso/log dos outros provedores gerenciados.
const EVENTO: &str = "provider";

/// Rótulo da janela do painel do dsh. Como a do 9router, NÃO está na
/// capability `default`: carrega uma aplicação inteira servida pelo processo
/// filho e não tem por que enxergar IPC.
const JANELA_PAINEL: &str = "dsh-panel";

/// Situação do dsh para a tela desenhar.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DshStatus {
    pub node_installed: bool,
    pub installed: bool,
    pub running: bool,
    /// Porta real (efêmera, escolhida pelo SO a cada subida).
    pub port: Option<u16>,
    /// URL do painel, quando está no ar.
    pub panel_url: Option<String>,
    /// Versão pinada quando instalado; vazio quando não.
    pub version: String,
}

pub(crate) fn layout(state: &AppState) -> Layout {
    Layout::new(&state.data_dir.join("providers"))
}

pub(crate) fn dsh_home(state: &AppState) -> PathBuf {
    state.data_dir.join("dsh-home")
}

async fn status_atual(state: &AppState) -> DshStatus {
    let l = layout(state);
    let instalado = l.instalado();
    let mut guard = state.dsh.lock().await;
    // Um processo que morreu sozinho não pode continuar contando como "no
    // ar": a tela decide o layout inteiro por isto, e ficaria com um quadro
    // apontado para uma porta morta em vez do botão de subir de novo.
    if guard.as_mut().is_some_and(|d| d.morreu()) {
        guard.take();
        state.dsh_pid.store(0, Ordering::SeqCst);
    }
    let port = guard.as_ref().and_then(|d| d.porta());
    DshStatus {
        node_installed: state.node.state().installed,
        installed: instalado,
        running: guard.is_some(),
        port,
        panel_url: port.map(|p| format!("http://127.0.0.1:{p}")),
        version: if instalado {
            lr_dshhost::PINNED_DSH.to_string()
        } else {
            String::new()
        },
    }
}

#[tauri::command]
pub async fn dsh_status(state: State<'_, AppState>) -> CmdResult<DshStatus> {
    Ok(status_atual(&state).await)
}

/// Node portátil + pacote do dsh, ambos com eventos no canal `provider` —
/// para quem olha a tela é uma instalação só.
async fn instalar_dsh(app: &AppHandle, state: &AppState) -> CmdResult<()> {
    let app_node = app.clone();
    state
        .node
        .ensure(move |ev| {
            let _ = app_node.emit(EVENTO, &ev);
        })
        .await
        .map_err(err_str)?;

    let l = layout(state);
    let app_npm = app.clone();
    let emitir = move |ev: DshEvent| {
        let _ = app_npm.emit(EVENTO, &ev);
    };
    lr_dshhost::instalar(&state.node, &l, &emitir)
        .await
        .map_err(err_str)
}

#[tauri::command]
pub async fn dsh_install(app: AppHandle, state: State<'_, AppState>) -> CmdResult<DshStatus> {
    instalar_dsh(&app, &state).await?;
    let _ = app.emit(EVENTO, &DshEvent::Ready);
    Ok(status_atual(&state).await)
}

/// Modelos do Router local: TODOS os que o servidor atende agora, sem as
/// entradas internas de visão (mesmo filtro do seletor do chat) — no dsh
/// elas seriam duplicatas confusas do mesmo arquivo.
async fn modelos_locais(state: &AppState) -> CmdResult<(String, Option<String>, Vec<ModeloDsh>)> {
    let cfg = {
        let guard = state.server.lock().await;
        match guard.as_ref() {
            Some(srv) if srv.is_spawned() => srv.config().clone(),
            _ => return Err("servidor não está rodando".to_string()),
        }
    };
    let base = cfg.connect_url();
    let chave = cfg.api_key.clone();
    let ids: Vec<String> = lr_engine::LlamaServer::new(cfg)
        .models_status()
        .await
        .map_err(err_str)?
        .into_iter()
        .map(|m| m.id)
        .filter(|id| !id.ends_with(crate::commands::VISION_SUFFIX))
        .collect();

    // Janela de contexto: perfil gravado > cabeçalho do GGUF > 32768. O scan
    // é um só; o cabeçalho só é lido para quem não tem perfil.
    let artefatos = lr_models::scan_local(&state.models_dir);
    let sem_gguf = |s: &str| {
        s.strip_suffix(".gguf")
            .or_else(|| s.strip_suffix(".GGUF"))
            .map(str::to_string)
            .unwrap_or_else(|| s.to_string())
    };
    let ctx_do_cabecalho = |id: &str| {
        artefatos
            .iter()
            .find(|a| a.name == id || sem_gguf(&a.name) == sem_gguf(id))
            .and_then(|a| lr_models::read_local_meta(&a.primary_path).context_length)
    };

    let modelos = ids
        .into_iter()
        .map(|id| {
            let ctx = crate::commands::profile_for(state, &id)
                .and_then(|p| p.ctx)
                .or_else(|| ctx_do_cabecalho(&id))
                .unwrap_or(32_768);
            ModeloDsh {
                name: id.clone(),
                id,
                context_window: Some(ctx),
            }
        })
        .collect();
    Ok((base, chave, modelos))
}

/// Monta o dict de provedores do yaml e as envs de chave do processo.
///
/// Regras (Parte C do design): `openweights` sempre que o Router tem modelos;
/// `openrouter` só com o provedor ligado, chave e favoritos; `ninerouter` só
/// instalado E rodando com catálogo não-vazio. Rota com `models` vazio é
/// inválida no schema do dsh — melhor omitir a rota que quebrar a seção.
async fn montar_provedores(
    state: &AppState,
) -> CmdResult<(Vec<(String, ProvedorDsh)>, Vec<(String, String)>)> {
    let mut provedores = Vec::new();
    let mut envs = Vec::new();

    // --- openweights (local) ---
    //
    // Sem servidor local o harness não morre: ele ainda vale pelos provedores
    // remotos. Só um harness SEM NENHUMA rota é que não faz sentido — esse
    // caso vira erro no fim da função.
    let (base_local, chave_local, modelos) = match modelos_locais(state).await {
        Ok(v) => v,
        Err(e) => {
            log::warn!("dsh sem o provedor local ({e}); seguindo com os remotos");
            (String::new(), None, Vec::new())
        }
    };
    if !modelos.is_empty() {
        provedores.push((
            "openweights".to_string(),
            ProvedorDsh {
                display_name: "OpenWeights (local)".to_string(),
                base_url: format!("{base_local}/v1"),
                api_key_env: OPENWEIGHTS_KEY_ENV.to_string(),
                models: modelos,
            },
        ));
        // `apiKeyEnv` é SEMPRE declarado, então a env sempre existe: a chave
        // real quando há, o dummy consagrado `local` quando não — o adapter
        // do dsh exige credencial mesmo de endpoint que não autentica.
        envs.push((
            OPENWEIGHTS_KEY_ENV.to_string(),
            chave_local.unwrap_or_else(|| "local".to_string()),
        ));
    }

    let cfg = crate::commands_providers::load_config(state);

    // --- openrouter (remoto, opcional) ---
    let chave_or = cfg.open_router.api_key.trim().to_string();
    if cfg.open_router.enabled && !chave_or.is_empty() && !cfg.open_router.favorites.is_empty() {
        // Contexto do catálogo em cache quando houver — sem rebuscar a rede
        // aqui: o launch não pode depender do openrouter.ai responder.
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
            .map(|id| ModeloDsh {
                id: id.clone(),
                name: id.clone(),
                context_window: ctx_de(id),
            })
            .collect();
        drop(cache);
        provedores.push((
            "openrouter".to_string(),
            ProvedorDsh {
                display_name: "OpenRouter".to_string(),
                // COM /v1: é o contrato do adapter do dsh (o interno
                // OPENROUTER_API_ROOT é a raiz sem /v1, de quem monta o
                // caminho sozinho).
                base_url: lr_providers::OPENROUTER_BASE_URL.to_string(),
                api_key_env: OPENROUTER_KEY_ENV.to_string(),
                models: modelos,
            },
        ));
        envs.push((OPENROUTER_KEY_ENV.to_string(), chave_or));
    }

    // --- ninerouter (local, opcional) ---
    let nove_rodando = state.ninerouter.lock().await.is_some();
    if cfg.nine_router.installed && nove_rodando {
        let porta = cfg.nine_router.port;
        let modelos9 = lr_ninerouter::listar_modelos(porta)
            .await
            .unwrap_or_default();
        if !modelos9.is_empty() {
            provedores.push((
                "ninerouter".to_string(),
                ProvedorDsh {
                    display_name: "9Router".to_string(),
                    base_url: format!("http://127.0.0.1:{porta}/v1"),
                    api_key_env: NINEROUTER_KEY_ENV.to_string(),
                    models: modelos9
                        .into_iter()
                        .map(|m| ModeloDsh {
                            name: m.id.clone(),
                            id: m.id,
                            context_window: m.context_length,
                        })
                        .collect(),
                },
            ));
            // O `/v1` do 9router exige chave; a normal vem do
            // `ninerouter_start`, e uma config anterior a essa rotina pode
            // não ter nenhuma — mesma rede de segurança do
            // `provider_endpoint`.
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
                    Err(e) => log::warn!("9router sem chave de API utilizável para o dsh: {e}"),
                }
            }
            if !chave9.is_empty() {
                envs.push((NINEROUTER_KEY_ENV.to_string(), chave9));
            }
        }
    }

    if provedores.is_empty() {
        return Err(
            "nenhum modelo para entregar ao harness: suba o Servidor Local (ou ligue um provedor remoto com favoritos) e tente de novo"
                .to_string(),
        );
    }

    Ok((provedores, envs))
}

/// Sobe o dsh de ponta a ponta. Reutilizado pelo `harness_launch` com id
/// `dsh` — é o MESMO caminho gerenciado, venha o clique de onde vier.
pub(crate) async fn dsh_start_inner(app: &AppHandle, state: &AppState) -> CmdResult<DshStatus> {
    if state.dsh.lock().await.is_some() {
        return Ok(status_atual(state).await);
    }

    // 1. Node + dsh instalados (com eventos; a primeira vez baixa de verdade).
    if !layout(state).instalado() || !state.node.state().installed {
        instalar_dsh(app, state).await?;
    }

    // 2. Servidor local de pé: o yaml lista os modelos do Router dele. Não
    // exige modelo SELECIONADO — todos entram.
    let rodando = {
        let guard = state.server.lock().await;
        guard.as_ref().map(|s| s.is_spawned()).unwrap_or(false)
    };
    if !rodando && let Err(e) = crate::commands::start_engine(app, state).await {
        // Sem runtime do llama.cpp, sem modelo baixado — o harness ainda abre
        // com os provedores remotos. Abortar aqui deixava o lançamento inteiro
        // refém de uma aba que a pessoa talvez nem use.
        log::warn!("dsh: o servidor local não subiu ({e}); seguindo sem ele");
    }

    // 3. Providers + envs de chave, e o settings.yaml cirúrgico. A escrita é
    // bloqueante (lock com backoff): fora do executor.
    let (provedores, envs) = montar_provedores(state).await?;
    let home = dsh_home(state);
    tauri::async_runtime::spawn_blocking(move || {
        lr_dshhost::settings::escrever_settings(&home, &provedores)
    })
    .await
    .map_err(err_str)?
    .map_err(err_str)?;

    // 4. Spawn sob o lock; a espera de readiness fica FORA dele — mesmo
    // desenho do 9router e do start_engine.
    let porta_handle;
    {
        let mut guard = state.dsh.lock().await;
        if guard.is_some() {
            drop(guard);
            return Ok(status_atual(state).await);
        }
        let run = RunEnv {
            dsh_home: dsh_home(state),
            extra_env: envs,
        };
        let mut host = DshHost::spawn(&state.node, &layout(state), &run).map_err(err_str)?;
        if let Some(pid) = host.pid() {
            state.dsh_pid.store(pid, Ordering::SeqCst);
        }
        porta_handle = host.porta_handle();

        // stdout: é onde a porta real aparece (`dsh web: http://127.0.0.1:N`)
        // — e o log vai para a mesma janela da instalação.
        let (stdout, stderr) = host.take_output();
        if let Some(saida) = stdout {
            let app2 = app.clone();
            let handle = porta_handle.clone();
            tauri::async_runtime::spawn(async move {
                use tokio::io::AsyncBufReadExt as _;
                let mut linhas = tokio::io::BufReader::new(saida).lines();
                while let Ok(Some(line)) = linhas.next_line().await {
                    if let Some(p) = lr_dshhost::parse_porta(&line) {
                        handle.store(p, Ordering::SeqCst);
                    }
                    let _ = app2.emit(EVENTO, &DshEvent::Log { line });
                }
            });
        }
        if let Some(saida) = stderr {
            let app2 = app.clone();
            tauri::async_runtime::spawn(async move {
                use tokio::io::AsyncBufReadExt as _;
                let mut linhas = tokio::io::BufReader::new(saida).lines();
                while let Ok(Some(line)) = linhas.next_line().await {
                    let _ = app2.emit(EVENTO, &DshEvent::Log { line });
                }
            });
        }
        *guard = Some(host);
    }

    // 5. Prazo ÚNICO de readiness: porta no stdout, depois HTTP 200 no `/`.
    let inicio = std::time::Instant::now();
    let resultado = async {
        let porta = lr_dshhost::aguardar_porta(&porta_handle, lr_dshhost::READY_TIMEOUT).await?;
        let restante = lr_dshhost::READY_TIMEOUT.saturating_sub(inicio.elapsed());
        lr_dshhost::aguardar_pronto(porta, restante).await
    }
    .await;
    if let Err(e) = resultado {
        // Não subiu: derruba para não deixar processo pendurado, e a falha
        // vai também ao canal de eventos — a tela pode estar só ouvindo.
        let _ = dsh_stop_inner(state).await;
        let message = err_str(e);
        let _ = app.emit(
            EVENTO,
            &DshEvent::Failed {
                message: message.clone(),
            },
        );
        return Err(message);
    }

    let _ = app.emit(EVENTO, &DshEvent::Ready);
    Ok(status_atual(state).await)
}

#[tauri::command]
pub async fn dsh_start(app: AppHandle, state: State<'_, AppState>) -> CmdResult<DshStatus> {
    dsh_start_inner(&app, &state).await
}

/// Abre o painel do dsh numa janela própria do app — molde exato do painel
/// do 9router: janela de primeiro nível (não iframe), reuso se já aberta,
/// fora das capabilities. A UI web do dsh não tem autenticação; a proteção é
/// o bind em loopback, e a janela conversa com ele como um navegador local.
pub(crate) async fn abrir_painel(app: &AppHandle, state: &AppState) -> CmdResult<()> {
    let porta = {
        let guard = state.dsh.lock().await;
        match guard.as_ref().and_then(|d| d.porta()) {
            Some(p) => p,
            None => return Err("o dsh não está no ar".to_string()),
        }
    };

    // Já aberta: trazer à frente. Recriar perderia a conversa em andamento.
    if let Some(janela) = app.get_webview_window(JANELA_PAINEL) {
        let _ = janela.unminimize();
        let _ = janela.show();
        let _ = janela.set_focus();
        return Ok(());
    }

    let url = format!("http://127.0.0.1:{porta}")
        .parse::<tauri::Url>()
        .map_err(err_str)?;
    tauri::WebviewWindowBuilder::new(app, JANELA_PAINEL, tauri::WebviewUrl::External(url))
        .title("DeepSeek Harness")
        .inner_size(1180.0, 820.0)
        .min_inner_size(900.0, 600.0)
        .build()
        .map_err(err_str)?;
    Ok(())
}

#[tauri::command]
pub async fn dsh_open_panel(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    abrir_painel(&app, &state).await
}

/// Fecha o painel quando o processo por trás dele morre — e quando a janela
/// principal morre, pelo mesmo motivo do painel do 9router: uma janela de
/// conteúdo remoto não pode manter o app (e o dsh) vivos sozinha.
pub fn fechar_painel(app: &AppHandle) {
    if let Some(janela) = app.get_webview_window(JANELA_PAINEL) {
        let _ = janela.close();
    }
}

async fn dsh_stop_inner(state: &AppState) -> CmdResult<()> {
    let host = {
        let mut guard = state.dsh.lock().await;
        guard.take()
    };
    if let Some(mut h) = host {
        // A graça do SIGTERM leva até ~6 s (dispose interno de 5 s): fora do
        // executor para não travar os outros comandos nesse meio-tempo.
        let _ = tauri::async_runtime::spawn_blocking(move || h.stop_blocking()).await;
    }
    state.dsh_pid.store(0, Ordering::SeqCst);
    Ok(())
}

/// Para o processo e apaga a instalação.
///
/// `remove_data` leva junto o `DSH_HOME`: settings, sessões e credenciais
/// criadas dentro do harness. A tela pergunta antes — quem instalou pelo app
/// também desinstala pelo app, sem precisar caçar pasta no disco.
#[tauri::command]
pub async fn dsh_uninstall(
    app: AppHandle,
    state: State<'_, AppState>,
    remove_data: bool,
) -> CmdResult<DshStatus> {
    dsh_stop_inner(&state).await?;
    fechar_painel(&app);
    let l = layout(&state);
    let home = dsh_home(&state);
    tauri::async_runtime::spawn_blocking(move || lr_dshhost::desinstalar(&l, &home, remove_data))
        .await
        .map_err(err_str)?
        .map_err(err_str)?;
    Ok(status_atual(&state).await)
}

#[tauri::command]
pub async fn dsh_stop(app: AppHandle, state: State<'_, AppState>) -> CmdResult<DshStatus> {
    dsh_stop_inner(&state).await?;
    fechar_painel(&app);
    Ok(status_atual(&state).await)
}
