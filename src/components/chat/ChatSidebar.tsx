// Barra lateral interna do Chat: busca local, lista de conversas
// persistidas, nova conversa, renomear, exportar (Markdown) e exclusão.

import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { ChatRow } from "../../lib/types";

export default function ChatSidebar({
  chats,
  activeId,
  onSelect,
  onNew,
  onDelete,
  onRename,
  onExport,
}: {
  chats: ChatRow[];
  activeId: number | null;
  onSelect: (chat: ChatRow) => void;
  onNew: () => void;
  onDelete: (chat: ChatRow) => void;
  onRename: (chat: ChatRow, title: string) => void;
  onExport: (chat: ChatRow) => Promise<void>;
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [renamingId, setRenamingId] = useState<number | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const [exportedId, setExportedId] = useState<number | null>(null);

  const q = query.trim().toLowerCase();
  const visible = q
    ? chats.filter((c) => c.title.toLowerCase().includes(q))
    : chats;

  const commitRename = (chat: ChatRow) => {
    const title = renameDraft.trim();
    setRenamingId(null);
    if (title && title !== chat.title) onRename(chat, title);
  };

  const exportChat = async (chat: ChatRow) => {
    try {
      await onExport(chat);
      setExportedId(chat.id);
      window.setTimeout(
        () => setExportedId((cur) => (cur === chat.id ? null : cur)),
        1500,
      );
    } catch {
      // clipboard indisponível — sem feedback
    }
  };

  return (
    <aside className="flex w-56 shrink-0 flex-col border-r border-edge bg-panel">
      <div className="flex flex-col gap-2 p-2">
        <button
          onClick={onNew}
          className="flex w-full items-center justify-center gap-2 rounded-lg border border-edge px-3 py-2 text-sm text-dim transition-colors hover:border-accent hover:text-ink"
        >
          <svg
            className="h-4 w-4"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.8"
            strokeLinecap="round"
            viewBox="0 0 24 24"
          >
            <path d="M12 5v14M5 12h14" />
          </svg>
          {t("chat.newChat")}
        </button>
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t("chat.search")}
          className="w-full rounded-lg border border-edge bg-panel2 px-3 py-1.5 text-xs outline-none placeholder:text-dim focus:border-accent"
        />
      </div>

      <div className="px-4 pt-2 pb-1 text-[11px] font-medium tracking-wide text-dim uppercase">
        {t("chat.history")}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
        {visible.map((c) => (
          <div
            key={c.id}
            className={`group flex items-center gap-0.5 rounded-lg ${
              c.id === activeId ? "bg-panel2" : "hover:bg-panel2/60"
            }`}
          >
            {renamingId === c.id ? (
              <input
                autoFocus
                value={renameDraft}
                onChange={(e) => setRenameDraft(e.target.value)}
                onBlur={() => commitRename(c)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") commitRename(c);
                  if (e.key === "Escape") setRenamingId(null);
                }}
                className="mx-1 my-1 min-w-0 flex-1 rounded border border-accent bg-panel px-2 py-1 text-sm outline-none"
              />
            ) : (
              <>
                <button
                  onClick={() => onSelect(c)}
                  className={`min-w-0 flex-1 truncate px-3 py-2 text-left text-sm ${
                    c.id === activeId ? "text-ink" : "text-dim"
                  }`}
                  title={c.title}
                >
                  {c.title}
                </button>

                {/* Exportar como Markdown — só no chat ativo */}
                {c.id === activeId && (
                  <button
                    onClick={() => void exportChat(c)}
                    title={
                      exportedId === c.id
                        ? t("chat.exported")
                        : t("chat.exportMd")
                    }
                    className="hidden h-6 w-6 shrink-0 items-center justify-center rounded text-dim group-hover:flex hover:bg-panel hover:text-ink"
                  >
                    {exportedId === c.id ? (
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
                      <svg
                        className="h-3.5 w-3.5"
                        fill="none"
                        stroke="currentColor"
                        strokeWidth="1.8"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        viewBox="0 0 24 24"
                      >
                        <path d="M8 8h12v12H8zM8 8V6a2 2 0 012-2h10a2 2 0 012 2v10a2 2 0 01-2 2h-2" />
                      </svg>
                    )}
                  </button>
                )}

                <button
                  onClick={() => {
                    setRenamingId(c.id);
                    setRenameDraft(c.title);
                  }}
                  title={t("chat.rename")}
                  className="hidden h-6 w-6 shrink-0 items-center justify-center rounded text-dim group-hover:flex hover:bg-panel hover:text-ink"
                >
                  <svg
                    className="h-3.5 w-3.5"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.8"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    viewBox="0 0 24 24"
                  >
                    <path d="M18.5 2.5a2.1 2.1 0 013 3L12 15l-4 1 1-4z" />
                  </svg>
                </button>

                <button
                  onClick={() => onDelete(c)}
                  title="Excluir conversa"
                  className="mr-1 hidden h-6 w-6 shrink-0 items-center justify-center rounded text-dim group-hover:flex hover:bg-bad/20 hover:text-bad"
                >
                  <svg
                    className="h-3.5 w-3.5"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.8"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    viewBox="0 0 24 24"
                  >
                    <path d="M3 6h18M8 6V4a1 1 0 011-1h6a1 1 0 011 1v2m3 0v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6" />
                  </svg>
                </button>
              </>
            )}
          </div>
        ))}
      </div>
    </aside>
  );
}
