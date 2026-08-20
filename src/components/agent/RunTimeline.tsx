// Execução do agente desenhada em dois lugares, com o mesmo vocabulário:
//
//  - `RunTrail`: a trilha dentro do fluxo do chat (texto do assistente +
//    card por chamada de ferramenta, na ordem dos eventos).
//  - `RunTimeline`: o painel da coluna direita (mesmo slot do ParamsPanel),
//    com o cabeçalho de status, o resumo de uso e a trilha completa.
//
// As duas fontes possíveis têm o mesmo formato: o run corrente do
// `runStore` (alimentado por eventos em streaming) ou um run antigo,
// reconstruído por `run_events_list`. Por isso tudo aqui recebe um
// `RunView` pronto e não conhece comandos.

import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import {
  isRunActive,
  loadRunView,
  type NoteKind,
  type RunView,
  type StepItem,
  type SubagentItem,
  type TimelineItem,
  type ToolsItem,
} from "../../lib/agent/runStore";
import type { RunSummary } from "../../lib/agent/types";
import { plansFirst, type TaskPlan } from "../../lib/agent/scout";
import { runPlanGet } from "../../lib/agent/agentApi";
import { formatDuration, formatEta } from "../../lib/format";
import { errorMessage } from "../../lib/serverSession";
import Markdown from "../chat/Markdown";
import ThinkingBlock from "../chat/ThinkingBlock";
import TaskBoard from "./TaskBoard";
import ToolCallCard from "./ToolCallCard";
import { toolLabel } from "./toolMeta";

/** Avisos da trilha → chave de i18n e cor. */
const NOTES: Record<NoteKind, { key: string; className: string }> = {
  compacted: { key: "agent.run.compacted", className: "text-dim" },
  verified: { key: "agent.run.verified", className: "text-ok" },
  verifyFailed: { key: "agent.run.verifyFailed", className: "text-warn" },
  error: { key: "agent.run.error", className: "text-bad" },
  elicitation: { key: "agent.mcp.elicitation", className: "text-accent" },
  toolsUnsupported: { key: "agent.run.toolsUnsupported", className: "text-warn" },
  toolsRejected: { key: "agent.run.toolsRejected", className: "text-warn" },
};

function timeLabel(tsMs: number): string {
  return new Date(tsMs).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
  });
}

function countSteps(items: TimelineItem[]): number {
  return items.reduce((n, i) => (i.kind === "step" ? n + 1 : n), 0);
}

/** Duração do run em ms: a do resumo quando existe, senão o relógio. */
function durationMs(run: RunView, now: number): number {
  if (run.usage) return run.usage.durationMs;
  const end = run.finishedAtMs ?? now;
  return Math.max(0, end - run.startedAtMs);
}

/** Relógio — só enquanto o run está vivo. */
function useNow(active: boolean, hz = 1): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const id = window.setInterval(() => setNow(Date.now()), 1000 / hz);
    return () => window.clearInterval(id);
  }, [active, hz]);
  return now;
}

/** Texto do estado do run (usa só chaves já existentes em `agent.run.*`). */
function statusLabel(t: TFunction, run: RunView): string {
  switch (run.status) {
    case "running":
      return t("agent.tool.running");
    case "waitingApproval":
      return t("agent.approval.pending");
    case "done":
      return t("agent.run.finished");
    case "cancelled":
      return t("agent.run.cancelled");
    case "maxSteps":
      return t("agent.run.maxSteps", {
        n: run.usage?.steps ?? countSteps(run.items),
      });
    case "escalated":
      return t("agent.run.escalated");
    default:
      return t("agent.run.error");
  }
}

function statusClass(run: RunView): string {
  switch (run.status) {
    case "done":
      return "text-ok";
    case "waitingApproval":
    case "maxSteps":
    case "escalated":
      return "text-warn";
    case "error":
      return "text-bad";
    default:
      return "text-dim";
  }
}

/** Linha "N passos · M ferramentas · Xs". */
function usageLabel(t: TFunction, run: RunView, now: number): string {
  return t("agent.run.usage", {
    steps: run.usage?.steps ?? countSteps(run.items),
    tools: run.usage?.toolCalls ?? Object.keys(run.tools).length,
    time: formatDuration(durationMs(run, now)),
  });
}

function Pulse({ label }: { label: string }) {
  return (
    <div className="flex items-center gap-2 text-sm text-dim">
      <span className="h-2 w-2 animate-pulse rounded-full bg-accent" />
      {label}
    </div>
  );
}

