// Nível de autorização do agente nesta conversa: pedir sempre, pedir só
// para alterações (padrão) ou automático (YOLO).
//
// YOLO é o único que muda o contrato de confiança, então exige duas coisas:
// uma pasta de projeto escolhida (é o limite do "automático") e uma
// confirmação explícita do usuário.

import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { RunMode } from "../../lib/agent/types";
import type { ChatParams } from "../../lib/types";

const MODES: Exclude<RunMode, "chat">[] = ["approve", "smart", "yolo"];

function ModeIcon({ mode }: { mode: RunMode }) {
  const path =
    mode === "approve"
      ? "M9 12l2 2 4-4M12 3l7 4v5c0 4.5-3 8-7 9-4-1-7-4.5-7-9V7z"
      : mode === "smart"
        ? "M12 3l1.9 4.6 4.6 1.9-4.6 1.9L12 16l-1.9-4.6L5.5 9.5l4.6-1.9zM18 15l.9 2.1 2.1.9-2.1.9L18 21l-.9-2.1-2.1-.9 2.1-.9z"
        : "M13 2L4.5 13.5H11l-1 8.5 8.5-11.5H12z";
  return (
    <svg
      className="h-4 w-4 shrink-0"
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

function Check() {
  return (
    <svg
      className="h-4 w-4 shrink-0 text-sky-400"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.4"
      strokeLinecap="round"
      strokeLinejoin="round"
      viewBox="0 0 24 24"
    >
      <path d="M20 6L9 17l-5-5" />
    </svg>
  );
}

export default function ModeSelect({
  params,
  onChange,
  disabled = false,
}: {
  params: ChatParams;
  onChange: (p: ChatParams) => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [confirmYolo, setConfirmYolo] = useState(false);
  const [needWorkspace, setNeedWorkspace] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  const mode: RunMode = params.mode ?? "smart";

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const pick = (next: RunMode) => {
    if (next === "yolo") {
      if (!params.workspaceDir) {
        setNeedWorkspace(true);
        return;
      }
      setNeedWorkspace(false);
      setConfirmYolo(true);
      return;
    }
    setNeedWorkspace(false);
    onChange({ ...params, mode: next });
    setOpen(false);
  };

  return (
    <div ref={ref} className="relative shrink-0">
      <button
        type="button"
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
        title={`${t("agent.mode.label")}: ${t(`agent.mode.${mode}`)}`}
        className={`flex items-center gap-1 rounded-full px-2 py-1 text-xs transition-colors disabled:opacity-40 hover:bg-panel ${
          mode === "yolo" ? "text-warn" : open ? "bg-panel text-ink" : "text-ink"
        }`}
      >
        <ModeIcon mode={mode} />
        <span className="max-w-32 truncate">{t(`agent.mode.${mode}`)}</span>
        <svg
          className="h-3 w-3 text-dim"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          viewBox="0 0 24 24"
        >
          <path d="M6 9l6 6 6-6" />
        </svg>
      </button>

      {open && (
        <div className="absolute bottom-full left-0 z-30 mb-2 w-80 rounded-xl border border-edge bg-panel py-1.5 shadow-[0_12px_40px_rgba(0,0,0,0.45)]">
          <p className="px-3 pt-1 pb-1.5 text-[11px] tracking-wide text-dim uppercase">
            {t("agent.mode.label")}
          </p>
          {MODES.map((option) => (
            <button
              key={option}
              type="button"
              onClick={() => pick(option)}
              className="flex w-full items-start gap-2.5 px-3 py-2.5 text-left hover:bg-panel2"
            >
              <span
                className={`mt-0.5 ${option === "yolo" ? "text-warn" : "text-dim"}`}
              >
                <ModeIcon mode={option} />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block text-[13px] text-ink">
                  {t(`agent.mode.${option}`)}
                </span>
                <span className="mt-0.5 block text-[11px] leading-relaxed text-dim">
                  {t(`agent.mode.${option}Hint`)}
                </span>
              </span>
              {mode === option && <Check />}
            </button>
          ))}
          {needWorkspace && (
            <p className="border-t border-edge px-3 py-2 text-[11px] text-warn">
              {t("agent.yolo.needWorkspace")}
            </p>
          )}
        </div>
      )}

      {confirmYolo && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6"
          onMouseDown={(e) => {
            if (e.target === e.currentTarget) setConfirmYolo(false);
          }}
        >
          <div className="w-full max-w-md rounded-2xl border border-warn/40 bg-panel p-5 shadow-[0_24px_80px_rgba(0,0,0,0.55)]">
            <h2 className="flex items-center gap-2 text-sm font-semibold text-ink">
              <span className="rounded-full bg-warn/15 px-2 py-0.5 text-[11px] font-semibold text-warn">
                {t("agent.yolo.badge")}
              </span>
              {t("agent.yolo.confirmTitle")}
            </h2>
            <p className="mt-2.5 text-[12px] leading-relaxed text-dim select-text">
              {t("agent.yolo.confirmBody")}
            </p>
            {params.workspaceDir && (
              <p className="mt-2 truncate font-mono text-[11px] text-dim">
                {params.workspaceDir}
              </p>
            )}
            <div className="mt-4 flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setConfirmYolo(false)}
                className="rounded-lg border border-edge px-3 py-1.5 text-[12px] text-dim transition-colors hover:text-ink"
              >
                {t("common.cancel")}
              </button>
              <button
                type="button"
                onClick={() => {
                  onChange({ ...params, mode: "yolo" });
                  setConfirmYolo(false);
                  setOpen(false);
                }}
                className="rounded-lg bg-warn px-3 py-1.5 text-[12px] font-medium text-black"
              >
                {t("agent.yolo.confirmOk")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
