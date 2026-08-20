// Espelho TS dos contratos do harness agêntico.
// Fonte da verdade: src-tauri/crates/types/src/agent.rs (serde camelCase).
// Qualquer mudança aqui exige a mudança correspondente lá — e vice-versa.

import type { WorkMode } from "./scout";

export type RunMode = "chat" | "approve" | "smart" | "yolo";

export type ToolCategory = "read" | "edit" | "execute" | "network" | "mcp" | "meta";

export type ToolTier = "safe" | "caution" | "danger";

/** Papel de um subagente: `explorer` só lê, `coder` também altera. */
export type SubagentRole = "explorer" | "coder";

/** Família de ferramentas: a pessoa liga/desliga famílias inteiras. */
export type ToolGroup =
  | "files"
  | "terminal"
  | "code"
  | "git"
  | "data"
  | "web"
  | "memory"
  | "project"
  | "plan"
  | "mcp"
  | "desktop";

/** Ordem em que os grupos aparecem nas preferências. */
export const TOOL_GROUPS: ToolGroup[] = [
  "files",
  "terminal",
  "code",
  "git",
  "data",
  "web",
  "memory",
  "project",
  "plan",
  "mcp",
  "desktop",
];

export type ToolPolicy = "alwaysAllow" | "ask" | "never";

export type PolicyScope = "global" | "workspace";

export type CommandClass = "readOnly" | "mutating" | "opaque";

export type ApprovalSource = "user" | "policy" | "yolo";

export type ApprovalDecision =
  | { kind: "allowOnce" }
  | { kind: "allowAlways"; scope: PolicyScope }
  | { kind: "deny"; reason: string | null }
  | { kind: "denyAlways"; scope: PolicyScope }
  | { kind: "allowEdited"; argsJson: string };

export type ToolOrigin =
  | { kind: "builtin" }
  | { kind: "mcp"; serverId: string };

export interface ToolSpec {
  name: string;
  description: string;
  parameters: unknown;
  category: ToolCategory;
  tier: ToolTier;
  origin: ToolOrigin;
  readOnly: boolean;
}

export type ToolPreview =
  | { kind: "diff"; path: string; unified: string; created: boolean }
  | {
      kind: "command";
      program: string;
      display: string;
      cwd: string;
      class: CommandClass;
    }
  | { kind: "text"; body: string };

export type RunStatus =
  | "running"
  | "waitingApproval"
  | "done"
  | "cancelled"
  | "error"
  | "maxSteps"
  | "escalated";

export type PauseReason = "waitingApproval" | "waitingElicitation" | "user";

export interface UsageStats {
  steps: number;
  toolCalls: number;
  promptTokens: number;
  completionTokens: number;
  durationMs: number;
}

/** Envelope: runId/seq/tsMs achatados com os campos do evento. */
interface RunEventBase {
  runId: string;
  seq: number;
  tsMs: number;
}

export type RunEvent = RunEventBase &
  (
    | {
        kind: "run.started";
        chatId: number;
        model: string;
        mode: RunMode;
        yolo: boolean;
        workspaceDir: string | null;
        tools: string[];
      }
    | { kind: "step.started"; stepId: string; index: number }
    | { kind: "assistant.delta"; stepId: string; text: string }
    | { kind: "reasoning.delta"; stepId: string; text: string }
    | {
        kind: "assistant.message";
        stepId: string;
        content: string;
        reasoning: string;
      }
    | {
        kind: "tool.requested";
        callId: string;
        tool: string;
        origin: ToolOrigin;
        category: ToolCategory;
        tier: ToolTier;
        argsJson: string;
        preview: ToolPreview | null;
        requiresApproval: boolean;
      }
    | { kind: "tool.approved"; callId: string; source: ApprovalSource }
    | { kind: "tool.denied"; callId: string; reason: string }
    | { kind: "tool.started"; callId: string }
    | { kind: "tool.output"; callId: string; chunk: string; truncated: boolean }
    | {
        kind: "tool.result";
        callId: string;
        ok: boolean;
        resultPreview: string;
        bytesTotal: number;
        durationMs: number;
      }
    | {
        kind: "tool.elicitation";
        callId: string;
        message: string;
        schemaJson: string;
      }
    | {
        kind: "checkpoint.created";
        checkpointId: string;
        label: string;
        backend: string;
      }
    | { kind: "focus.updated"; todoMd: string }
    | { kind: "context.compacted"; tokensBefore: number; tokensAfter: number }
    | { kind: "verification"; passed: boolean; notes: string }
    | { kind: "run.paused"; reason: PauseReason }
    | { kind: "run.resumed" }
    | { kind: "run.error"; message: string; retryable: boolean }
    | {
        kind: "subagent.started";
        callId: string;
        objective: string;
        role: SubagentRole;
      }
    | {
        kind: "subagent.finished";
        callId: string;
        status: RunStatus;
        steps: number;
        summary: string;
      }
    | {
        kind: "tools.selected";
        available: number;
        /** No início do run, o cardápio inteiro; num pedido do modelo
         * (`requested`), só as ferramentas que acabaram de entrar. */
        active: string[];
        limit: number;
        requested: boolean;
      }
    | {
        kind: "tools.off";
        /** `unsupported`: o template do modelo não renderiza `tools`.
         *  `rejected`: o servidor recusou o pedido com ferramentas. */
        reason: "unsupported" | "rejected";
      }
    | {
        kind: "run.finished";
        status: RunStatus;
        summary: string;
        usage: UsageStats;
      }
  );

