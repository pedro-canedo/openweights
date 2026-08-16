// Ponte tipada com os comandos das automações.
//
// Automação é um pedido que roda sozinho de tempos em tempos. O agendamento é
// simples de propósito: um intervalo em minutos e a hora da próxima vez em
// epoch. Quem calcula a primeira ocorrência é ESTA camada, que conhece o fuso
// e o relógio da pessoa — o Rust só soma o intervalo.
//
// Fora do Tauri (npm run dev no navegador) tudo é simulado em memória.

import { invoke, isTauri } from "../tauri";
import type { RunMode, RunStatus } from "./types";
import type { WorkMode } from "./scout";

/** Uma automação. Espelha `lr_types::automation::ScheduledTask`. */
export interface ScheduledTask {
  id: string;
  name: string;
  prompt: string;
  workspaceDir: string | null;
  /** Vazio = o primeiro modelo da biblioteca. */
  model: string | null;
  mode: RunMode;
  workMode: WorkMode;
  /** De quanto em quanto tempo, em minutos. `0` = só quando a pessoa mandar. */
  everyMinutes: number;
  /** Próxima execução (epoch ms). */
  nextRunAt: number | null;
  enabled: boolean;
  lastRunAt: number | null;
  lastRunId: string | null;
  lastStatus: RunStatus | null;
  lastSummary: string | null;
  createdAt: number;
}

/** O que a tela manda ao criar ou editar. */
export interface ScheduledTaskInput {
  /** Ausente = criar. */
  id?: string;
  name: string;
  prompt: string;
  workspaceDir?: string | null;
  model?: string | null;
  mode: RunMode;
  workMode: WorkMode;
  everyMinutes: number;
  nextRunAt?: number | null;
  enabled: boolean;
}

/** Intervalos oferecidos na tela, em minutos. */
export const INTERVALS = [0, 60, 60 * 4, 60 * 24, 60 * 24 * 7] as const;

/**
 * Quando esta automação roda pela primeira vez.
 *
 * Para intervalos de um dia ou mais, mira o próximo horário cheio escolhido
 * (padrão: 9h); para os curtos, conta a partir de agora. É aqui que o fuso da
 * pessoa entra na conta — o backend só soma minutos depois disso.
 */
export function firstRun(everyMinutes: number, hour = 9): number | null {
  if (everyMinutes <= 0) return null;
  const now = new Date();
  if (everyMinutes < 60 * 24) return now.getTime() + everyMinutes * 60_000;
  const alvo = new Date(now);
  alvo.setHours(hour, 0, 0, 0);
  if (alvo.getTime() <= now.getTime()) alvo.setDate(alvo.getDate() + 1);
  return alvo.getTime();
}

export function automationsList(): Promise<ScheduledTask[]> {
  if (!isTauri) return Promise.resolve([...mockTasks]);
  return invoke<ScheduledTask[]>("automations_list");
}

export function automationSave(task: ScheduledTaskInput): Promise<ScheduledTask> {
  if (!isTauri) return mockSave(task);
  return invoke<ScheduledTask>("automation_save", { task });
}

export function automationDelete(id: string): Promise<void> {
  if (!isTauri) {
    mockTasks = mockTasks.filter((t) => t.id !== id);
    return Promise.resolve();
  }
  return invoke<void>("automation_delete", { id });
}

/** Roda agora, sem esperar o horário. Devolve o id da execução. */
export function automationRunNow(id: string): Promise<string> {
  if (!isTauri) return mockRunNow(id);
  return invoke<string>("automation_run_now", { id });
}

// ------------------------------------------------------------- simulação ---

let mockTasks: ScheduledTask[] = [
  {
    id: "auto_exemplo",
    name: "Resumo do dia",
    prompt: "Liste o que mudou no projeto hoje e o que ficou pendente.",
    workspaceDir: "/projetos/openweights",
    model: null,
    mode: "approve",
    workMode: "agent",
    everyMinutes: 60 * 24,
    nextRunAt: Date.now() + 3_600_000,
    enabled: true,
    lastRunAt: Date.now() - 86_400_000,
    lastRunId: "run_exemplo",
    lastStatus: "done",
    lastSummary: "3 arquivos alterados, testes passando.",
    createdAt: Date.now() - 7 * 86_400_000,
  },
];

function mockSave(task: ScheduledTaskInput): Promise<ScheduledTask> {
  const anterior = mockTasks.find((t) => t.id === task.id);
  const saved: ScheduledTask = {
    id: task.id ?? `auto_${Date.now()}`,
    name: task.name.trim(),
    prompt: task.prompt.trim(),
    workspaceDir: task.workspaceDir ?? null,
    model: task.model ?? null,
    mode: task.mode,
    workMode: task.workMode,
    everyMinutes: task.everyMinutes,
    nextRunAt: task.nextRunAt ?? null,
    enabled: task.enabled,
    lastRunAt: anterior?.lastRunAt ?? null,
    lastRunId: anterior?.lastRunId ?? null,
    lastStatus: anterior?.lastStatus ?? null,
    lastSummary: anterior?.lastSummary ?? null,
    createdAt: anterior?.createdAt ?? Date.now(),
  };
  mockTasks = anterior
    ? mockTasks.map((t) => (t.id === saved.id ? saved : t))
    : [...mockTasks, saved];
  return Promise.resolve(saved);
}

function mockRunNow(id: string): Promise<string> {
  const runId = `run_${Date.now()}`;
  mockTasks = mockTasks.map((t) =>
    t.id === id
      ? {
          ...t,
          lastRunAt: Date.now(),
          lastRunId: runId,
          lastStatus: "done",
          lastSummary: "Execução simulada.",
        }
      : t,
  );
  return Promise.resolve(runId);
}
