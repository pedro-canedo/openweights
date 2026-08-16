//! Laço do agente: passos, ferramentas, confirmações e trilha de execução.
//!
//! A UI só observa e confirma. Quem conduz o run é o [`AgentHost`]: sobe o
//! laço, entrega eventos e espera decisões de aprovação sem bloquear o app.

mod checkpoint;
mod events;
mod plan_tools;
mod prompt;
mod reliability;
mod run;
mod scout;
mod verify;

use crate::run::{RunDeps, execute_run};
use lr_engine::ChatMessage;
use lr_store::Store;
use lr_tools::ToolRegistry;
use lr_types::agent::{ApprovalDecision, RunOptions};
use lr_types::scout::WorkMode;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::sync::{Notify, oneshot};

pub use checkpoint::{checkpoint_from_row, restore_blocking};
pub use events::{EventCallback, EventSink, now_ms};
pub use plan_tools::{AskUser, PlanCreate, PlanTools, PlanUpdate, SharedPlan, TaskComplete};
pub use prompt::{PromptContext, build_system_prompt};
pub use run::{append_prompt, history_from_messages};
pub use scout::{MAX_STEPS_PER_TASK, MAX_TASKS, decompose, menu_for, single_task_plan};

/// Identificador com prefixo, único o bastante para um processo local.
pub fn new_id(prefix: &str) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(1);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{n}", now_ms())
}

/// Ajustes do host (do processo, não da conversa).
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Pasta de dados do app — checkpoints de cópia moram aqui.
    pub store_dir: PathBuf,
    /// Teto de bytes que uma ferramenta pode devolver ao modelo.
    pub max_output_bytes: usize,
    /// Quanto tempo o run espera uma confirmação antes de cancelar.
    pub approval_timeout: Duration,
    /// Fração da janela de contexto que dispara compactação (0.0–1.0).
    pub context_ratio: f32,
}

impl AgentConfig {
    pub fn new(store_dir: PathBuf) -> Self {
        Self {
            store_dir,
            max_output_bytes: 30_000,
            approval_timeout: Duration::from_secs(15 * 60),
            context_ratio: 0.85,
        }
    }
}

/// Onde o llama-server está escutando.
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub base_url: String,
    pub api_key: Option<String>,
}

/// Pedido para começar uma execução.
pub struct StartRun {
    pub prompt: String,
    pub history: Vec<ChatMessage>,
    pub memory: Vec<String>,
    pub options: RunOptions,
    pub endpoint: Endpoint,
    /// Como o agente trabalha nesta conversa (Scout Rule).
    ///
    /// Mora aqui, e não em `RunOptions`, porque `lr_types` está congelado
    /// nesta fase. `WorkMode::Agent` é o comportamento de sempre.
    pub work_mode: WorkMode,
    /// Plano já pronto (a pessoa aprovou o que o modo planejamento propôs).
    /// Com ele, o laço executa direto em vez de dividir o objetivo de novo.
    pub plan: Option<lr_types::scout::TaskPlan>,
}

/// Controle de uma execução em andamento.
pub struct RunHandle {
    pub id: String,
    sink: Arc<EventSink>,
    cancel: Notify,
    cancelled: AtomicBool,
    pending: Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>,
}

impl RunHandle {
    fn new(id: String, sink: EventSink) -> Arc<Self> {
        Arc::new(Self {
            id,
            sink: Arc::new(sink),
            cancel: Notify::new(),
            cancelled: AtomicBool::new(false),
            pending: Mutex::new(HashMap::new()),
        })
    }

    pub fn sink(&self) -> Arc<EventSink> {
        self.sink.clone()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.cancel.notify_waiters();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// Future que completa quando o run é cancelado.
    pub async fn cancelled(&self) {
        let notified = self.cancel.notified();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }

    pub fn register_pending(&self, call_id: &str) -> oneshot::Receiver<ApprovalDecision> {
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(call_id.to_string(), tx);
        rx
    }

    pub fn clear_pending(&self, call_id: &str) {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(call_id);
    }

    /// Resolve uma confirmação pendente. `false` se o call_id já não espera.
    pub fn resolve(&self, call_id: &str, decision: ApprovalDecision) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(call_id)
            .and_then(|tx| tx.send(decision).ok())
            .is_some()
    }
}

/// Ponto de entrada: sobe e acompanha execuções.
pub struct AgentHost {
    store: Arc<Store>,
    /// Catálogo de ferramentas. Fica atrás de um cadeado porque conectores
    /// podem ser adicionados ou removidos com o app aberto — cada execução
    /// pega a foto do catálogo no momento em que começa.
    registry: RwLock<Arc<ToolRegistry>>,
    config: Arc<AgentConfig>,
    runs: Mutex<HashMap<String, Arc<RunHandle>>>,
}

impl AgentHost {
    pub fn new(store: Arc<Store>, registry: Arc<ToolRegistry>, config: AgentConfig) -> Self {
        Self {
            store,
            registry: RwLock::new(registry),
            config: Arc::new(config),
            runs: Mutex::new(HashMap::new()),
        }
    }

    pub fn start(
        &self,
        req: StartRun,
        on_event: Option<EventCallback>,
    ) -> Result<Arc<RunHandle>, String> {
        let id = new_id("run");
        let chat_id = (req.options.chat_id > 0).then_some(req.options.chat_id);
        self.store
            .create_run(
                &id,
                chat_id,
                &req.options.model,
                req.options.mode,
                req.options.yolo(),
                req.options.workspace_dir.as_deref(),
                &req.prompt,
            )
            .map_err(|e| e.to_string())?;

        let mut sink = EventSink::new(&id, Some(self.store.clone()));
        if let Some(cb) = on_event {
            sink = sink.with_listener(cb);
        }
        let handle = RunHandle::new(id.clone(), sink);
        self.runs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, handle.clone());

        let deps = RunDeps {
            store: self.store.clone(),
            registry: self.registry().clone(),
            config: self.config.clone(),
        };
        let h = handle.clone();
        tokio::spawn(async move {
            execute_run(req, h, deps).await;
        });
        Ok(handle)
    }

    /// Troca o catálogo de ferramentas (conector adicionado ou removido).
    /// Execuções em andamento seguem com o catálogo que já tinham.
    pub fn set_registry(&self, registry: Arc<ToolRegistry>) {
        *self.registry.write().unwrap_or_else(|e| e.into_inner()) = registry;
    }

    fn registry(&self) -> Arc<ToolRegistry> {
        self.registry
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn get(&self, id: &str) -> Option<Arc<RunHandle>> {
        self.runs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    pub fn cancel(&self, id: &str) {
        if let Some(h) = self.get(id) {
            h.cancel();
        }
    }

    pub fn resolve(&self, run_id: &str, call_id: &str, decision: ApprovalDecision) -> bool {
        self.get(run_id)
            .is_some_and(|h| h.resolve(call_id, decision))
    }
}
