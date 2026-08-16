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
use crate::prompt::{PromptContext, build_system_prompt};
use crate::reliability::{
    ContextBudget, ErrorStreak, ReadLedger, Repeat, RepeatDetector, StepBudget, apply_compaction,
    compaction_request, plan_compaction,
};
use crate::verify::{self, CommandRecord};
use crate::{AgentConfig, RunHandle, StartRun, new_id};
use lr_engine::{ChatDelta, ChatMessage, ChatRequest, LlamaClient, ToolCallReq, tool_specs_to_api};
use lr_policy::{Decision, PermissionOverride, PolicyEngine, ToolRequest, classify};
use lr_store::Store;
use lr_tools::{ToolContext, ToolRegistry};
use lr_types::agent::{
    ApprovalDecision, ApprovalSource, PolicyScope, RunEventKind, RunMode, RunStatus, ToolCategory,
    ToolOrigin, ToolPolicy, ToolSpec, ToolTier, UsageStats,
};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
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
}

impl ToolRunner {
    fn workspace_str(&self) -> Option<String> {
        self.workspace
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
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
        let Some(spec) = self.registry.spec_of(&name).await else {
            let message = format!(
                "A ferramenta `{name}` não existe. Use apenas as ferramentas listadas."
            );
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
        let builtin = self.registry.get(&name).cloned();
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
            r = self.registry.execute(name, args.clone(), &ctx) => r,
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
                let _ = self.store.finish_tool_call(
                    call_id,
                    true,
                    &result_json,
                    out.bytes_total,
                    None,
                );

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
            ApprovalDecision::Deny { reason } => Ask::Denied(
                reason.unwrap_or_else(|| "A pessoa recusou esta ação.".to_string()),
            ),
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
        if let Err(e) = self
            .store
            .set_tool_permission(scope, dir, tool, policy)
        {
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
        let _ = self.store.log_approval(
            &self.run_id,
            call_id,
            name,
            "deny",
            source,
            "",
        );
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
    Approved {
        source: ApprovalSource,
        args: Value,
    },
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

fn head_chars(s: &str, max: usize) -> String {
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

/// Roda a execução inteira. Só retorna quando o run termina.
pub(crate) async fn execute_run(req: StartRun, handle: Arc<RunHandle>, deps: RunDeps) {
    let sink = handle.sink();
    let run_id = handle.id.clone();
    let opts = req.options;
    let workspace = opts
        .workspace_dir
        .clone()
        .filter(|d| !d.is_empty())
        .map(PathBuf::from);
    let started_at = Instant::now();

    let client = LlamaClient::new(&req.endpoint.base_url)
        .with_optional_api_key(req.endpoint.api_key.clone());

    // Capacidades do modelo carregado. Se o servidor não responder, seguimos
    // sem ferramentas em vez de derrubar o run.
    let props = match client.props().await {
        Ok(p) => p,
        Err(e) => {
            log::warn!("/props indisponível ({e}); executando sem ferramentas");
            lr_engine::ServerProps::default()
        }
    };
    let tools_on = props.supports_tools() && opts.mode != RunMode::Chat;
    let specs = if tools_on {
        deps.registry.specs().await
    } else {
        Vec::new()
    };
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
    };

    let build_prompt = |focus: Option<&String>| {
        build_system_prompt(&PromptContext {
            workspace: workspace_str.as_deref(),
            focus_md: focus.map(String::as_str),
            memory: &req.memory,
            tools: &tool_names,
            user_system: opts.system_prompt.as_deref(),
            mode: opts.mode,
        })
    };

    let mut messages = vec![ChatMessage::system(build_prompt(None))];
    messages.extend(req.history);
    append_prompt(&mut messages, &req.prompt);

    let mut budget = StepBudget::new(opts.max_steps);
    let context = ContextBudget::new(props.n_ctx, deps.config.context_ratio);
    let mut usage = UsageStats::default();
    let mut last_focus: Option<String> = None;
    let mut final_text = String::new();
    let mut escalation: Option<String> = None;

    let status = 'run: loop {
        if handle.is_cancelled() {
            break RunStatus::Cancelled;
        }
        let Some(index) = budget.start_step() else {
            break RunStatus::MaxSteps;
        };

        // O plano entrou/mudou: o prompt de sistema acompanha.
        if runner.focus_md != last_focus {
            last_focus = runner.focus_md.clone();
            messages[0] = ChatMessage::system(build_prompt(last_focus.as_ref()));
        }

        let step_id = new_id("step");
        let _ = deps.store.create_step(&step_id, &run_id, index);
        sink.emit(RunEventKind::StepStarted {
            step_id: step_id.clone(),
            index,
        });

        let mut request = chat_request(&opts, &messages, &api_tools);
        if context.limit().is_some() {
            compact_if_needed(&client, &sink, &context, &opts, &api_tools, &mut messages).await;
            request = chat_request(&opts, &messages, &api_tools);
        }

        let outcome = {
            let sink_delta = sink.clone();
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
                _ = handle.cancelled() => break RunStatus::Cancelled,
                r = client.chat_stream(&request, &mut on_delta) => match r {
                    Ok(outcome) => outcome,
                    Err(e) => {
                        sink.emit(RunEventKind::RunError {
                            message: format!("O modelo não respondeu: {e}"),
                            retryable: true,
                        });
                        break RunStatus::Error;
                    }
                },
            }
        };

        usage.steps += 1;
        let (prompt_n, predicted_n) = match &outcome.timings {
            Some(t) => (t.prompt_n, t.predicted_n),
            None => (None, None),
        };
        usage.prompt_tokens += prompt_n.unwrap_or(0);
        usage.completion_tokens += predicted_n.unwrap_or(0);
        let _ = deps.store.finish_step(
            &step_id,
            &outcome.content,
            &outcome.reasoning,
            prompt_n,
            predicted_n,
        );
        sink.emit(RunEventKind::AssistantMessage {
            step_id: step_id.clone(),
            content: outcome.content.clone(),
            reasoning: outcome.reasoning.clone(),
        });

        // Sem ferramentas pedidas: é a resposta final.
        if !tools_on || !outcome.wants_tools() {
            final_text = outcome.content.clone();
            break RunStatus::Done;
        }

        messages.push(outcome.to_assistant_message());
        for tc in &outcome.tool_calls {
            match runner.call(&step_id, index, tc).await {
                CallFlow::Result(msg) => messages.push(msg),
                CallFlow::Cancelled => break 'run RunStatus::Cancelled,
                CallFlow::Escalate(reason) => {
                    escalation = Some(reason);
                    break 'run RunStatus::Escalated;
                }
            }
        }
    };

    // Verificação barata do que ficou em disco.
    if status == RunStatus::Done
        && let Some(report) = verify::verify(workspace.as_deref(), &runner.written, &runner.commands)
    {
        sink.emit(RunEventKind::Verification {
            passed: report.passed,
            notes: report.notes,
        });
    }

    usage.tool_calls = runner.tool_calls;
    usage.duration_ms = started_at.elapsed().as_millis() as u64;

    let summary = match status {
        RunStatus::Done => {
            if !final_text.trim().is_empty() && opts.chat_id > 0 {
                let _ = deps.store.add_message(
                    opts.chat_id,
                    "assistant",
                    &final_text,
                    None,
                    Some(usage.completion_tokens as i64),
                    Some(usage.duration_ms as i64),
                );
            }
            head_chars(final_text.trim(), 280)
        }
        RunStatus::MaxSteps => format!(
            "Parei no limite de {} passos sem terminar a tarefa.",
            budget.max()
        ),
        RunStatus::Escalated => escalation.unwrap_or_else(|| "Execução interrompida.".into()),
        RunStatus::Cancelled => "Execução cancelada.".to_string(),
        _ => "A execução falhou.".to_string(),
    };

    let usage_json = serde_json::to_string(&usage).unwrap_or_else(|_| "{}".into());
    let _ = deps
        .store
        .finish_run(&run_id, status, &summary, &usage_json);
    sink.emit(RunEventKind::RunFinished {
        status,
        summary,
        usage,
    });
}

fn chat_request(
    opts: &lr_types::agent::RunOptions,
    messages: &[ChatMessage],
    tools: &[Value],
) -> ChatRequest {
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
