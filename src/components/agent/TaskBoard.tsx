// Quadro do plano (Scout Rule): o objetivo grande dividido em entregas
// pequenas que cabem na janela do modelo local.
//
// É o entregável visual da fase: o usuário precisa VER a divisão acontecendo
// — a etapa em andamento destacada, as concluídas com o resumo curto
// (handoff) que foi passado adiante, e o contexto reiniciado a cada entrega.
//
// O componente é burro de propósito: recebe o `TaskPlan` pronto (o
// `runStore` busca o estruturado a cada `focus.updated`) e não conhece
// comandos. As ações de planejamento chegam por callback.

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { getServerProps } from "../../lib/api";
import { formatDuration, formatEta } from "../../lib/format";
import { describesModel } from "../../lib/agent/types";
import {
  isTaskFinished,
  planProgress,
  type Task,
  type TaskPlan,
  type TaskStatus,
} from "../../lib/agent/scout";

/**
 * Fração da janela reservada a UMA entrega. Espelha `WindowBudget` do Rust
 * (`crates/types/src/scout.rs`) — aqui só para a dica na tela.
 */
const TASK_RATIO = 0.45;

const STATUS_STYLE: Record<TaskStatus, { dot: string; text: string }> = {
  pending: { dot: "bg-edge", text: "text-dim" },
  running: { dot: "bg-accent", text: "text-accent" },
  done: { dot: "bg-ok", text: "text-ok" },
  blocked: { dot: "bg-warn", text: "text-warn" },
  failed: { dot: "bg-bad", text: "text-bad" },
  skipped: { dot: "bg-edge", text: "text-dim" },
};

function Chevron({ open }: { open: boolean }) {
  return (
    <svg
      className={`h-3.5 w-3.5 shrink-0 text-dim transition-transform ${
        open ? "rotate-90" : ""
      }`}
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      viewBox="0 0 24 24"
    >
      <path d="M9 6l6 6-6 6" />
    </svg>
  );
}

/** Janela do modelo (GET /props). `null` = desconhecida. */
function useWindowTokens(model?: string | null): number | null {
  const [nCtx, setNCtx] = useState<number | null>(null);
  useEffect(() => {
    let cancelled = false;
    void getServerProps(model)
      .then((props) => {
        // Sem `describesModel`, a janela do roteador (0) viraria a do modelo.
        if (!cancelled) setNCtx(describesModel(props) ? props.nCtx : null);
      })
      .catch(() => {
        if (!cancelled) setNCtx(null);
      });
    return () => {
      cancelled = true;
    };
  }, [model]);
  return nCtx;
}

function ProgressBar({ done, total }: { done: number; total: number }) {
  const pct = total > 0 ? Math.round((done / total) * 100) : 0;
  return (
    <div
      className="h-1.5 w-full overflow-hidden rounded-full bg-panel2"
      role="progressbar"
      aria-valuenow={done}
      aria-valuemin={0}
      aria-valuemax={total}
    >
      <div
        className="h-full rounded-full bg-accent transition-[width] duration-500"
        style={{ width: `${pct}%` }}
      />
    </div>
  );
}

/** Esqueleto de enquanto o modelo divide a tarefa. */
function PlanningSkeleton() {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col gap-2">
      <p className="flex items-center gap-2 text-[12px] text-dim">
        <span className="h-2 w-2 animate-pulse rounded-full bg-accent" />
        {t("plan.planning")}
      </p>
      {[0, 1, 2].map((i) => (
        <div
          key={i}
          className="h-7 animate-pulse rounded-lg bg-panel2"
          style={{ opacity: 1 - i * 0.28, animationDelay: `${i * 140}ms` }}
        />
      ))}
    </div>
  );
}

