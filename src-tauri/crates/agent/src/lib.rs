//! Laço do agente: passos, ferramentas, confirmações e trilha de execução.
//!
//! A UI só observa e confirma. Quem conduz o run é o [`AgentHost`]: sobe o
//! laço, entrega eventos e espera decisões de aprovação sem bloquear o app.

mod checkpoint;
mod codemode;
mod events;
mod judge;
mod menu;
mod output_stream;
mod plan_tools;
mod prompt;
mod reliability;
pub(crate) mod run;
mod scout;
mod subagent;
mod verify;

use crate::run::{RunDeps, execute_run};
use lr_engine::ChatMessage;
use lr_store::Store;
use lr_tools::ToolRegistry;
use lr_types::agent::{ApprovalDecision, RunEventKind, RunOptions, RunStatus, UsageStats};
use lr_types::scout::WorkMode;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::sync::{Notify, oneshot};

pub use checkpoint::{checkpoint_from_row, restore_blocking};
pub use events::{EventCallback, EventSink, now_ms};
pub use menu::group_of;
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
#[derive(Clone)]
pub struct AgentConfig {
    /// Pasta de dados do app — checkpoints de cópia moram aqui.
    pub store_dir: PathBuf,
    /// Teto de bytes que uma ferramenta pode devolver ao modelo.
    pub max_output_bytes: usize,
    /// Quanto tempo o run espera uma confirmação antes de cancelar.
    pub approval_timeout: Duration,
    /// Fração da janela de contexto que dispara compactação (0.0–1.0).
    pub context_ratio: f32,
    /// Prazo até o primeiro pedaço do stream. Generoso: processar um prompt
    /// de 8k tokens em CPU leva minutos, e isso não é travamento.
    pub first_token_timeout: Duration,
    /// Prazo entre pedaços depois que a geração começou. Silêncio além disso
    /// é servidor travado — sem este relógio, um stream pendurado segurava o
    /// run para sempre.
    pub idle_timeout: Duration,
    /// Teto de UMA chamada de ferramenta, do despacho ao resultado. É rede de
    /// segurança, não política: fica ACIMA do maior relógio interno (um
    /// comando de terminal pode pedir até 3600s) e só dispara quando uma
    /// ferramenta pendurou sem que nenhum relógio de baixo visse — antes
    /// disso, o run ficava preso para sempre, sem evento nenhum.
    pub tool_call_timeout: Duration,
    /// Onde achar o Node que o Code Mode usa, perguntado A CADA execução.
    ///
    /// É uma função, e não um caminho, porque o app instala o Node **depois**
    /// de subir: um valor fixado no boot ficaria `None` para sempre na
    /// máquina que ainda não tinha baixado, e o Code Mode nunca passaria a
    /// funcionar sem reiniciar o app. Devolver `None` aqui cai no `node` do
    /// PATH.
    pub node_path: Option<Arc<dyn Fn() -> Option<PathBuf> + Send + Sync>>,
}

impl std::fmt::Debug for AgentConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentConfig")
            .field("store_dir", &self.store_dir)
            .field("max_output_bytes", &self.max_output_bytes)
            .field("approval_timeout", &self.approval_timeout)
            .field("context_ratio", &self.context_ratio)
            .field("first_token_timeout", &self.first_token_timeout)
            .field("idle_timeout", &self.idle_timeout)
            .field("tool_call_timeout", &self.tool_call_timeout)
            .field("node_path", &self.node_path.is_some())
            .finish()
    }
}

impl AgentConfig {
    pub fn new(store_dir: PathBuf) -> Self {
        Self {
            store_dir,
            max_output_bytes: 30_000,
            approval_timeout: Duration::from_secs(15 * 60),
            context_ratio: 0.85,
            first_token_timeout: Duration::from_secs(180),
            idle_timeout: Duration::from_secs(90),
            tool_call_timeout: Duration::from_secs(3_900),
            node_path: None,
        }
    }
}

