// Ponte tipada com os comandos do harness agêntico (H1).
// Nenhuma tela chama `invoke` direto — tudo passa por aqui, como em `api.ts`.
//
// Os eventos do run chegam por um `Channel` do Tauri (não por `listen`): o
// canal é criado aqui e entregue ao comando `run_start`/`run_attach`, e cada
// evento carrega `runId` + `seq` para o `runStore` reaplicar de forma
// idempotente depois de um reload do webview.
//
// No navegador (npm run dev sem Tauri) tudo é simulado por um roteiro
// completo — é assim que a UI do agente é desenvolvida e testada sem o
// backend Rust: início → passo → texto → ferramenta com diff → espera de
// aprovação → resultado → comando de terminal com saída em streaming →
// resumo → fim.

import { invoke, isTauri } from "../tauri";
import { isTaskFinished } from "./scout";
import type { Task, TaskPlan, WorkMode } from "./scout";
import { TOOL_GROUPS } from "./types";
import type {
  ApprovalDecision,
  CheckpointRow,
  PolicyScope,
  RunEvent,
  RunMode,
  RunOptions,
  RunStatus,
  RunSummary,
  ToolPermissionRow,
  ToolPolicy,
} from "./types";

export type RunEventHandler = (event: RunEvent) => void;

/**
 * `RunOptions` mais o modo de trabalho da Scout Rule. Campo aditivo: o
 * backend cai em `"agent"` quando ele não vem, então conversas antigas (e
 * builds sem o H1c no Rust) continuam funcionando.
 */
export type RunStartOptions = RunOptions & { workMode?: WorkMode };

/**
 * Corpo de um `RunEvent` sem o envelope. O `T extends unknown` é o que faz o
 * `Omit` distribuir sobre a união (sem ele, sobrariam só os campos comuns).
 */
type EventBody<T = RunEvent> = T extends unknown
  ? Omit<T, "runId" | "seq" | "tsMs">
  : never;

/** `@tauri-apps/api/core` é carregado sob demanda (igual a `tauri.ts`). */
async function core() {
  return import("@tauri-apps/api/core");
}

// ------------------------------------------------------------- comandos ---

/** Inicia um run e devolve o `runId`. Os eventos chegam por `onEvent`. */
export async function runStart(
  prompt: string,
  opts: RunStartOptions,
  onEvent: RunEventHandler,
): Promise<string> {
  if (!isTauri) return mockRunStart(prompt, opts, onEvent);
  const { Channel, invoke: rawInvoke } = await core();
  const ch = new Channel<RunEvent>();
  ch.onmessage = onEvent;
  return rawInvoke<string>("run_start", { prompt, opts, onEvent: ch });
}

/**
 * Reata a um run já em curso, pedindo o replay dos eventos com `seq` maior
 * que `afterSeq`. É o que reconstrói a trilha quando a tela do Chat remonta
 * (navegar para outra tela DESMONTA o Chat) ou o app é reaberto.
 */
export async function runAttach(
  runId: string,
  afterSeq: number,
  onEvent: RunEventHandler,
): Promise<void> {
  if (!isTauri) return mockRunAttach(runId, afterSeq, onEvent);
  const { Channel, invoke: rawInvoke } = await core();
  const ch = new Channel<RunEvent>();
  ch.onmessage = onEvent;
  await rawInvoke<void>("run_attach", { runId, afterSeq, onEvent: ch });
}

export function runApprove(
  runId: string,
  callId: string,
  decision: ApprovalDecision,
): Promise<void> {
  return isTauri
    ? invoke<void>("run_approve", { runId, callId, decision })
    : mockRunApprove(runId, callId, decision);
}

export function runCancel(runId: string): Promise<void> {
  return isTauri
    ? invoke<void>("run_cancel", { runId })
    : mockRunCancel(runId);
}

export function runsList(chatId: number): Promise<RunSummary[]> {
  return isTauri
    ? invoke<RunSummary[]>("runs_list", { chatId })
    : mockRunsList(chatId);
}

/** Linha crua de `run_events`: o evento inteiro mora no `payloadJson`. */
interface RunEventRow {
  seq: number;
  kind: string;
  payloadJson: string;
  createdAt: number;
}

/**
 * Trilha gravada de uma execução.
 *
 * O backend devolve LINHAS de banco, não eventos: o `RunEvent` completo (com
 * `runId`/`seq`/`tsMs` e os campos do tipo) está serializado dentro de
 * `payloadJson`. Entregar a linha crua para o `reduceEvent` não estoura nada
 * — `seq` e `kind` existem nos dois — mas todo o resto chega `undefined`, e a
 * trilha reaparece vazia depois de qualquer reload. Desempacotar aqui é o que
 * faz reabrir uma execução mostrar o que aconteceu.
 */
export function runEventsList(
  runId: string,
  afterSeq = 0,
): Promise<RunEvent[]> {
  if (!isTauri) return mockRunEventsList(runId, afterSeq);
  return invoke<RunEventRow[]>("run_events_list", { runId, afterSeq }).then(
    (rows) =>
      rows.flatMap((row) => {
        try {
          return [JSON.parse(row.payloadJson) as RunEvent];
        } catch (e) {
          // Uma linha estragada não pode levar a trilha inteira junto.
          console.warn("evento ilegível na trilha:", row.seq, e);
          return [];
        }
      }),
  );
}

