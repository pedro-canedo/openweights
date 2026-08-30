// Menu do "+" no composer: anexar arquivos; o resto ainda fica como
// "em breve".

import { useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

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

function Chevron() {
  return (
    <svg
      className="h-3.5 w-3.5 shrink-0 text-dim"
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

function shortcutLabel(): string {
  return navigator.platform.toLowerCase().includes("mac") ? "⌘U" : "Ctrl+U";
}

export default function AttachMenu({
  disabled,
  onAttach,
}: {
  disabled?: boolean;
  onAttach: () => void;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

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

  const soon = (
    <span className="rounded-md bg-panel2 px-1.5 py-0.5 text-[10px] text-dim">
      {t("chat.plus.soon")}
    </span>
  );

  return (
    <div ref={ref} className="relative shrink-0">
      <button
        type="button"
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
        title={t("chat.plus.open")}
        className={`flex h-8 w-8 items-center justify-center rounded-full transition-colors disabled:opacity-40 ${
          open ? "bg-panel text-ink" : "text-dim hover:bg-panel hover:text-ink"
        }`}
      >
        <svg
          className="h-[18px] w-[18px]"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          strokeLinecap="round"
          strokeLinejoin="round"
          viewBox="0 0 24 24"
        >
          <path d="M12 5v14M5 12h14" />
        </svg>
      </button>

      {open && (
        <div className="absolute bottom-full left-0 z-30 mb-2 w-72 rounded-xl border border-edge bg-panel py-1.5 shadow-[0_12px_40px_rgba(0,0,0,0.45)]">
          <button
            type="button"
            onClick={() => {
              setOpen(false);
              onAttach();
            }}
            className="flex w-full items-center gap-2.5 px-3 py-2.5 text-left text-[13px] text-ink hover:bg-panel2"
          >
            <Icon>
              <path d="M21.4 11.6l-8.5 8.5a5 5 0 01-7.1-7.1l8.5-8.5a3.2 3.2 0 014.5 4.5L10.3 17.5a1.4 1.4 0 01-2-2l7.4-7.4" />
            </Icon>
            <span className="min-w-0 flex-1">{t("chat.plus.attach")}</span>
            <span className="shrink-0 text-[11px] text-dim">
              {shortcutLabel()}
            </span>
          </button>

          <button
            type="button"
            disabled
            title={t("chat.plus.soon")}
            className="flex w-full items-center gap-2.5 px-3 py-2.5 text-left text-[13px] text-dim opacity-60"
          >
            <Icon>
              <path d="M8 4h8l2 3H6l2-3z" />
              <path d="M6 7h12v11a2 2 0 01-2 2H8a2 2 0 01-2-2V7z" />
              <path d="M10 11h4M10 15h2" />
            </Icon>
            <span className="min-w-0 flex-1">{t("chat.plus.skills")}</span>
            {soon}
            <Chevron />
          </button>

          <button
            type="button"
            disabled
            title={t("chat.plus.soon")}
            className="flex w-full items-center gap-2.5 px-3 py-2.5 text-left text-[13px] text-dim opacity-60"
          >
            <Icon>
              <path d="M8 12h8" />
              <path d="M15 9l3 3-3 3" />
              <path d="M9 7V5a2 2 0 012-2h2" />
              <path d="M9 17v2a2 2 0 002 2h2" />
              <circle cx="7" cy="12" r="2" />
            </Icon>
            <span className="min-w-0 flex-1">{t("chat.plus.plugins")}</span>
            {soon}
          </button>
        </div>
      )}
    </div>
  );
}
