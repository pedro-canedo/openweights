//! Comandos Tauri expostos ao frontend. Glue fina: a lógica mora nos crates.

use crate::state::AppState;
use lr_advisor as advisor;
use lr_models::{DownloadRequest, DownloadStatus, ModelSummary, SortBy};
use lr_types::HardwareProfile;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::io::AsyncBufReadExt;

type CmdResult<T> = Result<T, String>;

fn err_str<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

// ------------------------------------------------------------ hardware ---

#[tauri::command]
pub fn hardware_profile(state: State<'_, AppState>) -> HardwareProfile {
    state.profile.clone()
}

#[tauri::command]
pub fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppPaths {
    pub data_dir: String,
    pub models_dir: String,
}

#[tauri::command]
pub fn app_paths(state: State<'_, AppState>) -> AppPaths {
    AppPaths {
        data_dir: state.data_dir.to_string_lossy().into_owned(),
        models_dir: state.models_dir.to_string_lossy().into_owned(),
    }
}

// ------------------------------------------------------------- runtime ---

#[tauri::command]
pub fn runtime_status(state: State<'_, AppState>) -> lr_runtime::RuntimeState {
    let variant = lr_runtime::select_variant(&state.profile);
    state.runtime_mgr.state(variant)
}

/// Baixa/instala o runtime ideal (com fallback CUDA→Vulkan→CPU), emitindo
/// eventos `runtime` com o progresso.
#[tauri::command]
pub async fn runtime_ensure(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<lr_runtime::RuntimeState> {
    state
        .runtime_mgr
        .ensure_best(&state.profile, move |ev| {
            let _ = app.emit("runtime", &ev);
        })
        .await
        .map_err(err_str)
}

// -------------------------------------------------------------- modelos ---

#[tauri::command]
pub async fn models_search(
    state: State<'_, AppState>,
    query: String,
    sort: Option<String>,
) -> CmdResult<Vec<ModelSummary>> {
    let sort = match sort.as_deref() {
        Some("downloads") => SortBy::Downloads,
        Some("likes") => SortBy::Likes,
        Some("updated") => SortBy::Updated,
        _ => SortBy::Trending,
    };
    state
        .hf
        .lock()
        .await
        .search(&query, sort, 30)
        .await
        .map_err(err_str)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuantView {
    pub artifact_name: String,
    pub files: Vec<String>,
    pub total_bytes: u64,
    #[serde(flatten)]
    pub option: advisor::QuantOption,
}

/// Lista as quantizações de um repo com veredito de compatibilidade
/// (badges verde/amarelo/cinza/vermelho) para o hardware desta máquina.
#[tauri::command]
pub async fn models_quants(
    state: State<'_, AppState>,
    repo_id: String,
    params_total: Option<u64>,
    ctx_len: Option<u32>,
) -> CmdResult<Vec<QuantView>> {
    let files = state
        .hf
        .lock()
        .await
        .repo_files(&repo_id)
        .await
        .map_err(err_str)?;
    let artifacts = lr_models::group_artifacts(&files);

    let budget = advisor::MemoryBudget::from_profile(&state.profile);
    let meta = advisor::ModelMeta::estimate_from_params(
        params_total.unwrap_or(8_000_000_000),
        ctx_len.unwrap_or(8192),
    );
    let qfiles: Vec<advisor::QuantFile> = artifacts
        .iter()
        .map(|a| advisor::QuantFile {
            filename: a.name.clone(),
            size_bytes: a.total_bytes,
        })
        .collect();
    let options = advisor::evaluate_files(&budget, &meta, &qfiles);

    Ok(artifacts
        .into_iter()
        .zip(options)
        .map(|(a, option)| QuantView {
            artifact_name: a.name,
            files: a.files.iter().map(|f| f.path.clone()).collect(),
            total_bytes: a.total_bytes,
            option,
        })
        .collect())
}

// ------------------------------------------------------------ downloads ---

#[tauri::command]
pub async fn download_start(
    state: State<'_, AppState>,
    repo_id: String,
    artifact_name: String,
) -> CmdResult<String> {
    let files = state
        .hf
        .lock()
        .await
        .repo_files(&repo_id)
        .await
        .map_err(err_str)?;
    let artifact = lr_models::group_artifacts(&files)
        .into_iter()
        .find(|a| a.name == artifact_name)
        .ok_or_else(|| format!("artefato não encontrado: {artifact_name}"))?;

    let token = state.store.get_setting("hf_token").ok().flatten();
    state
        .downloads
        .enqueue(DownloadRequest {
            repo_id,
            artifact_name,
            files: artifact.files,
            token,
        })
        .await
        .map_err(err_str)
}

#[tauri::command]
pub async fn download_pause(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.downloads.pause(&id).await.map_err(err_str)
}

#[tauri::command]
pub async fn download_resume(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let token = state.store.get_setting("hf_token").ok().flatten();
    state.downloads.resume(&id, token).await.map_err(err_str)
}

#[tauri::command]
pub async fn download_cancel(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.downloads.cancel(&id).await.map_err(err_str)
}

#[tauri::command]
pub async fn downloads_list(state: State<'_, AppState>) -> CmdResult<Vec<DownloadStatus>> {
    Ok(state.downloads.list().await)
}

// ------------------------------------------------------- biblioteca local ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelView {
    #[serde(flatten)]
    pub artifact: lr_models::LocalArtifact,
    pub quant_label: String,
}

#[tauri::command]
pub fn local_models(state: State<'_, AppState>) -> Vec<LocalModelView> {
    lr_models::scan_local(&state.models_dir)
        .into_iter()
        .map(|artifact| LocalModelView {
            quant_label: advisor::quant::parse_label(&artifact.name),
            artifact,
        })
        .collect()
}

#[tauri::command]
pub fn model_delete(state: State<'_, AppState>, repo_id: String, name: String) -> CmdResult<()> {
    let arts = lr_models::scan_local(&state.models_dir);
    let art = arts
        .iter()
        .find(|a| a.repo_id == repo_id && a.name == name)
        .ok_or_else(|| format!("modelo não encontrado: {name}"))?;
    for f in &art.files {
        std::fs::remove_file(f).map_err(err_str)?;
    }
    Ok(())
}

// -------------------------------------------------------------- servidor ---

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatusView {
    pub running: bool,
    pub base_url: Option<String>,
    pub port: u16,
    pub lan: bool,
}

const DEFAULT_PORT: u16 = 11711;

/// Preferências do motor que só valem no boot do processo.
struct ServerPrefs {
    port: u16,
    lan: bool,
    api_key: Option<String>,
    models_max: u32,
    parallel: u32,
}

fn server_prefs(state: &AppState) -> ServerPrefs {
    let get = |k: &str| state.store.get_setting(k).ok().flatten();
    ServerPrefs {
        port: get("server_port")
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_PORT),
        lan: get("server_lan").as_deref() == Some("true"),
        api_key: get("server_api_key").filter(|v| !v.is_empty()),
        // Um modelo por vez: o roteador descarrega o menos usado ao bater no
        // teto, então 1 é literalmente "troque o modelo carregado". Com 2, o
        // segundo modelo tentava caber junto do primeiro e a placa recusava.
        models_max: get("server_models_max")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1),
        // Conversas atendidas ao mesmo tempo. Cada uma leva uma fatia da
        // janela de contexto, então 4 significava entregar 8k a quem pediu
        // 32k — e o app anunciava os 32k. Uma conversa por vez é o uso real
        // de um app de desktop; quem quiser mais sobe aqui sabendo o preço.
        parallel: get("server_parallel")
            .and_then(|v| v.parse().ok())
            .filter(|n| *n >= 1)
            .unwrap_or(1),
    }
}

