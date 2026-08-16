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

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import {
  isRunActive,
  loadRunView,
  type NoteKind,
  type RunView,
  type StepItem,
  type TimelineItem,
} from "../../lib/agent/runStore";
import { errorMessage } from "../../lib/serverSession";
import Markdown from "../chat/Markdown";
import ThinkingBlock from "../chat/ThinkingBlock";
import ToolCallCard from "./ToolCallCard";

/** Avisos da trilha → chave de i18n e cor. */
const NOTES: Record<NoteKind, { key: string; className: string }> = {
  compacted: { key: "agent.run.compacted", className: "text-dim" },
  verified: { key: "agent.run.verified", className: "text-ok" },
  verifyFailed: { key: "agent.run.verifyFailed", className: "text-warn" },
  error: { key: "agent.run.error", className: "text-bad" },
  elicitation: { key: "agent.mcp.elicitation", className: "text-accent" },
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

/** Relógio de 1 Hz — só enquanto o run está vivo. */
function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [active]);
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
    seconds: (durationMs(run, now) / 1000).toFixed(1),
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
  if (!step.text && !step.reasoning) return null;
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
          thinkingMs={null}
          active={live && !step.text}
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

  return (
    <>
      {run.items.map((item) => {
        if (item.kind === "step") {
          return (
            <StepBlock
              key={item.id}
              step={item}
              live={active && item.id === lastId}
              compact={compact}
            />
          );
        }
        if (item.kind === "tool") {
          const call = run.tools[item.id];
          if (!call) return null;
          return <ToolCallCard key={item.id} call={call} />;
        }
        if (item.kind === "checkpoint") {
          return (
            <CheckpointLine
              key={item.id}
              label={item.label}
              backend={item.backend}
              tsMs={item.tsMs}
            />
          );
        }
        const note = NOTES[item.note];
        return (
          <p
            key={item.id}
            className={`text-[11px] leading-relaxed select-text ${note.className}`}
          >
            {t(note.key)}
            {item.detail ? ` — ${item.detail}` : ""}
          </p>
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

  return (
    <aside className="flex w-80 shrink-0 flex-col border-l border-edge bg-panel">
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
          <div className="text-[11px] tabular-nums text-dim">
            {usageLabel(t, view, now)}
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
        {!loading && !error && (!view || view.items.length === 0) && (
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