function CheckpointLine({
  label,
  tsMs,
  backend,
}: {
  label: string;
  tsMs: number;
  backend: string;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex items-center gap-2 text-[11px] text-dim">
      <svg
        className="h-3.5 w-3.5 shrink-0 text-accent"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
        viewBox="0 0 24 24"
      >
        <path d="M12 3v6M12 21v-6M4.9 7.5l5.2 3M18.1 16.5l-5.2-3M4.9 16.5l5.2-3M18.1 7.5l-5.2 3" />
      </svg>
      <span className="min-w-0 truncate">
        {t("agent.checkpoint.created")}
        {label ? ` — ${label}` : ""}
      </span>
      <span className="ml-auto shrink-0 tabular-nums">{timeLabel(tsMs)}</span>
      <span className="sr-only">{backend}</span>
    </div>
  );
}

/**
 * Cardápio de ferramentas do run. Sem o cardápio a pessoa não entende por que
 * o agente "não tem" uma ferramenta que ela ligou nas preferências — a linha
 * mostra quantas couberam na janela e, ao expandir, quais são.
 */
function ToolsLine({ item }: { item: ToolsItem }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const names = item.active.map((name) => toolLabel(t, name)).join(" · ");

  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-start gap-2 text-[11px] text-dim">
        <svg
          className="mt-0.5 h-3.5 w-3.5 shrink-0"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          strokeLinecap="round"
          strokeLinejoin="round"
          viewBox="0 0 24 24"
        >
          <path d="M4 6h16M4 12h10M4 18h7" />
        </svg>
        <span className="min-w-0 flex-1 leading-relaxed select-text">
          {item.requested
            ? t("agent.run.toolsRequested", { tools: names })
            : t("agent.run.toolsSelected", {
                active: item.active.length,
                available: item.available,
              })}
        </span>
        <span className="shrink-0 tabular-nums">{timeLabel(item.tsMs)}</span>
      </div>
      {/* Pedido do modelo já vem com os nomes na frase — nada a expandir. */}
      {!item.requested && item.active.length > 0 && (
        <>
          <button
            type="button"
            aria-expanded={open}
            onClick={() => setOpen((v) => !v)}
            className="self-start pl-5 text-[11px] text-dim transition-colors hover:text-ink"
          >
            {t(open ? "agent.tool.showLess" : "agent.tool.showMore")}
          </button>
          {open && (
            <p className="pl-5 text-[11px] leading-relaxed text-dim select-text">
              {names}
            </p>
          )}
        </>
      )}
    </div>
  );
}

/**
 * Começo e fim de um ajudante. Delegar existe para poupar a janela do modelo
 * principal: a investigação inteira acontece aqui dentro e o que sobe para o
 * agente é só o resumo — a trilha mostra as duas coisas.
 */
function SubagentLine({ item }: { item: SubagentItem }) {
  const { t } = useTranslation();
  const done = item.phase === "end";
  const ok = item.status === "done";
  return (
    <div
      className={`flex items-start gap-2 text-[11px] ${done && !ok ? "text-warn" : "text-dim"}`}
    >
      <svg
        className="mt-0.5 h-3.5 w-3.5 shrink-0 text-accent"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
        viewBox="0 0 24 24"
      >
        <path d="M12 3a3 3 0 013 3v1a3 3 0 01-6 0V6a3 3 0 013-3zM5.5 21v-2a4 4 0 014-4h5a4 4 0 014 4v2" />
      </svg>
      <span className="min-w-0 flex-1 leading-relaxed select-text">
        {done
          ? t(ok ? "agent.run.subagentDone" : "agent.run.subagentStopped", {
              steps: item.steps,
            })
          : t(`agent.run.subagentStarted.${item.role}`, {
              objective: item.objective,
            })}
      </span>
      <span className="shrink-0 tabular-nums">{timeLabel(item.tsMs)}</span>
    </div>
  );
}

