//! Contratos do harness agêntico, compartilhados por `lr_agent`, `lr_tools`,
//! `lr_policy`, `lr_mcp` e pelo app.
//!
//! Tudo aqui é serializado para o frontend em `camelCase` — mantenha em
//! sincronia com `src/lib/agent/types.ts`.
//!
//! ATENÇÃO serde: em enums, `rename_all` renomeia só os NOMES das variantes.
//! Campos de variantes-struct exigem `rename_all_fields = "camelCase"`.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- políticas ---

/// Como o run trata ferramentas que pedem confirmação.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum RunMode {
    /// Sem ferramentas: chat puro.
    Chat,
    /// Toda ferramenta pede confirmação (exceto leitura dentro do workspace).
    #[default]
    Approve,
    /// Auto-aprova leitura e `alwaysAllow`; escrita/execução pedem.
    Smart,
    /// Auto-aprova ações DENTRO do workspace (com checkpoint automático).
    /// Fora do workspace, rede e comandos opacos continuam pedindo.
    Yolo,
}

/// Categoria de impacto de uma ferramenta (base da matriz de permissões).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolCategory {
    /// Só lê (arquivos, listagens, buscas).
    Read,
    /// Cria/modifica/apaga arquivos.
    Edit,
    /// Executa processos.
    Execute,
    /// Acessa a rede.
    Network,
    /// Ferramenta vinda de um servidor MCP (categoria refinada por annotations).
    Mcp,
    /// Interna do harness (todo/memória) — sem efeito fora do app.
    Meta,
}

/// Risco padrão, usado para badge na UI e tier inicial de permissão.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolTier {
    /// Sem efeitos colaterais observáveis.
    Safe,
    /// Modifica estado recuperável (arquivo do workspace, com checkpoint).
    Caution,
    /// Efeito difícil de desfazer ou fora do workspace.
    Danger,
}

/// Política persistida por ferramenta (override do usuário).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolPolicy {
    AlwaysAllow,
    Ask,
    Never,
}

/// Escopo de um override de permissão.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PolicyScope {
    Global,
    Workspace,
}

/// Classificação determinística de um comando de terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommandClass {
    /// Reconhecido e comprovadamente somente-leitura.
    ReadOnly,
    /// Reconhecido, muda estado.
    Mutating,
    /// Não totalmente analisável (eval, encoded, redireções obscuras) —
    /// SEMPRE pede aprovação, inclusive em YOLO.
    Opaque,
}

/// Decisão do usuário (ou da política) sobre uma chamada de ferramenta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    rename_all_fields = "camelCase"
)]
pub enum ApprovalDecision {
    /// Aprova só esta chamada.
    AllowOnce,
    /// Aprova e grava `alwaysAllow` para esta ferramenta no escopo dado.
    AllowAlways { scope: PolicyScope },
    /// Recusa esta chamada; o motivo volta ao modelo como resultado.
    Deny { reason: Option<String> },
    /// Recusa e grava `never` para esta ferramenta.
    DenyAlways { scope: PolicyScope },
    /// Aprova com argumentos editados pelo usuário.
    AllowEdited { args_json: String },
}

/// Quem autorizou a execução.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalSource {
    User,
    Policy,
    Yolo,
}

// ----------------------------------------------------------------- catálogo ---

/// Origem de uma ferramenta no registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    rename_all_fields = "camelCase"
)]
pub enum ToolOrigin {
    Builtin,
    Mcp { server_id: String },
}

/// Descrição de uma ferramenta exposta ao modelo e à UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpec {
    /// Nome exposto ao modelo (`fs_read`, `github__create_issue`).
    pub name: String,
    pub description: String,
    /// JSON Schema dos parâmetros (flat, com enums — modelos pequenos).
    pub parameters: serde_json::Value,
    pub category: ToolCategory,
    pub tier: ToolTier,
    pub origin: ToolOrigin,
    /// `readOnlyHint` do MCP ou constante da builtin.
    #[serde(default)]
    pub read_only: bool,
}

/// Prévia mostrada ao usuário na barra de aprovação.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    rename_all_fields = "camelCase"
)]
pub enum ToolPreview {
    /// Diff unificado de uma edição de arquivo.
    Diff {
        path: String,
        unified: String,
        created: bool,
    },
    /// Comando de terminal já normalizado + sua classificação.
    Command {
        program: String,
        display: String,
        cwd: String,
        class: CommandClass,
    },
    /// Texto simples (leitura, busca, MCP).
    Text { body: String },
}

