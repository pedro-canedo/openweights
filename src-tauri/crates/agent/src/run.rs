//! O laço do agente: pensar, agir, ler o resultado, repetir — até responder.
//!
//! Duas metades vivem aqui:
//! - [`ToolRunner`]: uma chamada de ferramenta do começo ao fim (política,
//!   confirmação, checkpoint, execução, trilha). Não conhece o modelo, o que
//!   a torna testável sem rede.
//! - [`execute_run`]: os passos, o streaming e os guard-rails em volta.
//!
//! Invariante que sustenta tudo: **toda chamada de ferramenta produz uma
//! mensagem `role: "tool"`, na mesma ordem em que o modelo pediu**. Sem isso
//! o template perde o pareamento e o passo seguinte sai incoerente.

use crate::checkpoint;
use crate::events::EventSink;
use crate::plan_tools::{PlanTools, SharedPlan, shared_plan, snapshot};
use crate::prompt::{PromptContext, build_system_prompt};
use crate::reliability::{
    ContextBudget, ErrorStreak, ReadLedger, Repeat, RepeatDetector, StepBudget, apply_compaction,
    compaction_request, plan_compaction,
};
use crate::scout::{self, PlanRun};
use crate::verify::{self, CommandRecord};
use crate::{AgentConfig, RunHandle, StartRun, new_id};
use lr_engine::{
    ChatDelta, ChatMessage, ChatRequest, LlamaClient, ServerProps, ToolCallReq, tool_specs_to_api,
};
use lr_policy::{Decision, PermissionOverride, PolicyEngine, ToolRequest, classify};
use lr_store::Store;
use lr_tools::{SharedTool, ToolContext, ToolOutput, ToolRegistry, ToolResult};
use lr_types::agent::{
    ApprovalDecision, ApprovalSource, PolicyScope, RunEventKind, RunMode, RunOptions, RunStatus,
    ToolCategory, ToolOrigin, ToolPolicy, ToolSpec, ToolTier, UsageStats,
};
use lr_types::scout::{TaskPlan, WindowBudget, WorkMode};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// Ferramenta interna que mantém o plano do run.
const FOCUS_TOOL: &str = "todo_update";
/// Quanto do resultado vai para a prévia do evento (a UI expande depois).
const PREVIEW_CHARS: usize = 400;
/// Mensagens preservadas intactas na compactação (≈ dois passos).
const KEEP_TAIL_MESSAGES: usize = 6;

// ------------------------------------------------------------ uma chamada ---

/// O que fazer depois de uma chamada de ferramenta.
pub(crate) enum CallFlow {
    /// Resultado (bom ou ruim) para devolver ao modelo.
    Result(ChatMessage),
    /// Guard-rail estourou: encerra o run e explica para a pessoa.
    Escalate(String),
    /// O run foi cancelado no meio da chamada.
    Cancelled,
}

/// Executa uma chamada de ferramenta respeitando a política do run.
pub(crate) struct ToolRunner {
    pub run_id: String,
    pub mode: RunMode,
    pub workspace: Option<PathBuf>,
    pub sink: Arc<EventSink>,
    pub registry: Arc<ToolRegistry>,
    pub store: Arc<Store>,
    pub handle: Arc<RunHandle>,
    pub config: Arc<AgentConfig>,
    /// Overrides do usuário; a política é remontada quando um "sempre
    /// permitir" chega no meio do run.
    pub overrides: Vec<PermissionOverride>,
    pub policy: PolicyEngine,
    /// Já existe uma foto dos arquivos nesta execução?
    pub checkpoint_done: bool,
    pub reads: ReadLedger,
    pub repeats: RepeatDetector,
    pub errors: ErrorStreak,
    /// Arquivos alterados (alimenta a verificação final).
    pub written: Vec<String>,
    pub commands: Vec<CommandRecord>,
    pub focus_md: Option<String>,
    pub tool_calls: u32,
    /// Ferramentas que existem só neste run (as meta do plano). Ficam fora do
    /// registro compartilhado porque carregam o plano DESTE run; são
    /// procuradas antes dele.
    pub local_tools: Vec<SharedTool>,
    /// Levantado por uma ferramenta meta quando o trecho de laço atual acabou
    /// (etapa concluída, plano escrito ou pergunta para a pessoa).
    pub halt: Arc<AtomicBool>,
}

impl ToolRunner {
    fn workspace_str(&self) -> Option<String> {
        self.workspace
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
    }

    /// Ferramenta local com este nome, se houver.
    fn local(&self, name: &str) -> Option<SharedTool> {
        self.local_tools
            .iter()
            .find(|t| t.name() == name)
            .map(Arc::clone)
    }

    /// Metadados da ferramenta: local primeiro, registro depois.
    async fn spec_for(&self, name: &str) -> Option<ToolSpec> {
        match self.local(name) {
            Some(tool) => Some(tool.spec()),
            None => self.registry.spec_of(name).await,
        }
    }

    /// Executa pela ferramenta local ou pelo registro.
    async fn dispatch(&self, name: &str, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        match self.local(name) {
            Some(tool) => tool.execute(args, ctx).await,
            None => self.registry.execute(name, args, ctx).await,
        }
    }

    /// Uma ferramenta meta pediu para encerrar o trecho atual. Lê e desarma.
    pub(crate) fn take_halt(&self) -> bool {
        self.halt.swap(false, Ordering::SeqCst)
    }