function StepBlock({
  step,
  live,
  compact,
}: {
  step: StepItem;
  /** Último passo de um run em andamento: o raciocínio ainda chega. */
  live: boolean;
  compact: boolean;
}) {
  const { t } = useTranslation();
  // O cronômetro do "Pensando…" contava sempre 0,0 s: a trilha passava
  // `thinkingMs={null}`, e um contador parado num modelo lento é a diferença
  // entre "está trabalhando" e "travou". Enquanto o passo pensa, o relógio
  // corre; quando a resposta começa a sair, congela no que levou.
  const pensando = live && !step.text;
  const agora = useNow(pensando, 10);
  if (!step.text && !step.reasoning) return null;
  // Três fontes para o tempo do raciocínio, em ordem de precisão: o fecho
  // pelos deltas (só ao vivo), o relógio correndo, e — no replay, onde os
  // deltas não existem — a diferença entre os eventos PERSISTIDOS do passo.
  const pensou =
    step.thoughtMs ??
    (pensando
      ? agora - step.startedAtMs
      : step.finishedAtMs != null && step.startedAtMs > 0
        ? step.finishedAtMs - step.startedAtMs
        : null);
  return (
    <div className="flex flex-col">
      {compact && (
        <span className="mb-1 text-[10px] tracking-wide text-dim uppercase">
          {t("agent.run.step", { n: step.index + 1 })}
        </span>
      )}
      {step.reasoning && (
        <ThinkingBlock
          reasoning={step.reasoning}
          thinkingMs={pensou != null ? Math.max(0, pensou) : null}
          active={pensando}
        />
      )}
      {step.text &&
        (compact ? (
          <p className="text-[12px] leading-relaxed whitespace-pre-wrap text-dim select-text">
            {step.text}
          </p>
        ) : (
          <Markdown text={step.text} />
        ))}
    </div>
  );
}

/**
 * Itens do run na ordem dos eventos. `compact` é a versão do painel
 * lateral (texto simples, sem markdown) — no chat o texto é a resposta.
 */
function RunItems({
  run,
  compact,
}: {
  run: RunView;
  compact: boolean;
}) {
  const { t } = useTranslation();
  const active = isRunActive(run.status);
  const lastId = run.items[run.items.length - 1]?.id;

  // Tudo que chega entre o começo e o fim de um ajudante é trabalho dele:
  // recuar deixa isso visível sem precisar de dono em cada evento.
  let depth = 0;
  const nested = run.items.map((item) => {
    if (item.kind === "subagent") {
      if (item.phase === "start") {
        depth = 1;
        return 0;
      }
      depth = 0;
      return 0;
    }
    return depth;
  });

  return (
    <>
      {run.items.map((item, i) => {
        const indent = nested[i] > 0;
        const wrap = (node: ReactNode) =>
          indent ? (
            <div key={item.id} className="border-l border-edge pl-3">
              {node}
            </div>
          ) : (
            node
          );
        if (item.kind === "subagent") {
          return <SubagentLine key={item.id} item={item} />;
        }
        if (item.kind === "step") {
          return wrap(
            <StepBlock
              key={item.id}
              step={item}
              live={active && item.id === lastId}
              compact={compact}
            />,
          );
        }
        if (item.kind === "tool") {
          const call = run.tools[item.id];
          if (!call) return null;
          return wrap(
            <ToolCallCard
              key={item.id}
              call={call}
              // No chat, comando já nasce aberto: a saída chega em streaming
              // e ficaria escondida atrás de um clique.
              defaultOpen={!compact && call.category === "execute"}
              stale={!active}
            />,
          );
        }
        if (item.kind === "checkpoint") {
          return wrap(
            <CheckpointLine
              key={item.id}
              label={item.label}
              backend={item.backend}
              tsMs={item.tsMs}
            />,
          );
        }
        if (item.kind === "tools") {
          return wrap(<ToolsLine key={item.id} item={item} />);
        }
        const note = NOTES[item.note];
        return wrap(
          <p
            key={item.id}
            className={`text-[11px] leading-relaxed select-text ${note.className}`}
          >
            {t(note.key)}
            {item.detail ? ` — ${item.detail}` : ""}
          </p>,
        );
      })}
    </>
  );
}

/**
 * Trilha da execução dentro do fluxo do chat: entra depois das mensagens,
 * enquanto o run existir. Ao terminar com sucesso o Chat troca a trilha
 * pela resposta persistida (a trilha completa segue no painel).
 */
export function RunTrail({
  run,
  onOpenTrace,
}: {
  run: RunView;
  /** Abre o painel lateral com a execução inteira. */
  onOpenTrace?: () => void;
}) {
  const { t } = useTranslation();
  const active = isRunActive(run.status);
  const now = useNow(active);

  // Enquanto uma ferramenta roda (ou espera confirmação) ela já mostra o
  // próprio indicador — dois "carregando" na tela confundem.
  const last = run.items[run.items.length - 1];
  const lastCall = last?.kind === "tool" ? run.tools[last.id] : undefined;
  const toolBusy =
    lastCall != null &&
    (lastCall.state === "running" || lastCall.state === "waiting");

  return (
    <div className="flex max-w-full flex-col gap-3 self-start">
      <RunItems run={run} compact={false} />

      {run.status === "running" && !toolBusy && (
        <Pulse label={t("agent.run.thinking")} />
      )}

      {!active && (
        <div className="flex flex-wrap items-center gap-2 text-[11px] text-dim">
          <span className={statusClass(run)}>{statusLabel(t, run)}</span>
          <span aria-hidden>·</span>
          <span className="tabular-nums">{usageLabel(t, run, now)}</span>
          {onOpenTrace && (
            <button
              type="button"
              onClick={onOpenTrace}
              className="underline underline-offset-2 transition-colors hover:text-ink"
            >
              {t("agent.run.showTrace")}
            </button>
          )}
        </div>
      )}
    </div>
  );
}

