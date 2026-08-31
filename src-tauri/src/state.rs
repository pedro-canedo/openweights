//! Estado global do app.

use lr_types::HardwareProfile;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use tauri::{AppHandle, Manager};

pub struct AppState {
    pub profile: HardwareProfile,
    pub data_dir: PathBuf,
    pub models_dir: PathBuf,
    pub store: Arc<lr_store::Store>,
    /// Cliente HF; recriado quando o token muda.
    pub hf: tokio::sync::Mutex<lr_models::HfClient>,
    pub runtime_mgr: lr_runtime::RuntimeManager,
    pub downloads: lr_models::DownloadManager,
    /// llama-server em Router mode (um processo para todos os modelos).
    pub server: tokio::sync::Mutex<Option<lr_engine::LlamaServer>>,
    /// PID do llama-server (0 = nenhum). Sobrevive a um mutex ocupado no exit.
    pub server_pid: Arc<AtomicU32>,
    /// Coletor das estatísticas de serviço (counters do /metrics): tracker de
    /// deltas + acumulado da sessão. O laço periódico nasce em `main.rs`.
    pub serve_stats: crate::serve_stats::ServeStatsCollector,
    /// Catálogo do OpenRouter já buscado, com o instante da busca.
    ///
    /// São 400+ modelos e a tela consulta a cada abertura; sem isto cada
    /// visita à aba viraria uma requisição de rede.
    pub openrouter_cache:
        tokio::sync::Mutex<Option<(std::time::Instant, Vec<lr_providers::OpenRouterModel>)>>,
    /// Node portátil, isolado do Node do sistema. Serve o 9router.
    pub node: lr_nodejs::NodeManager,
    /// 9router em execução, quando ligado.
    pub ninerouter: tokio::sync::Mutex<Option<lr_ninerouter::NineRouter>>,
    /// PID do 9router (0 = nenhum), pelo mesmo motivo do `server_pid`: no
    /// exit o mutex pode estar ocupado e o processo não pode sobreviver.
    pub ninerouter_pid: AtomicU32,
    /// DeepSeek Harness (dsh) em execução, quando aberto pelo caminho
    /// gerenciado.
    pub dsh: tokio::sync::Mutex<Option<lr_dshhost::DshHost>>,
    /// PID do dsh (0 = nenhum) — mesma rede de segurança do `ninerouter_pid`.
    pub dsh_pid: AtomicU32,
    /// Ponto de entrada único (Traefik), quando ligado. Opcional: nada no
    /// chat depende dele.
    pub gateway: tokio::sync::Mutex<Option<lr_gateway::Gateway>>,
    pub gateway_pid: AtomicU32,
    /// Cluster RPC (1 host + 1 worker na LAN).
    pub cluster: std::sync::Arc<lr_cluster::ClusterHost>,
    pub rpc_pid: Arc<AtomicU32>,
    /// Instante (ms) do último tráfego VISTO no motor, medido pelos counters
    /// do `/metrics`.
    ///
    /// É o sinal de "a máquina está livre" que a medição automática espera —
    /// e é medido, não presumido: capta inclusive o tráfego de harnesses
    /// externos que batem direto no llama-server, que é o caso real deste app
    /// desde que o DeepSeek Harness passou a rodar embutido.
    pub last_engine_use: AtomicI64,
    shutdown_done: AtomicBool,
}

const LEGACY_APP_ID: &str = "dev.riftapp.desktop";

/// Pasta de dados: herda a do Rift (mesmo volume) se a nova ainda estiver vazia.
fn resolve_data_dir(app: &AppHandle) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = app.path().app_data_dir()?;
    if let Some(parent) = dir.parent() {
        let legacy = parent.join(LEGACY_APP_ID);
        let has_new = dir.join("openweights.db").exists()
            || dir.join("rift.db").exists()
            || dir.join("models").exists();
        if !has_new && legacy.exists() {
            if dir.exists() {
                let _ = std::fs::remove_dir(&dir);
            }
            if std::fs::rename(&legacy, &dir).is_err() {
                return Ok(legacy);
            }
        }
    }
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Caminho do banco, com herança do nome antigo (`rift.db`) quando o novo
/// ainda não existe.
fn database_path(data_dir: &Path) -> PathBuf {
    let fresh = data_dir.join("openweights.db");
    let legacy = data_dir.join("rift.db");
    if !fresh.exists() && legacy.exists() {
        let _ = std::fs::rename(&legacy, &fresh);
    }
    if fresh.exists() { fresh } else { legacy }
}

