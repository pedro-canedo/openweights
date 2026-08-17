// Anel de uso da janela de contexto no compositor (estilo Cursor) +
// popover com o detalhe por categoria. A contagem é estimada (~4 chars /
// token) até o llama-server expor tokenize; o teto vem de GET /props.

import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  engineBusyReason,
  getModelProfile,
  getServerProps,
  getServerStatus,
  restartServer,
  setModelCtx,
} from "../../lib/api";
import { describesModel } from "../../lib/agent/types";
import type { Attachment } from "./AttachmentChips";
import type { UiMessage } from "./MessageList";

const CTX_CHIPS = [8192, 16384, 32768, 65536];
const CTX_MIN = 512;
const CTX_MAX = 262_144;
const IMAGE_TOKENS = 256;
const RING_R = 7;
const RING_C = 2 * Math.PI * RING_R;

type Bucket = {
  key: string;
  color: string;
  tokens: number;
};

function estimateTokens(text: string): number {
  if (!text) return 0;
  return Math.max(0, Math.round(text.length / 4));
}

function formatTok(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1000) return `${(n / 1000).toFixed(1)}K`;
  return String(Math.round(n));
}

function formatChip(n: number): string {
  return n % 1024 === 0 ? `${n / 1024}k` : String(n);
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

export default function ContextMeter({
  model,
  messages,
  draft,
  attachments,
  systemPrompt,
  generating = false,
}: {
  model: string;
  messages: UiMessage[];
  draft: string;
  attachments: Attachment[];
  systemPrompt: string;
  generating?: boolean;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [limit, setLimit] = useState<number | null>(null);
  const [saved, setSaved] = useState<number | null>(null);
  const [applying, setApplying] = useState(false);
  /// Quem estava usando o motor quando o reinício foi recusado.
  const [busyWith, setBusyWith] = useState<string[]>([]);
  const ref = useDismiss(open, () => setOpen(false));

  const refreshLimit = async () => {
    const [props, profile] = await Promise.all([
      getServerProps(model).catch(() => null),
      getModelProfile(model).catch(() => null),
    ]);
    const configured =
      typeof profile?.ctx === "number" && profile.ctx > 0 ? profile.ctx : null;
    setSaved(configured);
    // `nCtx` do roteador é 0 e não descreve janela nenhuma.
    setLimit((describesModel(props) ? props.nCtx : null) ?? configured);
  };

  useEffect(() => {
    void refreshLimit();
  }, [model]);

  useEffect(() => {
    if (open) void refreshLimit();
  }, [open, model]);

  const buckets = useMemo<Bucket[]>(() => {
    let conversation = 0;
    let reasoning = 0;
    for (const m of messages) {
      conversation += estimateTokens(m.content);
      if (m.images) conversation += m.images.length * IMAGE_TOKENS;
      reasoning += estimateTokens(m.reasoning ?? "");
    }
    let attached = 0;
    for (const a of attachments) {
      attached +=
        a.kind === "image" ? IMAGE_TOKENS : estimateTokens(a.data);
    }
    return [
      {
        key: "system",
        color: "#8b909a",
        tokens: estimateTokens(systemPrompt),
      },
      { key: "conversation", color: "#e8a87c", tokens: conversation },
      { key: "reasoning", color: "#7dcea0", tokens: reasoning },
      { key: "draft", color: "#7eb8da", tokens: estimateTokens(draft) },
      { key: "attachments", color: "#c4a7e7", tokens: attached },
    ].filter((b) => b.tokens > 0);
  }, [messages, draft, attachments, systemPrompt]);

  const used = buckets.reduce((s, b) => s + b.tokens, 0);
  const pct = limit && limit > 0 ? Math.min(1, used / limit) : 0;
  const pctLabel = Math.round(pct * 100);
  const ringClass =
    pct >= 0.9 ? "text-bad" : pct >= 0.7 ? "text-warn" : "text-ink";

  const apply = async (next: number | null) => {
    if (!model || applying) return;
    const value =
      next == null ? null : Math.round(Math.min(CTX_MAX, Math.max(CTX_MIN, next)));
    // Reiniciar o motor recarrega o modelo — dezenas de segundos num 27B.
    // Clicar de novo no valor que já está gravado não pode custar isso.
    if (value === saved) return;
    setApplying(true);
    setBusyWith([]);
    try {
      const stored = await setModelCtx(model, value);
      setSaved(stored);
      const status = await getServerStatus().catch(() => null);
      if (status?.running && !generating) {
        // A guarda de quem está usando o motor mora no backend: aqui só
        // repassamos o motivo quando ele recusa.
        try {
          await restartServer();
        } catch (e) {
          const quem = engineBusyReason(e);
          if (!quem) throw e;
          setBusyWith(quem);
        }
      }
      await refreshLimit();
    } catch (e) {
      console.error("falha ao gravar janela de contexto:", e);
    } finally {
      setApplying(false);
    }
  };

  return (
    <div ref={ref} className="relative shrink-0">
      <button
        type="button"
        disabled={!model}
        onClick={() => setOpen((v) => !v)}
        title={
          limit
            ? t("chat.ctx.usage.ringTitle", {
                pct: pctLabel,
                used: formatTok(used),
                limit: formatTok(limit),
              })
            : t("chat.ctx.usage.title")
        }
        className={`flex h-8 w-8 items-center justify-center rounded-full transition-colors disabled:opacity-40 hover:bg-panel ${
          open ? "bg-panel" : ""
        }`}
      >
        <svg
          viewBox="0 0 20 20"
          className={`h-[18px] w-[18px] ${ringClass}`}
          aria-hidden
        >
          <circle
            cx="10"
            cy="10"
            r={RING_R}
            fill="none"
            className="text-edge"
            stroke="currentColor"
            strokeWidth="2.4"
          />
          <circle
            cx="10"
            cy="10"
            r={RING_R}
            fill="none"
            stroke="currentColor"
            strokeWidth="2.4"
            strokeLinecap="round"
            strokeDasharray={RING_C}
            strokeDashoffset={RING_C * (1 - pct)}
            transform="rotate(-90 10 10)"
          />
        </svg>
      </button>

      {open && (
        <div className="absolute right-0 bottom-full z-40 mb-2 w-80 rounded-xl border border-edge bg-panel p-3.5 shadow-[0_12px_40px_rgba(0,0,0,0.45)]">
          <div className="flex items-start justify-between gap-2">
            <div className="text-[13px] font-medium text-ink">
              {t("chat.ctx.usage.title")}
            </div>
            <button
              type="button"
              onClick={() => setOpen(false)}
              className="rounded p-0.5 text-dim hover:text-ink"
              aria-label={t("common.close")}
            >
              <svg
                className="h-3.5 w-3.5"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                viewBox="0 0 24 24"
              >
                <path d="M6 6l12 12M18 6L6 18" />
              </svg>
            </button>
          </div>

          <div className="mt-1 flex items-baseline justify-between gap-2 text-[12px]">
            <span className={pct >= 0.9 ? "text-bad" : "text-ink"}>
              {limit
                ? t("chat.ctx.usage.full", { pct: pctLabel })
                : t("chat.ctx.notLoaded")}
            </span>
            <span className="tabular-nums text-dim">
              {t("chat.ctx.usage.tokens", {
                used: formatTok(used),
                limit: limit ? formatTok(limit) : "—",
              })}
            </span>
          </div>

          <div className="mt-2.5 flex h-2 overflow-hidden rounded-full bg-panel2">
            {used === 0 || !limit ? (
              <div className="h-full w-0" />
            ) : (
              buckets.map((b) => (
                <div
                  key={b.key}
                  className="h-full min-w-px"
                  style={{
                    width: `${(b.tokens / Math.max(limit, used)) * 100}%`,
                    background: b.color,
                  }}
                />
              ))
            )}
          </div>

          <div className="mt-3 flex flex-col gap-1.5">
            {buckets.length === 0 ? (
              <p className="text-[11px] text-dim">{t("chat.ctx.usage.empty")}</p>
            ) : (
              buckets.map((b) => (
                <div
                  key={b.key}
                  className="flex items-center gap-2 text-[12px]"
                >
                  <span
                    className="h-2.5 w-2.5 shrink-0 rounded-[3px]"
                    style={{ background: b.color }}
                  />
                  <span className="min-w-0 flex-1 truncate text-ink">
                    {t(`chat.ctx.usage.${b.key}`)}
                  </span>
                  <span className="tabular-nums text-dim">
                    {formatTok(b.tokens)}
                  </span>
                </div>
              ))
            )}
          </div>

          <div className="mt-3 border-t border-edge pt-2.5">
            <div className="mb-1.5 text-[11px] text-dim">
              {t("chat.ctx.label")}
            </div>
            <div className="flex flex-wrap gap-1">
              <button
                type="button"
                disabled={!model || applying}
                onClick={() => void apply(null)}
                className={`rounded-lg border px-2 py-1 text-[11px] disabled:opacity-40 ${
                  saved == null
                    ? "border-accent bg-accent/15 text-ink"
                    : "border-edge text-dim hover:border-accent hover:text-ink"
                }`}
              >
                {t("chat.ctx.auto")}
              </button>
              {CTX_CHIPS.map((n) => (
                <button
                  key={n}
                  type="button"
                  disabled={!model || applying || generating}
                  title={generating ? t("chat.ctx.busy") : undefined}
                  onClick={() => void apply(n)}
                  className={`rounded-lg border px-2 py-1 text-[11px] tabular-nums disabled:opacity-40 ${
                    saved === n
                      ? "border-accent bg-accent/15 text-ink"
                      : "border-edge text-dim hover:border-accent hover:text-ink"
                  }`}
                >
                  {formatChip(n)}
                </button>
              ))}
            </div>
            {applying && (
              <p className="mt-1.5 text-[11px] text-dim">
                {t("chat.ctx.applying")}
              </p>
            )}
            {/* A janela nova só vale depois que o motor reinicia, e o motor
                não reinicia por cima de quem está usando. */}
            {busyWith.length > 0 && (
              <p className="mt-1.5 text-[11px] leading-relaxed text-warn">
                {t("server.busyToApply", {
                  who: busyWith
                    .map((w) => t(`server.busyWith.${w}`))
                    .join(", "),
                })}
              </p>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