/**
 * Trilha de uma execução JÁ GRAVADA, pendurada na resposta dela dentro da
 * conversa.
 *
 * Sem isto a conversa reaberta mostrava só o texto final: a resposta dizia
 * "criei três arquivos" e nada na tela dizia quais, nem que um comando
 * rodou, nem o que ele devolveu. O trabalho continuava no banco, mas só a
 * tela Atividade sabia dele — e ninguém volta a uma conversa passando por
 * outra tela para descobrir o que aconteceu nela.
 *
 * Colapsada mas COMPLETA: o plano de trabalho, o veredito da conferência e a
 * causa do fim ficam sempre visíveis (vêm do resumo e do `plan_json`, sem
 * abrir a trilha); os passos/ferramentas/pensamentos moram no grupo
 * expansível, carregado só quando alguém abre — reconstruir a trilha custa
 * duas consultas por execução, e uma conversa longa tem muitas.
 */
export function PastRunTrail({
  runId,
  summary = null,
  onOpenTrace,
}: {
  runId: string;
  /** Resumo já carregado (`runs_list` da conversa): evita abrir para contar. */
  summary?: RunSummary | null;
  onOpenTrace?: (runId: string) => void;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [view, setView] = useState<RunView | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [plan, setPlan] = useState<TaskPlan | null>(null);

  // O plano fica sempre à vista no cartão. Buscado uma vez, e só quando o
  // resumo diz que este run teve um — runs sem plano não pagam a consulta.
  useEffect(() => {
    if (!summary?.hasPlan || plan) return;
    let cancelled = false;
    void runPlanGet(runId)
      .then((p) => {
        if (!cancelled && p) setPlan(p);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [runId, summary?.hasPlan, plan]);

  useEffect(() => {
    if (!open || view || loading) return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    void loadRunView(runId)
      .then((v) => {
        if (!cancelled) setView(v);
      })
      .catch((e) => {
        if (!cancelled) setError(errorMessage(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [open, runId, view, loading]);

  // Quantas ações: do resumo quando ele veio, da trilha depois de aberta.
  const tools =
    summary?.usage?.toolCalls ??
    (view ? Object.keys(view.tools).length : null);
  const seconds = summary?.usage?.durationMs
    ? summary.usage.durationMs / 1000
    : view?.usage
      ? view.usage.durationMs / 1000
      : null;
  const verdict = summary?.verification ?? null;
  // A causa do fim, quando o run NÃO terminou bem — "concluído" é o padrão e
  // não precisa de selo; os outros desfechos precisam ficar à vista.
  const fim = summary?.status;
  const fimLabel =
    fim === "cancelled"
      ? t("agent.run.endCancelled")
      : fim === "error"
        ? t("agent.run.endError")
        : fim === "maxSteps"
          ? t("agent.run.endMaxSteps")
          : fim === "escalated"
            ? t("agent.run.endEscalated")
            : null;

  return (
    <div className="mt-1.5 flex flex-col gap-2">
      {plan && (
        <div className="rounded-lg border border-edge bg-panel p-2.5">
          <TaskBoard plan={plan} model={summary?.model ?? null} compact />
        </div>
      )}
      <div className="flex flex-wrap items-center gap-2 text-[11px] text-dim">
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="flex items-center gap-1.5 rounded border border-edge px-2 py-1 transition-colors hover:bg-panel2 hover:text-ink"
          aria-expanded={open}
        >
          <span aria-hidden className="text-[10px]">
            {open ? "▾" : "▸"}
          </span>
          {tools != null
            ? t("agent.run.actionsCount", { count: tools })
            : t("agent.run.actionsOpen")}
        </button>
        {seconds != null && (
          <span className="tabular-nums">{formatEta(seconds)}</span>
        )}
        {fimLabel && <span className="text-warn">{fimLabel}</span>}
        {verdict && (
          <span
            className={verdict.passed ? "text-ok" : "text-bad"}
            title={verdict.notes}
          >
            {verdict.passed
              ? t("agent.run.verdictOk")
              : t("agent.run.verdictFail")}
          </span>
        )}
        {onOpenTrace && (
          <button
            type="button"
            onClick={() => onOpenTrace(runId)}
            className="underline underline-offset-2 transition-colors hover:text-ink"
          >
            {t("agent.run.showTrace")}
          </button>
        )}
      </div>
      {verdict && !verdict.passed && verdict.notes && (
        <p className="text-[11px] text-bad">{verdict.notes}</p>
      )}

      {open && (
        <div className="flex flex-col gap-2.5 border-l border-edge pl-3">
          {loading && (
            <p className="text-[11px] text-dim">{t("common.loading")}</p>
          )}
          {error && <p className="text-[11px] text-bad">{error}</p>}
          {view && view.items.length > 0 && <RunItems run={view} compact />}
          {view && view.items.length === 0 && !loading && (
            <p className="text-[11px] text-dim">{t("agent.run.empty")}</p>
          )}
        </div>
      )}
    </div>
  );
}

/**
 * Painel lateral direito com a execução: passos, ferramentas, checkpoints e
 * o resumo de uso. Recebe o run corrente (`run`) ou o id de um run antigo
 * (`runId`), que é reconstruído a partir dos eventos gravados.
 */
export default function RunTimeline({
  run = null,
  runId = null,
  onClose,
}: {
  run?: RunView | null;
  runId?: string | null;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [loaded, setLoaded] = useState<RunView | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const liveId = run?.runId ?? null;

  // Run antigo: só busca quando não há run corrente para desenhar.
  useEffect(() => {
    if (liveId || !runId) {
      setLoaded(null);
      setError(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    void loadRunView(runId)
      .then((view) => {
        if (!cancelled) setLoaded(view);
      })
      .catch((e) => {
        if (!cancelled) setError(errorMessage(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [liveId, runId]);

  const view = run ?? loaded;
  const now = useNow(view != null && isRunActive(view.status));
  // Quadro do plano só para consulta: aprovar/refazer moram no fluxo do chat.
  const planning =
    view != null &&
    view.plan == null &&
    isRunActive(view.status) &&
    plansFirst(view.workMode);
  const showBoard = view != null && (view.plan != null || planning);

  return (
    <aside className="flex min-h-0 w-80 shrink-0 flex-col overflow-hidden border-l border-edge bg-panel">
      <div className="flex items-center gap-2 border-b border-edge px-3 py-2">
        <span className="min-w-0 flex-1 truncate text-[11px] font-medium tracking-wide text-dim uppercase">
          {t("agent.run.trace")}
        </span>
        <button
          type="button"
          onClick={onClose}
          title={t("common.close")}
          className="flex h-5 w-5 items-center justify-center rounded text-dim hover:text-ink"
        >
          ▸
        </button>
      </div>

      {view && (
        <div className="flex flex-col gap-1 border-b border-edge px-3 py-2">
          <div className="flex items-center gap-2 text-[12px]">
            <span className={statusClass(view)}>{statusLabel(t, view)}</span>
            {isRunActive(view.status) && (
              <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-accent" />
            )}
            <span className="ml-auto truncate text-[11px] text-dim">
              {t(`agent.mode.${view.mode}`)}
            </span>
          </div>
          <div className="flex items-center gap-2 text-[11px] text-dim">
            <span className="tabular-nums">{usageLabel(t, view, now)}</span>
            <span className="ml-auto truncate">
              {t(`plan.mode.${view.workMode}`)}
            </span>
          </div>
          {view.model && (
            <div className="truncate text-[11px] text-dim" title={view.model}>
              {view.model}
            </div>
          )}
        </div>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto">
        {loading && (
          <p className="px-3 py-4 text-[11px] text-dim">{t("common.loading")}</p>
        )}
        {error && <p className="px-3 py-4 text-[11px] text-bad">{error}</p>}
        {showBoard && view && (
          <div className="border-b border-edge">
            <TaskBoard plan={view.plan} model={view.model} planning={planning} compact />
          </div>
        )}
        {!loading && !error && !showBoard && (!view || view.items.length === 0) && (
          <p className="px-3 py-6 text-center text-[11px] text-dim">
            {t("agent.run.empty")}
          </p>
        )}
        {view && view.items.length > 0 && (
          <div className="flex flex-col gap-2.5 px-3 py-3">
            <RunItems run={view} compact />
          </div>
        )}
      </div>

      {view && view.summary && !isRunActive(view.status) && (
        <p className="border-t border-edge px-3 py-2 text-[11px] leading-relaxed text-dim select-text">
          {view.summary}
        </p>
      )}
    </aside>
  );
}