const CTX_MIN: u32 = 512;
const CTX_MAX: u32 = 262_144;

fn clamp_ctx(n: u32) -> u32 {
    n.clamp(CTX_MIN, CTX_MAX)
}

fn model_stem(name: &str) -> &str {
    name.strip_suffix(".gguf")
        .or_else(|| name.strip_suffix(".GGUF"))
        .unwrap_or(name)
}

/// Perfil de um modelo, tolerando a chave com e sem `.gguf`.
///
/// O nome que a interface manda nem sempre é o nome do arquivo — o seletor do
/// chat usa o id do Router, que às vezes perde a extensão.
pub(crate) fn profile_for(state: &AppState, name: &str) -> Option<lr_types::tuning::ModelProfile> {
    let stem = model_stem(name);
    let with_ext = format!("{stem}.gguf");
    for key in [name, stem, with_ext.as_str()] {
        if let Ok(Some(p)) = state.store.model_profile(key) {
            return Some(p);
        }
    }
    None
}

/// Reescreve `router-models.ini` com o perfil gravado de cada modelo.
fn write_router_preset(state: &AppState) -> Result<std::path::PathBuf, String> {
    let preset_path = state.data_dir.join("router-models.ini");
    let preset_models: Vec<lr_engine::PresetEntry> = lr_models::scan_local(&state.models_dir)
        .into_iter()
        .map(|a| {
            let mut entry = lr_engine::PresetEntry::new(a.name.clone(), a.primary_path);
            if let Some(perfil) = profile_for(state, &a.name) {
                for (chave, valor) in perfil.to_ini_extras() {
                    entry = entry.with_extra(chave, valor);
                }
            }
            entry
        })
        .collect();
    lr_engine::write_models_preset(&preset_path, &preset_models).map_err(err_str)?;
    Ok(preset_path)
}