/** Saída gravada de uma chamada de ferramenta, como ficou ao fim dela. */
export interface RunCallOutput {
  callId: string;
  /** `{"content": …, "changedFiles": …}` no sucesso; vazio quando falhou.
   *  `null` = a chamada ainda não terminou. */
  resultJson: string | null;
  /** Mensagem de erro ou motivo da negação, quando houve. */
  error: string | null;
}

/**
 * Saídas completas das ferramentas de um run, lidas do banco. O streaming
 * (`tool.output`) não é persistido de propósito, então a trilha reconstruída
 * pelos eventos só teria o preview de 400 caracteres do `tool.result` — este
 * é o caminho de volta do conteúdo inteiro depois de um reload.
 *
 * Nunca lança: comando ausente (build antiga) ou erro do banco viram lista
 * vazia — a trilha fica com o preview, como antes. No navegador a lista vazia
 * é a resposta certa: os eventos simulados moram na memória e o replay do
 * `mockRunAttach` já traz o `tool.output`.
 */
export function runCallOutputs(runId: string): Promise<RunCallOutput[]> {
  if (!isTauri) return Promise.resolve([]);
  return invoke<RunCallOutput[]>("run_call_outputs", { runId }).catch((e) => {
    console.warn("run_call_outputs falhou:", e);
    return [];
  });
}

/**
 * Plano estruturado do run (Scout Rule). O `focus.updated` só carrega o
 * markdown; o quadro precisa dos campos (status, handoff, arquivos), que
 * ficam gravados em `runs.plan_json`.
 *
 * Nunca lança: run sem plano, comando ausente (build antiga) ou erro do
 * banco viram `null` — o quadro simplesmente não aparece.
 */
export function runPlanGet(runId: string): Promise<TaskPlan | null> {
  if (!isTauri) return mockRunPlanGet(runId);
  return invoke<TaskPlan | null>("run_plan_get", { runId }).catch(() => null);
}

/**
 * Libera a execução do plano proposto (modo planejamento) e devolve o id da
 * execução NOVA — aprovar abre um run em modo laço com as mesmas etapas.
 *
 * Ele precisa do canal como o `run_start`: sem isso o comando nem chega a
 * rodar (argumento obrigatório faltando), e o plano aprovado ficava parado
 * para sempre. Devolve `null` quando falha, para a tela poder dizer algo.
 */
export async function runPlanApprove(
  runId: string,
  onEvent: RunEventHandler,
): Promise<string | null> {
  if (!isTauri) return mockRunPlanApprove(runId);
  try {
    const { Channel, invoke: rawInvoke } = await core();
    const ch = new Channel<RunEvent>();
    ch.onmessage = onEvent;
    return await rawInvoke<string>("run_plan_approve", {
      runId,
      onEvent: ch,
    });
  } catch (e) {
    console.warn("run_plan_approve falhou:", e);
    return null;
  }
}

/**
 * Retoma uma execução que parou esperando a pessoa: a resposta destrava o
 * plano persistido e um run NOVO continua de onde parou (id diferente de
 * propósito — a trilha antiga fica íntegra).
 */
export async function runAnswer(
  runId: string,
  answer: string,
  onEvent: RunEventHandler,
): Promise<string | null> {
  if (!isTauri) return null;
  try {
    const { Channel, invoke: rawInvoke } = await core();
    const ch = new Channel<RunEvent>();
    ch.onmessage = onEvent;
    return await rawInvoke<string>("run_answer", {
      runId,
      answer,
      onEvent: ch,
    });
  } catch (e) {
    console.warn("run_answer falhou:", e);
    return null;
  }
}


/** Pede uma nova divisão do mesmo objetivo. */
export function runPlanReplan(runId: string): Promise<void> {
  if (!isTauri) return mockRunPlanReplan(runId);
  return invoke<void>("run_plan_replan", { runId }).catch((e) => {
    console.warn("run_plan_replan falhou:", e);
  });
}

export function toolPermissionsList(
  workspaceDir: string | null,
): Promise<ToolPermissionRow[]> {
  return isTauri
    ? invoke<ToolPermissionRow[]>("tool_permissions_list", { workspaceDir })
    : mockPermissionsList(workspaceDir);
}

/** `policy = null` remove o override da ferramenta naquele escopo. */
export function toolPermissionSet(
  toolName: string,
  policy: ToolPolicy | null,
  scope: PolicyScope,
  workspaceDir: string | null,
): Promise<void> {
  return isTauri
    ? invoke<void>("tool_permission_set", {
        toolName,
        policy,
        scope,
        workspaceDir,
      })
    : mockPermissionSet(toolName, policy, scope, workspaceDir);
}

/**
 * Chaves das famílias de ferramentas habilitadas.
 *
 * Nunca lança: sem o comando (build antiga) ou com a preferência ilegível,
 * cai em TODAS habilitadas — o mesmo padrão do Rust. Esconder capacidade por
 * causa de uma falha de leitura seria pior do que mostrar demais.
 */
