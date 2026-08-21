//! Cluster RPC na LAN: descoberta, emparelhamento, GPU extra.

use crate::commands::restart_engine;
use crate::state::AppState;
use tauri::{AppHandle, Emitter, State};

type CmdResult<T> = Result<T, String>;

fn err_str<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

async fn ensure_rpc_binaries(app: &AppHandle, state: &AppState) -> CmdResult<bool> {
    let st = state
        .runtime_mgr
        .ensure_rpc(&state.profile, {
            let app = app.clone();
            move |ev| {
                let _ = app.emit("runtime", &ev);
            }
        })
        .await
        .map_err(err_str)?;
    state.cluster.set_rpc_ready(st.rpc_ready).await;
    Ok(st.rpc_ready)
}

async fn engine_is_running(state: &AppState) -> bool {
    state
        .server
        .lock()
        .await
        .as_ref()
        .is_some_and(|s| s.is_spawned())
}

/// Só reinicia se o usuário já tinha o motor ligado. Parear não liga o
/// servidor sozinho.
async fn restart_if_running(app: &AppHandle, state: &AppState) -> CmdResult<()> {
    if !engine_is_running(state).await {
        // Sem motor no ar não há o que reiniciar, mas o conjunto de
        // dispositivos mudou: a varredura precisa remedir mesmo assim.
        crate::commands_tuning::spawn_auto_tune(app, state);
        return Ok(());
    }
    restart_engine(app, state, false).await.map(|_| ())
}

#[tauri::command]
pub async fn cluster_status(state: State<'_, AppState>) -> CmdResult<lr_cluster::ClusterSnapshot> {
    let ready = {
        let variant = lr_runtime::select_variant(&state.profile);
        state.runtime_mgr.state(variant).rpc_ready
    };
    state.cluster.set_rpc_ready(ready).await;
    Ok(state.cluster.snapshot().await)
}

#[tauri::command]
pub async fn cluster_set_enabled(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> CmdResult<lr_cluster::ClusterSnapshot> {
    if enabled {
        let _ = ensure_rpc_binaries(&app, &state).await;
        state.cluster.set_enabled(true).await?;
    } else {
        state.cluster.set_enabled(false).await?;
        let _ = restart_if_running(&app, &state).await;
    }
    Ok(state.cluster.snapshot().await)
}

#[tauri::command]
pub async fn cluster_ensure_rpc(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<lr_runtime::RuntimeState> {
    ensure_rpc_binaries(&app, &state).await?;
    let variant = lr_runtime::select_variant(&state.profile);
    Ok(state.runtime_mgr.state(variant))
}

#[tauri::command]
pub async fn cluster_request_pair(
    app: AppHandle,
    state: State<'_, AppState>,
    peer_id: String,
) -> CmdResult<lr_cluster::ClusterSnapshot> {
    if !ensure_rpc_binaries(&app, &state).await? {
        return Err(
            "o motor instalado não traz o worker RPC. Atualize o motor de IA em Ajustes.".into(),
        );
    }
    state.cluster.request_pair(&peer_id).await?;
    Ok(state.cluster.snapshot().await)
}

#[tauri::command]
pub async fn cluster_accept(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<lr_cluster::ClusterSnapshot> {
    if engine_is_running(&state).await {
        return Err("pare o servidor local nesta máquina antes de emprestar a GPU".into());
    }
    if !ensure_rpc_binaries(&app, &state).await? {
        return Err(
            "o motor instalado não traz o worker RPC. Atualize o motor de IA em Ajustes.".into(),
        );
    }
    state.cluster.accept_incoming().await?;
    Ok(state.cluster.snapshot().await)
}

#[tauri::command]
pub async fn cluster_reject(state: State<'_, AppState>) -> CmdResult<lr_cluster::ClusterSnapshot> {
    state.cluster.reject_incoming().await?;
    Ok(state.cluster.snapshot().await)
}

#[tauri::command]
pub async fn cluster_forget(
    app: AppHandle,
    state: State<'_, AppState>,
    peer_id: String,
) -> CmdResult<lr_cluster::ClusterSnapshot> {
    let was_host = state.cluster.remote_vram().await > 0;
    state.cluster.forget(&peer_id).await;
    if was_host {
        let _ = restart_if_running(&app, &state).await;
    }
    Ok(state.cluster.snapshot().await)
}

#[tauri::command]
pub async fn cluster_disconnect(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<lr_cluster::ClusterSnapshot> {
    let was_host = state.cluster.remote_vram().await > 0;
    state.cluster.disconnect().await;
    if was_host {
        let _ = restart_if_running(&app, &state).await;
    }
    Ok(state.cluster.snapshot().await)
}

/// Relê `--rpc` no llama-server depois que o worker aceita — só se já estava ligado.
#[tauri::command]
pub async fn cluster_apply_engine(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<lr_cluster::ClusterSnapshot> {
    restart_if_running(&app, &state).await?;
    Ok(state.cluster.snapshot().await)
}