#[tauri::command]
pub async fn server_status(state: State<'_, AppState>) -> CmdResult<ServerStatusView> {
    let prefs = server_prefs(&state);
    let (port, lan) = (prefs.port, prefs.lan);
    let guard = state.server.lock().await;
    Ok(match guard.as_ref() {
        // connect_url, não base_url: em modo LAN o bind é 0.0.0.0, que não é
        // conectável pela própria UI (e é bloqueado pelo CSP).
        Some(srv) if srv.is_spawned() => ServerStatusView {
            running: true,
            base_url: Some(srv.config().connect_url()),
            port: srv.config().port,
            lan: srv.config().host != "127.0.0.1",
        },
        _ => ServerStatusView {
            running: false,
            base_url: None,
            port,
            lan,
        },
    })
}

#[tauri::command]
pub async fn server_start(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<ServerStatusView> {
    start_engine(&app, &state).await
}

pub(crate) async fn start_engine(app: &AppHandle, state: &AppState) -> CmdResult<ServerStatusView> {
    let runtime = {
        let variant = lr_runtime::select_variant(&state.profile);
        state.runtime_mgr.state(variant)
    };
    let exe = runtime
        .server_exe
        .ok_or("runtime do llama.cpp ainda não instalado")?;

    let ServerPrefs {
        port,
        lan,
        api_key,
        models_max,
        parallel,
    } = server_prefs(state);

    // Spawn + instalação no slot acontecem sob o lock; a espera do /health
    // fica FORA dele para não travar server_status/stop/exit por até 30 s.
    let view = {
        let mut guard = state.server.lock().await;
        if let Some(srv) = guard.as_ref()
            && srv.is_spawned()
        {
            return Ok(ServerStatusView {
                running: true,
                base_url: Some(srv.config().connect_url()),
                port: srv.config().port,
                lan: srv.config().host != "127.0.0.1",
            });
        }

        let mut cfg = lr_engine::ServerConfig::new(exe, state.models_dir.clone(), port);
        if lan {
            cfg.host = "0.0.0.0".to_string();
        }
        cfg.api_key = api_key;
        cfg.models_max = models_max;
        cfg.parallel = parallel;

        // --models-dir do llama.cpp não é recursivo; a biblioteca mora em
        // autor/repo/. O INI registra cada GGUF com o nome que a UI já usa,
        // mais `ctx-size`/`fit` quando o usuário fixou a janela de contexto.
        cfg.models_preset = Some(write_router_preset(state)?);

        let mut srv = lr_engine::LlamaServer::new(cfg);
        srv.spawn().map_err(err_str)?;
        if let Some(pid) = srv.pid() {
            state
                .server_pid
                .store(pid, std::sync::atomic::Ordering::SeqCst);
        }

        // Logs do processo → evento `server-log` (baixa frequência, IPC ok).
        let (stdout, stderr) = srv.take_output();
        if let Some(out) = stdout {
            let app2 = app.clone();
            tauri::async_runtime::spawn(async move {
                let mut lines = tokio::io::BufReader::new(out).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = app2.emit("server-log", &line);
                }
            });
        }
        if let Some(err) = stderr {
            let app2 = app.clone();
            tauri::async_runtime::spawn(async move {
                let mut lines = tokio::io::BufReader::new(err).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = app2.emit("server-log", &line);
                }
            });
        }

        let view = ServerStatusView {
            running: true,
            base_url: Some(srv.config().connect_url()),
            port: srv.config().port,
            lan,
        };
        *guard = Some(srv);
        view
    };

    // Espera de readiness com aquisições curtas do lock (health responde
    // rápido: conexão recusada enquanto o processo sobe).
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let health = {
            let guard = state.server.lock().await;
            match guard.as_ref() {
                Some(srv) => srv.health().await,
                // server_stop rodou no meio do start.
                None => return Err("servidor foi parado durante a inicialização".to_string()),
            }
        };
        if health == lr_engine::Health::Ready {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            // Desiste: remove e mata o processo que não ficou pronto.
            if let Some(mut srv) = state.server.lock().await.take() {
                srv.stop().await;
            }
            state
                .server_pid
                .store(0, std::sync::atomic::Ordering::SeqCst);
            let _ = app.emit(
                "server-status",
                &ServerStatusView {
                    running: false,
                    base_url: None,
                    port,
                    lan,
                },
            );
            return Err("llama-server não respondeu ao /health em 30s".to_string());
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    let _ = app.emit("server-status", &view);
    Ok(view)
}