export function toolGroupsGet(): Promise<string[]> {
  if (!isTauri) return mockToolGroupsGet();
  return invoke<string[]>("tool_groups_get").catch((e) => {
    console.warn("tool_groups_get falhou:", e);
    return [...TOOL_GROUPS];
  });
}

/**
 * Quantas ferramentas cada família tem no catálogo de agora (conectores MCP
 * incluídos). Vem do backend porque uma lista escrita na interface mentiria
 * na primeira ferramenta nova.
 */
export function toolGroupCounts(): Promise<Record<string, number>> {
  if (!isTauri) return mockToolGroupCounts();
  return invoke<Record<string, number>>("tool_group_counts").catch((e) => {
    console.warn("tool_group_counts falhou:", e);
    return {};
  });
}

/**
 * Grava as famílias habilitadas e devolve o que o backend aceitou (chave
 * desconhecida é descartada lá). Lança: a tela desfaz a mudança na falha.
 */
export function toolGroupsSet(groups: string[]): Promise<string[]> {
  return isTauri
    ? invoke<string[]>("tool_groups_set", { groups })
    : mockToolGroupsSet(groups);
}

/**
 * JSON cru da configuração da busca na web (setting `web.config`), ou `null`
 * quando nunca foi gravada.
 *
 * Nunca lança: comando ausente (build antiga) ou erro do banco viram `null`
 * e a tela começa nos padrões — o mesmo que o backend faz quando o JSON
 * gravado está estragado.
 */
export function webConfigGet(): Promise<string | null> {
  if (!isTauri) return Promise.resolve(null);
  return invoke<string | null>("web_config_get").catch((e) => {
    console.warn("web_config_get falhou:", e);
    return null;
  });
}

/**
 * Grava a configuração da busca e aplica NA HORA: o backend valida o JSON,
 * persiste e refaz o catálogo de ferramentas (sem reiniciar o app). Lança na
 * falha de propósito — a tela precisa avisar que nada foi salvo.
 */
export function webConfigSet(json: string): Promise<void> {
  if (!isTauri) return Promise.resolve();
  return invoke<void>("web_config_set", { json });
}

export function checkpointsList(
  workspaceDir: string,
): Promise<CheckpointRow[]> {
  return isTauri
    ? invoke<CheckpointRow[]>("checkpoints_list", { workspaceDir })
    : mockCheckpointsList(workspaceDir);
}

/**
 * Restaura o checkpoint e devolve os caminhos dos arquivos alterados.
 *
 * O nome do argumento é `checkpointId`: o Tauri converte o `checkpoint_id` do
 * Rust para camelCase, e mandar `id` fazia o desfazer falhar sempre.
 */
export function checkpointRestore(id: string): Promise<string[]> {
  return isTauri
    ? invoke<string[]>("checkpoint_restore", { checkpointId: id })
    : mockCheckpointRestore(id);
}

// ------------------------------------------------- simulação (navegador) ---

interface MockRun {
  id: string;
  chatId: number;
  model: string;
  mode: RunMode;
  workMode: WorkMode;
  prompt: string;
  workspaceDir: string | null;
  events: RunEvent[];
  seq: number;
  handler: RunEventHandler | null;
  status: RunStatus;
  summary: string | null;
  createdAt: number;
  finishedAt: number | null;
  cancelled: boolean;
  /** Resolve a espera de aprovação da ferramenta corrente. */
  gate: ((decision: ApprovalDecision) => void) | null;
  /** Resolve a espera da aprovação do plano (modo planejamento). */
  planGate: (() => void) | null;
}

/** Unwind do roteiro simulado quando o usuário aperta Parar. */
class MockCancelled extends Error {}

const mockRuns = new Map<string, MockRun>();
const mockCheckpoints: CheckpointRow[] = [];
/** Plano por run: o roteiro simulado mexe nele a cada entrega concluída. */
const mockPlans = new Map<string, TaskPlan>();
/** Runs cujo plano é movido pelo roteiro (os outros andam a cada consulta). */
const mockDrivenPlans = new Set<string>();
let mockPermissions: ToolPermissionRow[] = [];
/** Famílias habilitadas na simulação (o padrão do backend é todas). */
let mockGroups: string[] = [...TOOL_GROUPS];
let mockRunSeq = 0;
let mockCheckpointSeq = 0;
let mockPermissionSeq = 0;

const MOCK_TOOLS = [
  "fs_read",
  "fs_write",
  "fs_edit",
  "fs_list",
  "fs_glob",
  "fs_grep",
  "terminal_run",
  "todo_update",
];

/** Cardápio curado: total do catálogo e o que cabe na janela do modelo. */
const MOCK_TOOLS_AVAILABLE = 37;
const MOCK_TOOLS_LIMIT = 12;
/** O que o modelo pede no meio do run (o `tools_find` em ação). */
const MOCK_TOOLS_EXTRA = ["test_run", "git_status"];