// ------------------------------------------------------------------ eventos ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    Running,
    WaitingApproval,
    Done,
    Cancelled,
    Error,
    /// Bateu o teto de passos.
    MaxSteps,
    /// Erros consecutivos demais — devolvido ao usuário.
    Escalated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PauseReason {
    WaitingApproval,
    WaitingElicitation,
    User,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStats {
    pub steps: u32,
    pub tool_calls: u32,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub duration_ms: u64,
}

/// Evento do run. Todos carregam `runId` + `seq` (monotônico por run) para
/// permitir replay/reattach depois de recarregar a interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum RunEventKind {
    #[serde(rename = "run.started")]
    RunStarted {
        chat_id: i64,
        model: String,
        mode: RunMode,
        yolo: bool,
        workspace_dir: Option<String>,
        tools: Vec<String>,
    },
    #[serde(rename = "step.started")]
    StepStarted { step_id: String, index: u32 },
    #[serde(rename = "assistant.delta")]
    AssistantDelta { step_id: String, text: String },
    #[serde(rename = "reasoning.delta")]
    ReasoningDelta { step_id: String, text: String },
    #[serde(rename = "assistant.message")]
    AssistantMessage {
        step_id: String,
        content: String,
        reasoning: String,
    },
    #[serde(rename = "tool.requested")]
    ToolRequested {
        call_id: String,
        tool: String,
        origin: ToolOrigin,
        category: ToolCategory,
        tier: ToolTier,
        args_json: String,
        preview: Option<ToolPreview>,
        requires_approval: bool,
    },
    #[serde(rename = "tool.approved")]
    ToolApproved {
        call_id: String,
        source: ApprovalSource,
    },
    #[serde(rename = "tool.denied")]
    ToolDenied { call_id: String, reason: String },
    #[serde(rename = "tool.started")]
    ToolStarted { call_id: String },
    #[serde(rename = "tool.output")]
    ToolOutput {
        call_id: String,
        chunk: String,
        truncated: bool,
    },
    #[serde(rename = "tool.result")]
    ToolResult {
        call_id: String,
        ok: bool,
        result_preview: String,
        bytes_total: u64,
        duration_ms: u64,
    },
    #[serde(rename = "tool.elicitation")]
    ToolElicitation {
        call_id: String,
        message: String,
        schema_json: String,
    },
    #[serde(rename = "checkpoint.created")]
    CheckpointCreated {
        checkpoint_id: String,
        label: String,
        backend: String,
    },
    #[serde(rename = "focus.updated")]
    FocusUpdated { todo_md: String },
    #[serde(rename = "context.compacted")]
    ContextCompacted {
        tokens_before: u32,
        tokens_after: u32,
    },
    #[serde(rename = "verification")]
    Verification { passed: bool, notes: String },
    #[serde(rename = "run.paused")]
    RunPaused { reason: PauseReason },
    #[serde(rename = "run.resumed")]
    RunResumed,
    #[serde(rename = "run.error")]
    RunError { message: String, retryable: bool },
    #[serde(rename = "run.finished")]
    RunFinished {
        status: RunStatus,
        summary: String,
        usage: UsageStats,
    },
}

/// Envelope entregue à UI (Channel) e persistido em `run_events`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEvent {
    pub run_id: String,
    pub seq: u64,
    pub ts_ms: u64,
    #[serde(flatten)]
    pub event: RunEventKind,
}

impl RunEvent {
    /// Nome curto do evento (coluna `kind` em `run_events`).
    pub fn kind_name(&self) -> &'static str {
        match self.event {
            RunEventKind::RunStarted { .. } => "run.started",
            RunEventKind::StepStarted { .. } => "step.started",
            RunEventKind::AssistantDelta { .. } => "assistant.delta",
            RunEventKind::ReasoningDelta { .. } => "reasoning.delta",
            RunEventKind::AssistantMessage { .. } => "assistant.message",
            RunEventKind::ToolRequested { .. } => "tool.requested",
            RunEventKind::ToolApproved { .. } => "tool.approved",
            RunEventKind::ToolDenied { .. } => "tool.denied",
            RunEventKind::ToolStarted { .. } => "tool.started",
            RunEventKind::ToolOutput { .. } => "tool.output",
            RunEventKind::ToolResult { .. } => "tool.result",
            RunEventKind::ToolElicitation { .. } => "tool.elicitation",
            RunEventKind::CheckpointCreated { .. } => "checkpoint.created",
            RunEventKind::FocusUpdated { .. } => "focus.updated",
            RunEventKind::ContextCompacted { .. } => "context.compacted",
            RunEventKind::Verification { .. } => "verification",
            RunEventKind::RunPaused { .. } => "run.paused",
            RunEventKind::RunResumed => "run.resumed",
            RunEventKind::RunError { .. } => "run.error",
            RunEventKind::RunFinished { .. } => "run.finished",
        }
    }

    /// Deltas de token são ruidosos: não vão para o banco (só para a UI ao
    /// vivo). O texto final chega em `assistant.message`.
    pub fn is_persistable(&self) -> bool {
        !matches!(
            self.event,
            RunEventKind::AssistantDelta { .. }
                | RunEventKind::ReasoningDelta { .. }
                | RunEventKind::ToolOutput { .. }
        )
    }
}

