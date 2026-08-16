//! Estado global do app.

use lr_types::HardwareProfile;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use tauri::{AppHandle, Manager};

pub struct AppState {
    pub profile: HardwareProfile,
    pub data_dir: PathBuf,
    pub models_dir: PathBuf,
    pub store: Arc<lr_store::Store>,
    /// Catálogo de ferramentas do agente (nativas + conectores MCP).
    /// Fica atrás de um cadeado porque adicionar ou remover um conector
    /// reconstrói o catálogo com o app aberto.
    pub tools: std::sync::RwLock<Arc<lr_tools::ToolRegistry>>,
    /// Conectores MCP (processos filhos e sessões HTTP).
    pub mcp: lr_mcp::McpHost,
    /// Índice do projeto para busca por significado.
    pub rag: Arc<lr_rag::RagHandle>,
    /// Executa e acompanha as execuções do agente.
    pub agent: lr_agent::AgentHost,
    /// Cliente HF; recriado quando o token muda.
    pub hf: tokio::sync::Mutex<lr_models::HfClient>,
    pub runtime_mgr: lr_runtime::RuntimeManager,
    pub downloads: lr_models::DownloadManager,
    /// llama-server em Router mode (um processo para todos os modelos).
    pub server: tokio::sync::Mutex<Option<lr_engine::LlamaServer>>,
    /// PID do llama-server (0 = nenhum). Sobrevive a um mutex ocupado no exit.
    pub server_pid: AtomicU32,
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

/// Caminho do banco. O índice do projeto abre uma conexão própria para o
/// MESMO arquivo (o modo WAL permite), então os dois precisam concordar aqui.
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

        // A extensão de vetores entra como auto-extension do SQLite e só vale
        // para conexões abertas DEPOIS deste ponto — por isso vem antes de
        // abrir o banco.
        lr_rag::register_vector_extension();

        let db_path = database_path(&data_dir);
        let store = Arc::new(lr_store::Store::open(&db_path)?);
        let token = store.get_setting("hf_token").ok().flatten();

        // Um fechamento abrupto deixa runs "em andamento" no banco; marcá-los
        // como falha evita que a interface mostre execução fantasma.
        if let Ok(n) = store.fail_orphan_runs()
            && n > 0
        {
            log::info!("{n} execução(ões) interrompida(s) por fechamento anterior");
        }

        let rag = Arc::new(lr_rag::RagHandle::new(db_path.clone()));
        let mcp = lr_mcp::McpHost::new(store.clone());
        let tools = build_tool_registry(&mcp, &store, &rag)?;

        let agent = lr_agent::AgentHost::new(
            store.clone(),
            tools.clone(),
            lr_agent::AgentConfig::new(data_dir.clone()),
        );

        Ok(Self {
            profile,
            hf: tokio::sync::Mutex::new(lr_models::HfClient::new(token)),
            runtime_mgr: lr_runtime::RuntimeManager::new(data_dir.clone()),
            downloads: lr_models::DownloadManager::new(models_dir.clone()),
            server: tokio::sync::Mutex::new(None),
            server_pid: AtomicU32::new(0),
            tools: std::sync::RwLock::new(tools),
            mcp,
            rag,
            agent,
            store,
            data_dir,
            models_dir,
            shutdown_done: AtomicBool::new(false),
        })
    }

    /// Catálogo atual (foto do momento).
    pub fn tools(&self) -> Arc<lr_tools::ToolRegistry> {
        self.tools.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Refaz o catálogo depois de adicionar ou remover um conector, sem
    /// precisar reiniciar o app. Execuções em andamento seguem com o
    /// catálogo que já tinham.
    pub fn rebuild_tools(&self) -> Result<(), String> {
        let tools =
            build_tool_registry(&self.mcp, &self.store, &self.rag).map_err(|e| e.to_string())?;
        *self.tools.write().unwrap_or_else(|e| e.into_inner()) = tools.clone();
        self.agent.set_registry(tools);
        Ok(())
    }

    /// Endereço do llama-server para o agente falar. Erro claro quando o
    /// servidor ainda não subiu — é a causa mais comum de "o agente não faz
    /// nada".
    pub async fn agent_endpoint(&self) -> Result<lr_agent::Endpoint, String> {
        let guard = self.server.lock().await;
        let srv = guard
            .as_ref()
            .filter(|s| s.is_spawned())
            .ok_or("o motor de IA não está rodando — inicie o servidor local")?;
        Ok(lr_agent::Endpoint {
            base_url: srv.config().connect_url(),
            api_key: srv.config().api_key.clone(),
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
            self.kill_orphan_pid();
            return;
        }
        if let Ok(mut guard) = self.server.try_lock() {
            if let Some(srv) = guard.as_mut() {
                srv.stop_blocking();
            }
            *guard = None;
        }
        self.kill_orphan_pid();
    }

    fn kill_orphan_pid(&self) {
        let pid = self.server_pid.swap(0, Ordering::SeqCst);
        if pid != 0 {
            lr_engine::kill_process_tree(pid);
        }
    }
}

/// Monta o catálogo: ferramentas nativas + conectores MCP + memória + índice.
fn build_tool_registry(
    mcp: &lr_mcp::McpHost,
    store: &Arc<lr_store::Store>,
    rag: &Arc<lr_rag::RagHandle>,
) -> Result<Arc<lr_tools::ToolRegistry>, Box<dyn std::error::Error>> {
    // O host do MCP devolve o registro já com as nativas e um provedor por
    // conector configurado.
    let shared = mcp.build_registry()?;
    let mut registry = Arc::try_unwrap(shared).unwrap_or_else(|still_shared| {
        // Não deveria acontecer (o Arc acabou de nascer), mas se acontecer é
        // melhor um catálogo só com as nativas do que um panic na abertura.
        log::warn!("catálogo do MCP já compartilhado; sigo só com as nativas");
        let _ = still_shared;
        lr_tools::builtin_registry()
    });

    // A pasta certa vem do contexto de cada execução, por isso `None` aqui.
    for tool in lr_memory::memory_tools(store.clone(), None) {
        registry.register(tool);
    }
    for tool in lr_rag::rag_tools(rag.clone()) {
        registry.register(tool);
    }
    Ok(Arc::new(registry))
}
