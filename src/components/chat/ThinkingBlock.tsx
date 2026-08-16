// Bloco colapsável de raciocínio ("thinking") exibido antes do conteúdo
// da mensagem do assistente. Durante o streaming fica aberto com auto-scroll;
// ao concluir, colapsa mostrando o tempo pensado (expansível pelo usuário).

import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

export default function ThinkingBlock({
  reasoning,
  thinkingMs,
  active,
}: {
  reasoning: string;
  thinkingMs?: number | null;
  /** true enquanto o raciocínio ainda está sendo transmitido. */
  active: boolean;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const boxRef = useRef<HTMLDivElement>(null);

  // Auto-scroll para o fim enquanto o raciocínio chega.
  useEffect(() => {
    if (!active) return;
    const el = boxRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [reasoning, active]);

  const expanded = active || open;
  const seconds = ((thinkingMs ?? 0) / 1000).toFixed(1);
  const label = active
    ? t("chat.thinkingTimer", { seconds })
    : thinkingMs != null
      ? t("chat.thought", { seconds })
      : t("chat.thoughtNoTime");

  return (
    <div className="mb-2 rounded-lg border border-edge bg-panel/60">
      <button
        onClick={() => {
          if (!active) setOpen((o) => !o);
        }}
        className="flex w-full items-center gap-1.5 px-3 py-1.5 text-left text-xs text-dim transition-colors hover:text-ink"
      >
        <svg
          className={`h-3 w-3 shrink-0 transition-transform ${expanded ? "rotate-90" : ""}`}
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          viewBox="0 0 24 24"
        >
          <path d="M9 18l6-6-6-6" />
        </svg>
        {active && (
          <span className="h-1.5 w-1.5 shrink-0 animate-pulse rounded-full bg-accent" />
        )}
        <span>{label}</span>
      </button>
      {expanded && reasoning && (
        <div
          ref={boxRef}
          className="max-h-44 overflow-y-auto border-t border-edge px-3 py-2 text-xs leading-relaxed whitespace-pre-wrap text-dim select-text"
        >
          {reasoning}
        </div>
      )}
    </div>
  );
}
