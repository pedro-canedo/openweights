//! Loop de telemetria: amostra o hardware a 1 Hz e emite o evento
//! `telemetry` para a UI (baixa frequência — IPC de eventos é adequado;
//! streaming de tokens NÃO passa por aqui).

use lr_types::HardwareProfile;
use tauri::{AppHandle, Emitter};

pub fn spawn_loop(app: AppHandle, profile: &HardwareProfile) {
    let profile = profile.clone();
    tauri::async_runtime::spawn(async move {
        // Monitor mantém estado (leituras diferenciais de CPU e handle NVML).
        let mut monitor = lr_hw::Monitor::new(&profile);
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let sample = tokio::task::block_in_place(|| monitor.sample());
            if let Err(e) = app.emit("telemetry", &sample) {
                log::debug!("emissão de telemetria falhou (janela fechando?): {e}");
                break;
            }
        }
    });
}