function TaskRow({
  task,
  index,
  total,
  current,
  open,
  onToggle,
}: {
  task: Task;
  index: number;
  total: number;
  /** Entrega em foco: contexto novo rodando agora. */
  current: boolean;
  open: boolean;
  onToggle: () => void;
}) {
  const { t } = useTranslation();
  const style = STATUS_STYLE[task.status];
  const finished = isTaskFinished(task.status);
  // Quanto DUROU quando já acabou; quanto deve durar quando ainda não —
  // previsão marcada com "≈" porque é estimativa (tokens ÷ tok/s medido).
  const tempo = (() => {
    const { startedAtMs, finishedAtMs, etaSeconds } = task;
    if (startedAtMs && finishedAtMs && finishedAtMs > startedAtMs) {
      return formatDuration(finishedAtMs - startedAtMs);
    }
    if (etaSeconds) return `≈ ${formatEta(etaSeconds)}`;
    return null;
  })();
  const blocked = task.status === "blocked" || task.status === "failed";

  return (
    <li
      className={`rounded-lg border transition-colors ${
        current
          ? "border-accent/50 bg-accent/5"
          : blocked
            ? "border-warn/40 bg-warn/5"
            : "border-transparent hover:bg-panel2"
      }`}
    >
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        className="flex w-full items-start gap-2 px-2 py-1.5 text-left"
      >
        <span
          className={`mt-1 h-2 w-2 shrink-0 rounded-full ${style.dot} ${
            task.status === "running" ? "animate-pulse" : ""
          }`}
        />
        <span className="w-4 shrink-0 pt-px text-right text-[11px] tabular-nums text-dim">
          {index + 1}
        </span>
        <span className="min-w-0 flex-1">
          <span
            className={`block text-[12.5px] leading-snug ${
              finished ? "text-dim" : "text-ink"
            }`}
          >
            {task.title}
          </span>
          <span className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[10.5px]">
            <span className={style.text}>{t(`plan.status.${task.status}`)}</span>
            {tempo && (
              <>
                <span className="text-dim" aria-hidden>
                  ·
                </span>
                <span className="tabular-nums text-dim">{tempo}</span>
              </>
            )}
            {current && (
              <>
                <span className="text-dim" aria-hidden>
                  ·
                </span>
                <span className="text-dim">
                  {t("plan.taskOf", { n: index + 1, total })}
                </span>
                <span className="text-dim" aria-hidden>
                  ·
                </span>
                <span className="text-accent">{t("plan.contextFresh")}</span>
              </>
            )}
            {blocked && (
              <>
                <span className="text-dim" aria-hidden>
                  ·
                </span>
                <span className="text-warn">{t("plan.blockedReason")}</span>
              </>
            )}
          </span>
        </span>
        <span className="mt-0.5">
          <Chevron open={open} />
        </span>
      </button>

      {open && (
        <div className="flex flex-col gap-1.5 border-t border-edge/60 px-2 py-2 pl-8 select-text">
          {task.instruction && (
            <p className="text-[11.5px] leading-relaxed text-dim">
              {task.instruction}
            </p>
          )}
          {task.doneWhen && (
            <p className="text-[11px] leading-relaxed text-dim">
              <span className="text-ink">{t("plan.doneWhen")}: </span>
              {task.doneWhen}
            </p>
          )}
          {task.handoff && (
            <p className="rounded-md border-l-2 border-accent/60 bg-panel2 px-2 py-1.5 text-[11px] leading-relaxed text-dim">
              <span className="text-ink">{t("plan.handoff")}: </span>
              {task.handoff}
            </p>
          )}
          {task.error && (
            <p className="text-[11px] leading-relaxed text-warn">{task.error}</p>
          )}
          {task.dependsOn.length > 0 && (
            <p className="text-[10.5px] text-dim">
              {t("plan.dependsOn")}:{" "}
              {task.dependsOn.map((i) => i + 1).join(", ")}
            </p>
          )}
          {task.files.length > 0 && (
            <p className="flex flex-wrap items-center gap-1 text-[10.5px] text-dim">
              <span>{t("plan.files")}:</span>
              {task.files.map((file) => (
                <span
                  key={file}
                  className="rounded bg-panel2 px-1.5 py-0.5 font-mono text-[10px] text-dim"
                >
                  {file}
                </span>
              ))}
            </p>
          )}
        </div>
      )}
    </li>
  );
}

