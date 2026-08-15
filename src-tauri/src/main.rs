// Evita janela de console no Windows em release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod state;
mod telemetry;
mod workspace;

use tauri::{Emitter, Manager};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let profile = lr_hw::detect();
            log::info!(
                "hardware: {} | {} cores | {:.1} GiB RAM | {} GPU(s)",
                profile.cpu_name,
                profile.cpu_cores,
                profile.ram_total_bytes as f64 / (1u64 << 30) as f64,
                profile.gpus.len()
            );

            telemetry::spawn_loop(app.handle().clone(), &profile);

            let state = state::AppState::new(profile, app.handle())?;

            // Encaminha eventos de download para a UI. Lagged NÃO pode
            // encerrar o loop — só Closed (o manager morreu junto do app).
            let mut rx = state.downloads.subscribe();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use tokio::sync::broadcast::error::RecvError;
                loop {
                    match rx.recv().await {
                        Ok(ev) => {
                            let _ = handle.emit("download", &ev);
                        }
                        Err(RecvError::Lagged(n)) => {
                            log::warn!("forwarder de downloads pulou {n} eventos");
                        }
                        Err(RecvError::Closed) => break,
                    }
                }
            });

            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::hardware_profile,
            commands::app_version,
            commands::app_paths,
            commands::runtime_status,
            commands::runtime_ensure,
            commands::models_search,
            commands::models_quants,
            commands::download_start,
            commands::download_pause,
            commands::download_resume,
            commands::download_cancel,
            commands::downloads_list,
            commands::local_models,
            commands::model_delete,
            commands::server_status,
            commands::server_start,
            commands::server_stop,
            commands::chats_list,
            commands::chat_create,
            commands::chat_delete,
            commands::chat_rename,
            commands::chat_set_params,
            commands::messages_list,
            commands::message_add,
            commands::message_delete,
            commands::message_update,
            commands::presets_list,
            commands::preset_save,
            commands::preset_delete,
            commands::settings_get,
            commands::settings_set,
            commands::workspace_pick,
            commands::workspace_list,
            commands::workspace_read,
            commands::workspace_write,
        ])
        .build(tauri::generate_context!())
        .expect("erro ao iniciar o OpenWeights")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                // Sidecars NÃO morrem sozinhos com o app (Tauri #3273):
                // encerramento explícito e síncrono aqui.
                if let Some(state) = app.try_state::<state::AppState>() {
                    state.shutdown_blocking();
                }
            }
        });
}
