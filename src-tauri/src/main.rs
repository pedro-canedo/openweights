// Evita janela de console no Windows em release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod catalogo;
mod commands;
mod commands_agenticow;
mod commands_cluster;
mod commands_flags;
mod commands_harness;
mod commands_jev;
mod commands_owcli;
mod commands_power;
mod commands_providers;
mod commands_terminal;
mod commands_tuning;
mod comparison;
mod desktop_host;
mod externo;
mod gpu_lease;
mod janela;
mod optimization;
mod owcli_historico;
mod serve_stats;
mod spec_bench;
mod state;
mod studio;
mod telemetry;
mod tts;
mod update;
#[cfg(windows)]
mod webview_perm;
mod workspace;

use tauri::{Emitter, Manager};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    // SAFETY: primeira coisa do `main`: nenhuma outra thread existe ainda e o
    // GTK (que carrega o fontconfig) só sobe no `Builder` abaixo.
    unsafe { lr_proc::esconder_fontes_colrv1_do_webkit() };

    tauri::Builder::default()
        // Sem o script que o plugin injeta para abrir os links clicados: no
        // Linux ele leva ao `xdg-open` de dentro do AppImage, que não abre
        // nada. A interface instala o equivalente dela (`openExternal.ts`),
        // que passa pelo `externo::open_external`.
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // A janela principal nasce em código: nua, com a interface do app
            // numa webview filha — o que deixa a UI do AgenticOw entrar noutra.
            janela::criar(app)?;
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
            gpu_lease::init(&state.data_dir);

            // Encaminha eventos de download para a UI. Lagged NÃO pode
            // encerrar o loop — só Closed (o manager morreu junto do app).
            let mut rx = state.downloads.subscribe();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use tokio::sync::broadcast::error::RecvError;
                loop {
                    match rx.recv().await {
                        Ok(ev) => {
                            // O modelo do decisor local acabou de chegar: o
                            // processo pode subir agora, sem esperar ninguém
                            // reiniciar o motor.
                            if commands_jev::e_download_do_decisor(&ev) {
                                let app = handle.clone();
                                tauri::async_runtime::spawn(async move {
                                    let state = app.state::<state::AppState>();
                                    commands_jev::sincronizar_decisor_local(&app, &state).await;
                                    commands_jev::sincronizar_shim(&app, &state).await;
                                });
                            }
                            // Modelo novo no disco: o AgenticOw o vê assim
                            // que o Router passa a atendê-lo.
                            if matches!(&ev, lr_models::DownloadEvent::Update { status }
                                if status.state == lr_models::DownloadState::Done)
                            {
                                commands_agenticow::agendar_catalogo(&handle);
                            }
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

            // Quem já usa o OwCLI tem o gateway de pé desde o boot — o `owcli`
            // do terminal do sistema pode estar esperando.
            {
                let h = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    commands_owcli::sincronizar(&h).await;
                });
            }

            let cluster = app.state::<state::AppState>().cluster.clone();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let on = std::sync::Arc::new(move |snap: lr_cluster::ClusterSnapshot| {
                    let _ = handle.emit("cluster", &snap);
                });
                if cluster.snapshot().await.enabled && gpu_lease::acquire("cluster").is_err() {
                    let _ = cluster.set_enabled(false).await;
                }
                if let Err(e) = cluster.start(on).await {
                    gpu_lease::release("cluster");
                    log::warn!("cluster: {e}");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            studio::studio_status,
            studio::studio_request,
            studio::studio_upload,
            studio::studio_train,
            studio::studio_install,
            studio::studio_import_legacy,
            studio::studio_import_model,
            studio::studio_uninstall,
            commands::hardware_profile,
            commands::app_version,
            tts::tts_speak,
            update::update_check,
            update::update_install,
            commands::app_paths,
            commands::runtime_status,
            commands::runtime_ensure,
            commands::runtime_prism_status,
            commands::runtime_prism_ensure,
            commands::runtime_prism_cancel,
            commands::runtime_prism_retry_cuda,
            commands::models_search,
            commands::runtime_check,
            commands::runtime_prune,
            commands::models_readme,
            commands::models_author_avatars,
            commands::server_live,
            commands::models_quants,
            commands::models_access,
            commands::hf_whoami,
            commands::hf_login,
            commands::hf_logout,
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
            // AgenticOw: o fork do DeepSeek Harness, dentro da janela.
            commands_agenticow::agenticow_status,
            commands_agenticow::agenticow_start,
            commands_agenticow::agenticow_stop,
            commands_agenticow::agenticow_uninstall,
            commands_agenticow::agenticow_set_locale,
            commands_agenticow::agenticow_refresh_catalog,
            commands_agenticow::agenticow_show,
            commands_agenticow::agenticow_hide,
            commands_agenticow::agenticow_set_bounds,
            commands_terminal::terminal_abrir_shell,
            commands_terminal::terminal_abrir_owcli,
            commands_terminal::terminal_anexar,
            commands_terminal::terminal_desanexar,
            commands_terminal::terminal_escrever,
            commands_terminal::terminal_redimensionar,
            commands_terminal::terminal_confirmar,
            commands_terminal::terminal_fechar,
            commands_terminal::terminal_visto,
            commands_terminal::terminal_listar,
            commands_terminal::area_de_transferencia_ler,
            commands_terminal::area_de_transferencia_escrever,
            commands_owcli::owcli_ligar,
            commands_owcli::owcli_disponivel,
            commands_owcli::owcli_historico,
            commands_owcli::owcli_modelos,
            commands_owcli::owcli_renomear,
            // Ajustar para esta máquina.
            commands_tuning::tune_advise,
            commands_tuning::tune_apply,
            comparison::compare_run,
            optimization::optimize_run,
            optimization::optimize_prepare_model,
            comparison::compare_apply,
            comparison::compare_latest,
            commands_tuning::tune_bench,
            commands_tuning::tune_bench_cancel,
            commands_tuning::tune_sweep,
            commands_tuning::tune_spec_bench,
            commands_tuning::tune_spec_cancel,
            commands_power::gpu_power_status,
            commands_power::gpu_power_set,
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
            externo::open_external,
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
            commands_providers::ninerouter_check,
            commands_providers::ninerouter_update,
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
            commands_jev::jev_config_get,
            commands_jev::jev_config_set,
            commands_jev::jev_status,
            commands_jev::jev_testar,
            commands_jev::jev_decidir_esforco,
            commands_jev::chat_reasoning_effort,
            commands_jev::jev_local_install,
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
                // A janela principal fechou: o painel do 9router não pode
                // segurar o app de pé sozinho. Sem isto o processo
                // continua vivo — e os sidecars junto, que é o que o
                // `shutdown_blocking` abaixo existe para evitar.
                tauri::RunEvent::WindowEvent {
                    label,
                    event: tauri::WindowEvent::Destroyed,
                    ..
                } if label == "main" => {
                    commands_providers::fechar_painel(app);
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