const MOCK_DIFF = `--- /dev/null
+++ b/notas.md
@@ -0,0 +1,9 @@
+# Notas do projeto
+
+- Stack: Tauri 2 + Rust + React 19 (Tailwind v4).
+- Os modelos rodam no llama-server local em Router mode.
+- O loop do agente vive em Rust; a UI só desenha os eventos.
+
+## Próximos passos
+
+- [ ] Revisar as permissões padrão das ferramentas.
`;

const MOCK_LS_CHUNKS = [
  "total 48\n",
  "drwxr-xr-x  8 dev dev  4096 ago 15 19:59 .\n",
  "drwxr-xr-x 12 dev dev  4096 ago 15 12:06 ..\n",
  "-rw-r--r--  1 dev dev   802 ago 15 19:24 package.json\n",
  "-rw-r--r--  1 dev dev   241 ago 15 20:11 notas.md\n",
  "drwxr-xr-x  9 dev dev  4096 ago 15 17:38 src\n",
  "drwxr-xr-x  6 dev dev  4096 ago 15 22:45 src-tauri\n",
];

const MOCK_FOCUS = `- [x] Ler o README do projeto
- [x] Criar \`notas.md\` com o resumo
- [ ] Conferir a pasta depois da alteração
`;

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function emit(run: MockRun, body: EventBody): void {
  run.seq += 1;
  const event = {
    ...body,
    runId: run.id,
    seq: run.seq,
    tsMs: Date.now(),
  } as RunEvent;
  run.events.push(event);
  run.handler?.(event);
}

/** Pausa que respeita o Parar: desenrola o roteiro via exceção. */
async function tick(run: MockRun, ms: number): Promise<void> {
  await sleep(ms);
  if (run.cancelled) throw new MockCancelled();
}

async function typeOut(
  run: MockRun,
  stepId: string,
  text: string,
): Promise<void> {
  const words = text.split(/(\s+)/);
  let buffer = "";
  for (const word of words) {
    buffer += word;
    if (buffer.length < 12 && word.trim() !== "") continue;
    emit(run, { kind: "assistant.delta", stepId, text: buffer });
    buffer = "";
    await tick(run, 45);
  }
  if (buffer) emit(run, { kind: "assistant.delta", stepId, text: buffer });
  emit(run, {
    kind: "assistant.message",
    stepId,
    content: text,
    reasoning: "",
  });
}

/**
 * Espera a decisão do usuário. Devolve `false` quando negada — o roteiro
 * então encerra com uma mensagem, como o loop real faria.
 */
async function gate(
  run: MockRun,
  callId: string,
  requiresApproval: boolean,
): Promise<boolean> {
  if (!requiresApproval) {
    emit(run, {
      kind: "tool.approved",
      callId,
      source: run.mode === "yolo" ? "yolo" : "policy",
    });
    return true;
  }
  run.status = "waitingApproval";
  emit(run, { kind: "run.paused", reason: "waitingApproval" });
  const decision = await new Promise<ApprovalDecision>((resolve) => {
    run.gate = resolve;
  });
  run.gate = null;
  if (run.cancelled) throw new MockCancelled();
  run.status = "running";
  if (decision.kind === "deny" || decision.kind === "denyAlways") {
    const reason = decision.kind === "deny" ? (decision.reason ?? "") : "";
    emit(run, { kind: "tool.denied", callId, reason });
    emit(run, { kind: "run.resumed" });
    return false;
  }
  emit(run, { kind: "tool.approved", callId, source: "user" });
  emit(run, { kind: "run.resumed" });
  return true;
}

function finish(
  run: MockRun,
  status: RunStatus,
  summary: string,
  steps: number,
  toolCalls: number,
): void {
  run.status = status;
  run.summary = summary;
  run.finishedAt = Date.now();
  emit(run, {
    kind: "run.finished",
    status,
    summary,
    usage: {
      steps,
      toolCalls,
      promptTokens: 1840 + steps * 320,
      completionTokens: 260 + steps * 90,
      durationMs: Date.now() - run.createdAt,
    },
  });
}

// ------------------------------------------------- plano simulado (Scout) ---

function mockTask(
  id: string,
  title: string,
  instruction: string,
  doneWhen: string,
  dependsOn: number[],
  estTokens: number,
): Task {
  return {
    id,
    title,
    instruction,
    doneWhen,
    status: "pending",
    handoff: null,
    files: [],
    dependsOn,
    estTokens,
    error: null,
  };
}

/**
 * Divisão que acompanha o roteiro simulado (ler → escrever → conferir →
 * fechar). `variant` 1 é a resposta ao "Refazer plano": as mesmas entregas
 * quebradas ainda menores.
 */
