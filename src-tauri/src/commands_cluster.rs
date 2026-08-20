//! Cluster RPC na LAN: descoberta, emparelhamento, GPU extra.

use crate::commands::{restart_engine, start_engine};
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

#[tauri::command]
pub async fn cluster_status(
    state: State<'_, AppState>,
) -> CmdResult<lr_cluster::ClusterSnapshot> {
    let ready = {
        let variant = lr_runtime::select_variant(&state.profile);
        state.runtime_mgr.state(variant).rpc_ready
    };
    state.cluster.set_rpc_ready(ready).await;
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
            "o motor instalado não traz RPC. Os dois apps precisam do overlay da mesma tag."
                .into(),
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
    if !ensure_rpc_binaries(&app, &state).await? {
        return Err(
            "o motor instalado não traz RPC. Os dois apps precisam do overlay da mesma tag."
                .into(),
        );
    }
    state.cluster.accept_incoming().await?;
    state
        .rpc_pid
        .store(
            state.cluster.worker_pid().await,
            std::sync::atomic::Ordering::SeqCst,
        );
    Ok(state.cluster.snapshot().await)
}

#[tauri::command]
pub async fn cluster_reject(
    state: State<'_, AppState>,
) -> CmdResult<lr_cluster::ClusterSnapshot> {
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
    state
        .rpc_pid
        .store(0, std::sync::atomic::Ordering::SeqCst);
    if was_host {
        let _ = restart_or_start(&app, &state).await;
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
    state
        .rpc_pid
        .store(0, std::sync::atomic::Ordering::SeqCst);
    if was_host {
        let _ = restart_or_start(&app, &state).await;
    }
    Ok(state.cluster.snapshot().await)
}

/// Relê `--rpc` no llama-server depois que o worker aceita.
#[tauri::command]
pub async fn cluster_apply_engine(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<lr_cluster::ClusterSnapshot> {
    restart_or_start(&app, &state).await?;
    Ok(state.cluster.snapshot().await)
}

async fn restart_or_start(app: &AppHandle, state: &AppState) -> CmdResult<()> {
    match restart_engine(app, state, false).await {
        Ok(_) => Ok(()),
        Err(e) if e.starts_with("engine-busy:") => Err(e),
        Err(_) => {
            start_engine(app, state).await.map(|_| ())
        }
    }
}
