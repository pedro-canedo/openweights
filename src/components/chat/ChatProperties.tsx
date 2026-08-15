// Aprovação na barra esquerda do composer (esforço fica no seletor de modelo).

import { useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { type ApprovalMode, type ChatParams } from "../../lib/types";

const APPROVALS: ApprovalMode[] = ["manual", "auto", "ignore"];

function Check() {
  return (
    <svg
      className="h-4 w-4 text-sky-400"
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

function ChevronDown() {
  return (
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
  );
}

function Icon({ children }: { children: ReactNode }) {
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
      {children}
    </svg>
  );
}

function ApprovalIcon({ mode }: { mode: ApprovalMode }) {
  if (mode === "manual") {
    return (
      <Icon>
        <path d="M8 13V6a2 2 0 114 0v7" />
        <path d="M12 13V8a2 2 0 114 0v5" />
        <path d="M16 13v-2a2 2 0 114 0v6a5 5 0 01-5 5h-2.5a6 6 0 01-5.2-3L5 15a2 2 0 012-2h1" />
      </Icon>
    );
  }
  if (mode === "auto") {
    return (
      <Icon>
        <path d="M5 5l7 7-7 7M13 5l7 7-7 7" />
      </Icon>
    );
  }
  return (
    <Icon>
      <path d="M12 9v4M12 17h.01" />
      <path d="M10.3 4.7L2.8 18a2 2 0 001.7 3h15a2 2 0 001.7-3L13.7 4.7a2 2 0 00-3.4 0z" />
    </Icon>
  );
}

function MenuShell({
  children,
  widthClass = "w-72",
  align = "left",
}: {
  children: ReactNode;
  widthClass?: string;
  align?: "left" | "right";
}) {
  return (
    <div
      className={`absolute bottom-full z-30 mb-2 rounded-xl border border-edge bg-panel py-1.5 shadow-[0_12px_40px_rgba(0,0,0,0.45)] ${widthClass} ${
        align === "right" ? "right-0" : "left-0"
      }`}
    >
      {children}
    </div>
  );
}

function useDismiss(open: boolean, onClose: () => void) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, onClose]);
  return ref;
}

const triggerClass =
  "flex items-center gap-1 rounded-full px-2 py-1 text-xs transition-colors disabled:opacity-40 hover:bg-panel";

export function ApprovalSelect({
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
  const ref = useDismiss(open, () => setOpen(false));

  return (
    <div ref={ref} className="relative shrink-0">
      <button
        type="button"
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
        title={t(`chat.approval.${params.approval}`)}
        className={`${triggerClass} ${open ? "bg-panel text-ink" : "text-ink"}`}
      >
        <ApprovalIcon mode={params.approval} />
        <span>{t(`chat.approval.short.${params.approval}`)}</span>
        <ChevronDown />
      </button>
      {open && (
        <MenuShell>
          {APPROVALS.map((mode) => (
            <button
              key={mode}
              type="button"
              onClick={() => {
                onChange({ ...params, approval: mode });
                setOpen(false);
              }}
              className="flex w-full items-center gap-2.5 px-3 py-2.5 text-left text-[13px] text-ink hover:bg-panel2"
            >
              <ApprovalIcon mode={mode} />
              <span className="flex-1">{t(`chat.approval.${mode}`)}</span>
              {params.approval === mode && <Check />}
            </button>
          ))}
        </MenuShell>
      )}
    </div>
  );
}

