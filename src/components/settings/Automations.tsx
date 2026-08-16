// Automações: um pedido que roda sozinho de tempos em tempos.
//
// A lista responde três perguntas de uma olhada — o que a automação faz,
// quando ela roda de novo e como terminou da última vez. É o mínimo para
// confiar em algo que dispara sem ninguém por perto.
//
// E é justamente o "sem ninguém por perto" que manda no desenho desta tela:
// no nível "pedir sempre" a execução PARA no primeiro pedido de confirmação e
// fica esperando uma resposta que não vai chegar de madrugada. O cartão avisa
// isso no formulário e marca a linha quando acontece, com um caminho para
// abrir a trilha, ver o que a automação queria fazer e RESPONDER dali mesmo:
// a gaveta reata à execução, então a confirmação chega a quem está esperando.

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import {
  INTERVALS,
  automationDelete,
  automationRunNow,
  automationSave,
  automationsList,
  firstRun,
  type ScheduledTask,
  type ScheduledTaskInput,
} from "../../lib/agent/automation";
import type { RunMode, RunStatus } from "../../lib/agent/types";
import { listLocalModels, pickWorkspace } from "../../lib/api";
import { formatAgo } from "../../lib/format";
import { errorMessage } from "../../lib/serverSession";
import { listen } from "../../lib/tauri";
import TraceDrawer from "../agent/TraceDrawer";

/** Os mesmos níveis do chat: "sem ferramentas" não automatizaria nada. */
const AUTH_MODES: RunMode[] = ["approve", "smart", "yolo"];

/** Rótulo de cada intervalo oferecido, em minutos. */
const INTERVAL_KEYS: Record<number, string> = {
  0: "manual",
  60: "hourly",
  240: "every4h",
  1440: "daily",
  10080: "weekly",
};

/** Padrão de uma automação nova: todo dia, o caso que a pessoa vem escrever. */
const DEFAULT_INTERVAL = 1440;

const input =
  "w-full rounded-lg border border-edge bg-panel2 px-3 py-2 text-sm outline-none placeholder:text-dim focus:border-accent";
const ghostButton =
  "rounded-lg border border-edge px-3 py-1.5 text-[12px] text-dim transition-colors hover:border-accent hover:text-ink disabled:opacity-40";

/**
 * Execução em curso: ao começar, o backend limpa o status e só grava de novo
 * quando termina. O id da execução chega um pouco depois (subir o motor de IA
 * demora), então quem denuncia o começo é a hora, não o id.
 */
function isRunning(task: ScheduledTask): boolean {
  return task.lastStatus === null && task.lastRunAt !== null;
}

function intervalLabel(t: TFunction, minutes: number): string {
  const key = INTERVAL_KEYS[minutes];
  return key
    ? t(`agent.automations.every.${key}`)
    : t("agent.automations.every.custom", { minutes });
}

/** Distância em dias de calendário — "amanhã" é o dia seguinte, não 24 h. */
function dayDistance(target: Date): number {
  const hoje = new Date();
  hoje.setHours(0, 0, 0, 0);
  const alvo = new Date(target);
  alvo.setHours(0, 0, 0, 0);
  return Math.round((alvo.getTime() - hoje.getTime()) / 86_400_000);
}