function buildMockPlan(goal: string, variant: number, approved: boolean): TaskPlan {
  const tasks =
    variant === 0
      ? [
          mockTask(
            "t1",
            "Ler o README e mapear o projeto",
            "Abrir o README.md e anotar stack, entrypoints e o que já existe.",
            "Resumo do stack anotado em uma frase.",
            [],
            2400,
          ),
          mockTask(
            "t2",
            "Escrever notas.md com o resumo",
            "Criar notas.md com o resumo do projeto e a lista de próximos passos.",
            "Arquivo notas.md existe com resumo e próximos passos.",
            [0],
            3200,
          ),
          mockTask(
            "t3",
            "Conferir a pasta depois da alteração",
            "Listar a pasta do projeto e confirmar que notas.md está lá.",
            "A listagem mostra notas.md na raiz.",
            [1],
            1800,
          ),
          mockTask(
            "t4",
            "Fechar com o resumo do que mudou",
            "Responder ao usuário contando o que foi feito e como desfazer.",
            "Resposta final entregue com o checkpoint citado.",
            [2],
            1200,
          ),
        ]
      : [
          mockTask(
            "r1",
            "Ler o README",
            "Abrir o README.md e extrair só o parágrafo de apresentação.",
            "Parágrafo de apresentação copiado.",
            [],
            1600,
          ),
          mockTask(
            "r2",
            "Levantar os próximos passos",
            "Listar de 2 a 3 pendências reais a partir do que foi lido.",
            "Lista de pendências fechada.",
            [0],
            1600,
          ),
          mockTask(
            "r3",
            "Escrever notas.md",
            "Criar notas.md juntando o resumo e as pendências.",
            "Arquivo notas.md existe com as duas seções.",
            [1],
            2600,
          ),
          mockTask(
            "r4",
            "Conferir a pasta",
            "Listar a pasta e confirmar o arquivo criado.",
            "A listagem mostra notas.md na raiz.",
            [2],
            1400,
          ),
          mockTask(
            "r5",
            "Fechar com o resumo",
            "Responder ao usuário com o que mudou.",
            "Resposta final entregue.",
            [3],
            1000,
          ),
        ];
  return {
    goal: goal.length > 90 ? `${goal.slice(0, 90)}…` : goal,
    tasks,
    current: 0,
    notes:
      "Cada entrega roda com contexto novo e recebe só o resumo da anterior.",
    approved,
  };
}

/** Markdown do plano no formato que o `FocusChip` entende. */
function planToMarkdown(plan: TaskPlan): string {
  const lines = plan.tasks.map(
    (task) => `- [${isTaskFinished(task.status) ? "x" : " "}] ${task.title}`,
  );
  return `${lines.join("\n")}\n`;
}

function clonePlan(plan: TaskPlan): TaskPlan {
  return JSON.parse(JSON.stringify(plan)) as TaskPlan;
}

/** Grava o estado novo e avisa a UI — o quadro recarrega no `focus.updated`. */
function publishPlan(run: MockRun): void {
  const plan = mockPlans.get(run.id);
  if (!plan) return;
  emit(run, { kind: "focus.updated", todoMd: planToMarkdown(plan) });
}

/** Começa a primeira entrega (depois do plano aprovado). */
function beginPlan(run: MockRun): void {
  const plan = mockPlans.get(run.id);
  if (!plan || plan.tasks.length === 0) return;
  plan.current = 0;
  plan.tasks[0].status = "running";
  publishPlan(run);
}

/** Fecha a entrega corrente com o handoff e abre a próxima. */
function stepPlan(plan: TaskPlan, handoff: string, files: string[]): void {
  const task = plan.tasks[plan.current];
  if (task && task.status === "running") {
    task.status = "done";
    task.handoff = handoff;
    task.files = files;
  }
  const next = plan.tasks.findIndex((x) => x.status === "pending");
  if (next >= 0) {
    plan.current = next;
    plan.tasks[next].status = "running";
  } else {
    plan.current = plan.tasks.length - 1;
  }
}

function advancePlan(run: MockRun, handoff: string, files: string[]): void {
  const plan = mockPlans.get(run.id);
  if (!plan) return;
  stepPlan(plan, handoff, files);
  publishPlan(run);
}

/** Entrega travada por uma negação do usuário. */
function blockPlan(run: MockRun, reason: string): void {
  const plan = mockPlans.get(run.id);
  if (!plan) return;
  const task = plan.tasks[plan.current];
  if (task) {
    task.status = "blocked";
    task.error = reason;
  }
  publishPlan(run);
}

/** Espera o usuário aprovar o plano proposto (modo planejamento). */
async function waitPlanApproval(run: MockRun): Promise<void> {
  emit(run, { kind: "run.paused", reason: "user" });
  await new Promise<void>((resolve) => {
    run.planGate = resolve;
  });
  run.planGate = null;
  if (run.cancelled) throw new MockCancelled();
  emit(run, { kind: "run.resumed" });
}