#[tauri::command]
pub async fn server_stop(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    stop_engine(&app, &state).await
}

pub(crate) async fn stop_engine(app: &AppHandle, state: &AppState) -> CmdResult<()> {
    // Um único guard para parar E limpar o slot: dois lock() sequenciais
    // abririam janela para um server_start intercalado ser morto em seguida.
    {
        let mut guard = state.server.lock().await;
        if let Some(srv) = guard.as_mut() {
            srv.stop().await;
        }
        *guard = None;
    }
    state
        .server_pid
        .store(0, std::sync::atomic::Ordering::SeqCst);
    let prefs = server_prefs(state);
    let (port, lan) = (prefs.port, prefs.lan);
    let _ = app.emit(
        "server-status",
        &ServerStatusView {
            running: false,
            base_url: None,
            port,
            lan,
        },
    );
    Ok(())
}

/// Quem está usando o motor agora — vazio quer dizer "pode derrubar".
///
/// A guarda mora aqui, e não na tela, porque metade dos usuários do motor não
/// passa pela tela: o relógio das automações dispara execuções sozinho e o
/// índice do projeto pede embeddings ao mesmo servidor. Uma guarda no
/// frontend enxerga só a conversa aberta.
fn engine_busy_with(state: &AppState) -> Vec<&'static str> {
    let mut quem = Vec::new();
    if state.agent.live_count() > 0 {
        quem.push("agent");
    }
    if state.rag.is_indexing() {
        quem.push("rag");
    }
    quem
}

/// Situação do motor para a interface decidir o que oferecer.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineBusy {
    /// `agent` (execução do agente ou automação), `rag` (indexação).
    pub busy_with: Vec<&'static str>,
}

#[tauri::command]
pub fn server_busy(state: State<'_, AppState>) -> EngineBusy {
    EngineBusy {
        busy_with: engine_busy_with(&state),
    }
}

