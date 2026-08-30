// Evita janela de console no Windows em release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod commands_cluster;
mod commands_dsh;
mod commands_flags;
mod commands_harness;
mod commands_providers;
mod commands_tuning;
mod desktop_host;
mod serve_stats;
mod spec_bench;
mod state;
mod telemetry;
mod tts;
mod update;
#[cfg(windows)]
mod webview_perm;
mod workspace;

use tauri::{Emitter, Manager};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
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
            // O coletor das estatísticas de serviço: cada tick pega o
            // estado pelo handle, então só pode nascer DEPOIS do manage.
            serve_stats::spawn_loop(app.handle().clone());

            let cluster = app.state::<state::AppState>().cluster.clone();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let on = std::sync::Arc::new(move |snap: lr_cluster::ClusterSnapshot| {
                    let _ = handle.emit("cluster", &snap);
                });
                if let Err(e) = cluster.start(on).await {
                    log::warn!("cluster: {e}");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::hardware_profile,
            commands::app_version,
            tts::tts_speak,
            update::update_check,
            update::update_install,
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
            commands::server_restart,
            commands::server_busy,
            commands::server_props,
            commands::server_generate_api_key,
            commands::server_lan_urls,
            // Estatísticas de serviço (tokens servidos a todos os clientes).
            serve_stats::serve_stats,
            serve_stats::serve_stats_clear,
            commands_cluster::cluster_status,
            commands_cluster::cluster_set_enabled,
            commands_cluster::cluster_ensure_rpc,
            commands_cluster::cluster_request_pair,
            commands_cluster::cluster_accept,
            commands_cluster::cluster_reject,
            commands_cluster::cluster_forget,
            commands_cluster::cluster_disconnect,
            commands_cluster::cluster_apply_engine,
            commands::model_set_ctx,
            commands::model_get_profile,
            commands::model_set_profile,
            // Configuração avançada do llama.cpp (catálogo de flags).
            commands_flags::flags_catalog,
            commands_flags::flags_validate,
            commands_flags::engine_preview,
            commands_flags::router_models,
            commands_flags::router_load_model,
            commands_flags::router_unload_model,
            commands_flags::model_capabilities,
            commands_flags::engine_presets_list,
            commands_flags::engine_preset_save,
            commands_flags::engine_preset_delete,
            commands_flags::engine_preset_apply,
            // Abrir o modelo carregado num harness externo.
            commands_harness::harness_list,
            commands_harness::harness_launch,
            // DeepSeek Harness gerenciado (instala, supervisiona, painel).
            commands_dsh::dsh_status,
            commands_dsh::dsh_install,
            commands_dsh::dsh_start,
            commands_dsh::dsh_stop,
            commands_dsh::dsh_open_panel,
            // Ajustar para esta máquina.
            commands_tuning::tune_advise,
            commands_tuning::tune_apply,
            commands_tuning::tune_bench,
            commands_tuning::tune_bench_cancel,
            commands_tuning::tune_spec_bench,
            commands_tuning::perf_history,
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
            commands::workspace_reveal,
            commands::chat_set_model,
            // Outras fontes de LLM (OpenRouter, 9router).
            commands_providers::providers_config_get,
            commands_providers::providers_config_set,
            commands_providers::providers_list,
            commands_providers::provider_endpoint,
            commands_providers::openrouter_models,
            commands_providers::openrouter_key_info,
            commands_providers::ninerouter_status,
            commands_providers::ninerouter_install,
            commands_providers::ninerouter_start,
            commands_providers::ninerouter_stop,
            commands_providers::ninerouter_open_panel,
            commands_providers::ninerouter_models,
            commands_providers::ninerouter_uninstall,
            commands_providers::gateway_status,
            commands_providers::gateway_config_set,
            commands_providers::gateway_install,
            commands_providers::gateway_start,
            commands_providers::gateway_stop,
            commands_providers::gateway_refresh_routes,
            commands_providers::gateway_uninstall,
        ])
        .build(tauri::generate_context!())
        .expect("erro ao iniciar o OpenWeights")
        .run(|app, event| {
            // Matar o llama-server ANTES do runtime Tokio acabar.
            // `Exit` sozinho chegava tarde demais e `block_on` podia travar.
            match event {
                tauri::RunEvent::Ready => {
                    #[cfg(windows)]
                    webview_perm::allow_microphone(app);
                }
                // A janela principal fechou: os painéis do 9router e do dsh
                // não podem segurar o app de pé sozinhos. Sem isto o processo
                // continua vivo — e os sidecars junto, que é o que o
                // `shutdown_blocking` abaixo existe para evitar.
                tauri::RunEvent::WindowEvent {
                    label,
                    event: tauri::WindowEvent::Destroyed,
                    ..
                } if label == "main" => {
                    commands_providers::fechar_painel(app);
                    commands_dsh::fechar_painel(app);
                }
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit => {
                    if let Some(state) = app.try_state::<state::AppState>() {
                        state.shutdown_blocking();
                    }
                }
                _ => {}
            }
        });
}