async function driveMock(run: MockRun): Promise<void> {
  const yolo = run.mode === "yolo";
  const dir = run.workspaceDir ?? "C:/projetos/exemplo";
  // `smart` só pergunta em escrita/execução; `approve` pergunta em tudo.
  const asksRead = run.mode === "approve";
  const asksWrite = run.mode !== "yolo";
  let callSeq = 0;
  const nextCall = () => `${run.id}-call-${++callSeq}`;

  emit(run, {
    kind: "run.started",
    chatId: run.chatId,
    model: run.model,
    mode: run.mode,
    yolo,
    workspaceDir: dir,
    tools: MOCK_TOOLS,
  });
  emit(run, {
    kind: "tools.selected",
    available: MOCK_TOOLS_AVAILABLE,
    active: MOCK_TOOLS,
    limit: MOCK_TOOLS_LIMIT,
    requested: false,
  });

  // 0) Scout Rule: antes de agir, o objetivo vira entregas pequenas. Em
  // planejamento o run PARA aqui até o usuário aprovar o plano.
  const planned = run.workMode !== "chat";
  if (planned) {
    await tick(run, 900);
    mockPlans.set(
      run.id,
      buildMockPlan(run.prompt, 0, run.workMode !== "plan"),
    );
    mockDrivenPlans.add(run.id);
    publishPlan(run);
    if (!mockPlans.get(run.id)?.approved) await waitPlanApproval(run);
    beginPlan(run);
  }

  // 1) Leitura (auto-aprovada em smart/yolo).
  emit(run, { kind: "step.started", stepId: "step-1", index: 0 });
  await tick(run, 350);
  await typeOut(
    run,
    "step-1",
    "Vou ler o `README.md` para entender o projeto antes de escrever qualquer coisa.",
  );

  const readCall = nextCall();
  emit(run, {
    kind: "tool.requested",
    callId: readCall,
    tool: "fs_read",
    origin: { kind: "builtin" },
    category: "read",
    tier: "safe",
    argsJson: JSON.stringify({ path: "README.md" }, null, 2),
    preview: { kind: "text", body: "README.md" },
    requiresApproval: asksRead,
  });
  if (!(await gate(run, readCall, asksRead))) {
    blockPlan(run, "Leitura do README negada.");
    emit(run, { kind: "step.started", stepId: "step-x", index: 1 });
    await typeOut(
      run,
      "step-x",
      "Tudo bem, não vou ler o arquivo. Me diga como prefere seguir.",
    );
    finish(run, "done", "Leitura negada pelo usuário.", 2, 1);
    return;
  }
  emit(run, { kind: "tool.started", callId: readCall });
  await tick(run, 420);
  emit(run, {
    kind: "tool.result",
    callId: readCall,
    ok: true,
    resultPreview:
      "# OpenWeights\n\nApp desktop para rodar modelos GGUF locais via llama-server...",
    bytesTotal: 4312,
    durationMs: 41,
  });
  if (planned) {
    advancePlan(
      run,
      "Stack: Tauri 2 + Rust + React 19 (Tailwind v4); o laço do agente vive no Rust e a UI só desenha os eventos.",
      ["README.md"],
    );
  }

  // 2) Escrita com diff (pede confirmação fora do YOLO).
  emit(run, { kind: "step.started", stepId: "step-2", index: 1 });
  await tick(run, 300);
  await typeOut(
    run,
    "step-2",
    "Entendi o projeto. Vou criar `notas.md` com um resumo curto e os próximos passos.",
  );

  const writeCall = nextCall();
  emit(run, {
    kind: "tool.requested",
    callId: writeCall,
    tool: "fs_write",
    origin: { kind: "builtin" },
    category: "edit",
    tier: "caution",
    argsJson: JSON.stringify(
      {
        path: "notas.md",
        content: "# Notas do projeto\n\n- Stack: Tauri 2 + Rust + React 19...",
      },
      null,
      2,
    ),
    preview: {
      kind: "diff",
      path: "notas.md",
      unified: MOCK_DIFF,
      created: true,
    },
    requiresApproval: asksWrite,
  });
  if (!(await gate(run, writeCall, asksWrite))) {
    blockPlan(run, "Criação de notas.md negada.");
    emit(run, { kind: "step.started", stepId: "step-3", index: 2 });
    await typeOut(
      run,
      "step-3",
      "Ok, não vou criar o arquivo. Posso apenas te mostrar o resumo aqui no chat, se preferir.",
    );
    finish(run, "done", "Escrita negada pelo usuário.", 3, 2);
    return;
  }

  // Checkpoint SEMPRE antes da primeira mutação.
  const checkpointId = `ckpt-${++mockCheckpointSeq}`;
  mockCheckpoints.unshift({
    id: checkpointId,
    runId: run.id,
    workspaceDir: dir,
    backend: "gitShadow",
    label: "antes de notas.md",
    createdAt: Date.now(),
  });
  emit(run, {
    kind: "checkpoint.created",
    checkpointId,
    label: "antes de notas.md",
    backend: "gitShadow",
  });
  emit(run, { kind: "tool.started", callId: writeCall });
  await tick(run, 500);
  emit(run, {
    kind: "tool.result",
    callId: writeCall,
    ok: true,
    resultPreview: "notas.md criado (241 bytes)",
    bytesTotal: 241,
    durationMs: 63,
  });
  if (planned) {
    advancePlan(run, "notas.md criado (241 bytes) com o resumo e 1 pendência.", [
      "notas.md",
    ]);
  } else {
    emit(run, { kind: "focus.updated", todoMd: MOCK_FOCUS });
  }

  // 3) O agente delega uma investigação: o que acontece entre os dois
  //    marcadores é trabalho do ajudante, e a trilha desenha recuado.
  const helper = nextCall();
  emit(run, {
    kind: "subagent.started",
    callId: helper,
    objective: "descobrir onde o roteamento de modelos é decidido",
    role: "explorer",
  });
  emit(run, { kind: "step.started", stepId: "step-sub", index: 90 });
  await typeOut(
    run,
    "step-sub",
    "Procurei por `router` no projeto: a decisão fica em src/engine/router.rs.",
  );
  emit(run, {
    kind: "subagent.finished",
    callId: helper,
    status: "done",
    steps: 3,
    summary: "O roteamento vive em src/engine/router.rs, função resolve_model.",
  });

  // 4) O modelo pede ferramentas que não vieram no cardápio inicial e depois
  //    roda um comando, com a saída em streaming.
  emit(run, {
    kind: "tools.selected",
    available: MOCK_TOOLS_AVAILABLE,
    active: MOCK_TOOLS_EXTRA,
    limit: MOCK_TOOLS_LIMIT,
    requested: true,
  });
  emit(run, { kind: "step.started", stepId: "step-3", index: 2 });
  await tick(run, 300);
  await typeOut(
    run,
    "step-3",
    "Arquivo criado. Vou listar a pasta para confirmar o resultado.",
  );

  const runCall = nextCall();
  emit(run, {
    kind: "tool.requested",
    callId: runCall,
    tool: "terminal_run",
    origin: { kind: "builtin" },
    category: "execute",
    tier: "caution",
    argsJson: JSON.stringify({ command: "ls -la", cwd: dir }, null, 2),
    preview: {
      kind: "command",
      program: "ls",
      display: "ls -la",
      cwd: dir,
      class: "readOnly",
    },
    requiresApproval: asksWrite,
  });
  if (!(await gate(run, runCall, asksWrite))) {
    blockPlan(run, "Conferência da pasta negada.");
    emit(run, { kind: "step.started", stepId: "step-4", index: 3 });
    await typeOut(
      run,
      "step-4",
      "Sem problema — o arquivo já foi criado, só não confirmei pela listagem.",
    );
    finish(run, "done", "Comando negado; arquivo criado.", 4, 3);
    return;
  }
  emit(run, { kind: "tool.started", callId: runCall });
  for (const chunk of MOCK_LS_CHUNKS) {
    await tick(run, 220);
    emit(run, { kind: "tool.output", callId: runCall, chunk, truncated: false });
  }
  await tick(run, 200);
  emit(run, {
    kind: "tool.result",
    callId: runCall,
    ok: true,
    resultPreview: MOCK_LS_CHUNKS.join(""),
    bytesTotal: MOCK_LS_CHUNKS.join("").length,
    durationMs: 190,
  });
  emit(run, {
    kind: "verification",
    passed: true,
    notes: "notas.md existe e tem conteúdo.",
  });
  if (planned) {
    advancePlan(run, "Listagem confirma notas.md na raiz do projeto.", []);
  }

  // 4) Resumo final.
  emit(run, { kind: "step.started", stepId: "step-4", index: 3 });
  await tick(run, 300);
  await typeOut(
    run,
    "step-4",
    "Pronto: criei **notas.md** com o resumo do projeto e confirmei pela listagem da pasta. Um checkpoint foi criado antes da alteração, então dá para desfazer a qualquer momento.",
  );
  if (planned) {
    advancePlan(run, "Resumo entregue ao usuário com o checkpoint citado.", []);
  }
  finish(run, "done", "notas.md criado e verificado.", 4, 3);
}