    /// Esquece o que era memória de UM contexto.
    ///
    /// Cada etapa do plano recomeça com histórico vazio: acusar o modelo de
    /// reler um arquivo "que já está no histórico" seria mentira, porque o
    /// histórico não existe mais. O que atravessa (arquivos escritos,
    /// comandos, checkpoint) fica.
    pub(crate) fn reset_task_memory(&mut self) {
        self.reads = ReadLedger::default();
        self.repeats = RepeatDetector::default();
        self.errors = ErrorStreak::default();
    }

    fn tool_context(&self, call_id: &str) -> ToolContext {
        let mut ctx = ToolContext::new(self.workspace.clone(), call_id);
        ctx.max_output_bytes = self.config.max_output_bytes;
        ctx
    }

    /// Uma chamada, do pedido do modelo ao resultado de volta.
    pub(crate) async fn call(
        &mut self,
        step_id: &str,
        step_index: u32,
        tc: &ToolCallReq,
    ) -> CallFlow {
        self.tool_calls += 1;
        let call_id = tc.id.clone();
        let name = tc.name.clone();

        // Ferramenta inexistente: erro acionável, sem passar pela política.
        let Some(spec) = self.spec_for(&name).await else {
            let message =
                format!("A ferramenta `{name}` não existe. Use apenas as ferramentas listadas.");
            self.trace_rejection(step_id, &call_id, &name, &tc.arguments_json, None, &message);
            return self.record_failure(&call_id, &name, message);
        };

        // JSON inválido é comum em modelos pequenos: vira erro de ferramenta,
        // nunca queda do run.
        let args = match tc.arguments() {
            Ok(Value::Object(map)) => Value::Object(map),
            Ok(Value::Null) => json!({}),
            Ok(_) => {
                let message = "Os argumentos precisam ser um objeto JSON com os campos do \
                               schema da ferramenta."
                    .to_string();
                self.trace_rejection(
                    step_id,
                    &call_id,
                    &name,
                    &tc.arguments_json,
                    Some(&spec),
                    &message,
                );
                return self.record_failure(&call_id, &name, message);
            }
            Err(e) => {
                let message = format!(
                    "Não consegui ler os argumentos: {e}. Reenvie a chamada com um objeto \
                     JSON válido (aspas duplas, sem vírgula sobrando)."
                );
                self.trace_rejection(
                    step_id,
                    &call_id,
                    &name,
                    &tc.arguments_json,
                    Some(&spec),
                    &message,
                );
                return self.record_failure(&call_id, &name, message);
            }
        };

        // Girando em falso: avisa e, se insistir, devolve para a pessoa.
        match self.repeats.observe(&name, &args) {
            Repeat::Fresh => {}
            Repeat::Warn { .. } => {
                let message = RepeatDetector::warning_for(&name);
                self.trace_rejection(
                    step_id,
                    &call_id,
                    &name,
                    &args.to_string(),
                    Some(&spec),
                    &message,
                );
                return CallFlow::Result(ChatMessage::tool_result(&call_id, &name, message));
            }
            Repeat::Escalate { .. } => {
                return CallFlow::Escalate(RepeatDetector::escalation_message(&name));
            }
        }

        // Releitura: devolve o ponteiro para o que já está no histórico.
        let read_key = ReadLedger::key_for(&name, &args);
        if let Some(key) = &read_key
            && let Some(previous) = self.reads.seen(key)
        {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let message = ReadLedger::duplicate_message(&path, previous);
            self.trace_rejection(
                step_id,
                &call_id,
                &name,
                &args.to_string(),
                Some(&spec),
                &message,
            );
            return CallFlow::Result(ChatMessage::tool_result(&call_id, &name, message));
        }

        let ctx = self.tool_context(&call_id);
        let builtin = self
            .local(&name)
            .or_else(|| self.registry.get(&name).cloned());
        let preview = match &builtin {
            Some(tool) => tool.preview(&args, &ctx).await,
            None => None,
        };
        let within = builtin
            .as_ref()
            .map(|t| t.within_workspace(&args, &ctx))
            .unwrap_or(true);

        // `terminal_run` e afins: a classificação do comando é o que faz o
        // modo automático continuar pedindo confirmação para o que não dá
        // para analisar.
        let command_text = (spec.category == ToolCategory::Execute)
            .then(|| {
                args.get("command")
                    .or_else(|| args.get("cmd"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .flatten();
        let analysis = command_text.as_deref().map(classify);

        let mut request = ToolRequest::new(&name, spec.category);
        if !within {
            request = request.outside_workspace();
        }
        if let Some(a) = &analysis {
            request = request.with_command(a.class);
        }
        let decision = self.policy.decide(&request, self.mode);

        let args_json = args.to_string();
        let _ = self.store.create_tool_call(
            &call_id,
            &self.run_id,
            Some(step_id),
            &name,
            &origin_str(&spec.origin),
            &args_json,
        );
        self.sink.emit(RunEventKind::ToolRequested {
            call_id: call_id.clone(),
            tool: name.clone(),
            origin: spec.origin.clone(),
            category: spec.category,
            tier: spec.tier,
            args_json: args_json.clone(),
            preview,
            requires_approval: matches!(decision, Decision::Ask { .. }),
        });

        // Política e, se preciso, a pessoa.
        let (source, args) = match decision {
            Decision::Allow { source } => {
                self.log_approval(&call_id, &name, "allowOnce", source, &args);
                self.sink.emit(RunEventKind::ToolApproved {
                    call_id: call_id.clone(),
                    source,
                });
                (source, args)
            }
            Decision::Deny { reason } => {
                return self.denied(&call_id, &name, &reason, ApprovalSource::Policy);
            }
            Decision::Ask { reason } => match self.ask_user(&call_id, &name, args, &reason).await {
                Ask::Approved { source, args } => {
                    self.sink.emit(RunEventKind::ToolApproved {
                        call_id: call_id.clone(),
                        source,
                    });
                    (source, args)
                }
                Ask::Denied(reason) => {
                    return self.denied(&call_id, &name, &reason, ApprovalSource::User);
                }
                Ask::Cancelled => return CallFlow::Cancelled,
            },
        };
        let _ = source;

        // Foto antes da primeira alteração — depois de aprovada, antes de rodar.
        if !self.checkpoint_done && PolicyEngine::needs_checkpoint(&request) {
            let files = builtin
                .as_ref()
                .map(|t| t.files_at_risk(&args, &ctx))
                .unwrap_or_default();
            self.take_checkpoint(&name, files).await;
        }

        self.execute(&name, &call_id, args, read_key, step_index, analysis)
            .await
    }

    /// Roda a ferramenta e traduz o resultado para o modelo.
    async fn execute(
        &mut self,
        name: &str,
        call_id: &str,
        args: Value,
        read_key: Option<String>,
        step_index: u32,
        analysis: Option<lr_policy::CommandAnalysis>,
    ) -> CallFlow {
        let _ = self.store.set_tool_call_state(call_id, "running");
        self.sink.emit(RunEventKind::ToolStarted {
            call_id: call_id.to_string(),
        });

        let ctx = self.tool_context(call_id);
        let started = Instant::now();
        let outcome = tokio::select! {
            biased;
            _ = self.handle.cancelled() => {
                let _ = self.store.finish_tool_call(call_id, false, "", 0, Some("cancelado"));
                return CallFlow::Cancelled;
            }
            r = self.dispatch(name, args.clone(), &ctx) => r,
        };
        let duration_ms = started.elapsed().as_millis() as u64;

        match outcome {
            Ok(out) => {
                let truncated = out.bytes_total > out.content.len() as u64;
                self.sink.emit(RunEventKind::ToolOutput {
                    call_id: call_id.to_string(),
                    chunk: out.content.clone(),
                    truncated,
                });
                self.sink.emit(RunEventKind::ToolResult {
                    call_id: call_id.to_string(),
                    ok: true,
                    result_preview: head_chars(&out.content, PREVIEW_CHARS),
                    bytes_total: out.bytes_total,
                    duration_ms,
                });
                let result_json = json!({
                    "content": out.content,
                    "changedFiles": out.changed_files,
                })
                .to_string();
                let _ =
                    self.store
                        .finish_tool_call(call_id, true, &result_json, out.bytes_total, None);

                self.errors.record_success();
                if let Some(key) = read_key {
                    self.reads.note(&key, step_index);
                }
                for file in &out.changed_files {
                    if !self.written.contains(file) {
                        self.written.push(file.clone());
                    }
                }
                if let Some(a) = &analysis {
                    self.commands.push(CommandRecord {
                        display: a.program.clone(),
                        ok: true,
                        exit_code: verify::extract_exit_code(&out.content),
                    });
                }
                if name == FOCUS_TOOL {
                    self.update_focus(&args, &out.content);
                }

                CallFlow::Result(ChatMessage::tool_result(call_id, name, out.content))
            }
            Err(e) => {
                let message = e.to_model_message();
                self.sink.emit(RunEventKind::ToolResult {
                    call_id: call_id.to_string(),
                    ok: false,
                    result_preview: head_chars(&message, PREVIEW_CHARS),
                    bytes_total: 0,
                    duration_ms,
                });
                let _ = self
                    .store
                    .finish_tool_call(call_id, false, "", 0, Some(&message));
                if let Some(a) = &analysis {
                    self.commands.push(CommandRecord {
                        display: a.program.clone(),
                        ok: false,
                        exit_code: None,
                    });
                }
                self.record_failure(call_id, name, message)
            }
        }
    }

    /// Pergunta para a pessoa e espera (o run fica pausado até a resposta).
    async fn ask_user(&mut self, call_id: &str, name: &str, args: Value, reason: &str) -> Ask {
        let rx = self.handle.register_pending(call_id);
        let _ = self
            .store
            .set_run_status(&self.run_id, RunStatus::WaitingApproval);
        self.sink.emit(RunEventKind::RunPaused {
            reason: lr_types::agent::PauseReason::WaitingApproval,
        });

        let decision = tokio::select! {
            biased;
            _ = self.handle.cancelled() => {
                self.handle.clear_pending(call_id);
                return Ask::Cancelled;
            }
            answer = tokio::time::timeout(self.config.approval_timeout, rx) => match answer {
                Ok(Ok(decision)) => decision,
                // Emissor sumiu (run derrubado): trata como cancelamento.
                Ok(Err(_)) => {
                    self.handle.clear_pending(call_id);
                    return Ask::Cancelled;
                }
                Err(_) => {
                    self.handle.clear_pending(call_id);
                    log::warn!("confirmação de `{name}` expirou; cancelando a execução");
                    self.handle.cancel();
                    return Ask::Cancelled;
                }
            },
        };
        self.handle.clear_pending(call_id);
        let _ = self.store.set_run_status(&self.run_id, RunStatus::Running);
        self.sink.emit(RunEventKind::RunResumed);
        log::debug!("confirmação de `{name}` resolvida ({reason})");

        match decision {
            ApprovalDecision::AllowOnce => {
                self.log_approval(call_id, name, "allowOnce", ApprovalSource::User, &args);
                Ask::Approved {
                    source: ApprovalSource::User,
                    args,
                }
            }
            ApprovalDecision::AllowAlways { scope } => {
                self.remember(name, scope, ToolPolicy::AlwaysAllow);
                self.log_approval(call_id, name, "allowAlways", ApprovalSource::User, &args);
                Ask::Approved {
                    source: ApprovalSource::User,
                    args,
                }
            }
            ApprovalDecision::AllowEdited { args_json } => {
                match serde_json::from_str::<Value>(&args_json) {
                    Ok(edited) if edited.is_object() => {
                        self.log_approval(
                            call_id,
                            name,
                            "allowEdited",
                            ApprovalSource::User,
                            &edited,
                        );
                        Ask::Approved {
                            source: ApprovalSource::User,
                            args: edited,
                        }
                    }
                    _ => Ask::Denied(
                        "Os argumentos editados não formavam um objeto JSON válido.".into(),
                    ),
                }
            }
            ApprovalDecision::Deny { reason } => {
                Ask::Denied(reason.unwrap_or_else(|| "A pessoa recusou esta ação.".to_string()))
            }
            ApprovalDecision::DenyAlways { scope } => {
                self.remember(name, scope, ToolPolicy::Never);
                Ask::Denied(format!(
                    "A pessoa recusou esta ação e desativou `{name}` para as próximas."
                ))
            }
        }
    }

    /// Grava um "sempre permitir"/"nunca" e recarrega a política do run.
    fn remember(&mut self, tool: &str, scope: PolicyScope, policy: ToolPolicy) {
        let workspace = self.workspace_str();
        let dir = match scope {
            PolicyScope::Workspace => workspace.as_deref(),
            PolicyScope::Global => None,
        };
        if let Err(e) = self.store.set_tool_permission(scope, dir, tool, policy) {
            log::warn!("não consegui guardar a permissão de `{tool}`: {e}");
        }
        self.overrides.retain(|o| o.tool_name != tool);
        self.overrides.push(PermissionOverride {
            tool_name: tool.to_string(),
            policy,
            scope,
        });
        self.policy = PolicyEngine::new(self.overrides.clone());
    }

    /// Recusa: o modelo PRECISA saber para tentar outro caminho.
    fn denied(
        &mut self,
        call_id: &str,
        name: &str,
        reason: &str,
        source: ApprovalSource,
    ) -> CallFlow {
        self.sink.emit(RunEventKind::ToolDenied {
            call_id: call_id.to_string(),
            reason: reason.to_string(),
        });
        let _ = self
            .store
            .finish_tool_call(call_id, false, "", 0, Some(reason));
        let _ = self
            .store
            .log_approval(&self.run_id, call_id, name, "deny", source, "");
        CallFlow::Result(ChatMessage::tool_result(
            call_id,
            name,
            format!(
                "A ação foi recusada: {reason} Não tente de novo do mesmo jeito — \
                 proponha outro caminho ou explique o que precisa."
            ),
        ))
    }

    /// Falha de ferramenta: devolve ao modelo e conta para a escalada.
    fn record_failure(&mut self, call_id: &str, name: &str, message: String) -> CallFlow {
        let msg = ChatMessage::tool_result(call_id, name, format!("ERRO: {message}"));
        if self.errors.record_error() {
            return CallFlow::Escalate(self.errors.escalation_message());
        }
        CallFlow::Result(msg)
    }

    /// Registra na trilha uma chamada que nem chegou a rodar.
    fn trace_rejection(
        &self,
        step_id: &str,
        call_id: &str,
        name: &str,
        args_json: &str,
        spec: Option<&ToolSpec>,
        message: &str,
    ) {
        let origin = spec
            .map(|s| s.origin.clone())
            .unwrap_or(ToolOrigin::Builtin);
        let _ = self.store.create_tool_call(
            call_id,
            &self.run_id,
            Some(step_id),
            name,
            &origin_str(&origin),
            args_json,
        );
        self.sink.emit(RunEventKind::ToolRequested {
            call_id: call_id.to_string(),
            tool: name.to_string(),
            origin,
            category: spec.map(|s| s.category).unwrap_or(ToolCategory::Meta),
            tier: spec.map(|s| s.tier).unwrap_or(ToolTier::Safe),
            args_json: args_json.to_string(),
            preview: None,
            requires_approval: false,
        });
        self.sink.emit(RunEventKind::ToolResult {
            call_id: call_id.to_string(),
            ok: false,
            result_preview: head_chars(message, PREVIEW_CHARS),
            bytes_total: 0,
            duration_ms: 0,
        });
        let _ = self
            .store
            .finish_tool_call(call_id, false, "", 0, Some(message));
    }

    fn log_approval(
        &self,
        call_id: &str,
        name: &str,
        decision: &str,
        source: ApprovalSource,
        args: &Value,
    ) {
        let _ = self.store.log_approval(
            &self.run_id,
            call_id,
            name,
            decision,
            source,
            &args_hash(args),
        );
    }

    /// Tira a foto dos arquivos antes da primeira alteração do run.
    async fn take_checkpoint(&mut self, tool: &str, files: Vec<String>) {
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        let store_dir = self.config.store_dir.clone();
        let label = format!("Antes de {tool}");
        let label_task = label.clone();
        let result = tokio::task::spawn_blocking(move || {
            checkpoint::snapshot_blocking(&workspace, &store_dir, &label_task, &files)
        })
        .await;

        match result {
            Ok(Ok(cp)) => {
                self.checkpoint_done = true;
                let files_json = serde_json::to_string(&cp.files).ok();
                let _ = self.store.add_checkpoint(
                    &cp.id,
                    Some(&self.run_id),
                    &self.workspace_str().unwrap_or_default(),
                    &cp.backend,
                    &cp.ref_id,
                    Some(&label),
                    files_json.as_deref(),
                );
                self.sink.emit(RunEventKind::CheckpointCreated {
                    checkpoint_id: cp.id,
                    label,
                    backend: cp.backend,
                });
            }
            Ok(Err(e)) => {
                // Sem foto o run continua, mas a pessoa precisa saber que o
                // "desfazer" não vai existir desta vez.
                log::warn!("checkpoint falhou: {e}");
                self.sink.emit(RunEventKind::RunError {
                    message: format!("Não consegui criar o ponto de restauração: {e}"),
                    retryable: true,
                });
            }
            Err(e) => log::warn!("tarefa do checkpoint falhou: {e}"),
        }
    }

    /// Plano atualizado pela ferramenta interna de todo.
    fn update_focus(&mut self, args: &Value, output: &str) {
        let md = args
            .get("todo_md")
            .or_else(|| args.get("plan"))
            .or_else(|| args.get("markdown"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| output.to_string());
        if md.trim().is_empty() {
            return;
        }
        let _ = self.store.set_run_focus(&self.run_id, &md);
        self.focus_md = Some(md.clone());
        self.sink.emit(RunEventKind::FocusUpdated { todo_md: md });
    }
}

/// Resultado da conversa com a pessoa sobre uma chamada.
enum Ask {
    Approved { source: ApprovalSource, args: Value },
    Denied(String),
    Cancelled,
}

fn origin_str(origin: &ToolOrigin) -> String {
    match origin {
        ToolOrigin::Builtin => "builtin".to_string(),
        ToolOrigin::Mcp { server_id } => format!("mcp:{server_id}"),
    }
}

/// Impressão digital dos argumentos (auditoria — não é segredo, é rastro).
fn args_hash(args: &Value) -> String {
    let mut hash: u64 = 1469598103934665603;
    for b in args.to_string().as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    format!("{hash:016x}")
}

pub(crate) fn head_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect::<String>() + "…"
}

// ------------------------------------------------------------------ laço ---

/// Histórico do chat convertido em mensagens para o modelo.
pub fn history_from_messages(rows: &[lr_store::MessageRow], max: usize) -> Vec<ChatMessage> {
    let usable: Vec<&lr_store::MessageRow> = rows
        .iter()
        .filter(|m| matches!(m.role.as_str(), "user" | "assistant"))
        .filter(|m| !m.content.trim().is_empty())
        .collect();
    let start = usable.len().saturating_sub(max);
    usable[start..]
        .iter()
        .map(|m| match m.role.as_str() {
            "assistant" => ChatMessage::assistant(m.content.clone()),
            _ => ChatMessage::user(m.content.clone()),
        })
        .collect()
}

/// Acrescenta o pedido atual, a menos que ele já seja a última mensagem
/// (a interface grava a mensagem da pessoa antes de chamar o agente).
pub fn append_prompt(messages: &mut Vec<ChatMessage>, prompt: &str) {
    let already = messages
        .last()
        .is_some_and(|m| m.role == "user" && m.text().trim() == prompt.trim());
    if !already && !prompt.trim().is_empty() {
        messages.push(ChatMessage::user(prompt.to_string()));
    }
}

/// Dependências que o laço recebe pronto do [`crate::AgentHost`].
pub(crate) struct RunDeps {
    pub store: Arc<Store>,
    pub registry: Arc<ToolRegistry>,
    pub config: Arc<AgentConfig>,
}

/// Monta o prompt de sistema com o plano atual dentro (quando há um).
///
/// `Send + Sync` porque o run inteiro vive numa task do tokio: o fecho
/// atravessa `await`s enquanto o laço do plano roda.
pub(crate) type SystemPrompt<'a> = dyn Fn(Option<&String>) -> String + Send + Sync + 'a;

/// Motor de passos: pensar → agir → ler o resultado, até o modelo responder
/// sem pedir ferramenta.
///
/// Serve para o run inteiro (modo agente) e para UMA etapa do plano (modo
/// laço). A diferença está só no histórico que entra: no plano ele recomeça
/// vazio a cada etapa, e é isso que mantém a janela do modelo pequena.
pub(crate) struct StepEngine<'a> {
    pub client: &'a LlamaClient,
    pub sink: Arc<EventSink>,
    pub handle: Arc<RunHandle>,
    pub store: Arc<Store>,
    pub run_id: String,
    pub opts: &'a RunOptions,
    /// Cardápio já no formato da API (vazio quando não há ferramentas).
    pub api_tools: Vec<Value>,
    pub tools_on: bool,
    pub context: ContextBudget,
}

/// O que um trecho de laço produziu.
pub(crate) struct StepOutcome {
    pub status: RunStatus,
    /// Resposta final em texto (vazia quando o trecho não chegou lá).
    pub text: String,
    pub escalation: Option<String>,
    pub steps: u32,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

impl StepEngine<'_> {
    /// Roda o laço até uma resposta final, o teto de passos ou uma parada.
    ///
    /// `max_local_steps` é o teto DESTE trecho (uma etapa do plano); o
    /// `budget` é o teto do run inteiro e continua valendo por cima.
    pub(crate) async fn drive(
        &self,
        runner: &mut ToolRunner,
        messages: &mut Vec<ChatMessage>,
        budget: &mut StepBudget,
        max_local_steps: u32,
        build_system: &SystemPrompt<'_>,
    ) -> StepOutcome {
        let mut out = StepOutcome {
            status: RunStatus::Done,
            text: String::new(),
            escalation: None,
            steps: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
        };
        // O prompt já foi montado com o plano que o runner conhece: começar
        // igual evita reconstruí-lo à toa no primeiro passo.
        let mut last_focus = runner.focus_md.clone();
        let mut local: u32 = 0;

        let status = 'run: loop {
            if self.handle.is_cancelled() {
                break RunStatus::Cancelled;
            }
            if local >= max_local_steps {
                break RunStatus::MaxSteps;
            }
            let Some(index) = budget.start_step() else {
                break RunStatus::MaxSteps;
            };
            local += 1;

            // O plano entrou/mudou: o prompt de sistema acompanha.
            if runner.focus_md != last_focus {
                last_focus = runner.focus_md.clone();
                messages[0] = ChatMessage::system(build_system(last_focus.as_ref()));
            }

            let step_id = new_id("step");
            let _ = self.store.create_step(&step_id, &self.run_id, index);
            self.sink.emit(RunEventKind::StepStarted {
                step_id: step_id.clone(),
                index,
            });

            let mut request = chat_request(self.opts, messages, &self.api_tools);
            if self.context.limit().is_some() {
                compact_if_needed(
                    self.client,
                    &self.sink,
                    &self.context,
                    self.opts,
                    &self.api_tools,
                    messages,
                )
                .await;
                request = chat_request(self.opts, messages, &self.api_tools);
            }

            let outcome = {
                let sink_delta = self.sink.clone();
                let step = step_id.clone();
                let mut on_delta = move |delta: ChatDelta| match delta {
                    ChatDelta::Text(text) => {
                        sink_delta.emit(RunEventKind::AssistantDelta {
                            step_id: step.clone(),
                            text,
                        });
                    }
                    ChatDelta::Reasoning(text) => {
                        sink_delta.emit(RunEventKind::ReasoningDelta {
                            step_id: step.clone(),
                            text,
                        });
                    }
                    // Fragmentos de tool call viram eventos completos depois.
                    ChatDelta::ToolCall { .. } => {}
                };
                tokio::select! {
                    biased;
                    _ = self.handle.cancelled() => break RunStatus::Cancelled,
                    r = self.client.chat_stream(&request, &mut on_delta) => match r {
                        Ok(outcome) => outcome,
                        Err(e) => {
                            self.sink.emit(RunEventKind::RunError {
                                message: format!("O modelo não respondeu: {e}"),
                                retryable: true,
                            });
                            break RunStatus::Error;
                        }
                    },
                }
            };

            out.steps += 1;
            let (prompt_n, predicted_n) = match &outcome.timings {
                Some(t) => (t.prompt_n, t.predicted_n),
                None => (None, None),
            };
            out.prompt_tokens += prompt_n.unwrap_or(0);
            out.completion_tokens += predicted_n.unwrap_or(0);
            let _ = self.store.finish_step(
                &step_id,
                &outcome.content,
                &outcome.reasoning,
                prompt_n,
                predicted_n,
            );
            self.sink.emit(RunEventKind::AssistantMessage {
                step_id: step_id.clone(),
                content: outcome.content.clone(),
                reasoning: outcome.reasoning.clone(),
            });

            // Sem ferramentas pedidas: é a resposta final.
            if !self.tools_on || !outcome.wants_tools() {
                out.text = outcome.content.clone();
                break RunStatus::Done;
            }

            messages.push(outcome.to_assistant_message());
            for tc in &outcome.tool_calls {
                match runner.call(&step_id, index, tc).await {
                    CallFlow::Result(msg) => messages.push(msg),
                    CallFlow::Cancelled => break 'run RunStatus::Cancelled,
                    CallFlow::Escalate(reason) => {
                        out.escalation = Some(reason);
                        break 'run RunStatus::Escalated;
                    }
                }
            }

            // Uma ferramenta meta fechou a etapa (ou parou para perguntar):
            // não há mais nada a decidir neste trecho.
            if runner.take_halt() {
                out.text = outcome.content.clone();
                break RunStatus::Done;
            }
        };
        out.status = status;
        out
    }
}

/// Roda a execução inteira. Só retorna quando o run termina.
pub(crate) async fn execute_run(req: StartRun, handle: Arc<RunHandle>, deps: RunDeps) {
    let StartRun {
        prompt,
        history,
        memory,
        options: opts,
        endpoint,
        work_mode,
        plan: approved_plan,
    } = req;
    let sink = handle.sink();
    let run_id = handle.id.clone();
    let workspace = opts
        .workspace_dir
        .clone()
        .filter(|d| !d.is_empty())
        .map(PathBuf::from);
    let started_at = Instant::now();

    let client =
        LlamaClient::new(&endpoint.base_url).with_optional_api_key(endpoint.api_key.clone());

    // Capacidades do modelo carregado. Se o servidor não responder, seguimos
    // sem ferramentas em vez de derrubar o run.
    let props = match client.props().await {
        Ok(p) => p,
        Err(e) => {
            log::warn!("/props indisponível ({e}); executando sem ferramentas");
            ServerProps::default()
        }
    };
    let tools_on =
        props.supports_tools() && opts.mode != RunMode::Chat && work_mode != WorkMode::Chat;

    // Cardápio: o modo de trabalho manda no que o modelo pode usar.
    let mut specs = if tools_on {
        scout::menu_for(work_mode, deps.registry.specs().await)
    } else {
        Vec::new()
    };
    // Plano do run + ferramentas meta que o conduzem (vazias fora dos modos
    // que planejam).
    // Um plano já aprovado (o laço que continua um planejamento) entra
    // pronto: `run_plan` só divide o objetivo quando não há etapas.
    let plan = shared_plan(approved_plan.unwrap_or(TaskPlan {
        goal: prompt.trim().to_string(),
        ..Default::default()
    }));
    let plan_tools = PlanTools::for_mode(work_mode, plan.clone());
    if tools_on {
        specs.extend(plan_tools.specs());
    }
    let tool_names: Vec<String> = specs.iter().map(|s| s.name.clone()).collect();
    let api_tools = tool_specs_to_api(&specs);

    sink.emit(RunEventKind::RunStarted {
        chat_id: opts.chat_id,
        model: opts.model.clone(),
        mode: opts.mode,
        yolo: opts.yolo(),
        workspace_dir: opts.workspace_dir.clone(),
        tools: tool_names.clone(),
    });

    let workspace_str = workspace.as_ref().map(|p| p.to_string_lossy().into_owned());
    let overrides: Vec<PermissionOverride> = deps
        .store
        .list_tool_permissions(workspace_str.as_deref())
        .unwrap_or_default()
        .into_iter()
        .map(|row| PermissionOverride {
            tool_name: row.tool_name,
            policy: row.policy,
            scope: row.scope,
        })
        .collect();

    let mut runner = ToolRunner {
        run_id: run_id.clone(),
        mode: opts.mode,
        workspace: workspace.clone(),
        sink: sink.clone(),
        registry: deps.registry.clone(),
        store: deps.store.clone(),
        handle: handle.clone(),
        config: deps.config.clone(),
        policy: PolicyEngine::new(overrides.clone()),
        overrides,
        checkpoint_done: false,
        reads: ReadLedger::default(),
        repeats: RepeatDetector::default(),
        errors: ErrorStreak::default(),
        written: Vec::new(),
        commands: Vec::new(),
        focus_md: None,
        tool_calls: 0,
        local_tools: if tools_on {
            plan_tools.tools.clone()
        } else {
            Vec::new()
        },
        halt: plan_tools.halt.clone(),
    };

    let build_prompt = |focus: Option<&String>| {
        build_system_prompt(&PromptContext {
            workspace: workspace_str.as_deref(),
            focus_md: focus.map(String::as_str),
            memory: &memory,
            tools: &tool_names,
            user_system: opts.system_prompt.as_deref(),
            mode: opts.mode,
        })
    };

    let engine = StepEngine {
        client: &client,
        sink: sink.clone(),
        handle: handle.clone(),
        store: deps.store.clone(),
        run_id: run_id.clone(),
        opts: &opts,
        api_tools,
        tools_on,
        context: ContextBudget::new(props.n_ctx, deps.config.context_ratio),
    };
    let window = props.n_ctx.map(WindowBudget::new).unwrap_or_default();

    let mut usage = UsageStats::default();
    let mut escalation: Option<String> = None;
    // Frase pronta do modo laço/planejamento; nos demais o resumo sai do
    // texto final.
    let mut summary_override: Option<String> = None;
    let mut final_text;

    // No laço, o teto global precisa caber o plano inteiro: um teto pensado
    // para UM pedido pararia o run no meio da terceira entrega.
    let max_steps = match work_mode {
        WorkMode::Loop => opts
            .max_steps
            .max(scout::MAX_TASKS as u32 * scout::MAX_STEPS_PER_TASK),
        _ => opts.max_steps,
    };
    let mut budget = StepBudget::new(max_steps);

    let status = if work_mode == WorkMode::Loop {
        // Divide o objetivo e executa entrega por entrega, cada uma com
        // contexto novo.
        let plan_run = PlanRun {
            engine: &engine,
            build_system: &build_prompt,
            plan: plan.clone(),
            workspace: workspace.clone(),
            goal: prompt.clone(),
            context: String::new(),
            window,
        };
        let outcome = scout::run_plan(&plan_run, &mut runner, &mut budget).await;
        usage.steps += outcome.steps;
        usage.prompt_tokens += outcome.prompt_tokens;
        usage.completion_tokens += outcome.completion_tokens;
        final_text = outcome.summary.clone();
        summary_override = Some(outcome.summary);
        outcome.status
    } else {
        let mut messages = vec![ChatMessage::system(build_prompt(None))];
        if work_mode == WorkMode::Plan {
            // Segunda mensagem de sistema: a reconstrução do prompt a cada
            // passo troca só a primeira, então esta sobrevive ao plano mudar.
            messages.push(ChatMessage::system(scout::PLAN_MODE_BRIEF));
        }
        messages.extend(history);
        append_prompt(&mut messages, &prompt);

        let outcome = engine
            .drive(
                &mut runner,
                &mut messages,
                &mut budget,
                max_steps,
                &build_prompt,
            )
            .await;
        usage.steps += outcome.steps;
        usage.prompt_tokens += outcome.prompt_tokens;
        usage.completion_tokens += outcome.completion_tokens;
        final_text = outcome.text.clone();
        escalation = outcome.escalation;

        // Planejamento entrega o plano, não o trabalho.
        if work_mode == WorkMode::Plan && outcome.status == RunStatus::Done {
            let proposal = present_plan(&engine, &plan, &prompt, &outcome.text, window).await;
            if final_text.trim().is_empty() {
                final_text = proposal;
            }
        }
        outcome.status
    };

    // Verificação barata do que ficou em disco.
    if status == RunStatus::Done
        && let Some(report) =
            verify::verify(workspace.as_deref(), &runner.written, &runner.commands)
    {
        sink.emit(RunEventKind::Verification {
            passed: report.passed,
            notes: report.notes,
        });
    }

    usage.tool_calls = runner.tool_calls;
    usage.duration_ms = started_at.elapsed().as_millis() as u64;

    // Guarda a resposta na conversa. O laço também guarda quando para no
    // meio: o motivo da parada (uma pergunta, uma etapa travada) é o que a
    // pessoa precisa ler.
    let keep_in_chat = |text: &str| {
        if !text.trim().is_empty() && opts.chat_id > 0 {
            let _ = deps.store.add_message(
                opts.chat_id,
                "assistant",
                text,
                None,
                Some(usage.completion_tokens as i64),
                Some(usage.duration_ms as i64),
            );
        }
    };

    let summary = match status {
        RunStatus::Done => {
            keep_in_chat(&final_text);
            summary_override.unwrap_or_else(|| head_chars(final_text.trim(), 280))
        }
        RunStatus::MaxSteps => match summary_override {
            Some(plan_summary) => {
                keep_in_chat(&plan_summary);
                plan_summary
            }
            None => format!(
                "Parei no limite de {} passos sem terminar a tarefa.",
                budget.max()
            ),
        },
        RunStatus::Escalated => match summary_override {
            Some(plan_summary) => {
                keep_in_chat(&plan_summary);
                plan_summary
            }
            None => escalation.unwrap_or_else(|| "Execução interrompida.".into()),
        },
        RunStatus::Cancelled => "Execução cancelada.".to_string(),
        _ => "A execução falhou.".to_string(),
    };

    let usage_json = serde_json::to_string(&usage).unwrap_or_else(|_| "{}".into());
    let _ = deps
        .store
        .finish_run(&run_id, status, &summary, &usage_json);

    // Episódio: o que aconteceu nesta execução, para a memória consolidar
    // depois em fatos duráveis. Só vale a pena guardar o que produziu algo —
    // cancelar ou falhar de cara não ensina nada.
    if matches!(
        status,
        RunStatus::Done | RunStatus::Escalated | RunStatus::MaxSteps
    ) && usage.tool_calls > 0
    {
        let _ = deps.store.add_memory_episode(
            workspace.as_deref().and_then(|p| p.to_str()),
            (opts.chat_id > 0).then_some(opts.chat_id),
            Some(&run_id),
            &summary,
        );
    }
    sink.emit(RunEventKind::RunFinished {
        status,
        summary,
        usage,
    });
}

/// Fecha o modo planejamento: garante um plano gravado e devolve o texto que
/// vai para a conversa.
///
/// O plano fica com `approved = false` — nada executa até a pessoa aprovar.
async fn present_plan(
    engine: &StepEngine<'_>,
    plan: &SharedPlan,
    goal: &str,
    notes: &str,
    window: WindowBudget,
) -> String {
    // O modelo pode ter escrito o plano sozinho com `plan_create`.
    if snapshot(plan).tasks.is_empty() {
        let built = scout::decompose(engine.client, &engine.opts.model, goal, window, notes)
            .await
            .unwrap_or_else(|e| {
                log::warn!("não consegui dividir o objetivo ({e}); proponho uma entrega só");
                scout::single_task_plan(goal, window.per_task())
            });
        *crate::plan_tools::lock_plan(plan) = built;
    }

    let saved = {
        let mut current = crate::plan_tools::lock_plan(plan);
        current.approved = false;
        if current.goal.is_empty() {
            current.goal = goal.trim().to_string();
        }
        current.clone()
    };
    if let Ok(json) = serde_json::to_string(&saved) {
        let _ = engine.store.set_run_plan(&engine.run_id, &json);
    }
    let md = saved.to_markdown();
    let _ = engine.store.set_run_focus(&engine.run_id, &md);
    engine.sink.emit(RunEventKind::FocusUpdated {
        todo_md: md.clone(),
    });
    md
}

fn chat_request(opts: &RunOptions, messages: &[ChatMessage], tools: &[Value]) -> ChatRequest {
    let mut req = ChatRequest::new(&opts.model, messages.to_vec());
    if !tools.is_empty() {
        req = req.with_tools(tools.to_vec());
    }
    req.temperature = opts.temperature;
    req.top_p = opts.top_p;
    req.top_k = opts.top_k;
    req.max_tokens = opts.max_tokens;
    // Reaproveita o KV do prefixo entre passos: é o que torna um laço de 10
    // passos viável numa máquina doméstica.
    req.with_extra("cache_prompt", json!(true))
}

/// Compacta o histórico quando ele encosta no teto da janela de contexto.
async fn compact_if_needed(
    client: &LlamaClient,
    sink: &Arc<EventSink>,
    context: &ContextBudget,
    opts: &lr_types::agent::RunOptions,
    tools: &[Value],
    messages: &mut Vec<ChatMessage>,
) {
    let request = chat_request(opts, messages, tools);
    let Ok(before) = client.input_tokens(&request).await else {
        return;
    };
    if !context.needs_compaction(before) {
        return;
    }
    let Some(plan) = plan_compaction(messages, KEEP_TAIL_MESSAGES) else {
        log::warn!("contexto cheio, mas não há histórico antigo para resumir");
        return;
    };

    let ask = ChatRequest::new(
        &opts.model,
        vec![ChatMessage::user(compaction_request(&plan))],
    );
    let summary = match client.complete_once(&ask).await {
        Ok(o) if !o.content.trim().is_empty() => o.content,
        Ok(_) => return,
        Err(e) => {
            log::warn!("não consegui resumir o histórico: {e}");
            return;
        }
    };

    // O cabeçalho já é uma mensagem de sistema, então usá-la de novo é seguro.
    *messages = apply_compaction(&plan, &summary, true);
    let after = client
        .input_tokens(&chat_request(opts, messages, tools))
        .await
        .unwrap_or(before);
    sink.emit(RunEventKind::ContextCompacted {
        tokens_before: before,
        tokens_after: after,
    });
}

#[cfg(test)]
mod tests;
