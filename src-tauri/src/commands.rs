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
    state.downloads.resume(&id).await.map_err(err_str)
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

fn server_prefs(state: &AppState) -> (u16, bool, Option<String>, u32) {
    let get = |k: &str| state.store.get_setting(k).ok().flatten();
    (
        get("server_port")
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_PORT),
        get("server_lan").as_deref() == Some("true"),
        get("server_api_key").filter(|v| !v.is_empty()),
        get("server_models_max")
            .and_then(|v| v.parse().ok())
            .unwrap_or(2),
    )
}

#[tauri::command]
pub async fn server_status(state: State<'_, AppState>) -> CmdResult<ServerStatusView> {
    let (port, lan, _, _) = server_prefs(&state);
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
    let runtime = {
        let variant = lr_runtime::select_variant(&state.profile);
        state.runtime_mgr.state(variant)
    };
    let exe = runtime
        .server_exe
        .ok_or("runtime do llama.cpp ainda não instalado")?;

    let (port, lan, api_key, models_max) = server_prefs(&state);

    // Spawn + instalação no slot acontecem sob o lock; a espera do /health
    // fica FORA dele para não travar server_status/stop/exit por até 30 s.
    let view = {
        let mut guard = state.server.lock().await;
        if let Some(srv) = guard.as_ref() {
            if srv.is_spawned() {
                return Ok(ServerStatusView {
                    running: true,
                    base_url: Some(srv.config().connect_url()),
                    port: srv.config().port,
                    lan: srv.config().host != "127.0.0.1",
                });
            }
        }

        let mut cfg = lr_engine::ServerConfig::new(exe, state.models_dir.clone(), port);
        if lan {
            cfg.host = "0.0.0.0".to_string();
        }
        cfg.api_key = api_key;
        cfg.models_max = models_max;

        let mut srv = lr_engine::LlamaServer::new(cfg);
        srv.spawn().map_err(err_str)?;

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
    // Um único guard para parar E limpar o slot: dois lock() sequenciais
    // abririam janela para um server_start intercalado ser morto em seguida.
    {
        let mut guard = state.server.lock().await;
        if let Some(srv) = guard.as_mut() {
            srv.stop().await;
        }
        *guard = None;
    }
    let (port, lan, _, _) = server_prefs(&state);
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
pub fn message_add(
    state: State<'_, AppState>,
    chat_id: i64,
    role: String,
    content: String,
    tokens_per_sec: Option<f64>,
    gen_tokens: Option<i64>,
    gen_ms: Option<i64>,
) -> CmdResult<i64> {
    state
        .store
        .add_message(chat_id, &role, &content, tokens_per_sec, gen_tokens, gen_ms)
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
