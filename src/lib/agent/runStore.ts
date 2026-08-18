// Estado dos runs do agente (um por conversa), no padrão do projeto:
// `useSyncExternalStore` com `subscribe`/`get`, igual a `generationStore.ts`
// e `chatStore.ts`. O loop vive no Rust — aqui só reduzimos `RunEvent` em
// uma linha do tempo renderizável.
//
// Três invariantes que este módulo protege:
//  1. Replay idempotente: todo evento tem `seq` monotônico por run; eventos
//     com `seq` menor ou igual ao último visto são IGNORADOS. É o que deixa
//     o `run_attach` reenviar o histórico sem duplicar cards.
//  2. Reattach: navegar para outra tela DESMONTA o Chat. O run continua no
//     Rust; ao voltar, `ensureAttached` reconstrói a trilha (`run_events_list`)
//     e reabre o canal (`run_attach`) a partir do último `seq`.
//  3. Preparação do servidor é a MESMA do chat normal (`serverSession.ts`):
//     `ensureServer` + `matchServerModel`, sem duplicar a lógica.

import { ensureServer, errorMessage, listLoadedModels, matchServerModel } from "../serverSession";
import type { ChatParams } from "../types";
import {
  runAnswer,
  runApprove,
  runAttach,
  runCallOutputs,
  runCancel,
  runEventsList,
  runPlanApprove,
  runPlanGet,
  runPlanReplan,
  runStart,
  runsList,
  type RunCallOutput,
  type RunStartOptions,
} from "./agentApi";
import type { TaskPlan, WorkMode } from "./scout";
import type {
  ApprovalDecision,
  ApprovalSource,
  RunEvent,
  RunMode,
  RunStatus,
  RunSummary,
  SubagentRole,
  ToolCategory,
  ToolOrigin,
  ToolPreview,
  ToolTier,
  UsageStats,
} from "./types";

/** Estado de uma chamada de ferramenta ao longo do run. */
export type ToolCallState =
  | "waiting"
  | "approved"
  | "denied"
  | "running"
  | "ok"
  | "failed";

export interface ToolCallView {
  callId: string;
  tool: string;
  origin: ToolOrigin;
  category: ToolCategory;
  tier: ToolTier;
  argsJson: string;
  preview: ToolPreview | null;
  requiresApproval: boolean;
  state: ToolCallState;
  approvalSource: ApprovalSource | null;
  denyReason: string | null;
  /** Saída em streaming (terminal); limitada para não estourar a memória. */
  output: string;
  outputTruncated: boolean;
  resultPreview: string;
  bytesTotal: number;
  durationMs: number | null;
}

/** Avisos do run que viram uma linha discreta na trilha. */
export type NoteKind =
  | "compacted"
  | "verified"
  | "verifyFailed"
  | "error"
  | "elicitation"
  /** O run está sem ferramentas porque o template do modelo não as aceita. */
  | "toolsUnsupported"
  /** O servidor recusou o pedido com ferramentas; seguiu sem elas. */
  | "toolsRejected";

export interface StepItem {
  kind: "step";
  id: string;
  index: number;
  text: string;
  reasoning: string;
  /** Quando o passo começou — é o relógio do "Pensando…" da trilha. */
  startedAtMs: number;
  /** Quanto durou o raciocínio, fechado quando a resposta começa a sair.
   *  `null` enquanto ele ainda está pensando. */
  thoughtMs: number | null;
  /** Quando o passo terminou (tsMs do `assistant.message`, que É persistido
   *  — ao contrário dos deltas). É o que faz o tempo por etapa sobreviver
   *  ao recarregar. */
  finishedAtMs: number | null;
}

/**
 * Cardápio entregue ao modelo. Vira uma linha própria (e não um `note`)
 * porque a trilha precisa dos nomes das ferramentas para traduzi-los.
 */
export interface ToolsItem {
  kind: "tools";
  id: string;
  /** Ferramentas desta entrega — o cardápio inicial ou o que foi pedido. */
  active: string[];
  /** Tamanho do catálogo habilitado, para a linha "N de M". */
  available: number;
  /** `true` quando o próprio modelo pediu mais ferramentas no meio do run. */
  requested: boolean;
  tsMs: number;
}

/**
 * Começo ou fim de um ajudante. São dois itens, e não um só com o conteúdo
 * dentro: o ajudante roda dentro da chamada de ferramenta do agente, então
 * tudo que a trilha recebe entre um marcador e o outro é dele — a marcação
 * em pares é o que deixa a trilha desenhar isso recuado sem cada evento
 * precisar carregar um identificador de dono.
 */
export interface SubagentItem {
  kind: "subagent";
  id: string;
  /** `end` fecha o par aberto pelo `start` de mesmo `callId`. */
  phase: "start" | "end";
  callId: string;
  objective: string;
  role: SubagentRole;
  status: RunStatus | null;
  steps: number;
  summary: string;
  tsMs: number;
}

