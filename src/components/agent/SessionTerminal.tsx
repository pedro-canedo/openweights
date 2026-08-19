// Terminal da sessão: tudo que o agente executou nesta conversa, num lugar só.
//
// Antes, a saída de um comando aparecia dentro do card da chamada, e só
// quando o comando terminava — um `cargo test` de três minutos ficava mudo e
// depois despejava tudo de uma vez, espalhado pela trilha. Aqui os comandos
// aparecem em sequência, com a saída chegando enquanto rodam.
//
// É uma projeção pura do `RunView`: não há store próprio nem comando de
// backend. Quem alimenta é o `runStore` (ao vivo, pelo evento `tool.output`)
// ou o `loadRunView`, que repõe as saídas gravadas de um run antigo.
//
// Uma diferença esperada entre os dois: ao vivo isto é o stream cru do
// processo; no replay é o corpo formatado que foi ao banco ("exit code N",
// seções [stdout]/[stderr]), porque os pedaços em streaming de propósito não
// são persistidos.
//
// Nota: a saída aparece como o comando a escreveu. Um comando que ecoa
// segredo o mostra aqui — como já mostrava no card da chamada.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import {
  isRunActive,
  loadRunView,
  type RunView,
  type ToolCallView,
} from "../../lib/agent/runStore";
import { errorMessage } from "../../lib/serverSession";
import { toolLabel } from "./toolMeta";

/** Distância do fim, em px, dentro da qual ainda contamos como "no fim". */
const STICK_THRESHOLD = 24;

/**
 * As chamadas que este painel mostra: as que rodaram um comando.
 *
 * O `preview` de comando é o critério principal (é ele que traz programa e
 * pasta). A saída não vazia entra junto para não perder a ferramenta que
 * executou algo sem preview — Code Mode, por exemplo.
 */
function commandCalls(run: RunView): ToolCallView[] {
  const out: ToolCallView[] = [];
  for (const item of run.items) {
    if (item.kind !== "tool") continue;
    const call = run.tools[item.id];
    if (!call) continue;
    const isCommand = call.preview?.kind === "command";
    const ran = call.category === "execute" && call.output.length > 0;
    if (isCommand || ran) out.push(call);
  }
  return out;
}

function stateLabel(t: TFunction, call: ToolCallView): string {
  switch (call.state) {
    case "running":
      return t("agent.tool.running");
    case "ok":
      return t("agent.tool.ok");
    case "failed":
      return t("agent.tool.failed");
    case "denied":
      return t("agent.tool.denied");
    default:
      return t("agent.tool.waiting");
  }
}

function stateClass(call: ToolCallView): string {
  if (call.state === "running") return "text-accent";
  if (call.state === "ok") return "text-ok";
  if (call.state === "failed" || call.state === "denied") return "text-bad";
  return "text-dim";
}

/** Um comando: cabeçalho com a linha e a pasta, corpo com a saída. */
function CommandBlock({ call }: { call: ToolCallView }) {
  const { t } = useTranslation();
  const preview = call.preview?.kind === "command" ? call.preview : null;
  const line = preview?.display ?? toolLabel(t, call.tool);

  return (
    <div className="border-b border-edge/60 last:border-b-0">
      <div className="flex items-baseline gap-2 px-3 pt-2 font-mono text-[11px]">
        <span className="text-accent">$</span>
        <span className="min-w-0 flex-1 break-all text-ink select-text">
          {line}
        </span>
      </div>
      {preview?.cwd && (
        <div className="truncate px-3 pt-0.5 pl-6 font-mono text-[10px] text-dim select-text">
          {preview.cwd}
        </div>
      )}
      {call.output && (
        <pre className="mt-1 overflow-x-auto px-3 pb-1 pl-6 font-mono text-[11px] leading-[1.5] whitespace-pre text-dim select-text">
          {call.output}
        </pre>
      )}
      <div className="flex items-center gap-2 px-3 pb-2 pl-6 text-[10px]">
        <span className={stateClass(call)}>{stateLabel(t, call)}</span>
        {call.durationMs != null && (
          <span className="text-dim tabular-nums">
            {t("agent.tool.duration", { ms: call.durationMs })}
          </span>
        )}
        {call.outputTruncated && (
          <span className="text-warn">{t("agent.tool.truncated")}</span>
        )}
      </div>
    </div>
  );
}

/**
 * Painel da coluna direita. Recebe o run corrente (`run`) ou o identificador
 * de um run antigo (`runId`) — o mesmo contrato do `RunTimeline`.
 */
export default function SessionTerminal({
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
  const [error, setError] = useState<string | null>(null);
  const liveId = run?.runId ?? null;

  useEffect(() => {
    if (liveId || !runId) {
      setLoaded(null);
      setError(null);
      return;
    }
    let cancelled = false;
    setError(null);
    void loadRunView(runId)
      .then((view) => {
        if (!cancelled) setLoaded(view);
      })
      .catch((e) => {
        if (!cancelled) setError(errorMessage(e));
      });
    return () => {
      cancelled = true;
    };
  }, [liveId, runId]);

  const view = run ?? loaded;
  const calls = view ? commandCalls(view) : [];
  const bodyRef = useRef<HTMLDivElement>(null);
  const stuck = useRef(true);

  // Gruda no fim só se a pessoa já estava no fim: rolar para trás durante um
  // build e ser puxado de volta a cada chunk é insuportável.
  const total = calls.reduce((n, c) => n + c.output.length, 0);
  useLayoutEffect(() => {
    const el = bodyRef.current;
    if (el && stuck.current) el.scrollTop = el.scrollHeight;
  }, [total, calls.length]);

  const onScroll = () => {
    const el = bodyRef.current;
    if (!el) return;
    stuck.current =
      el.scrollHeight - el.scrollTop - el.clientHeight < STICK_THRESHOLD;
  };

  return (
    <aside className="flex min-h-0 w-80 shrink-0 flex-col overflow-hidden border-l border-edge bg-panel">
      <div className="flex items-center gap-2 border-b border-edge px-3 py-2">
        <span className="min-w-0 flex-1 truncate text-[11px] font-medium tracking-wide text-dim uppercase">
          {t("agent.terminal.title")}
        </span>
        {view != null && isRunActive(view.status) && (
          <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-accent" />
        )}
        <button
          type="button"
          onClick={onClose}
          title={t("common.close")}
          className="flex h-5 w-5 items-center justify-center rounded text-dim hover:text-ink"
        >
          ▸
        </button>
      </div>

      <div
        ref={bodyRef}
        onScroll={onScroll}
        className="min-h-0 flex-1 overflow-y-auto"
      >
        {error && <p className="px-3 py-2 text-[11px] text-bad">{error}</p>}
        {!error && calls.length === 0 && (
          <p className="px-3 py-2 text-[11px] text-dim">
            {t("agent.terminal.empty")}
          </p>
        )}
        {calls.map((call) => (
          <CommandBlock key={call.callId} call={call} />
        ))}
      </div>
    </aside>
  );
}
