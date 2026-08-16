// Painel global de jobs de geração: visível enquanto houver stream em
// fila ou rodando. A UI só observa e cancela; o store é o keep-alive.

import { useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { chatStore } from "../lib/chatStore";
import { generationStore } from "../lib/generationStore";
import { navigate } from "../lib/nav";

export default function GenerationPanel() {
  const { t } = useTranslation();
  const gen = useSyncExternalStore(
    generationStore.subscribe,
    generationStore.get,
  );
  const { chats } = useSyncExternalStore(chatStore.subscribe, chatStore.get);
  const [open, setOpen] = useState(false);

  const jobs = gen.jobs.filter(
    (j) => j.state === "queued" || j.state === "running",
  );
  if (jobs.length === 0) return null;

  const titleOf = (chatId: number) =>
    chats.find((c) => c.id === chatId)?.title ?? t("chat.title");

  return (
    <div className="fixed right-4 bottom-24 z-40 flex flex-col items-end">
      {open && (
        <div className="mb-2 flex max-h-[60vh] w-80 max-w-[calc(100vw-2rem)] flex-col overflow-hidden rounded-xl border border-edge bg-panel shadow-2xl">
          <header className="flex items-center gap-2 border-b border-edge px-4 py-2.5">
            <span className="text-sm font-semibold">{t("jobs.title")}</span>
            <span className="rounded-full bg-panel2 px-2 py-0.5 text-[11px] text-dim">
              {jobs.length}
            </span>
            <button
              onClick={() => setOpen(false)}
              aria-label={t("common.close")}
              className="ml-auto rounded-md p-1 text-dim transition-colors hover:bg-panel2 hover:text-ink"
            >
              <svg
                className="h-4 w-4"
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
          </header>
          <div className="min-h-0 flex-1 overflow-y-auto">
            {jobs.map((job) => (
              <div
                key={job.id}
                className="flex items-center gap-2 border-b border-edge px-4 py-3 last:border-b-0"
              >
                <button
                  type="button"
                  onClick={() => {
                    chatStore.requestOpen(job.chatId);
                    navigate("chat");
                  }}
                  className="min-w-0 flex-1 text-left"
                >
                  <p className="truncate text-xs font-medium text-ink">
                    {titleOf(job.chatId)}
                  </p>
                  <p className="truncate text-[11px] text-dim">
                    {job.model} ·{" "}
                    {job.state === "queued"
                      ? t("jobs.queued")
                      : t("jobs.running")}
                  </p>
                </button>
                <button
                  type="button"
                  onClick={() => generationStore.cancel(job.chatId)}
                  title={t("jobs.stop")}
                  className="shrink-0 rounded-md px-2 py-1 text-[11px] font-medium text-dim transition-colors hover:bg-panel2 hover:text-ink"
                >
                  {t("jobs.stop")}
                </button>
              </div>
            ))}
          </div>
        </div>
      )}

      <button
        onClick={() => setOpen((v) => !v)}
        title={t("jobs.title")}
        className="flex items-center gap-2.5 rounded-full border border-edge bg-panel py-2 pr-4 pl-3 shadow-lg transition-colors hover:border-accent"
      >
        <span className="h-3.5 w-3.5 animate-spin rounded-full border-2 border-accent border-t-transparent" />
        <span className="text-xs font-medium tabular-nums text-ink">
          {jobs.length}
        </span>
      </button>
    </div>
  );
}
