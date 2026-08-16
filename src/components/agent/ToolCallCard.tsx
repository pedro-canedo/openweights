// Card de uma chamada de ferramenta dentro do fluxo do chat: ícone por
// categoria, nome amigável, status e (ao expandir) argumentos, prévia e
// resultado. A saída de terminal chega em streaming e rola sozinha.

import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ToolCallView } from "../../lib/agent/runStore";
import DiffView from "./DiffView";
import { ToolIcon, categoryClass, prettyArgs, tierClass, toolLabel } from "./toolMeta";

/** Uma linha curta que resume o alvo da ferramenta (caminho, comando...). */
export function toolTarget(call: ToolCallView): string {
  const preview = call.preview;
  if (preview) {
    if (preview.kind === "diff") return preview.path;
    if (preview.kind === "command") return preview.display;
    return preview.body.split("\n")[0] ?? "";
  }
  if (!call.argsJson.trim()) return "";
  try {
    const args = JSON.parse(call.argsJson) as Record<string, unknown>;
    for (const key of ["path", "command", "pattern", "query", "dir"]) {
      const value = args[key];
      if (typeof value === "string" && value) return value;
    }
  } catch {
    // args inválidos (comum em modelos pequenos) — sem alvo para mostrar
  }
  return "";
}

function StatusPill({ call, stale }: { call: ToolCallView; stale: boolean }) {
  const { t } = useTranslation();
  const { state } = call;
  // Run encerrado antes desta chamada resolver (cancelamento, limite de
  // passos): "Aguardando confirmação" piscando para sempre seria mentira —
  // a chamada simplesmente não teve desfecho.
  if (stale && (state === "waiting" || state === "running")) {
    return <span className="shrink-0 text-[11px] text-dim">—</span>;
  }
  if (state === "waiting") {
    return (
      <span className="flex shrink-0 items-center gap-1 text-[11px] text-warn">
        <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-warn" />
        {t("agent.tool.waiting")}
      </span>
    );
  }
  if (state === "denied") {
    return (
      <span className="shrink-0 text-[11px] text-dim">
        {t("agent.tool.denied")}
      </span>
    );
  }
  if (state === "failed") {
    return (
      <span className="shrink-0 text-[11px] text-bad">
        {t("agent.tool.failed")}
      </span>
    );
  }
  if (state === "ok") {
    return (
      <span className="shrink-0 text-[11px] text-ok">{t("agent.tool.ok")}</span>
    );
  }
  return (
    <span className="flex shrink-0 items-center gap-1 text-[11px] text-dim">
      <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-accent" />
      {t("agent.tool.running")}
    </span>
  );
}

export default function ToolCallCard({
  call,
  defaultOpen = false,
  stale = false,
}: {
  call: ToolCallView;
  defaultOpen?: boolean;
  /** O run já terminou: chamadas sem desfecho param de "pulsar". */
  stale?: boolean;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(defaultOpen);
  const [argsOpen, setArgsOpen] = useState(false);
  const outRef = useRef<HTMLPreElement>(null);

  // Saída em streaming: gruda no fim enquanto o comando roda.
  useEffect(() => {
    if (call.state !== "running") return;
    const el = outRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [call.output, call.state]);

  const target = toolTarget(call);
  const args = prettyArgs(call.argsJson);
  const showOutput = call.output.length > 0;
  const showResult = call.resultPreview.length > 0 && !showOutput;

  return (
    <div
      className={`overflow-hidden rounded-xl border bg-panel/50 ${tierClass(call.tier)}`}
    >
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-panel2/60"
      >
        <span className={`shrink-0 ${categoryClass(call.category)}`}>
          <ToolIcon category={call.category} />
        </span>
        <span className="shrink-0 text-[12px] font-medium text-ink">
          {toolLabel(t, call.tool)}
        </span>
        {target && (
          <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-dim">
            {target}
          </span>
        )}
        <span className="ml-auto flex shrink-0 items-center gap-2">
          {call.durationMs != null && (
            <span className="text-[11px] tabular-nums text-dim">
              {t("agent.tool.duration", { ms: call.durationMs })}
            </span>
          )}
          <StatusPill call={call} stale={stale} />
          <svg
            className={`h-3 w-3 text-dim transition-transform ${open ? "rotate-90" : ""}`}
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            viewBox="0 0 24 24"
          >
            <path d="M9 18l6-6-6-6" />
          </svg>
        </span>
      </button>

      {open && (
        <div className="flex flex-col gap-2 border-t border-edge px-3 py-2.5">
          {call.preview?.kind === "diff" && (
            <DiffView unified={call.preview.unified} />
          )}

          {call.preview?.kind === "command" && (
            <div className="overflow-x-auto rounded-lg border border-edge bg-bg px-2.5 py-1.5">
              <code className="font-mono text-[11px] whitespace-pre text-ink select-text">
                {call.preview.display}
              </code>
              <div className="mt-0.5 truncate font-mono text-[10px] text-dim">
                {t("agent.approval.commandIn", { dir: call.preview.cwd })}
              </div>
            </div>
          )}

          {call.preview?.kind === "text" && (
            <p className="text-[11px] leading-relaxed whitespace-pre-wrap text-dim select-text">
              {call.preview.body}
            </p>
          )}

          {args && (
            <div>
              <button
                type="button"
                onClick={() => setArgsOpen((v) => !v)}
                className="text-[11px] text-dim transition-colors hover:text-ink"
              >
                {argsOpen
                  ? t("agent.approval.hideArgs")
                  : t("agent.approval.showArgs")}
              </button>
              {argsOpen && (
                <div className="mt-1 max-h-48 overflow-auto rounded-lg border border-edge bg-bg">
                  <pre className="min-w-max px-2.5 py-1.5 font-mono text-[11px] leading-relaxed text-dim select-text">
                    {args}
                  </pre>
                </div>
              )}
            </div>
          )}

          {call.denyReason && (
            <p className="text-[11px] text-dim select-text">
              {t("agent.tool.denied")}: {call.denyReason}
            </p>
          )}

          {showOutput && (
            <div>
              <div className="mb-1 text-[11px] text-dim">
                {t("agent.tool.output")}
              </div>
              <pre
                ref={outRef}
                className="max-h-56 overflow-auto rounded-lg border border-edge bg-bg px-2.5 py-1.5 font-mono text-[11px] leading-[1.5] whitespace-pre text-ink select-text"
              >
                {call.output}
              </pre>
              {call.outputTruncated && (
                <p className="mt-1 text-[11px] text-warn">
                  {t("agent.tool.truncated")}
                </p>
              )}
            </div>
          )}

          {showResult && (
            <div>
              <div className="mb-1 text-[11px] text-dim">
                {t("agent.tool.result")}
              </div>
              <pre className="max-h-48 overflow-auto rounded-lg border border-edge bg-bg px-2.5 py-1.5 font-mono text-[11px] leading-[1.5] whitespace-pre-wrap text-dim select-text">
                {call.resultPreview}
              </pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