/// Onde o modelo desta execução atende.
///
/// Nem sempre é o llama-server local: uma conversa apontada para o OpenRouter
/// ou para o 9router resolve para o endereço deles. Por isso os dois campos
/// extras — `headers` carrega a atribuição que o provedor pede, e `dialect`
/// diz se as extensões do llama.cpp podem viajar no corpo da requisição.
#[derive(Debug, Clone, Default)]
pub struct Endpoint {
    pub base_url: String,
    pub api_key: Option<String>,
    /// Cabeçalhos extras exigidos pelo provedor. Vazio no servidor local.
    pub headers: Vec<(String, String)>,
    pub dialect: lr_engine::Dialect,
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

/// Faxina garantida do fim de um run: tira o handle do mapa de vivos e
/// devolve a trava do workspace no `Drop` — isto é, mesmo quando o caminho
/// normal explodiu em pânico. É deliberadamente um guarda, não código no fim
/// da task: código no fim não roda quando algo antes dele cai.
struct FaxinaDoRun {
    id: String,
    vivos: Arc<Mutex<HashMap<String, Arc<RunHandle>>>>,
    travas: Arc<Mutex<HashMap<String, String>>>,
}

impl Drop for FaxinaDoRun {
    fn drop(&mut self) {
        self.vivos
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
        self.travas
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
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
    /// Execuções VIVAS. Cada uma se remove ao terminar: sem isso o mapa
    /// cresce a sessão inteira segurando `EventSink`, canal da interface e o
    /// que mais o ouvinte tiver capturado. Ele também é a resposta para
    /// "esta execução ainda está de pé?".
    runs: Arc<Mutex<HashMap<String, Arc<RunHandle>>>>,
    /// Pasta de projeto de cada run vivo — a trava de "um por workspace".
    workspaces: Arc<Mutex<HashMap<String, String>>>,
}

impl AgentHost {
    pub fn new(store: Arc<Store>, registry: Arc<ToolRegistry>, config: AgentConfig) -> Self {
        Self {
            store,
            registry: RwLock::new(registry),
            config: Arc::new(config),
            runs: Arc::new(Mutex::new(HashMap::new())),
            workspaces: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn start(
        &self,
        req: StartRun,
        on_event: Option<EventCallback>,
    ) -> Result<Arc<RunHandle>, String> {
        // Uma execução por pasta de projeto. Dois runs no mesmo workspace se
        // atropelam de verdade: escrevem no mesmo arquivo, cruzam checkpoints
        // e cada um "verifica" o trabalho do outro. O cenário real é o
        // relógio disparando uma automação enquanto a pessoa usa o agente.
        // A trava é o próprio mapa de runs vivos (que se limpa sozinho) —
        // nunca um arquivo em disco, que um travamento deixaria para sempre.
        let workspace = req
            .options
            .workspace_dir
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_string);
        if let Some(dir) = &workspace {
            let ocupados = self.workspaces.lock().unwrap_or_else(|e| e.into_inner());
            if ocupados.values().any(|w| w == dir) {
                return Err(
                    "já existe uma execução em andamento nesta pasta de projeto — \
                     espere-a terminar ou cancele-a antes de começar outra"
                        .to_string(),
                );
            }
        }

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
        // O modo escolhido fica no run (a interface o perdia num reload).
        // Persistimos o modo COMO PEDIDO — a normalização Agent→Loop é
        // detalhe do laço, não da escolha da pessoa.
        if let Err(e) = self.store.set_run_work_mode(&id, req.work_mode) {
            log::warn!("run {id}: não gravou o modo de trabalho: {e}");
        }

        let mut sink = EventSink::new(&id, Some(self.store.clone()));
        if let Some(cb) = on_event {
            sink = sink.with_listener(cb);
        }
        let handle = RunHandle::new(id.clone(), sink);
        self.runs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.clone(), handle.clone());
        if let Some(dir) = workspace {
            self.workspaces
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(id, dir);
        }

        let deps = RunDeps {
            store: self.store.clone(),
            registry: self.registry().clone(),
            config: self.config.clone(),
        };
        let h = handle.clone();
        let store = self.store.clone();
        let chat_id = req.options.chat_id;
        let model = req.options.model.clone();
        let faxina = FaxinaDoRun {
            id: h.id.clone(),
            vivos: self.runs.clone(),
            travas: self.workspaces.clone(),
        };
        tokio::spawn(async move {
            // A faxina vive DENTRO da task e é um guarda de `Drop`: aconteça
            // o que acontecer daqui para baixo — inclusive um pânico no
            // próprio código de socorro — o handle sai do mapa de vivos e a
            // pasta de projeto é destravada. Antes disso, um pânico deixava
            // o workspace recusando execuções até reiniciar o app.
            let faxina = faxina;
            // O laço roda numa task própria para o pânico ficar contido no
            // JoinError em vez de matar a execução sem rastro: um run nunca
            // pode acabar sem um `run.finished` que diga o porquê.
            let caiu = tokio::spawn(execute_run(req, h.clone(), deps)).await;
            if let Err(e) = caiu {
                let summary = if e.is_panic() {
                    "A execução caiu por um erro interno do aplicativo. O que \
                     aparece na trilha foi feito; o restante, não. Detalhes \
                     estão no log."
                } else {
                    "A execução foi interrompida pelo encerramento do \
                     aplicativo antes de terminar."
                };
                log::error!("run {}: laço morreu sem epílogo: {e}", faxina.id);
                // Só carimba run ainda aberto: um pânico DEPOIS do
                // `finish_run` legítimo (no episódio de memória, num
                // listener do sink) não pode sobrescrever Done com Error nem
                // duplicar a âncora que `keep_in_chat` já gravou. O evento
                // sai pelo sink vivo (que também o persiste) — por isso
                // NÃO usamos o `reap_run`, que inseriria um segundo
                // `run.finished` sintético na trilha.
                match store.finish_run_if_open(&faxina.id, RunStatus::Error, summary) {
                    Ok(false) => {
                        // A linha já tinha desfecho — mas a trilha pode não
                        // ter: o pânico cabe entre o `finish_run` e o emit
                        // do `run.finished`. Completa a trilha com o
                        // desfecho REAL da linha, senão a interface fica
                        // "trabalhando" sem fim para um run que terminou.
                        if let Ok(false) = store.run_has_finish_event(&faxina.id)
                            && let Ok(Some(r)) = store.get_run(&faxina.id)
                        {
                            h.sink().emit(RunEventKind::RunFinished {
                                status: r.status,
                                summary: r.summary.unwrap_or_default(),
                                usage: r.usage.unwrap_or_default(),
                            });
                        }
                    }
                    Ok(true) => {
                        if chat_id > 0
                            && let Err(e) = store.add_message(
                                chat_id,
                                "assistant",
                                summary,
                                None,
                                None,
                                None,
                                Some(&model),
                                None,
                                Some(&faxina.id),
                            )
                        {
                            log::error!(
                                "run {}: não ancorou o fim de socorro no chat: {e}",
                                faxina.id
                            );
                        }
                        h.sink().emit(RunEventKind::RunFinished {
                            status: RunStatus::Error,
                            summary: summary.to_string(),
                            usage: UsageStats::default(),
                        });
                    }
                    Err(e) => {
                        log::error!("run {}: não gravou o fim de socorro: {e}", faxina.id);
                    }
                }
            }
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

    /// Quantas execuções estão de pé agora.
    ///
    /// O mapa só guarda execução viva (cada uma se remove ao terminar), então
    /// contar é a resposta para "posso derrubar o motor?".
    pub fn live_count(&self) -> usize {
        self.runs.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Cancela todas as execuções vivas (fechamento do app). Devolve quantas.
    /// É melhor esforço: se o runtime viver o bastante, cada uma fecha pelo
    /// caminho normal de cancelamento, com trilha íntegra; o que não fechar
    /// é carimbado com causa no próximo boot (`fail_orphan_runs`).
    pub fn cancel_all(&self) -> usize {
        let handles: Vec<Arc<RunHandle>> = self
            .runs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        for h in &handles {
            h.cancel();
        }
        handles.len()
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