impl AppState {
    pub fn new(
        profile: HardwareProfile,
        app: &AppHandle,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let data_dir = resolve_data_dir(app)?;
        std::fs::create_dir_all(&data_dir)?;
        let models_dir = data_dir.join("models");
        std::fs::create_dir_all(&models_dir)?;

        let db_path = database_path(&data_dir);
        let store = Arc::new(lr_store::Store::open(&db_path)?);
        let token = store.get_setting("hf_token").ok().flatten();

        // Copiados antes do literal: `profile` é movido logo na primeira
        // linha dele, e o Node precisa saber a plataforma.
        let (os, arch) = (profile.os.clone(), profile.arch.clone());

        let mut persist: lr_cluster::ClusterPersist = store
            .get_setting(lr_cluster::SETTING_KEY)
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if persist.instance_id.is_empty() {
            persist.instance_id = lr_cluster::new_instance_id();
            if let Ok(json) = serde_json::to_string(&persist) {
                let _ = store.set_setting(lr_cluster::SETTING_KEY, &json);
            }
        }
        let identity = lr_cluster::NodeIdentity::from_profile(
            persist.instance_id.clone(),
            &profile,
            lr_runtime::PINNED_TAG,
        );
        let store_save = store.clone();
        let save: lr_cluster::SaveFn = std::sync::Arc::new(move |p| {
            if let Ok(json) = serde_json::to_string(p) {
                let _ = store_save.set_setting(lr_cluster::SETTING_KEY, &json);
            }
        });
        let rpc_pid = Arc::new(AtomicU32::new(0));
        let data_rpc = data_dir.clone();
        let profile_rpc = profile.clone();
        let rpc_exe: lr_cluster::RpcExeFn = std::sync::Arc::new(move || {
            let variant = lr_runtime::select_variant(&profile_rpc);
            let dir = lr_runtime::runtime_dir(&data_rpc, lr_runtime::PINNED_TAG, variant);
            let exe = dir.join(lr_runtime::rpc_exe_name());
            exe.is_file().then_some(exe)
        });
        // O aviso do sistema vai direto pelo plugin de notificação — o app
        // não tem mais um host de desktop genérico (era das tools do agente).
        let app_notify = app.clone();
        let on_notify: lr_cluster::NotifyFn = std::sync::Arc::new(move |title, body| {
            crate::desktop_host::notify(&app_notify, title, body);
        });
        let rpc_pid_cb = Arc::clone(&rpc_pid);
        let on_pid: lr_cluster::PidFn = std::sync::Arc::new(move |pid| {
            rpc_pid_cb.store(pid, Ordering::SeqCst);
        });
        // Emprestar a GPU com o llama-server local no ar reserva a mesma VRAM
        // duas vezes. O aceite manual já recusa no comando; o automático não
        // passa por comando nenhum, então a pergunta desce até o crate.
        let server_pid = Arc::new(AtomicU32::new(0));
        let server_pid_cb = Arc::clone(&server_pid);
        let engine_busy: lr_cluster::EngineBusyFn =
            std::sync::Arc::new(move || server_pid_cb.load(Ordering::SeqCst) != 0);
        // Quem responde "quais dispositivos existem e quanto sobra em cada um"
        // é o próprio motor, com o peer já no ar. Nossa tabela de pinos e a
        // fração de 75% ficam só para o anúncio no mDNS, que acontece antes de
        // existir conexão para perguntar.
        let data_dev = data_dir.clone();
        let profile_dev = profile.clone();
        let list_devices: lr_cluster::DeviceListFn = std::sync::Arc::new(move |rpc_addr| {
            let variant = lr_runtime::select_variant(&profile_dev);
            let dir = lr_runtime::runtime_dir(&data_dev, lr_runtime::PINNED_TAG, variant);
            Box::pin(async move {
                match lr_advisor::devices::list_devices(&dir, Some(&rpc_addr)).await {
                    Ok(ds) => ds
                        .into_iter()
                        .map(|d| (d.name, d.free_bytes))
                        .collect::<Vec<_>>(),
                    Err(e) => {
                        log::warn!("não consegui listar os dispositivos: {e}");
                        Vec::new()
                    }
                }
            })
        });
        let cluster = lr_cluster::ClusterHost::new(
            identity,
            persist,
            lr_cluster::ClusterHooks {
                save,
                rpc_exe,
                on_notify,
                on_pid,
                engine_busy,
                list_devices,
            },
        );

        Ok(Self {
            profile,
            hf: tokio::sync::Mutex::new(lr_models::HfClient::new(token)),
            runtime_mgr: lr_runtime::RuntimeManager::new(data_dir.clone()),
            downloads: lr_models::DownloadManager::new(models_dir.clone()),
            server: tokio::sync::Mutex::new(None),
            server_pid,
            serve_stats: crate::serve_stats::ServeStatsCollector::new(),
            openrouter_cache: tokio::sync::Mutex::new(None),
            node: lr_nodejs::NodeManager::new(data_dir.join("providers"), os, arch),
            ninerouter: tokio::sync::Mutex::new(None),
            ninerouter_pid: AtomicU32::new(0),
            dsh: tokio::sync::Mutex::new(None),
            dsh_pid: AtomicU32::new(0),
            gateway: tokio::sync::Mutex::new(None),
            gateway_pid: AtomicU32::new(0),
            cluster,
            rpc_pid,
            store,
            data_dir,
            models_dir,
            last_engine_use: AtomicI64::new(0),
            shutdown_done: AtomicBool::new(false),
        })
    }