// ------------------------------------------------------------- configuração ---

/// Opções de uma execução, vindas da UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunOptions {
    pub chat_id: i64,
    pub model: String,
    #[serde(default)]
    pub mode: RunMode,
    #[serde(default)]
    pub workspace_dir: Option<String>,
    /// Teto de passos do loop (guard-rail principal).
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,
    /// Servidores MCP habilitados nesta execução.
    #[serde(default)]
    pub mcp_servers: Vec<String>,
    /// Parâmetros de amostragem (espelha `ChatParams` do frontend).
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub top_p: Option<f32>,
    #[serde(default)]
    pub top_k: Option<u32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub system_prompt: Option<String>,
}

fn default_max_steps() -> u32 {
    20
}

impl RunOptions {
    pub fn yolo(&self) -> bool {
        matches!(self.mode, RunMode::Yolo)
    }
}

/// Resumo de um run para listagens e para o painel de execuções.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub id: String,
    pub chat_id: Option<i64>,
    pub model: String,
    pub mode: RunMode,
    pub status: RunStatus,
    pub prompt: String,
    pub summary: Option<String>,
    pub workspace_dir: Option<String>,
    pub created_at: i64,
    pub finished_at: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_event_serializes_flat_with_camel_case_fields() {
        let ev = RunEvent {
            run_id: "r1".into(),
            seq: 3,
            ts_ms: 1_700_000_000_000,
            event: RunEventKind::ToolRequested {
                call_id: "c1".into(),
                tool: "fs_write".into(),
                origin: ToolOrigin::Builtin,
                category: ToolCategory::Edit,
                tier: ToolTier::Caution,
                args_json: "{}".into(),
                preview: Some(ToolPreview::Diff {
                    path: "a.md".into(),
                    unified: "+x".into(),
                    created: true,
                }),
                requires_approval: true,
            },
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["kind"], "tool.requested");
        assert_eq!(v["runId"], "r1");
        assert_eq!(v["callId"], "c1");
        assert_eq!(v["requiresApproval"], true);
        assert_eq!(v["category"], "edit");
        assert_eq!(v["preview"]["kind"], "diff");
        assert_eq!(v["preview"]["path"], "a.md");
        assert_eq!(v["origin"]["kind"], "builtin");
    }

    #[test]
    fn deltas_are_not_persisted_but_messages_are() {
        let mk = |event| RunEvent {
            run_id: "r".into(),
            seq: 1,
            ts_ms: 0,
            event,
        };
        assert!(
            !mk(RunEventKind::AssistantDelta {
                step_id: "s".into(),
                text: "a".into()
            })
            .is_persistable()
        );
        assert!(
            mk(RunEventKind::AssistantMessage {
                step_id: "s".into(),
                content: "a".into(),
                reasoning: String::new()
            })
            .is_persistable()
        );
    }

    #[test]
    fn approval_decision_roundtrip() {
        let d = ApprovalDecision::AllowAlways {
            scope: PolicyScope::Workspace,
        };
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["kind"], "allowAlways");
        assert_eq!(v["scope"], "workspace");
        let back: ApprovalDecision = serde_json::from_value(v).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn run_options_defaults() {
        let o: RunOptions = serde_json::from_str(r#"{"chatId":1,"model":"m"}"#).unwrap();
        assert_eq!(o.max_steps, 20);
        assert_eq!(o.mode, RunMode::Approve);
        assert!(!o.yolo());
    }

    #[test]
    fn kind_name_matches_serde_tag() {
        let ev = RunEvent {
            run_id: "r".into(),
            seq: 0,
            ts_ms: 0,
            event: RunEventKind::RunFinished {
                status: RunStatus::Done,
                summary: String::new(),
                usage: UsageStats::default(),
            },
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["kind"], ev.kind_name());
    }
}