/// Para e sobe o motor de novo, de uma vez só.
///
/// Existe porque mudar porta, chave, número de conversas simultâneas ou
/// qualquer ajuste por modelo exige reler os argumentos e o INI, que só são
/// lidos no boot. Era feito em dois passos pela tela do chat, sem saber quem
/// mais estava usando o servidor — e derrubava execução de agente, automação
/// e indexação no meio.
///
/// `force` é a saída para quem sabe o que está fazendo: a interface pergunta
/// antes, dizendo quem vai ser interrompido.
#[tauri::command]
pub async fn server_restart(
    app: AppHandle,
    state: State<'_, AppState>,
    force: Option<bool>,
) -> CmdResult<ServerStatusView> {
    restart_engine(&app, &state, force.unwrap_or(false)).await
}

/// O reinício de verdade, alcançável de dentro do processo.
///
/// Recebe `&AppState` em vez do `State` do Tauri porque quem mais precisa
/// dele é código que já tem o estado na mão (aplicar uma configuração, por
/// exemplo) e não um comando vindo da tela.
pub(crate) async fn restart_engine(
    app: &AppHandle,
    state: &AppState,
    force: bool,
) -> CmdResult<ServerStatusView> {
    let ocupado = engine_busy_with(state);
    if !ocupado.is_empty() && !force {
        return Err(format!("engine-busy:{}", ocupado.join(",")));
    }
    stop_engine(app, state).await?;
    start_engine(app, state).await
}

/// `GET /props` do servidor em execução: capacidades do chat template do
/// modelo carregado.
///
/// É a ÚNICA fonte de verdade sobre suporte a ferramentas — decidir por nome
/// de modelo dá falso positivo (e o agente trava pedindo tools que o template
/// não sabe renderizar).
#[tauri::command]
pub async fn server_props(state: State<'_, AppState>) -> CmdResult<lr_engine::ServerProps> {
    // O lock sai de cena antes do HTTP: /props pode demorar e o loop de
    // readiness do server_start disputa o mesmo mutex.
    let (url, api_key) = {
        let guard = state.server.lock().await;
        match guard.as_ref() {
            Some(srv) if srv.is_spawned() => {
                (srv.config().connect_url(), srv.config().api_key.clone())
            }
            _ => return Err("servidor não está rodando".to_string()),
        }
    };
    lr_engine::LlamaClient::new(url)
        .with_optional_api_key(api_key)
        .props()
        .await
        .map_err(err_str)
}

/// Grava a janela de contexto de um modelo e atualiza o INI do Router.
///
/// `null` volta ao automático (`--fit`). A janela só entra em vigor no
/// próximo carregamento do modelo (reinício do servidor, se já estiver no ar).
#[tauri::command]
pub fn model_set_ctx(
    state: State<'_, AppState>,
    model: String,
    ctx_len: Option<u32>,
) -> CmdResult<Option<u32>> {
    let model = model.trim();
    if model.is_empty() {
        return Err("modelo vazio".into());
    }
    let mut perfil = profile_for(&state, model).unwrap_or_default();
    let stored = ctx_len.map(clamp_ctx);
    perfil.ctx = stored;
    // Mexer na janela na mão é escolha da pessoa, e a etiqueta precisa dizer
    // isso: um perfil recomendado deixa de ser recomendado quando é editado.
    perfil.source = lr_types::tuning::ProfileSource::Manual;
    state
        .store
        .set_model_profile(model, &perfil)
        .map_err(err_str)?;
    write_router_preset(&state)?;
    Ok(stored)
}

// ---------------------------------------------------------------- chats ---

#[tauri::command]
pub fn chats_list(state: State<'_, AppState>) -> CmdResult<Vec<lr_store::ChatRow>> {
    state.store.list_chats().map_err(err_str)
}

#[tauri::command]
pub fn chat_create(
    state: State<'_, AppState>,
    title: String,
    model_id: Option<String>,
) -> CmdResult<i64> {
    state
        .store
        .create_chat(&title, model_id.as_deref())
        .map_err(err_str)
}

#[tauri::command]
pub fn chat_delete(state: State<'_, AppState>, chat_id: i64) -> CmdResult<()> {
    state.store.delete_chat(chat_id).map_err(err_str)
}

#[tauri::command]
pub fn chat_rename(state: State<'_, AppState>, chat_id: i64, title: String) -> CmdResult<()> {
    state.store.rename_chat(chat_id, &title).map_err(err_str)
}