export interface RunOptions {
  chatId: number;
  model: string;
  mode?: RunMode;
  workspaceDir?: string | null;
  maxSteps?: number;
  mcpServers?: string[];
  temperature?: number;
  topP?: number;
  topK?: number;
  maxTokens?: number | null;
  systemPrompt?: string;
  /** O modelo escreve um programa em vez de pedir uma ferramenta por passo. */
  codeMode?: boolean;
}

export interface RunSummary {
  id: string;
  chatId: number | null;
  model: string;
  mode: RunMode;
  status: RunStatus;
  prompt: string;
  summary: string | null;
  workspaceDir: string | null;
  /**
   * Contas da execução, quando ela já terminou. Modelo local cobra em tempo e
   * tokens — é o que a tela de atividade mostra no lugar de um preço.
   */
  usage?: UsageStats | null;
  /** Veredito da conferência automática (`null` = não houve conferência). */
  verification?: { passed: boolean; notes: string } | null;
  /** O run tem plano de trabalho gravado — o cartão pode buscá-lo. */
  hasPlan?: boolean;
  /** Como o agente trabalhou (persistido; sobrevive a reload). */
  workMode?: WorkMode;
  /** Epoch em SEGUNDOS (a unidade da tabela `runs`, não a dos eventos). */
  createdAt: number;
  finishedAt: number | null;
}

// --------------------------------------------------------------- permissões ---

export interface ToolPermissionRow {
  id: number;
  scope: PolicyScope;
  workspaceDir: string | null;
  toolName: string;
  policy: ToolPolicy;
}

export interface CheckpointRow {
  id: string;
  runId: string | null;
  workspaceDir: string;
  backend: string;
  label: string | null;
  createdAt: number;
}

// --------------------------------------------------------------------- MCP ---

export type McpTransport = "stdio" | "http";

export interface McpServerRow {
  id: string;
  name: string;
  transport: McpTransport;
  configJson: string;
  enabled: boolean;
  /** Hash atual das definições de tools (muda = pede re-aprovação). */
  toolsHash: string | null;
  /** Hash que o usuário aprovou. */
  toolsApprovedHash: string | null;
}

export interface McpToolView {
  name: string;
  description: string;
  readOnly: boolean;
  destructive: boolean;
  openWorld: boolean;
}

export interface McpServerStatus {
  id: string;
  connected: boolean;
  error: string | null;
  tools: McpToolView[];
  needsApproval: boolean;
}

// ------------------------------------------------------------------ memória ---

export interface MemoryFact {
  id: number;
  workspaceDir: string | null;
  content: string;
  sourceRunId: string | null;
  createdAt: number;
}

// ---------------------------------------------------------------------- RAG ---

export interface RagStatus {
  workspaceDir: string;
  indexing: boolean;
  filesIndexed: number;
  filesTotal: number;
  chunks: number;
  embedModel: string | null;
  vectorSearch: boolean;
  error: string | null;
}

export interface RagHit {
  path: string;
  snippet: string;
  score: number;
  line: number | null;
}

// ------------------------------------------------------- props do servidor ---

export interface ChatTemplateCaps {
  supportsTools: boolean;
  supportsParallelToolCalls: boolean;
  supportsSystemRole: boolean;
}

export interface ServerProps {
  modelPath: string | null;
  chatTemplateCaps: ChatTemplateCaps;
  nCtx: number | null;
  modalities: string[];
  /** `"router"` quando quem respondeu foi o roteador, não um modelo. */
  role: string | null;
}

/**
 * Esta resposta fala de um modelo?
 *
 * No modo Router, `/props` sem `?model=` devolve as props do ROTEADOR — e
 * elas parecem um modelo sem capacidade nenhuma. Quem lê `supportsTools`
 * precisa passar por aqui antes, senão "ainda não sei" vira "não suporta" e
 * a tela desabilita o modo agente sozinha.
 */
export function describesModel(p: ServerProps | null): p is ServerProps {
  return p != null && p.role !== "router" && p.modelPath != null;
}