export type TimelineItem =
  | StepItem
  | { kind: "tool"; id: string }
  | { kind: "checkpoint"; id: string; label: string; backend: string; tsMs: number }
  | ToolsItem
  | SubagentItem
  | { kind: "note"; id: string; note: NoteKind; detail: string; tsMs: number };

export interface CheckpointMark {
  id: string;
  label: string;
  backend: string;
  tsMs: number;
}

export interface RunView {
  runId: string;
  chatId: number;
  model: string;
  mode: RunMode;
  /** Como o agente trabalha neste run (Scout Rule). */
  workMode: WorkMode;
  yolo: boolean;
  workspaceDir: string | null;
  /** Ferramentas expostas ao modelo neste run. */
  toolNames: string[];
  status: RunStatus;
  lastSeq: number;
  items: TimelineItem[];
  tools: Record<string, ToolCallView>;
  /** Ferramenta esperando decisão do usuário (alimenta a ApprovalBar). */
  pendingCallId: string | null;
  focusMd: string;
  /**
   * Plano estruturado (Scout Rule). O evento só traz o markdown; os campos
   * que o quadro precisa vêm do `run_plan_get`, buscado com throttle a cada
   * `focus.updated`. `null` = ainda não há plano.
   */
  plan: TaskPlan | null;
  checkpoints: CheckpointMark[];
  usage: UsageStats | null;
  summary: string;
  error: string | null;
  startedAtMs: number;
  finishedAtMs: number | null;
  /** Canal aberto: falso enquanto o reattach não terminou. */
  attached: boolean;
}

export interface RunSnapshot {
  runs: RunView[];
  /** chatId → runId do run corrente daquela conversa. */
  byChat: Record<number, string>;
  /** Conversas preparando o servidor (antes do primeiro evento). */
  starting: number[];
  /** Falhas ao iniciar (servidor fora do ar, modelo inexistente...). */
  startErrors: { chatId: number; message: string }[];
}

export interface StartRunOpts {
  chatId: number;
  prompt: string;
  model: string;
  params: ChatParams;
  mode: RunMode;
  /** Modo de trabalho da Scout Rule. Ausente = `"agent"`. */
  workMode?: WorkMode;
  maxSteps?: number;
}

const OUTPUT_CAP = 200_000;

/** Status em que o run ainda consome o composer (Parar visível). */
export function isRunActive(status: RunStatus): boolean {
  return status === "running" || status === "waitingApproval";
}

function emptyRun(runId: string, chatId: number): RunView {
  return {
    runId,
    chatId,
    model: "",
    mode: "smart",
    workMode: "agent",
    yolo: false,
    workspaceDir: null,
    toolNames: [],
    status: "running",
    lastSeq: 0,
    items: [],
    tools: {},
    pendingCallId: null,
    focusMd: "",
    plan: null,
    checkpoints: [],
    usage: null,
    summary: "",
    error: null,
    startedAtMs: Date.now(),
    finishedAtMs: null,
    attached: false,
  };
}

function placeholderTool(callId: string): ToolCallView {
  return {
    callId,
    tool: callId,
    origin: { kind: "builtin" },
    category: "meta",
    tier: "safe",
    argsJson: "",
    preview: null,
    requiresApproval: false,
    state: "running",
    approvalSource: null,
    denyReason: null,
    output: "",
    outputTruncated: false,
    resultPreview: "",
    bytesTotal: 0,
    durationMs: null,
  };
}

function countSteps(items: TimelineItem[]): number {
  return items.reduce((n, i) => (i.kind === "step" ? n + 1 : n), 0);
}

/** Atualiza (ou cria) o passo `stepId` devolvendo uma lista NOVA. */
function withStep(
  items: TimelineItem[],
  stepId: string,
  fn: (step: StepItem) => StepItem,
  /** Relógio do evento — só usado quando o passo é criado agora. */
  tsMs = 0,
): TimelineItem[] {
  const idx = items.findIndex((i) => i.kind === "step" && i.id === stepId);
  if (idx < 0) {
    const created: StepItem = {
      kind: "step",
      id: stepId,
      index: countSteps(items),
      text: "",
      reasoning: "",
      startedAtMs: tsMs,
      thoughtMs: null,
      finishedAtMs: null,
    };
    return [...items, fn(created)];
  }
  const copy = items.slice();
  copy[idx] = fn(copy[idx] as StepItem);
  return copy;
}

function withTool(
  run: RunView,
  callId: string,
  fn: (tool: ToolCallView) => ToolCallView,
): Record<string, ToolCallView> {
  const current = run.tools[callId] ?? placeholderTool(callId);
  return { ...run.tools, [callId]: fn(current) };
}

