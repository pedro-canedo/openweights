// Lista de mensagens do chat com auto-scroll para o fim durante o
// streaming (só "gruda" no fundo se o usuário já estiver perto dele).
// Cada mensagem tem uma barra de ações discreta no hover (copiar,
// regenerar, editar-e-reenviar, apagar) e as respostas do assistente
// exibem raciocínio (ThinkingBlock) e métricas de geração.

import { useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import Markdown from "./Markdown";
import ThinkingBlock from "./ThinkingBlock";

export interface UiMessage {
  role: "user" | "assistant";
  content: string;
  tokensPerSec: number | null;
  /** id da linha no DB quando a mensagem está persistida. */
  rowId?: number;
  /** Raciocínio do modelo (modelos thinking). */
  reasoning?: string;
  thinkingMs?: number | null;
  genTokens?: number | null;
  genMs?: number | null;
  startedAt?: number | null;
  thinkStartedAt?: number | null;
  answerStartedAt?: number | null;
  /** Imagens anexadas nesta sessão (enviadas como image_url à API). */
  images?: { name: string; dataUrl: string }[];
  error?: boolean;
}

function ActionButton({
  title,
  onClick,
  danger = false,
  children,
}: {
  title: string;
  onClick: () => void;
  danger?: boolean;
  children: ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      title={title}
      className={`flex h-6 w-6 items-center justify-center rounded text-dim transition-colors ${
        danger ? "hover:bg-bad/20 hover:text-bad" : "hover:bg-panel2 hover:text-ink"
      }`}
    >
      {children}
    </button>
  );
}

const icon = (path: string) => (
  <svg
    className="h-3.5 w-3.5"
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

const COPY_ICON = icon(
  "M8 8h12v12H8zM8 8V6a2 2 0 012-2h10a2 2 0 012 2v10a2 2 0 01-2 2h-2",
);
const REGEN_ICON = icon("M21 12a9 9 0 11-2.64-6.36M21 3v6h-6");
const EDIT_ICON = icon(
  "M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7M18.5 2.5a2.1 2.1 0 013 3L12 15l-4 1 1-4z",
);
const TRASH_ICON = icon(
  "M3 6h18M8 6V4a1 1 0 011-1h6a1 1 0 011 1v2m3 0v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6",
);

function formatSec(ms: number): string {
  return (Math.max(0, ms) / 1000).toFixed(1);
}

/** Relógio local (~10 Hz) enquanto a última resposta ainda está em curso. */
function useNow(active: boolean): number {
  const [now, setNow] = useState(() => performance.now());
  useEffect(() => {
    if (!active) return;
    setNow(performance.now());
    const id = window.setInterval(() => setNow(performance.now()), 100);
    return () => window.clearInterval(id);
  }, [active]);
  return now;
}

export default function MessageList({
  messages,
  generating,
  loadingModel,
  onRegenerate,
  onEditResend,
  onDeleteMsg,
}: {
  messages: UiMessage[];
  generating: boolean;
  loadingModel: boolean;
  onRegenerate?: () => void;
  onEditResend?: (index: number) => void;
  onDeleteMsg?: (index: number) => void;
}) {
  const { t } = useTranslation();
  const scrollRef = useRef<HTMLDivElement>(null);
  const stickRef = useRef(true);
  const [copiedIdx, setCopiedIdx] = useState<number | null>(null);
  const now = useNow(generating);

  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    stickRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
  };

  useEffect(() => {
    const el = scrollRef.current;
    if (el && stickRef.current) el.scrollTop = el.scrollHeight;
  }, [messages, generating, loadingModel]);

  const copyMsg = (i: number, text: string) => {
    void navigator.clipboard.writeText(text).then(() => {
      setCopiedIdx(i);
      window.setTimeout(() => setCopiedIdx((c) => (c === i ? null : c)), 1500);
    });
  };

  // Índices da última user e última assistant (ações contextuais).
  let lastUserIdx = -1;
  let lastAssistantIdx = -1;
  for (let i = messages.length - 1; i >= 0; i--) {
    if (lastUserIdx < 0 && messages[i].role === "user") lastUserIdx = i;
    if (lastAssistantIdx < 0 && messages[i].role === "assistant") {
      lastAssistantIdx = i;
    }
    if (lastUserIdx >= 0 && lastAssistantIdx >= 0) break;
  }

  const liveTimes = (m: UiMessage, live: boolean) => {
    let thinkingMs = m.thinkingMs ?? null;
    let genMs = m.genMs ?? null;
    if (live) {
      if (m.answerStartedAt != null) {
        genMs = Math.round(now - m.answerStartedAt);
        const thinkOrigin = m.thinkStartedAt ?? m.startedAt;
        if (thinkOrigin != null) {
          thinkingMs = Math.round(m.answerStartedAt - thinkOrigin);
        }
      } else {
        const origin = m.thinkStartedAt ?? m.startedAt;
        if (origin != null) thinkingMs = Math.round(now - origin);
      }
    }
    return { thinkingMs, genMs };
  };

  /** Linha sutil de métricas sob a resposta — só os campos disponíveis. */
  const statsFor = (m: UiMessage, live: boolean): string | null => {
    const { genMs: ms } = liveTimes(m, live);
    const tokens = m.genTokens ?? null;
    if (tokens != null && ms != null) {
      const tps =
        m.tokensPerSec ?? (ms > 0 ? tokens / (ms / 1000) : null);
      if (tps != null) {
        return t("chat.statsLine", {
          tps: tps.toFixed(1),
          tokens,
          seconds: formatSec(ms),
        });
      }
    }
    if (m.tokensPerSec != null && ms != null) {
      return t("chat.statsTpsTime", {
        tps: m.tokensPerSec.toFixed(1),
        seconds: formatSec(ms),
      });
    }
    if (ms != null) {
      return live
        ? t("chat.answeringTimer", { seconds: formatSec(ms) })
        : t("chat.statsTime", { seconds: formatSec(ms) });
    }
    if (m.tokensPerSec != null) {
      return `${m.tokensPerSec.toFixed(1)} ${t("status.tokensPerSec")}`;
    }
    return null;
  };

  const actionBar = (m: UiMessage, i: number) => {
    if (generating) return null;
    const isLastUser = i === lastUserIdx;
    const isLastAssistant = i === lastAssistantIdx;
    return (
      <div
        className={`mt-0.5 flex items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 ${
          m.role === "user" ? "justify-end" : ""
        }`}
      >
        <ActionButton
          title={copiedIdx === i ? t("chat.copied") : t("chat.copyMsg")}
          onClick={() => copyMsg(i, m.content)}
        >
          {copiedIdx === i ? (
            <svg
              className="h-3.5 w-3.5 text-ok"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
              viewBox="0 0 24 24"
            >
              <path d="M20 6L9 17l-5-5" />
            </svg>
          ) : (
            COPY_ICON
          )}
        </ActionButton>
        {isLastAssistant && m.role === "assistant" && onRegenerate && (
          <ActionButton title={t("chat.regenerate")} onClick={onRegenerate}>
            {REGEN_ICON}
          </ActionButton>
        )}
        {isLastUser && m.role === "user" && onEditResend && (
          <ActionButton
            title={t("chat.editResend")}
            onClick={() => onEditResend(i)}
          >
            {EDIT_ICON}
          </ActionButton>
        )}
        {(isLastUser || isLastAssistant) && onDeleteMsg && (
          <ActionButton
            title={t("chat.deleteMsg")}
            onClick={() => onDeleteMsg(i)}
            danger
          >
            {TRASH_ICON}
          </ActionButton>
        )}
      </div>
    );
  };

  const last = messages[messages.length - 1];
  // A bolha do assistente já mostra o próprio indicador enquanto o 1º token
  // não chega; sem esta checagem apareceriam dois "carregando" na tela.
  const waitingFirstToken =
    generating &&
    last?.role === "assistant" &&
    last.content === "" &&
    !last.reasoning;

  return (
    <div
      ref={scrollRef}
      onScroll={onScroll}
      className="min-h-0 flex-1 overflow-y-auto"
    >
      <div className="mx-auto flex max-w-3xl flex-col gap-4 px-6 py-6">
        {messages.map((m, i) =>
          m.role === "user" ? (
            <div key={i} className="group flex max-w-[80%] flex-col self-end">
              <div className="rounded-2xl rounded-br-sm bg-accent px-4 py-2.5 text-sm whitespace-pre-wrap text-white select-text">
                {m.content}
              </div>
              {actionBar(m, i)}
            </div>
          ) : (
            <div key={i} className="group max-w-full self-start select-text">
              {m.error ? (
                <div className="rounded-xl border border-bad/40 bg-bad/10 px-4 py-2.5 text-sm text-bad">
                  {m.content}
                </div>
              ) : m.content === "" &&
                !m.reasoning &&
                generating &&
                i === messages.length - 1 ? (
                <div className="flex items-center gap-2 py-1 text-sm text-dim">
                  <span className="h-2 w-2 animate-pulse rounded-full bg-accent" />
                  {loadingModel
                    ? t("chat.loadingModel")
                    : m.startedAt == null
                      ? t("jobs.queued")
                      : t("chat.generatingTimer", {
                          seconds: formatSec(
                            liveTimes(m, true).thinkingMs ??
                              now - m.startedAt,
                          ),
                        })}
                </div>
              ) : (
                <>
                  {m.reasoning && (
                    <ThinkingBlock
                      reasoning={m.reasoning}
                      thinkingMs={
                        liveTimes(m, generating && i === messages.length - 1)
                          .thinkingMs
                      }
                      active={
                        generating &&
                        i === messages.length - 1 &&
                        m.content === ""
                      }
                    />
                  )}
                  {m.content !== "" && <Markdown text={m.content} />}
                  {loadingModel && generating && i === messages.length - 1 && (
                    <div className="mt-1 text-xs text-dim">
                      {t("chat.loadingModel")}
                    </div>
                  )}
                  {(() => {
                    const live = generating && i === messages.length - 1;
                    const stats = statsFor(m, live);
                    return stats ? (
                      <div className="mt-1 text-[11px] tabular-nums text-dim">
                        {stats}
                      </div>
                    ) : null;
                  })()}
                </>
              )}
              {actionBar(m, i)}
            </div>
          ),
        )}

        {(loadingModel && !waitingFirstToken && messages.length === 0) ||
        (generating && last?.role === "user") ? (
          <div className="flex items-center gap-2 py-1 text-sm text-dim">
            <span className="h-2 w-2 animate-pulse rounded-full bg-accent" />
            {loadingModel ? t("chat.loadingModel") : t("chat.generating")}
          </div>
        ) : null}
      </div>
    </div>
  );
}
