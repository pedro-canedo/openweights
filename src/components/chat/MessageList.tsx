// Lista de mensagens do chat com auto-scroll para o fim durante o
// streaming (só "gruda" no fundo se o usuário já estiver perto dele).

import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import Markdown from "./Markdown";

export interface UiMessage {
  role: "user" | "assistant";
  content: string;
  tokensPerSec: number | null;
  error?: boolean;
}

export default function MessageList({
  messages,
  generating,
  loadingModel,
}: {
  messages: UiMessage[];
  generating: boolean;
  loadingModel: boolean;
}) {
  const { t } = useTranslation();
  const scrollRef = useRef<HTMLDivElement>(null);
  const stickRef = useRef(true);

  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    stickRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
  };

  useEffect(() => {
    const el = scrollRef.current;
    if (el && stickRef.current) el.scrollTop = el.scrollHeight;
  }, [messages, generating, loadingModel]);

  const last = messages[messages.length - 1];
  const waitingFirstToken =
    generating && last?.role === "assistant" && last.content === "";

  return (
    <div
      ref={scrollRef}
      onScroll={onScroll}
      className="min-h-0 flex-1 overflow-y-auto"
    >
      <div className="mx-auto flex max-w-3xl flex-col gap-4 px-6 py-6">
        {messages.map((m, i) =>
          m.role === "user" ? (
            <div
              key={i}
              className="max-w-[80%] self-end rounded-2xl rounded-br-sm bg-accent px-4 py-2.5 text-sm whitespace-pre-wrap text-white select-text"
            >
              {m.content}
            </div>
          ) : (
            <div key={i} className="max-w-full self-start select-text">
              {m.error ? (
                <div className="rounded-xl border border-bad/40 bg-bad/10 px-4 py-2.5 text-sm text-bad">
                  {m.content}
                </div>
              ) : m.content === "" && generating && i === messages.length - 1 ? (
                <div className="flex items-center gap-2 py-1 text-sm text-dim">
                  <span className="h-2 w-2 animate-pulse rounded-full bg-accent" />
                  {loadingModel ? t("chat.loadingModel") : t("chat.generating")}
                </div>
              ) : (
                <>
                  <Markdown text={m.content} />
                  {loadingModel && generating && i === messages.length - 1 && (
                    <div className="mt-1 text-xs text-dim">
                      {t("chat.loadingModel")}
                    </div>
                  )}
                  {m.tokensPerSec != null && (
                    <div className="mt-1 text-[11px] tabular-nums text-dim">
                      {m.tokensPerSec.toFixed(1)} {t("status.tokensPerSec")}
                    </div>
                  )}
                </>
              )}
            </div>
          ),
        )}

        {/* Servidor/modelo carregando antes mesmo da 1ª mensagem existir */}
        {loadingModel && !waitingFirstToken && messages.length === 0 && (
          <div className="flex items-center gap-2 py-1 text-sm text-dim">
            <span className="h-2 w-2 animate-pulse rounded-full bg-accent" />
            {t("chat.loadingModel")}
          </div>
        )}
      </div>
    </div>
  );
}
