//! Estado global do app.

use lr_types::HardwareProfile;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

pub struct AppState {
    pub profile: HardwareProfile,
    pub data_dir: PathBuf,
    pub models_dir: PathBuf,
    pub store: lr_store::Store,
    /// Cliente HF; recriado quando o token muda.
    pub hf: tokio::sync::Mutex<lr_models::HfClient>,
    pub runtime_mgr: lr_runtime::RuntimeManager,
    pub downloads: lr_models::DownloadManager,
    /// llama-server em Router mode (um processo para todos os modelos).
    pub server: tokio::sync::Mutex<Option<lr_engine::LlamaServer>>,
    shutdown_done: std::sync::Mutex<bool>,
}

impl AppState {
    pub fn new(
        profile: HardwareProfile,
        app: &AppHandle,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let data_dir = app.path().app_data_dir()?;
        std::fs::create_dir_all(&data_dir)?;
        let models_dir = data_dir.join("models");
        std::fs::create_dir_all(&models_dir)?;

        let store = lr_store::Store::open(&data_dir.join("llamarunner.db"))?;
        let token = store.get_setting("hf_token").ok().flatten();

        Ok(Self {
            profile,
            hf: tokio::sync::Mutex::new(lr_models::HfClient::new(token)),
            runtime_mgr: lr_runtime::RuntimeManager::new(data_dir.clone()),
            downloads: lr_models::DownloadManager::new(models_dir.clone()),
            server: tokio::sync::Mutex::new(None),
            store,
            data_dir,
            models_dir,
            shutdown_done: std::sync::Mutex::new(false),
        })
    }

    /// Encerra sidecars de forma síncrona no exit do app. Os processos filhos
    /// NÃO morrem sozinhos no Tauri (issue #3273) — este é o ponto garantido.
    pub fn shutdown_blocking(&self) {
        let mut done = self.shutdown_done.lock().unwrap();
        if *done {
            return;
        }
        *done = true;

        tauri::async_runtime::block_on(async {
            if let Some(srv) = self.server.lock().await.as_mut() {
                srv.stop().await;
            }
        });
    }
}