/** "hoje às 09:00", "amanhã às 09:00" ou a data, no fuso e no formato locais. */
function whenLabel(t: TFunction, lang: string, tsMs: number): string {
  if (tsMs <= Date.now()) return t("agent.automations.soon");
  const date = new Date(tsMs);
  const time = new Intl.DateTimeFormat(lang, {
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
  const dias = dayDistance(date);
  if (dias === 0) return t("agent.automations.today", { time });
  if (dias === 1) return t("agent.automations.tomorrow", { time });
  return new Intl.DateTimeFormat(lang, {
    day: "2-digit",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

function scheduleLabel(t: TFunction, lang: string, task: ScheduledTask): string {
  if (!task.enabled) return t("agent.automations.paused");
  if (task.everyMinutes <= 0 || task.nextRunAt == null) {
    return t("agent.automations.manualOnly");
  }
  return t("agent.automations.nextRun", {
    when: whenLabel(t, lang, task.nextRunAt),
  });
}

/**
 * Estado da última vez. Reaproveita as frases que o resto do app já usa para
 * `RunStatus`; só os dois casos sem frase curta pronta ganharam chave nova.
 */
function statusLabel(t: TFunction, status: RunStatus): string {
  switch (status) {
    case "done":
      return t("agent.run.finished");
    case "error":
      return t("agent.run.error");
    case "cancelled":
      return t("agent.run.cancelled");
    case "waitingApproval":
      return t("agent.approval.pending");
    case "maxSteps":
      return t("agent.automations.statusMaxSteps");
    case "escalated":
      return t("agent.automations.statusEscalated");
    default:
      return t("agent.tool.running");
  }
}

function statusTone(status: RunStatus): string {
  switch (status) {
    case "done":
      return "text-ok";
    case "error":
      return "text-bad";
    case "waitingApproval":
    case "maxSteps":
    case "escalated":
      return "text-warn";
    default:
      return "text-dim";
  }
}

function StatusIcon({ status }: { status: RunStatus }) {
  const path =
    status === "done"
      ? "M20 6L9 17l-5-5"
      : status === "error"
        ? "M12 8v5M12 16.5v.01M10.3 3.9L2.6 17a2 2 0 001.7 3h15.4a2 2 0 001.7-3L13.7 3.9a2 2 0 00-3.4 0z"
        : "M12 7v5l3 2M21 12a9 9 0 11-18 0 9 9 0 0118 0z";
  return (
    <svg
      aria-hidden
      className="mt-0.5 h-3.5 w-3.5 shrink-0"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      viewBox="0 0 24 24"
    >
      <path d={path} />
    </svg>
  );
}

export default function Automations() {
  const { t, i18n } = useTranslation();
  // null = ainda carregando (lista vazia é um estado normal).
  const [tasks, setTasks] = useState<ScheduledTask[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** `"new"` = formulário em branco; uma tarefa = edição daquela linha. */
  const [editing, setEditing] = useState<ScheduledTask | "new" | null>(null);
  const [traceId, setTraceId] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      setTasks(await automationsList());
      setError(null);
    } catch (e) {
      // Uma recarga que falhou não apaga o que já está na tela: a lista velha
      // ainda diz mais do que um cartão vazio.
      setTasks((prev) => prev ?? []);
      setError(errorMessage(e));
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  // O backend avisa quando uma automação termina: a lista se atualiza sozinha
  // com a pessoa parada nesta tela.
  useEffect(() => {
    let un: (() => void) | undefined;
    let cancelled = false;
    listen("automation", () => void reload()).then((f) => {
      // Se o cleanup rodou antes de o listen() resolver (StrictMode),
      // desregistra imediatamente para não vazar o listener.
      if (cancelled) f();
      else un = f;
    });
    return () => {
      cancelled = true;
      un?.();
    };
  }, [reload]);

  // O aviso só vem no FIM da execução — quando ela para esperando confirmação,
  // ninguém avisa. Enquanto houver algo em curso a lista se recarrega sozinha,
  // senão a linha ficaria em "Executando..." para sempre.
  const running = tasks?.some(isRunning) ?? false;
  useEffect(() => {
    if (!running) return;
    const id = window.setInterval(() => void reload(), 10_000);
    return () => window.clearInterval(id);
  }, [running, reload]);

  const closeTrace = useCallback(() => setTraceId(null), []);

  return (
    <div id="automations" className="rounded-xl border border-edge bg-panel px-5 py-4">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="text-sm">{t("agent.automations.title")}</div>
          <div className="mt-1 text-[12px] text-dim">
            {t("agent.automations.subtitle")}
          </div>
        </div>
        <button
          type="button"
          onClick={() => setEditing((v) => (v === "new" ? null : "new"))}
          className="shrink-0 rounded-lg bg-accent px-3 py-1.5 text-[12px] font-medium text-white"
        >
          {t("agent.automations.add")}
        </button>
      </div>

      {editing && (
        <TaskForm
          key={editing === "new" ? "new" : editing.id}
          task={editing === "new" ? null : editing}
          onCancel={() => setEditing(null)}
          onSaved={async () => {
            setEditing(null);
            await reload();
          }}
        />
      )}

      {error && (
        <p className="mt-3 rounded-lg border border-bad/40 bg-bad/10 px-3 py-2 text-[12px] text-bad">
          {error}
        </p>
      )}

      <div className="mt-4 flex flex-col gap-2">
        {!tasks && <p className="text-[12px] text-dim">{t("common.loading")}</p>}
        {tasks?.length === 0 && (
          <p className="text-[12px] text-dim">{t("agent.automations.empty")}</p>
        )}
        {tasks?.map((task) => (
          <TaskRow
            key={task.id}
            task={task}
            lang={i18n.language}
            open={editing !== "new" && editing?.id === task.id}
            onEdit={() => setEditing(task)}
            onChanged={reload}
            onError={setError}
            onOpenTrace={setTraceId}
          />
        ))}
      </div>

      <p className="mt-3 text-[11px] text-dim">{t("agent.automations.appHint")}</p>

      {traceId && <TraceDrawer runId={traceId} onClose={closeTrace} />}
    </div>
  );
}

// ------------------------------------------------------------------ linha ---

function TaskRow({
  task,
  lang,
  open,
  onEdit,
  onChanged,
  onError,
  onOpenTrace,
}: {
  task: ScheduledTask;
  lang: string;
  /** Esta linha é a que está aberta no formulário. */
  open: boolean;
  onEdit: () => void;
  onChanged: () => Promise<void>;
  onError: (message: string | null) => void;
  onOpenTrace: (runId: string) => void;
}) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const running = isRunning(task);
  const status: RunStatus | null = task.lastStatus ?? (running ? "running" : null);
  const waiting = task.lastStatus === "waitingApproval";
  const lastRunId = task.lastRunId;

  async function run(action: () => Promise<unknown>) {
    setBusy(true);
    onError(null);
    try {
      await action();
      await onChanged();
    } catch (e) {
      onError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  function toggle() {
    void run(() =>
      automationSave({
        id: task.id,
        name: task.name,
        prompt: task.prompt,
        workspaceDir: task.workspaceDir,
        model: task.model,
        mode: task.mode,
        workMode: task.workMode,
        everyMinutes: task.everyMinutes,
        // O intervalo não mudou: preservar a hora marcada evita que ligar e
        // desligar a chavinha empurre a próxima execução para longe.
        nextRunAt: task.nextRunAt,
        enabled: !task.enabled,
      }),
    );
  }

  return (
    <div
      className={`rounded-xl border bg-panel2 px-4 py-3 ${
        open ? "border-accent" : "border-edge"
      }`}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span
              className={`truncate text-sm ${task.enabled ? "text-ink" : "text-dim"}`}
            >
              {task.name}
            </span>
            <span className="shrink-0 rounded-md bg-panel px-1.5 py-0.5 text-[10px] text-dim">
              {intervalLabel(t, task.everyMinutes)}
            </span>
          </div>
          <p className="mt-1 truncate text-[11px] text-dim" title={task.prompt}>
            {task.prompt}
          </p>
          <p className="mt-1 text-[11px] text-dim">
            {scheduleLabel(t, lang, task)}
          </p>
        </div>

        <button
          type="button"
          role="switch"
          aria-checked={task.enabled}
          aria-label={t("agent.automations.toggle", { name: task.name })}
          disabled={busy}
          onClick={toggle}
          className={`mt-0.5 flex h-4 w-7 shrink-0 items-center rounded-full p-0.5 transition-colors disabled:opacity-40 ${
            task.enabled ? "bg-accent" : "bg-edge"
          }`}
        >
          <span
            aria-hidden
            className={`h-3 w-3 rounded-full bg-panel transition-transform ${
              task.enabled ? "translate-x-3" : ""
            }`}
          />
        </button>
      </div>

      {status && (
        <div className="mt-2 flex items-start gap-1.5 text-[11px]">
          <span className={`flex shrink-0 items-start gap-1.5 ${statusTone(status)}`}>
            <StatusIcon status={status} />
            {statusLabel(t, status)}
          </span>
          {task.lastSummary && (
            <span className="min-w-0 flex-1 truncate text-dim" title={task.lastSummary}>
              — {task.lastSummary}
            </span>
          )}
          {task.lastRunAt != null && (
            <span className="ml-auto shrink-0 pl-2 text-dim">
              {formatAgo(lang, task.lastRunAt)}
            </span>
          )}
        </div>
      )}
      {!status && (
        <p className="mt-2 text-[11px] text-dim">{t("agent.automations.never")}</p>
      )}

      {waiting && (
        <div className="mt-2 rounded-lg border border-warn/40 bg-warn/10 px-3 py-2.5">
          <div className="text-[12px] font-medium text-ink">
            {t("agent.automations.waitingTitle")}
          </div>
          <p className="mt-1 text-[11px] leading-relaxed text-dim">
            {t("agent.automations.waitingBody")}
          </p>
          {/* Sem o id da execução não há trilha para abrir — em vez de um botão
              que não leva a lugar nenhum, fica só o aviso. */}
          {lastRunId && (
            <button
              type="button"
              onClick={() => onOpenTrace(lastRunId)}
              className="mt-2 rounded-lg border border-warn/40 px-3 py-1.5 text-[12px] text-warn transition-colors hover:bg-warn/10"
            >
              {t("agent.run.showTrace")}
            </button>
          )}
        </div>
      )}

      <div className="mt-2.5 flex flex-wrap gap-1.5">
        <button
          type="button"
          disabled={busy || running}
          onClick={() => void run(() => automationRunNow(task.id))}
          className={ghostButton}
        >
          {busy || running
            ? t("agent.tool.running")
            : t("agent.automations.runNow")}
        </button>
        <button type="button" disabled={busy} onClick={onEdit} className={ghostButton}>
          {t("agent.automations.edit")}
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={() => {
            // Apagar não tem volta: a automação e o histórico dela somem.
            if (!confirm(t("agent.automations.deleteConfirm", { name: task.name })))
              return;
            void run(() => automationDelete(task.id));
          }}
          className={ghostButton}
        >
          {t("agent.automations.delete")}
        </button>
        {lastRunId && !waiting && (
          <button
            type="button"
            onClick={() => onOpenTrace(lastRunId)}
            className={ghostButton}
          >
            {t("agent.run.showTrace")}
          </button>
        )}
      </div>
    </div>
  );
}

// ------------------------------------------------------------- formulário ---

function TaskForm({
  task,
  onCancel,
  onSaved,
}: {
  /** `null` = automação nova. */
  task: ScheduledTask | null;
  onCancel: () => void;
  onSaved: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState(task?.name ?? "");
  const [prompt, setPrompt] = useState(task?.prompt ?? "");
  const [dir, setDir] = useState<string | null>(task?.workspaceDir ?? null);
  const [model, setModel] = useState(task?.model ?? "");
  const [everyMinutes, setEveryMinutes] = useState(
    task?.everyMinutes ?? DEFAULT_INTERVAL,
  );
  // Numa automação nova, "pedir para alterações" é o único padrão honesto:
  // "pedir sempre" travaria na primeira ferramenta, sem ninguém para responder.
  const [mode, setMode] = useState<RunMode>(task?.mode ?? "smart");
  const [models, setModels] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void listLocalModels()
      .then((rows) => setModels(rows.map((m) => m.name)))
      .catch(() => setModels([]));
  }, []);

  async function pick() {
    const picked = await pickWorkspace().catch(() => null);
    if (picked) setDir(picked);
  }

  const yoloNeedsDir = mode === "yolo" && !dir;
  const canSubmit =
    !busy && name.trim().length > 0 && prompt.trim().length > 0 && !yoloNeedsDir;

  async function submit() {
    if (!canSubmit) return;
    setBusy(true);
    setError(null);
    try {
      // A próxima vez só é recalculada quando o intervalo muda: senão, corrigir
      // uma palavra no texto adiaria a execução já marcada para amanhã.
      const nextRunAt =
        !task || task.everyMinutes !== everyMinutes
          ? firstRun(everyMinutes)
          : task.nextRunAt;
      const payload: ScheduledTaskInput = {
        id: task?.id,
        name: name.trim(),
        prompt: prompt.trim(),
        workspaceDir: dir,
        model: model.trim() || null,
        mode,
        // Automação é para executar; o modo de trabalho não se escolhe aqui.
        workMode: task?.workMode ?? "agent",
        everyMinutes,
        nextRunAt,
        enabled: task?.enabled ?? true,
      };
      await automationSave(payload);
      await onSaved();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mt-4 rounded-xl border border-edge bg-panel2 p-4">
      <label className="block text-[11px] text-dim">
        {t("agent.automations.name")}
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder={t("agent.automations.namePlaceholder")}
          className={`mt-1 ${input}`}
        />
      </label>

      <label className="mt-3 block text-[11px] text-dim">
        {t("agent.automations.prompt")}
        <textarea
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          rows={3}
          placeholder={t("agent.automations.promptPlaceholder")}
          className={`mt-1 ${input}`}
        />
      </label>

      <div className="mt-3 text-[11px] text-dim">
        {t("agent.automations.workspace")}
        <div className="mt-1 flex items-center gap-2">
          <span
            className="min-w-0 flex-1 truncate font-mono text-[12px]"
            title={dir ?? undefined}
          >
            {dir ?? t("agent.automations.noWorkspace")}
          </span>
          <button type="button" onClick={() => void pick()} className={ghostButton}>
            {t(dir ? "workspace.change" : "workspace.add")}
          </button>
          {dir && (
            <button
              type="button"
              onClick={() => setDir(null)}
              className={ghostButton}
            >
              {t("agent.automations.clearWorkspace")}
            </button>
          )}
        </div>
      </div>

      <label className="mt-3 block text-[11px] text-dim">
        {t("agent.automations.model")}
        <select
          value={model}
          onChange={(e) => setModel(e.target.value)}
          className={`mt-1 ${input}`}
        >
          <option value="">{t("agent.automations.modelDefault")}</option>
          {/* Um modelo que saiu da biblioteca continua listado enquanto for o
              escolhido — some da tela seria pior do que mostrar o nome morto. */}
          {model && !models.includes(model) && (
            <option value={model}>{model}</option>
          )}
          {models.map((m) => (
            <option key={m} value={m}>
              {m}
            </option>
          ))}
        </select>
      </label>

      <label className="mt-3 block text-[11px] text-dim">
        {t("agent.automations.interval")}
        <select
          value={everyMinutes}
          onChange={(e) => setEveryMinutes(Number(e.target.value))}
          className={`mt-1 ${input}`}
        >
          {INTERVALS.map((minutes) => (
            <option key={minutes} value={minutes}>
              {intervalLabel(t, minutes)}
            </option>
          ))}
          {!INTERVAL_KEYS[everyMinutes] && (
            <option value={everyMinutes}>{intervalLabel(t, everyMinutes)}</option>
          )}
        </select>
      </label>

      <label className="mt-3 block text-[11px] text-dim">
        {t("agent.mode.label")}
        <select
          value={mode}
          onChange={(e) => setMode(e.target.value as RunMode)}
          className={`mt-1 ${input}`}
        >
          {AUTH_MODES.map((option) => (
            <option key={option} value={option}>
              {t(`agent.mode.${option}`)}
            </option>
          ))}
        </select>
      </label>
      <p className="mt-1 text-[11px] leading-relaxed text-dim">
        {t(`agent.mode.${mode}Hint`)}
      </p>
      {/* O aviso que a tela precisa dar: quando a automação dispara, não há
          ninguém para responder a um pedido de confirmação. */}
      <p
        className={`mt-1 text-[11px] leading-relaxed ${
          mode === "approve" ? "text-warn" : "text-dim"
        }`}
      >
        {t("agent.automations.unattendedHint", {
          mode: t("agent.mode.approve"),
        })}
      </p>
      {yoloNeedsDir && (
        <p className="mt-1 text-[11px] leading-relaxed text-warn">
          {t("agent.yolo.needWorkspace")}
        </p>
      )}

      {error && (
        <p className="mt-3 text-[12px] text-bad" role="alert">
          {error}
        </p>
      )}

      <div className="mt-4 flex justify-end gap-2">
        <button type="button" onClick={onCancel} className={ghostButton}>
          {t("common.cancel")}
        </button>
        <button
          type="button"
          disabled={!canSubmit}
          onClick={() => void submit()}
          className="rounded-lg bg-accent px-4 py-1.5 text-[12px] font-medium text-white disabled:opacity-40"
        >
          {t("common.save")}
        </button>
      </div>
    </div>
  );
}