function note(
  items: TimelineItem[],
  id: string,
  kind: NoteKind,
  detail: string,
  tsMs: number,
): TimelineItem[] {
  return [...items, { kind: "note", id, note: kind, detail, tsMs }];
}

/**
 * Reduz um evento na visão do run. Devolve o MESMO objeto quando o evento é
 * repetido (`seq` já visto) — é isso que torna o replay idempotente.
 */
export function reduceEvent(run: RunView, event: RunEvent): RunView {
  if (event.seq <= run.lastSeq) return run;
  const next: RunView = { ...run, lastSeq: event.seq };

  switch (event.kind) {
    case "run.started":
      next.chatId = event.chatId;
      next.model = event.model;
      next.mode = event.mode;
      next.yolo = event.yolo;
      next.workspaceDir = event.workspaceDir;
      next.toolNames = event.tools;
      next.status = "running";
      next.startedAtMs = event.tsMs;
      break;

    case "step.started":
      next.items = withStep(
        next.items,
        event.stepId,
        (s) => ({ ...s, index: event.index, startedAtMs: event.tsMs }),
        event.tsMs,
      );
      break;

    case "assistant.delta":
      next.items = withStep(
        next.items,
        event.stepId,
        (s) => ({
          ...s,
          text: s.text + event.text,
          // A resposta começou: o tempo de pensar está fechado.
          thoughtMs: s.thoughtMs ?? Math.max(0, event.tsMs - s.startedAtMs),
        }),
        event.tsMs,
      );
      break;

    case "reasoning.delta":
      next.items = withStep(
        next.items,
        event.stepId,
        (s) => ({ ...s, reasoning: s.reasoning + event.text }),
        event.tsMs,
      );
      break;

    case "assistant.message":
      // O conteúdo final substitui os deltas (evita texto duplicado quando
      // o backend emite ambos). O tsMs deste evento fecha o relógio do
      // passo — e como ele É persistido, o tempo por etapa sobrevive ao
      // recarregar (os deltas, que alimentavam o cronômetro ao vivo, não).
      next.items = withStep(
        next.items,
        event.stepId,
        (s) => ({
          ...s,
          text: event.content || s.text,
          reasoning: event.reasoning || s.reasoning,
          finishedAtMs: event.tsMs,
        }),
        event.tsMs,
      );
      break;

    case "tool.requested": {
      const known = next.tools[event.callId];
      next.tools = {
        ...next.tools,
        [event.callId]: {
          callId: event.callId,
          tool: event.tool,
          origin: event.origin,
          category: event.category,
          tier: event.tier,
          argsJson: event.argsJson,
          preview: event.preview,
          requiresApproval: event.requiresApproval,
          state: event.requiresApproval ? "waiting" : "approved",
          approvalSource: known?.approvalSource ?? null,
          denyReason: null,
          output: known?.output ?? "",
          outputTruncated: known?.outputTruncated ?? false,
          resultPreview: known?.resultPreview ?? "",
          bytesTotal: known?.bytesTotal ?? 0,
          durationMs: known?.durationMs ?? null,
        },
      };
      if (!known) {
        next.items = [...next.items, { kind: "tool", id: event.callId }];
      }
      if (event.requiresApproval) {
        next.pendingCallId = event.callId;
        next.status = "waitingApproval";
      }
      break;
    }

    case "tool.approved":
      next.tools = withTool(next, event.callId, (tool) => ({
        ...tool,
        state: tool.state === "waiting" ? "approved" : tool.state,
        approvalSource: event.source,
      }));
      if (next.pendingCallId === event.callId) next.pendingCallId = null;
      break;

    case "tool.denied":
      next.tools = withTool(next, event.callId, (tool) => ({
        ...tool,
        state: "denied",
        denyReason: event.reason || null,
      }));
      if (next.pendingCallId === event.callId) next.pendingCallId = null;
      break;

    case "tool.started":
      next.tools = withTool(next, event.callId, (tool) => ({
        ...tool,
        state: "running",
      }));
      break;

    case "tool.output":
      next.tools = withTool(next, event.callId, (tool) => {
        const merged = tool.output + event.chunk;
        const overflow = merged.length > OUTPUT_CAP;
        return {
          ...tool,
          output: overflow ? merged.slice(merged.length - OUTPUT_CAP) : merged,
          outputTruncated: tool.outputTruncated || event.truncated || overflow,
        };
      });
      break;

    case "tool.result":
      next.tools = withTool(next, event.callId, (tool) => ({
        ...tool,
        state: event.ok ? "ok" : "failed",
        resultPreview: event.resultPreview,
        bytesTotal: event.bytesTotal,
        durationMs: event.durationMs,
      }));
      break;

    case "tool.elicitation":
      next.items = note(
        next.items,
        `elicit-${event.seq}`,
        "elicitation",
        event.message,
        event.tsMs,
      );
      break;

    case "checkpoint.created":
      if (!next.checkpoints.some((c) => c.id === event.checkpointId)) {
        const mark: CheckpointMark = {
          id: event.checkpointId,
          label: event.label,
          backend: event.backend,
          tsMs: event.tsMs,
        };
        next.checkpoints = [...next.checkpoints, mark];
        next.items = [
          ...next.items,
          {
            kind: "checkpoint",
            id: mark.id,
            label: mark.label,
            backend: mark.backend,
            tsMs: mark.tsMs,
          },
        ];
      }
      break;

    case "tools.selected":
      // O cardápio do começo substitui; o que o modelo pediu depois SOMA —
      // ferramenta ativada por ele não sai mais do alcance no meio do run.
      next.toolNames = event.requested
        ? [
            ...next.toolNames,
            ...event.active.filter((name) => !next.toolNames.includes(name)),
          ]
        : event.active;
      next.items = [
        ...next.items,
        {
          kind: "tools",
          id: `tools-${event.seq}`,
          active: event.active,
          available: event.available,
          requested: event.requested,
          tsMs: event.tsMs,
        },
      ];
      break;

    // Sem ferramentas o agente vira um chat, e a diferença precisa estar
    // ESCRITA: senão a pessoa lê "não tenho acesso à internet" e conclui que
    // o modelo é ruim, quando pode ter sido o app.
    case "tools.off":
      next.toolNames = [];
      next.items = note(
        next.items,
        `tools-off-${event.seq}`,
        event.reason === "rejected" ? "toolsRejected" : "toolsUnsupported",
        "",
        event.tsMs,
      );
      break;

    case "subagent.started":
      next.items = [
        ...next.items,
        {
          kind: "subagent",
          id: `sub-${event.callId}`,
          phase: "start",
          callId: event.callId,
          objective: event.objective,
          role: event.role,
          status: null,
          steps: 0,
          summary: "",
          tsMs: event.tsMs,
        },
      ];
      break;

    case "subagent.finished":
      next.items = [
        ...next.items,
        {
          kind: "subagent",
          id: `sub-${event.callId}-end`,
          phase: "end",
          callId: event.callId,
          objective: "",
          role: "explorer",
          status: event.status,
          steps: event.steps,
          summary: event.summary,
          tsMs: event.tsMs,
        },
      ];
      break;

    case "focus.updated":
      next.focusMd = event.todoMd;
      break;

    case "context.compacted":
      next.items = note(
        next.items,
        `compact-${event.seq}`,
        "compacted",
        `${event.tokensBefore} → ${event.tokensAfter}`,
        event.tsMs,
      );
      break;

    case "verification":
      next.items = note(
        next.items,
        `verify-${event.seq}`,
        event.passed ? "verified" : "verifyFailed",
        event.notes,
        event.tsMs,
      );
      break;

    case "run.paused":
      if (event.reason === "waitingApproval") next.status = "waitingApproval";
      break;

    case "run.resumed":
      if (next.status === "waitingApproval") next.status = "running";
      break;

    case "run.error":
      next.error = event.message;
      next.items = note(
        next.items,
        `error-${event.seq}`,
        "error",
        event.message,
        event.tsMs,
      );
      break;

    case "run.finished":
      next.status = event.status;
      next.summary = event.summary;
      next.usage = event.usage;
      next.finishedAtMs = event.tsMs;
      next.pendingCallId = null;
      break;
  }

  return next;
}

