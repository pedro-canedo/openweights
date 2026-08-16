// Ponte tipada com a tela de atividade: o histórico do que o agente fez.
//
// Sobre "custo": modelo local não cobra em dinheiro. O que ele cobra é tempo
// e tokens, e é isso que estas contas trazem — inventar um valor em dólar
// seria mentira confortável.
//
// Atenção às unidades, que vêm do banco e não são iguais em toda parte:
// `createdAt`/`finishedAt` de uma execução estão em SEGUNDOS; `startedAt` de
// uma ação e `durationMs` estão em milissegundos.

import { invoke, isTauri } from "../tauri";
import type { RunSummary } from "./types";

/** Uma ação do agente, com quem a autorizou. */
export interface ActivityCall {
  callId: string;
  runId: string;
  toolName: string;
  /** `builtin` ou `mcp:<servidor>`. */
  origin: string;
  /** `requested` | `running` | `ok` | `error` | `denied`. */
  state: string;
  error: string | null;
  startedAt: number | null;
  finishedAt: number | null;
  /** `allowOnce`, `allowAlways`, `deny`… quando houve decisão gravada. */
  decision: string | null;
  /** `user`, `policy` ou `yolo`. Ausente = não precisou de confirmação. */
  source: string | null;
}

/** Contas de um período. */
export interface ActivityStats {
  runs: number;
  /** Execuções que não terminaram bem. */
  failed: number;
  toolCalls: number;
  /** Ações recusadas: a conta do que NÃO aconteceu. */
  denied: number;
  promptTokens: number;
  completionTokens: number;
  durationMs: number;
}

/** Períodos oferecidos na tela, em horas. `0` = desde sempre. */
export const PERIODS = [24, 24 * 7, 0] as const;

/** Epoch em SEGUNDOS, que é a unidade de `runs.created_at`. */
export function since(hours: number): number {
  if (hours <= 0) return 0;
  return Math.floor((Date.now() - hours * 3_600_000) / 1000);
}

export function activityRuns(): Promise<RunSummary[]> {
  if (!isTauri) return Promise.resolve(mockRuns);
  return invoke<RunSummary[]>("activity_runs");
}

export function activityCalls(limit?: number): Promise<ActivityCall[]> {
  if (!isTauri) return Promise.resolve(mockCalls);
  return invoke<ActivityCall[]>("activity_calls", { limit });
}

export function activityRunCalls(runId: string): Promise<ActivityCall[]> {
  if (!isTauri) {
    return Promise.resolve(mockCalls.filter((c) => c.runId === runId));
  }
  return invoke<ActivityCall[]>("activity_run_calls", { runId });
}

export function activityStats(sinceEpochS: number): Promise<ActivityStats> {
  if (!isTauri) return Promise.resolve(mockStats);
  return invoke<ActivityStats>("activity_stats", { since: sinceEpochS });
}

// ------------------------------------------------------------- simulação ---

const agoraS = Math.floor(Date.now() / 1000);

const mockRuns: RunSummary[] = [
  {
    id: "run_1",
    chatId: 3,
    model: "Qwen3-14B",
    mode: "smart",
    status: "done",
    prompt: "conserte o teste que está falhando",
    summary: "Corrigi o import quebrado em src/lib/parse.ts; suíte passando.",
    workspaceDir: "/projetos/openweights",
    usage: {
      steps: 6,
      promptTokens: 18_420,
      completionTokens: 2_130,
      toolCalls: 9,
      durationMs: 74_000,
    },
    createdAt: agoraS - 1800,
    finishedAt: agoraS - 1726,
  },
  {
    id: "run_2",
    chatId: null,
    model: "Qwen3-14B",
    mode: "approve",
    status: "waitingApproval",
    prompt: "resuma o que mudou no projeto hoje",
    summary: null,
    workspaceDir: "/projetos/openweights",
    usage: null,
    createdAt: agoraS - 600,
    finishedAt: null,
  },
  {
    id: "run_3",
    chatId: 2,
    model: "Llama-3.2-8B",
    mode: "yolo",
    status: "maxSteps",
    prompt: "renomeie o módulo inteiro",
    summary: "Parou no limite de passos.",
    workspaceDir: "/projetos/site",
    usage: {
      steps: 20,
      promptTokens: 61_000,
      completionTokens: 7_400,
      toolCalls: 24,
      durationMs: 320_000,
    },
    createdAt: agoraS - 86_400,
    finishedAt: agoraS - 86_080,
  },
];

const mockCalls: ActivityCall[] = [
  {
    callId: "c9",
    runId: "run_1",
    toolName: "fs_edit",
    origin: "builtin",
    state: "ok",
    error: null,
    startedAt: Date.now() - 1_740_000,
    finishedAt: Date.now() - 1_739_400,
    decision: "allowOnce",
    source: "user",
  },
  {
    callId: "c8",
    runId: "run_1",
    toolName: "test_run",
    origin: "builtin",
    state: "ok",
    error: null,
    startedAt: Date.now() - 1_760_000,
    finishedAt: Date.now() - 1_752_000,
    decision: "allowAlways",
    source: "user",
  },
  {
    callId: "c7",
    runId: "run_3",
    toolName: "terminal_run",
    origin: "builtin",
    state: "denied",
    error: null,
    startedAt: Date.now() - 86_400_000,
    finishedAt: Date.now() - 86_399_000,
    decision: "deny",
    source: "user",
  },
  {
    callId: "c6",
    runId: "run_1",
    toolName: "fs_read",
    origin: "builtin",
    state: "ok",
    error: null,
    startedAt: Date.now() - 1_800_000,
    finishedAt: Date.now() - 1_799_800,
    decision: null,
    source: null,
  },
];

const mockStats: ActivityStats = {
  runs: 3,
  failed: 1,
  toolCalls: 33,
  denied: 1,
  promptTokens: 79_420,
  completionTokens: 9_530,
  durationMs: 394_000,
};
