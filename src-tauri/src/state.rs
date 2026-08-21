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
    /// Quem fala com o computador em volta (área de transferência, avisos,
    /// abrir arquivo). Mora aqui porque o catálogo é remontado sem o
    /// `AppHandle` à mão.
    desktop: Arc<dyn lr_desktop::DesktopHost>,
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
    pub server_pid: Arc<AtomicU32>,
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
    /// Ponto de entrada único (Traefik), quando ligado. Opcional: nada no
    /// chat depende dele.
    pub gateway: tokio::sync::Mutex<Option<lr_gateway::Gateway>>,
    pub gateway_pid: AtomicU32,
    /// Cluster RPC (1 host + 1 worker na LAN).
    pub cluster: std::sync::Arc<lr_cluster::ClusterHost>,
    pub rpc_pid: Arc<AtomicU32>,
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
        // O mesmo acerto vale para as automações: com o run já marcado como
        // falha mas o status da tarefa ainda vazio, o card diria "rodando"
        // para sempre.
        if let Ok(n) = store.fail_orphan_scheduled_tasks()
            && n > 0
        {
            log::info!("{n} automação(ões) marcada(s) como interrompida(s)");
        }

        let rag = Arc::new(lr_rag::RagHandle::new(db_path.clone()));
        let mcp = lr_mcp::McpHost::new(store.clone());
        let desktop: Arc<dyn lr_desktop::DesktopHost> =
            Arc::new(crate::desktop_host::TauriDesktop::new(app.clone()));
        let tools = build_tool_registry(&mcp, &store, &rag, &desktop)?;

        let mut agent_config = lr_agent::AgentConfig::new(data_dir.clone());
        // Onde achar o Node para o Code Mode. É um fechamento, e não um
        // caminho: o Node portátil pode ser baixado DEPOIS de o app subir, e
        // um valor lido aqui ficaria `None` até alguém reiniciar. Construir
        // um `NodeManager` a cada consulta é barato — dois caminhos e duas
        // strings, sem I/O.
        let (providers_dir, os_n, arch_n) = (
            data_dir.join("providers"),
            profile.os.clone(),
            profile.arch.clone(),
        );
        agent_config.node_path = Some(Arc::new(move || {
            lr_nodejs::NodeManager::new(providers_dir.clone(), os_n.clone(), arch_n.clone())
                .node_exe()
        }));
        let agent = lr_agent::AgentHost::new(store.clone(), tools.clone(), agent_config);

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
        let desktop_n = desktop.clone();
        let on_notify: lr_cluster::NotifyFn = std::sync::Arc::new(move |title, body| {
            let _ = desktop_n.notify(title, body);
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
            openrouter_cache: tokio::sync::Mutex::new(None),
            node: lr_nodejs::NodeManager::new(data_dir.join("providers"), os, arch),
            ninerouter: tokio::sync::Mutex::new(None),
            ninerouter_pid: AtomicU32::new(0),
            gateway: tokio::sync::Mutex::new(None),
            gateway_pid: AtomicU32::new(0),
            cluster,
            rpc_pid,
            tools: std::sync::RwLock::new(tools),
            mcp,
            desktop,
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
        let tools = build_tool_registry(&self.mcp, &self.store, &self.rag, &self.desktop)
            .map_err(|e| e.to_string())?;
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
            headers: Vec::new(),
            dialect: lr_engine::Dialect::LlamaCpp,
        })
    }

    /// Endereço do modelo DESTA conversa, que pode não ser o servidor local.
    ///
    /// Existe ao lado de `agent_endpoint` em vez de substituí-lo: embeddings
    /// (RAG), medição de desempenho, consolidação de memória e o relógio das
    /// automações precisam continuar falando com o llama.cpp local, e trocar
    /// o endpoint deles por um provedor remoto inutilizaria o índice do
    /// projeto (outro modelo de embedding) e mediria a máquina errada.
    pub async fn endpoint_for(&self, model_ref: &str) -> Result<lr_agent::Endpoint, String> {
        let referencia = lr_providers::ModelRef::parse(model_ref);
        if referencia.is_local() {
            return self.agent_endpoint().await;
        }

        let cfg = crate::commands_providers::load_config(self);
        let resolvido = cfg
            .resolve(referencia.provider, None)
            .map_err(|e| e.to_string())?;
        Ok(lr_agent::Endpoint {
            base_url: resolvido.base_url,
            api_key: resolvido.api_key,
            headers: resolvido.headers,
            // Provedor de terceiros não conhece as extensões do llama.cpp.
            dialect: lr_engine::Dialect::OpenAi,
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
        // Runs vivos: cancela e deixa cada um fechar pelo caminho normal
        // enquanto o processo ainda respira (trilha íntegra, desfecho
        // "cancelado"). NÃO carimbamos o banco aqui: o carimbo correndo
        // contra um run vivo rouba o `seq` do próximo evento real e pode
        // contradizer um desfecho legítimo — o que não fechar a tempo é
        // carimbado com causa no próximo boot (`fail_orphan_runs`), onde
        // não há corrida nenhuma.
        let vivos = self.agent.cancel_all();
        if vivos > 0 {
            log::info!("{vivos} execução(ões) cancelada(s) pelo fechamento do app");
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

/// Monta o catálogo: ferramentas nativas + conectores MCP + memória + índice.
fn build_tool_registry(
    mcp: &lr_mcp::McpHost,
    store: &Arc<lr_store::Store>,
    rag: &Arc<lr_rag::RagHandle>,
    desktop: &Arc<dyn lr_desktop::DesktopHost>,
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
    // Rodar o que escreve, versionar e analisar dados: sem isso o agente
    // edita no escuro.
    lr_codetools::register_all(&mut registry);
    for tool in lr_gittools::git_tools() {
        registry.register(tool);
    }
    for tool in lr_datatools::data_tools() {
        registry.register(tool);
    }
    // Internet: a configuração da busca (provedor e chave) fica nas
    // preferências; sem chave, o padrão não exige nenhuma.
    let web_cfg = Arc::new(lr_webtools::WebConfig::from_json_or_default(
        store.get_setting("web.config").ok().flatten().as_deref(),
    ));
    for tool in lr_webtools::web_tools(web_cfg) {
        registry.register(tool);
    }
    // O computador em volta do projeto: copiar, avisar, abrir.
    for tool in lr_desktop::desktop_tools(desktop.clone()) {
        registry.register(tool);
    }
    Ok(Arc::new(registry))
}
