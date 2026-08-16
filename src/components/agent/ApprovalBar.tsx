// Barra de confirmação inline, logo acima do composer: o agente parou e
// espera uma decisão. Mostra a prévia exata do que vai acontecer (diff,
// comando + pasta ou texto) e os avisos que não podem passar batido —
// comando não analisável e ação fora da pasta do projeto.
//
// Atalhos: Enter permite, Esc nega (não valem enquanto o campo "negar e
// explicar" está aberto, onde Enter envia o motivo).

import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ToolCallView } from "../../lib/agent/runStore";
import type { ApprovalDecision, PolicyScope } from "../../lib/agent/types";
import DiffView from "./DiffView";
import { ToolIcon, categoryClass, isOutsideWorkspace, prettyArgs, toolLabel } from "./toolMeta";

export default function ApprovalBar({
  call,
  workspaceDir,
  onDecide,
}: {
  call: ToolCallView;
  workspaceDir: string | null;
  onDecide: (decision: ApprovalDecision) => void;
}) {
  const { t } = useTranslation();
  const [reasonOpen, setReasonOpen] = useState(false);
  const [reason, setReason] = useState("");
  const [argsOpen, setArgsOpen] = useState(false);
  const [scope, setScope] = useState<PolicyScope>(
    workspaceDir ? "workspace" : "global",
  );
  const reasonRef = useRef<HTMLTextAreaElement>(null);

  // Cada nova solicitação recomeça do zero (a barra é reaproveitada).
  useEffect(() => {
    setReasonOpen(false);
    setReason("");
    setArgsOpen(false);
    setScope(workspaceDir ? "workspace" : "global");
  }, [call.callId, workspaceDir]);

  useEffect(() => {
    if (reasonOpen) reasonRef.current?.focus();
  }, [reasonOpen]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (reasonOpen) return;
      const el = e.target as HTMLElement | null;
      const typing =
        el instanceof HTMLTextAreaElement || el instanceof HTMLInputElement;
      if (e.key === "Enter" && !e.shiftKey && !typing) {
        e.preventDefault();
        onDecide({ kind: "allowOnce" });
      }
      if (e.key === "Escape") {
        e.preventDefault();
        onDecide({ kind: "deny", reason: null });
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [reasonOpen, onDecide]);

  const preview = call.preview;
  const opaque = preview?.kind === "command" && preview.class === "opaque";
  const targetPath =
    preview?.kind === "diff"
      ? preview.path
      : preview?.kind === "command"
        ? preview.cwd
        : null;
  const outside = isOutsideWorkspace(targetPath, workspaceDir);
  const args = prettyArgs(call.argsJson);

  const sendReason = () => {
    onDecide({ kind: "deny", reason: reason.trim() || null });
  };

  return (
    <div className="mx-auto mb-2 max-w-2xl overflow-hidden rounded-2xl border border-warn/40 bg-warn/5">
      <div className="flex items-center gap-2 px-3 pt-2.5 pb-1.5">
        <span className={`shrink-0 ${categoryClass(call.category)}`}>
          <ToolIcon category={call.category} className="h-4 w-4" />
        </span>
        <span className="min-w-0 flex-1 truncate text-[13px] font-medium text-ink">
          {t("agent.approval.title", { tool: toolLabel(t, call.tool) })}
        </span>
        <span className="shrink-0 text-[11px] text-dim">
          {t(`agent.tool.category.${call.category}`)}
        </span>
      </div>

      <div className="flex flex-col gap-2 px-3 pb-2">
        {preview?.kind === "diff" && (
          <>
            <div className="truncate font-mono text-[11px] text-dim">
              {preview.path}
            </div>
            <DiffView unified={preview.unified} maxLines={14} />
          </>
        )}

        {preview?.kind === "command" && (
          <div className="overflow-x-auto rounded-lg border border-edge bg-bg px-2.5 py-1.5">
            <code className="font-mono text-[12px] whitespace-pre text-ink select-text">
              {preview.display}
            </code>
            <div className="mt-0.5 truncate font-mono text-[10px] text-dim">
              {t("agent.approval.commandIn", { dir: preview.cwd })}
            </div>
          </div>
        )}

        {preview?.kind === "text" && (
          <p className="text-[12px] leading-relaxed whitespace-pre-wrap text-dim select-text">
            {preview.body}
          </p>
        )}

        {opaque && (
          <p className="flex items-start gap-1.5 text-[11px] text-warn">
            <span className="mt-1 h-1.5 w-1.5 shrink-0 rounded-full bg-warn" />
            {t("agent.approval.opaqueWarning")}
          </p>
        )}
        {outside && (
          <p className="flex items-start gap-1.5 text-[11px] text-bad">
            <span className="mt-1 h-1.5 w-1.5 shrink-0 rounded-full bg-bad" />
            {t("agent.approval.outsideWorkspace")}
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
              <div className="mt-1 max-h-40 overflow-auto rounded-lg border border-edge bg-bg">
                <pre className="min-w-max px-2.5 py-1.5 font-mono text-[11px] text-dim select-text">
                  {args}
                </pre>
              </div>
            )}
          </div>
        )}

        {reasonOpen && (
          <textarea
            ref={reasonRef}
            value={reason}
            onChange={(e) => setReason(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                sendReason();
              }
              if (e.key === "Escape") {
                e.preventDefault();
                setReasonOpen(false);
              }
            }}
            rows={2}
            placeholder={t("agent.approval.denyReason")}
            className="w-full resize-none rounded-lg border border-edge bg-panel2 px-2.5 py-2 text-[12px] outline-none select-text placeholder:text-dim focus:border-accent"
          />
        )}
      </div>

      <div className="flex flex-wrap items-center gap-1.5 border-t border-warn/25 px-3 py-2">
        {reasonOpen ? (
          <>
            <button
              type="button"
              onClick={sendReason}
              className="rounded-lg bg-bad px-3 py-1.5 text-[12px] font-medium text-white"
            >
              {t("agent.approval.deny")}
            </button>
            <button
              type="button"
              onClick={() => setReasonOpen(false)}
              className="rounded-lg border border-edge px-3 py-1.5 text-[12px] text-dim transition-colors hover:text-ink"
            >
              {t("common.cancel")}
            </button>
          </>
        ) : (
          <>
            <button
              type="button"
              onClick={() => onDecide({ kind: "allowOnce" })}
              title="Enter"
              className="rounded-lg bg-accent px-3 py-1.5 text-[12px] font-medium text-white"
            >
              {t("agent.approval.allow")}
            </button>
            <button
              type="button"
              onClick={() => onDecide({ kind: "allowAlways", scope })}
              title={t(
                scope === "workspace"
                  ? "agent.approval.scopeWorkspace"
                  : "agent.approval.scopeGlobal",
              )}
              className="rounded-lg border border-edge px-3 py-1.5 text-[12px] text-ink transition-colors hover:border-accent"
            >
              {t("agent.approval.allowAlways")}
            </button>
            {workspaceDir && (
              <button
                type="button"
                onClick={() =>
                  setScope((s) => (s === "workspace" ? "global" : "workspace"))
                }
                className="rounded-full border border-edge px-2 py-1 text-[11px] text-dim transition-colors hover:text-ink"
              >
                {t(
                  scope === "workspace"
                    ? "agent.approval.scopeWorkspace"
                    : "agent.approval.scopeGlobal",
                )}
              </button>
            )}
            <span className="flex-1" />
            <button
              type="button"
              onClick={() => setReasonOpen(true)}
              className="rounded-lg border border-edge px-3 py-1.5 text-[12px] text-dim transition-colors hover:text-ink"
            >
              {t("agent.approval.denyAndTell")}
            </button>
            <button
              type="button"
              onClick={() => onDecide({ kind: "deny", reason: null })}
              title="Esc"
              className="rounded-lg border border-edge px-3 py-1.5 text-[12px] text-ink transition-colors hover:border-bad hover:text-bad"
            >
              {t("agent.approval.deny")}
            </button>
          </>
        )}
      </div>
    </div>
  );
}