function mockRunStart(
  prompt: string,
  opts: RunStartOptions,
  onEvent: RunEventHandler,
): Promise<string> {
  const id = `run-${++mockRunSeq}`;
  const run: MockRun = {
    id,
    chatId: opts.chatId,
    model: opts.model,
    mode: opts.mode ?? "smart",
    workMode: opts.workMode ?? "agent",
    prompt,
    workspaceDir: opts.workspaceDir ?? null,
    events: [],
    seq: 0,
    handler: onEvent,
    status: "running",
    summary: null,
    createdAt: Date.now(),
    finishedAt: null,
    cancelled: false,
    gate: null,
    planGate: null,
  };
  mockRuns.set(id, run);
  void driveMock(run).catch((err) => {
    if (err instanceof MockCancelled) {
      finish(run, "cancelled", "Execução cancelada.", 1, 0);
      return;
    }
    const message = err instanceof Error ? err.message : String(err);
    emit(run, { kind: "run.error", message, retryable: false });
    finish(run, "error", message, 1, 0);
  });
  return Promise.resolve(id);
}

function mockRunAttach(
  runId: string,
  afterSeq: number,
  onEvent: RunEventHandler,
): Promise<void> {
  const run = mockRuns.get(runId);
  if (!run) return Promise.reject(new Error(`run desconhecido: ${runId}`));
  run.handler = onEvent;
  for (const event of run.events) {
    if (event.seq > afterSeq) onEvent(event);
  }
  return Promise.resolve();
}

function mockRunApprove(
  runId: string,
  callId: string,
  decision: ApprovalDecision,
): Promise<void> {
  const run = mockRuns.get(runId);
  if (!run) return Promise.reject(new Error(`run desconhecido: ${runId}`));
  if (decision.kind === "allowAlways" || decision.kind === "denyAlways") {
    void mockPermissionSet(
      callId.includes("call-1") ? "fs_read" : "fs_write",
      decision.kind === "allowAlways" ? "alwaysAllow" : "never",
      decision.scope,
      run.workspaceDir,
    );
  }
  run.gate?.(decision);
  return Promise.resolve();
}