    /// Endereço do llama-server local. Erro claro quando o servidor ainda
    /// não subiu — a medição de desempenho (bench/tuning) depende dele.
    pub async fn llama_endpoint(&self) -> Result<lr_engine::Endpoint, String> {
        let guard = self.server.lock().await;
        let srv = guard
            .as_ref()
            .filter(|s| s.is_spawned())
            .ok_or("o motor de IA não está rodando — inicie o servidor local")?;
        Ok(lr_engine::Endpoint {
            base_url: srv.config().connect_url(),
            api_key: srv.config().api_key.clone(),
            headers: Vec::new(),
            dialect: lr_engine::Dialect::LlamaCpp,
        })
    }

    /// Encerra sidecars de forma síncrona no exit do app. Os processos filhos
    /// NÃO morrem sozinhos no Tauri (issue #3273) — este é o ponto garantido.
    ///
    /// Não usa `block_on`: no `Exit` o runtime Tokio já está a morrer e um
    /// `lock().await` no mutex do servidor deixava o llama-server órfão
    /// (VRAM cheia com o app fechado).
    pub fn shutdown_blocking(&self) {
        if self.shutdown_done.swap(true, Ordering::SeqCst) {
            self.kill_orphan_pids();
            return;
        }
        if let Ok(mut guard) = self.server.try_lock() {
            if let Some(srv) = guard.as_mut() {
                srv.stop_blocking();
            }
            *guard = None;
        }
        if let Ok(mut guard) = self.ninerouter.try_lock() {
            if let Some(nr) = guard.as_mut() {
                nr.stop_blocking();
            }
            *guard = None;
        }
        if let Ok(mut guard) = self.dsh.try_lock() {
            if let Some(d) = guard.as_mut() {
                d.stop_blocking();
            }
            *guard = None;
        }
        if let Ok(mut guard) = self.gateway.try_lock() {
            if let Some(gw) = guard.as_mut() {
                gw.stop_blocking();
            }
            *guard = None;
        }
        self.cluster.stop_blocking();
        self.kill_orphan_pids();
    }

    /// Mata o que sobrou de cada sidecar pelo PID guardado.
    ///
    /// É a rede de segurança para quando o mutex estava ocupado: sem ela um
    /// `node` do 9router fica segurando a porta com o app já fechado.
    fn kill_orphan_pids(&self) {
        for slot in [
            &*self.server_pid,
            &self.ninerouter_pid,
            &self.dsh_pid,
            &self.gateway_pid,
            &*self.rpc_pid,
        ] {
            let pid = slot.swap(0, Ordering::SeqCst);
            if pid != 0 {
                lr_engine::kill_process_tree(pid);
            }
        }
    }
}