/** Monta uma visão a partir de uma lista de eventos (histórico/replay). */
export function buildRunView(runId: string, events: RunEvent[]): RunView {
  let view = emptyRun(runId, -1);
  for (const event of events) view = reduceEvent(view, event);
  return view;
}

/** Extrai o `content` do `result_json` gravado; linha ilegível vira vazio. */
function storedContent(resultJson: string | null): string {
  if (!resultJson) return "";
  try {
    const parsed = JSON.parse(resultJson) as { content?: unknown };
    return typeof parsed.content === "string" ? parsed.content : "";
  } catch {
    return "";
  }
}

/**
 * Hidrata a visão reconstruída com as saídas que ficaram no banco.
 *
 * O replay dos eventos não traz `tool.output` (o streaming não é persistido
 * de propósito), então a trilha reaberta ficava só com o preview de 400
 * caracteres — um `terminal_run` que falhou perdia o stdout inteiro. Só
 * preenche campo VAZIO: saída viva, recém-reduzida dos eventos, nunca é
 * sobrescrita.
 */
function hydrateStoredOutputs(view: RunView, rows: RunCallOutput[]): RunView {
  if (rows.length === 0) return view;
  let tools = view.tools;
  let changed = false;
  for (const row of rows) {
    const tool = tools[row.callId];
    if (!tool) continue;
    let next = tool;
    if (next.output === "") {
      // Sucesso: o conteúdo completo. Falha: a mensagem inteira de erro —
      // é o que o modelo viu, e o preview a cortava em 400 caracteres.
      const content =
        storedContent(row.resultJson) ||
        (next.state === "failed" && row.error ? row.error : "");
      if (content !== "") {
        const overflow = content.length > OUTPUT_CAP;
        next = {
          ...next,
          output: overflow
            ? content.slice(content.length - OUTPUT_CAP)
            : content,
          outputTruncated: next.outputTruncated || overflow,
        };
      }
    }
    // Negação sem motivo na trilha (evento antigo/estragado): o banco tem.
    if (next.state === "denied" && !next.denyReason && row.error) {
      next = { ...next, denyReason: row.error };
    }
    if (next !== tool) {
      if (!changed) tools = { ...tools };
      tools[row.callId] = next;
      changed = true;
    }
  }
  return changed ? { ...view, tools } : view;
}