#[tauri::command]
pub fn chat_set_params(
    state: State<'_, AppState>,
    chat_id: i64,
    params_json: String,
) -> CmdResult<()> {
    state
        .store
        .set_chat_params(chat_id, &params_json)
        .map_err(err_str)
}

#[tauri::command]
pub fn messages_list(
    state: State<'_, AppState>,
    chat_id: i64,
) -> CmdResult<Vec<lr_store::MessageRow>> {
    state.store.list_messages(chat_id).map_err(err_str)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn message_add(
    state: State<'_, AppState>,
    chat_id: i64,
    role: String,
    content: String,
    tokens_per_sec: Option<f64>,
    gen_tokens: Option<i64>,
    gen_ms: Option<i64>,
    model: Option<String>,
) -> CmdResult<i64> {
    // A configuração não vem da tela: ela é lida aqui, do que está gravado
    // para este modelo. Assim o carimbo nunca discorda do que o motor
    // recebeu — e os tokens/s que já gravávamos passam a ter a que
    // configuração pertencem.
    let modelo = model.as_deref().map(str::trim).filter(|m| !m.is_empty());
    let chave = modelo
        .and_then(|m| profile_for(&state, m))
        .and_then(|p| p.key());

    state
        .store
        .add_message(
            chat_id,
            &role,
            &content,
            tokens_per_sec,
            gen_tokens,
            gen_ms,
            modelo,
            chave.as_deref(),
        )
        .map_err(err_str)
}

#[tauri::command]
pub fn message_delete(state: State<'_, AppState>, message_id: i64) -> CmdResult<()> {
    state.store.delete_message(message_id).map_err(err_str)
}

#[tauri::command]
pub fn message_update(
    state: State<'_, AppState>,
    message_id: i64,
    content: String,
) -> CmdResult<()> {
    state
        .store
        .update_message_content(message_id, &content)
        .map_err(err_str)
}

// -------------------------------------------------------------- presets ---

#[tauri::command]
pub fn presets_list(state: State<'_, AppState>) -> CmdResult<Vec<lr_store::PresetRow>> {
    state.store.list_presets().map_err(err_str)
}

#[tauri::command]
pub fn preset_save(state: State<'_, AppState>, name: String, json: String) -> CmdResult<i64> {
    state.store.save_preset(&name, &json).map_err(err_str)
}

#[tauri::command]
pub fn preset_delete(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    state.store.delete_preset(id).map_err(err_str)
}

// ------------------------------------------------------------- settings ---

#[tauri::command]
pub fn settings_get(state: State<'_, AppState>, key: String) -> CmdResult<Option<String>> {
    state.store.get_setting(&key).map_err(err_str)
}

#[tauri::command]
pub async fn settings_set(state: State<'_, AppState>, key: String, value: String) -> CmdResult<()> {
    state.store.set_setting(&key, &value).map_err(err_str)?;
    if key == "hf_token" {
        let token = (!value.is_empty()).then_some(value);
        *state.hf.lock().await = lr_models::HfClient::new(token);
    }
    Ok(())
}

// ----------------------------------------------------------- workspace ---

#[tauri::command]
pub async fn workspace_pick(window: tauri::WebviewWindow) -> CmdResult<Option<String>> {
    Ok(crate::workspace::pick_folder(Some(window)).await)
}

#[tauri::command]
pub fn workspace_list(root: String) -> CmdResult<Vec<crate::workspace::WorkspaceFile>> {
    crate::workspace::list_files(&root).map_err(err_str)
}

#[tauri::command]
pub fn workspace_read(root: String, rel: String) -> CmdResult<String> {
    crate::workspace::read_file(&root, &rel).map_err(err_str)
}

#[tauri::command]
pub fn workspace_write(root: String, rel: String, content: String) -> CmdResult<()> {
    crate::workspace::write_file(&root, &rel, &content).map_err(err_str)
}

#[tauri::command]
pub fn workspace_reveal(root: String, rel: Option<String>) -> CmdResult<()> {
    crate::workspace::reveal(&root, rel.as_deref()).map_err(err_str)
}