function mockRunCancel(runId: string): Promise<void> {
  const run = mockRuns.get(runId);
  if (!run) return Promise.resolve();
  run.cancelled = true;
  // Parado enquanto espera aprovação: destrava o roteiro para ele encerrar.
  run.gate?.({ kind: "deny", reason: null });
  run.planGate?.();
  return Promise.resolve();
}

/**
 * Plano simulado. Quando o roteiro do run cuida do plano, devolve o estado
 * corrente; senão (run reconstruído, sem roteiro) o plano anda uma entrega
 * a cada consulta — é o suficiente para ver o quadro avançando.
 */
function mockRunPlanGet(runId: string): Promise<TaskPlan | null> {
  const run = mockRuns.get(runId);
  const known = mockPlans.get(runId);
  if (!known) {
    if (!run || run.workMode === "chat") return Promise.resolve(null);
    const fresh = buildMockPlan(run.prompt, 0, true);
    fresh.tasks[0].status = "running";
    mockPlans.set(runId, fresh);
    return Promise.resolve(clonePlan(fresh));
  }
  // Sem `publishPlan` aqui: o `focus.updated` é o que dispara esta consulta,
  // e reemiti-lo viraria laço infinito.
  if (!mockDrivenPlans.has(runId)) {
    stepPlan(known, "Entrega concluída (simulação).", []);
  }
  return Promise.resolve(clonePlan(known));
}

function mockRunPlanApprove(runId: string): Promise<string | null> {
  const plan = mockPlans.get(runId);
  if (plan) plan.approved = true;
  const run = mockRuns.get(runId);
  if (run) {
    publishPlan(run);
    run.planGate?.();
  }
  // Na simulação o mesmo run segue executando, então o id não muda.
  return Promise.resolve(runId);
}

function mockRunPlanReplan(runId: string): Promise<void> {
  const run = mockRuns.get(runId);
  if (!run) return Promise.resolve();
  const previous = mockPlans.get(runId);
  const variant = previous && previous.tasks.length === 4 ? 1 : 0;
  mockPlans.set(runId, buildMockPlan(run.prompt, variant, false));
  publishPlan(run);
  return Promise.resolve();
}

function mockRunsList(chatId: number): Promise<RunSummary[]> {
  const rows = [...mockRuns.values()]
    .filter((r) => r.chatId === chatId)
    .sort((a, b) => b.createdAt - a.createdAt)
    .map<RunSummary>((r) => ({
      id: r.id,
      chatId: r.chatId,
      model: r.model,
      mode: r.mode,
      status: r.status,
      prompt: r.prompt,
      summary: r.summary,
      workspaceDir: r.workspaceDir,
      createdAt: r.createdAt,
      finishedAt: r.finishedAt,
    }));
  return Promise.resolve(rows);
}

function mockRunEventsList(
  runId: string,
  afterSeq: number,
): Promise<RunEvent[]> {
  const run = mockRuns.get(runId);
  if (!run) return Promise.resolve([]);
  return Promise.resolve(run.events.filter((e) => e.seq > afterSeq));
}

function mockPermissionsList(
  workspaceDir: string | null,
): Promise<ToolPermissionRow[]> {
  return Promise.resolve(
    mockPermissions.filter(
      (p) => p.scope === "global" || p.workspaceDir === workspaceDir,
    ),
  );
}

function mockPermissionSet(
  toolName: string,
  policy: ToolPolicy | null,
  scope: PolicyScope,
  workspaceDir: string | null,
): Promise<void> {
  const dir = scope === "workspace" ? workspaceDir : null;
  mockPermissions = mockPermissions.filter(
    (p) => !(p.toolName === toolName && p.scope === scope && p.workspaceDir === dir),
  );
  if (policy) {
    mockPermissions.push({
      id: ++mockPermissionSeq,
      scope,
      workspaceDir: dir,
      toolName,
      policy,
    });
  }
  return Promise.resolve();
}

function mockToolGroupsGet(): Promise<string[]> {
  return Promise.resolve([...mockGroups]);
}

function mockToolGroupCounts(): Promise<Record<string, number>> {
  return Promise.resolve({
    files: 6,
    terminal: 1,
    code: 6,
    git: 8,
    data: 5,
    web: 4,
    memory: 1,
    project: 1,
    plan: 5,
    mcp: 0,
  });
}

function mockToolGroupsSet(groups: string[]): Promise<string[]> {
  mockGroups = TOOL_GROUPS.filter((g) => groups.includes(g));
  return Promise.resolve([...mockGroups]);
}

function mockCheckpointsList(workspaceDir: string): Promise<CheckpointRow[]> {
  return Promise.resolve(
    mockCheckpoints.filter((c) => c.workspaceDir === workspaceDir),
  );
}

function mockCheckpointRestore(id: string): Promise<string[]> {
  const row = mockCheckpoints.find((c) => c.id === id);
  return Promise.resolve(row ? ["notas.md"] : []);
}