/**
 * Carrega um run antigo (usado pelo painel de execução). Não vira ativo.
 *
 * O modo de trabalho não viaja nos eventos: vem da memória da sessão
 * (`workModeByRun`) e cai no padrão depois de um reload do app.
 */
export async function loadRunView(runId: string): Promise<RunView> {
  const events = await runEventsList(runId, 0);
  let view = buildRunView(runId, events);
  // A saída completa das ferramentas não viaja nos eventos: vem do banco.
  view = hydrateStoredOutputs(view, await runCallOutputs(runId));
  view.workMode = workModeByRun.get(runId) ?? view.workMode;
  view.plan = await runPlanGet(runId).catch(() => null);
  return view;
}

// ------------------------------------------------------------------ store ---

const runs = new Map<string, RunView>();
const byChat = new Map<number, string>();
const starting = new Set<number>();
const startErrors = new Map<number, string>();
const deletedChats = new Set<number>();
const attaching = new Set<string>();
/** Parar apertado antes de o `run_start` responder. */
const cancelPending = new Set<number>();
/** Modo de trabalho escolhido na conversa (o evento `run.started` não traz). */
const workModeByChat = new Map<number, WorkMode>();
/** O mesmo por run, para o painel de execução reabrir com o modo certo. */
const workModeByRun = new Map<string, WorkMode>();

let snap: RunSnapshot = {
  runs: [],
  byChat: {},
  starting: [],
  startErrors: [],
};
const listeners = new Set<() => void>();

function emit(): void {
  snap = {
    runs: [...runs.values()],
    byChat: Object.fromEntries(byChat),
    starting: [...starting],
    startErrors: [...startErrors].map(([chatId, message]) => ({
      chatId,
      message,
    })),
  };
  for (const fn of listeners) fn();
}

// ------------------------------------------------- tok/s ao vivo (agente) ---
//
// O chat normal mede tok/s no próprio stream do navegador; o laço do agente
// vive no Rust e os timings só chegam no FIM de cada passo. Para a barra de
// status não ficar muda por minutos (que é o que faz "trabalhando" parecer
// "travou"), estimamos ao vivo pelos deltas: caracteres ÷ 4 numa janela
// deslizante. É estimativa e a interface a mostra como tal (~).
const TPS_JANELA_MS = 6_000;
const tpsAmostras = new Map<string, { t: number; chars: number }[]>();

function notaDelta(runId: string, chars: number): void {
  const agora = Date.now();
  const lista = tpsAmostras.get(runId) ?? [];
  lista.push({ t: agora, chars });
  while (lista.length > 0 && agora - lista[0].t > TPS_JANELA_MS) {
    lista.shift();
  }
  tpsAmostras.set(runId, lista);
}

/** Estimativa de tokens/s do run vivo desta conversa; `null` sem atividade. */
function liveTpsOf(chatId: number | null): number | null {
  if (chatId == null) return null;
  const runId = byChat.get(chatId);
  if (!runId) return null;
  const lista = tpsAmostras.get(runId);
  if (!lista || lista.length < 3) return null;
  const agora = Date.now();
  const ultimo = lista[lista.length - 1];
  // Sem delta há mais de 2,5 s (pensando, rodando ferramenta): não há taxa.
  if (agora - ultimo.t > 2_500) return null;
  const primeiro = lista[0];
  const dt = (ultimo.t - primeiro.t) / 1000;
  if (dt < 0.5) return null;
  const chars = lista.reduce((s, a) => s + a.chars, 0);
  return chars / 4 / dt;
}

