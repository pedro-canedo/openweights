// Espelho TS da Scout Rule.
// Fonte da verdade: src-tauri/crates/types/src/scout.rs (serde camelCase).
//
// A ideia: modelo local com janela curta não aguenta uma tarefa grande de
// uma vez. O plano quebra o objetivo em entregas pequenas e cada uma roda
// com contexto novo, recebendo só um resumo do que veio antes.

/** Como o agente trabalha (independe do nível de autorização). */
export type WorkMode = "chat" | "plan" | "agent" | "loop";

export type TaskStatus =
  | "pending"
  | "running"
  | "done"
  | "blocked"
  | "failed"
  | "skipped";

export interface Task {
  id: string;
  title: string;
  instruction: string;
  /** Como saber que a etapa terminou. */
  doneWhen: string;
  status: TaskStatus;
  /** Resumo do que a etapa produziu, escrito para a próxima. */
  handoff: string | null;
  files: string[];
  dependsOn: number[];
  estTokens: number;
  error: string | null;
  /** Comando de aceitação (DoD executável): sai 0 quando a entrega está pronta. */
  checkCmd?: string | null;
  /** Código de saída do comando de aceitação antes/depois da etapa (TDD). */
  checkBefore?: number | null;
  checkAfter?: number | null;
  /** Quando a etapa começou e terminou (epoch ms) — duração real na timeline. */
  startedAtMs?: number | null;
  finishedAtMs?: number | null;
  /** Tempo previsto da entrega, em segundos (`null` = sem medição). */
  etaSeconds?: number | null;
}

/** Uma pergunta estruturada à pessoa (as opções viram botões). */
export interface QuestionItem {
  text: string;
  options: string[];
}

/** Perguntas que pausaram a execução, à espera de resposta. */
export interface PendingQuestion {
  items: QuestionItem[];
  /** Etapa em que a pausa aconteceu (`null` = antes da primeira etapa). */
  taskIndex: number | null;
  askedAtMs: number;
}

export interface TaskPlan {
  goal: string;
  tasks: Task[];
  current: number;
  notes: string;
  approved: boolean;
  /** Perguntas que pausaram a execução (a resposta as limpa). */
  pendingQuestion?: PendingQuestion | null;
}

export type PlanEvent =
  | { kind: "plan.created"; plan: TaskPlan }
  | { kind: "plan.updated"; plan: TaskPlan }
  | { kind: "plan.approved" }
  | { kind: "task.started"; index: number; task: Task }
  | { kind: "task.finished"; index: number; task: Task };

/** Quantas etapas já terminaram. */
export function planProgress(plan: TaskPlan): { done: number; total: number } {
  return {
    done: plan.tasks.filter((t) => t.status === "done" || t.status === "skipped")
      .length,
    total: plan.tasks.length,
  };
}

export function isTaskFinished(status: TaskStatus): boolean {
  return status === "done" || status === "skipped";
}

/** Modos em que o agente pode alterar alguma coisa. */
export function canExecute(mode: WorkMode): boolean {
  return mode === "agent" || mode === "loop";
}

/** Modos que produzem um plano antes de agir. */
export function plansFirst(mode: WorkMode): boolean {
  return mode === "plan" || mode === "loop";
}