export default function TaskBoard({
  plan,
  planning = false,
  canApprove = false,
  compact = false,
  onApprove,
  onReplan,
  model,
}: {
  plan: TaskPlan | null;
  /** O modelo ainda está dividindo a tarefa (mostra o esqueleto). */
  planning?: boolean;
  /** Modo que planeja antes de agir: libera Aprovar/Refazer. */
  canApprove?: boolean;
  /** Versão do painel lateral: sem card, tipografia menor. */
  compact?: boolean;
  onApprove?: () => void;
  onReplan?: () => void;
  /** Modelo do run — a janela mostrada é a dele. */
  model?: string | null;
}) {
  const { t, i18n } = useTranslation();
  const [open, setOpen] = useState<Set<string>>(() => new Set());
  // O selo "plano aprovado" só faz sentido depois de o plano ter esperado
  // por você: em laço/agente ele já nasce liberado.
  const [wasPending, setWasPending] = useState(false);
  const nCtx = useWindowTokens(model);
  // O que ainda falta, somado: é a resposta para "quanto tempo isso leva".
  const etaTotal = (() => {
    if (!plan) return null;
    const s = plan.tasks
      .filter((t) => !isTaskFinished(t.status))
      .reduce((soma, t) => soma + (t.etaSeconds ?? 0), 0);
    return s > 0 ? s : null;
  })();

  const pending = plan != null && canApprove && !plan.approved;
  useEffect(() => {
    if (pending) setWasPending(true);
  }, [pending]);

  const toggle = (id: string) =>
    setOpen((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      return next;
    });

  const shell = compact
    ? "flex flex-col gap-2 px-3 py-3"
    : "flex flex-col gap-2.5 rounded-xl border border-edge bg-panel px-3 py-3";

  if (!plan) {
    return (
      <div className={shell}>
        {planning ? (
          <PlanningSkeleton />
        ) : (
          <p className="text-[12px] text-dim">{t("plan.empty")}</p>
        )}
      </div>
    );
  }

  const { done, total } = planProgress(plan);
  const num = (n: number) => n.toLocaleString(i18n.language);

  return (
    <div className={shell}>
      <div className="flex items-center gap-2">
        <svg
          className="h-4 w-4 shrink-0 text-accent"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          strokeLinecap="round"
          strokeLinejoin="round"
          viewBox="0 0 24 24"
        >
          <path d="M9 5h9M9 12h9M9 19h9M4 5l1 1 2-2M4 12l1 1 2-2M4 19l1 1 2-2" />
        </svg>
        <span className="min-w-0 flex-1 truncate text-[11px] font-medium tracking-wide text-dim uppercase">
          {t("plan.title")}
        </span>
        {wasPending && plan.approved && (
          <span className="shrink-0 rounded-full bg-ok/15 px-1.5 py-0.5 text-[10px] text-ok">
            {t("plan.approved")}
          </span>
        )}
        <span className="shrink-0 text-[11px] tabular-nums text-dim">
          {t("plan.progress", { done, total })}
        </span>
      </div>

      {plan.goal && (
        <p className="text-[12px] leading-relaxed text-ink select-text">
          <span className="text-dim">{t("plan.goal")}: </span>
          {plan.goal}
        </p>
      )}

      <ProgressBar done={done} total={total} />

      <ul className="flex flex-col gap-0.5">
        {plan.tasks.map((task, i) => (
          <TaskRow
            key={task.id || `${i}`}
            task={task}
            index={i}
            total={plan.tasks.length}
            current={i === plan.current && task.status === "running"}
            open={open.has(task.id || `${i}`)}
            onToggle={() => toggle(task.id || `${i}`)}
          />
        ))}
      </ul>

      {pending && (onApprove || onReplan) && (
        <div className="flex flex-col gap-1.5 border-t border-edge pt-2">
          <div className="flex flex-wrap items-center gap-2">
            {onApprove && (
              <button
                type="button"
                onClick={onApprove}
                className="rounded-lg bg-accent px-3 py-1.5 text-[12px] font-medium text-white transition-opacity hover:opacity-90"
              >
                {t("plan.approve")}
              </button>
            )}
            {onReplan && (
              <button
                type="button"
                onClick={onReplan}
                className="rounded-lg border border-edge px-3 py-1.5 text-[12px] text-dim transition-colors hover:text-ink"
              >
                {t("plan.replan")}
              </button>
            )}
          </div>
          <p className="text-[11px] text-dim">{t("plan.approveHint")}</p>
        </div>
      )}

      {etaTotal != null && (
        <p className="text-[10.5px] text-dim">
          {t("plan.etaTotal", { time: formatEta(etaTotal) })}
        </p>
      )}
      {nCtx != null && nCtx > 0 && (
        <p className="text-[10.5px] text-dim">
          {t("plan.windowHint", {
            ctx: num(nCtx),
            perTask: num(Math.round(nCtx * TASK_RATIO)),
          })}
        </p>
      )}
    </div>
  );
}