function handleEvent(event: RunEvent): void {
  if (event.kind === "assistant.delta" || event.kind === "reasoning.delta") {
    notaDelta(event.runId, event.text.length);
  } else if (event.kind === "run.finished") {
    tpsAmostras.delete(event.runId);
  }
  const known = runs.get(event.runId);
  let base =
    known ??
    emptyRun(event.runId, event.kind === "run.started" ? event.chatId : -1);
  if (!known) {
    const chosen =
      workModeByRun.get(base.runId) ??
      (base.chatId >= 0 ? workModeByChat.get(base.chatId) : undefined);
    if (chosen) {
      base = { ...base, workMode: chosen };
      workModeByRun.set(base.runId, chosen);
    }
  }
  const next = reduceEvent(base, event);
  if (known && next === known) return; // seq repetido
  next.attached = true;
  runs.set(next.runId, next);
  if (next.chatId >= 0 && byChat.get(next.chatId) !== next.runId) {
    byChat.set(next.chatId, next.runId);
  }
  emit();
  // O plano mudou de forma (etapa começou/terminou): recarrega o estruturado.
  if (event.kind === "focus.updated" || event.kind === "run.finished") {
    schedulePlanFetch(next.runId);
  }
}

// ------------------------------------------------------- plano (Scout) ---

/** Janela mínima entre duas leituras do plano do mesmo run. */
const PLAN_THROTTLE_MS = 500;

const planTimers = new Map<string, number>();
const planFetchedAt = new Map<string, number>();

async function fetchPlan(runId: string): Promise<void> {
  const plan = await runPlanGet(runId).catch(() => null);
  // Sem plano (run comum, comando ausente): mantém o que já estava na tela.
  if (!plan) return;
  const run = runs.get(runId);
  if (!run) return;
  runs.set(runId, { ...run, plan });
  emit();
}

/**
 * Agenda a leitura do plano respeitando a janela de throttle. Vários
 * `focus.updated` seguidos viram UMA busca — e a última sempre acontece,
 * porque a chamada pendente é agendada, não descartada.
 */
function schedulePlanFetch(runId: string): void {
  if (planTimers.has(runId)) return;
  const elapsed = Date.now() - (planFetchedAt.get(runId) ?? 0);
  const wait = Math.max(0, PLAN_THROTTLE_MS - elapsed);
  const timer = window.setTimeout(() => {
    planTimers.delete(runId);
    planFetchedAt.set(runId, Date.now());
    void fetchPlan(runId);
  }, wait);
  planTimers.set(runId, timer);
}

function forgetPlan(runId: string): void {
  const timer = planTimers.get(runId);
  if (timer) window.clearTimeout(timer);
  planTimers.delete(runId);
  planFetchedAt.delete(runId);
}

function patchRun(runId: string, patch: Partial<RunView>): void {
  const run = runs.get(runId);
  if (!run) return;
  runs.set(runId, { ...run, ...patch });
  emit();
}

async function attachTo(run: RunView): Promise<void> {
  if (attaching.has(run.runId)) return;
  attaching.add(run.runId);
  try {
    await runAttach(run.runId, run.lastSeq, handleEvent);
    patchRun(run.runId, { attached: true });
  } catch (e) {
    // Run já encerrado no backend (ou app reiniciado): a trilha reconstruída
    // pelo `run_events_list` continua na tela; só não há mais streaming.
    console.warn("run_attach falhou:", errorMessage(e));
    patchRun(run.runId, { attached: false });
  } finally {
    attaching.delete(run.runId);
  }
}

export const runStore = {
  subscribe(fn: () => void): () => void {
    listeners.add(fn);
    return () => {
      listeners.delete(fn);
    };
  },

  get(): RunSnapshot {
    return snap;
  },

  /** Run corrente da conversa (ativo ou recém-terminado, até ser dispensado). */
  runFor(chatId: number | null): RunView | undefined {
    if (chatId == null) return undefined;
    const runId = byChat.get(chatId);
    return runId ? runs.get(runId) : undefined;
  },

  runById(runId: string): RunView | undefined {
    return runs.get(runId);
  },

  activeRuns(): RunView[] {
    return snap.runs.filter((r) => isRunActive(r.status));
  },

  /** Conversa ocupada: preparando o servidor ou com run em andamento. */
  /** Tokens/s estimados do run vivo (qualquer conversa): o maior ativo. */
  liveTps(): number | null {
    for (const [chatId, runId] of byChat) {
      const run = runs.get(runId);
      if (run && isRunActive(run.status)) {
        const tps = liveTpsOf(chatId);
        if (tps != null) return tps;
      }
    }
    return null;
  },

  isBusy(chatId: number | null): boolean {
    if (chatId == null) return false;
    if (starting.has(chatId)) return true;
    const run = this.runFor(chatId);
    return run != null && isRunActive(run.status);
  },

  isStarting(chatId: number | null): boolean {
    return chatId != null && starting.has(chatId);
  },

  startErrorFor(chatId: number | null): string | null {
    if (chatId == null) return null;
    return startErrors.get(chatId) ?? null;
  },

  clearStartError(chatId: number): void {
    if (!startErrors.delete(chatId)) return;
    emit();
  },

  /** Plano estruturado do run corrente da conversa. */
  planFor(chatId: number | null): TaskPlan | null {
    return this.runFor(chatId)?.plan ?? null;
  },

  /** Ferramenta aguardando decisão do usuário nesta conversa. */
  pendingApproval(chatId: number | null): ToolCallView | null {
    const run = this.runFor(chatId);
    if (!run || !run.pendingCallId) return null;
    return run.tools[run.pendingCallId] ?? null;
  },

  /**
   * Sobe o servidor, resolve o modelo e dispara o run. Nunca lança: a falha
   * fica em `startErrorFor(chatId)` para a tela mostrar.
   */
  async start(opts: StartRunOpts): Promise<string | null> {
    const { chatId } = opts;
    if (deletedChats.has(chatId) || this.isBusy(chatId)) return null;
    starting.add(chatId);
    startErrors.delete(chatId);
    cancelPending.delete(chatId);
    const workMode: WorkMode = opts.workMode ?? "agent";
    workModeByChat.set(chatId, workMode);
    // Run anterior sai da tela quando um novo começa nesta conversa.
    const previous = byChat.get(chatId);
    if (previous) {
      const prev = runs.get(previous);
      if (prev && !isRunActive(prev.status)) runs.delete(previous);
      byChat.delete(chatId);
    }
    emit();

    try {
      const { baseUrl } = await ensureServer();
      const loaded = await listLoadedModels(baseUrl);
      const model = matchServerModel(opts.model, loaded);
      const runOptions: RunStartOptions = {
        chatId,
        model,
        mode: opts.mode,
        workMode,
        workspaceDir: opts.params.workspaceDir,
        temperature: opts.params.temperature,
        topP: opts.params.topP,
        topK: opts.params.topK,
        maxTokens: opts.params.maxTokens,
        systemPrompt: opts.params.systemPrompt || undefined,
        maxSteps: opts.maxSteps,
        codeMode: opts.params.codeMode === true,
      };
      const runId = await runStart(opts.prompt, runOptions, handleEvent);
      workModeByRun.set(runId, workMode);
      const started = runs.get(runId);
      if (started) {
        // O `run.started` chegou antes da resposta do comando: só carimba o
        // modo de trabalho, que não viaja no evento.
        runs.set(runId, { ...started, workMode });
      } else {
        // O primeiro evento pode chegar depois da resposta do comando.
        const view = emptyRun(runId, chatId);
        view.model = model;
        view.mode = opts.mode;
        view.workMode = workMode;
        view.yolo = opts.mode === "yolo";
        view.workspaceDir = opts.params.workspaceDir;
        view.attached = true;
        runs.set(runId, view);
      }
      byChat.set(chatId, runId);
      if (cancelPending.delete(chatId)) void runCancel(runId).catch(() => {});
      return runId;
    } catch (e) {
      startErrors.set(chatId, errorMessage(e));
      return null;
    } finally {
      starting.delete(chatId);
      emit();
    }
  },

  /** Envia a decisão do usuário (otimista: a barra some na hora). */
  async approve(
    runId: string,
    callId: string,
    decision: ApprovalDecision,
  ): Promise<void> {
    const run = runs.get(runId);
    if (run) {
      const denied = decision.kind === "deny" || decision.kind === "denyAlways";
      const tool = run.tools[callId];
      runs.set(runId, {
        ...run,
        pendingCallId: run.pendingCallId === callId ? null : run.pendingCallId,
        status: run.status === "waitingApproval" ? "running" : run.status,
        tools: tool
          ? {
              ...run.tools,
              [callId]: { ...tool, state: denied ? "denied" : "approved" },
            }
          : run.tools,
      });
      emit();
    }
    try {
      await runApprove(runId, callId, decision);
    } catch (e) {
      console.warn("run_approve falhou:", errorMessage(e));
    }
  },

  /**
   * Libera a execução do plano proposto (modo planejamento). Otimista: o
   * quadro já mostra "aprovado" antes da confirmação do backend.
   */
  async approvePlan(chatId: number | null): Promise<void> {
    const run = this.runFor(chatId);
    if (!run) return;
    if (run.plan && !run.plan.approved) {
      runs.set(run.runId, { ...run, plan: { ...run.plan, approved: true } });
      emit();
    }
    // Aprovar abre uma execução NOVA (mesmo plano, modo laço). Sem amarrar a
    // conversa a ela, o chat continuaria olhando para o run do planejamento,
    // que já acabou — e a execução aprovada rodaria invisível.
    const novo = await runPlanApprove(run.runId, handleEvent);
    if (novo && novo !== run.runId) {
      workModeByRun.set(novo, "loop");
      if (chatId != null) byChat.set(chatId, novo);
      emit();
      schedulePlanFetch(novo);
      return;
    }
    schedulePlanFetch(run.runId);
  },

  /** Pede outra divisão do mesmo objetivo (volta para "dividindo a tarefa"). */
  /**
   * Responde a uma execução que parou esperando a pessoa (escalada com
   * pergunta, etapa bloqueada) e ADOTA a continuação: o run novo assume a
   * conversa, senão a resposta rodaria invisível enquanto o chat olha para o
   * run que já acabou.
   */
  async answer(chatId: number | null, runId: string, text: string): Promise<boolean> {
    const novo = await runAnswer(runId, text, handleEvent);
    if (!novo) return false;
    if (novo !== runId) {
      if (chatId != null) byChat.set(chatId, novo);
      emit();
      schedulePlanFetch(novo);
    }
    return true;
  },

  async replan(chatId: number | null): Promise<void> {
    const run = this.runFor(chatId);
    if (!run) return;
    runs.set(run.runId, { ...run, plan: null });
    emit();
    await runPlanReplan(run.runId);
    schedulePlanFetch(run.runId);
  },

  /** Parar: cancela o run em curso (ou o start ainda em preparação). */
  cancel(chatId: number | null): void {
    if (chatId == null) return;
    if (starting.has(chatId)) cancelPending.add(chatId);
    const run = this.runFor(chatId);
    if (!run) return;
    void runCancel(run.runId).catch((e) =>
      console.warn("run_cancel falhou:", errorMessage(e)),
    );
  },

  /**
   * Reata a uma execução avulsa: a de uma automação, que não pertence a
   * conversa nenhuma.
   *
   * Reconstrói a trilha pelos eventos gravados e, se ela ainda estiver viva,
   * abre o canal. É isso que permite responder a um pedido de confirmação de
   * fora do chat — sem o canal, a automação ficaria parada esperando uma
   * resposta que não teria por onde chegar.
   */
  async attachToRun(runId: string): Promise<void> {
    const known = runs.get(runId);
    if (known) {
      if (isRunActive(known.status) && !known.attached) await attachTo(known);
      return;
    }
    const events = await runEventsList(runId, 0).catch(() => [] as RunEvent[]);
    if (events.length === 0) return;
    let view = buildRunView(runId, events);
    // Saída das ferramentas: o replay não traz `tool.output`; o banco traz.
    view = hydrateStoredOutputs(view, await runCallOutputs(runId));
    runs.set(runId, view);
    emit();
    if (isRunActive(view.status)) await attachTo(view);
  },

  /** Para uma execução pelo id (a de uma automação não tem conversa). */
  cancelRun(runId: string): void {
    void runCancel(runId).catch((e) =>
      console.warn("run_cancel falhou:", errorMessage(e)),
    );
  },

  /** Tira o run do fluxo do chat (a trilha continua no painel/histórico). */
  dismiss(chatId: number): void {
    const runId = byChat.get(chatId);
    if (!runId) return;
    const run = runs.get(runId);
    if (run && isRunActive(run.status)) return;
    byChat.delete(chatId);
    if (run) runs.delete(runId);
    forgetPlan(runId);
    emit();
  },

  markDeleted(chatId: number): void {
    deletedChats.add(chatId);
    const runId = byChat.get(chatId);
    starting.delete(chatId);
    startErrors.delete(chatId);
    workModeByChat.delete(chatId);
    if (runId) {
      forgetPlan(runId);
      const run = runs.get(runId);
      if (run && isRunActive(run.status)) {
        void runCancel(runId).catch(() => {});
      }
      runs.delete(runId);
      byChat.delete(chatId);
    }
    emit();
  },

  /**
   * Reata ao run desta conversa depois de uma remontagem da tela. Quando a
   * memória do processo foi perdida (reload do app), reconstrói a trilha
   * inteira pelo `run_events_list` antes de reabrir o canal.
   */
  async ensureAttached(chatId: number): Promise<void> {
    if (deletedChats.has(chatId)) return;
    const knownId = byChat.get(chatId);
    if (knownId) {
      const known = runs.get(knownId);
      if (known && isRunActive(known.status) && !known.attached) {
        await attachTo(known);
      }
      return;
    }
    const list = await runsList(chatId).catch(() => [] as RunSummary[]);
    const active = list.find((r) => isRunActive(r.status));
    if (!active) return;
    const events = await runEventsList(active.id, 0).catch(() => [] as RunEvent[]);
    let view = buildRunView(active.id, events);
    // Saída das ferramentas: o replay não traz `tool.output`; o banco traz.
    view = hydrateStoredOutputs(view, await runCallOutputs(active.id));
    view.chatId = chatId;
    view.workMode = workModeByChat.get(chatId) ?? view.workMode;
    runs.set(view.runId, view);
    byChat.set(chatId, view.runId);
    emit();
    // Reload do app: o plano não vem nos eventos, só do banco.
    schedulePlanFetch(view.runId);
    await attachTo(view);
  },
};
