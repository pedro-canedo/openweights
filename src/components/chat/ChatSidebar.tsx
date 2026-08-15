// Barra lateral interna do Chat: lista de conversas persistidas,
// nova conversa e exclusão.

import { useTranslation } from "react-i18next";
import type { ChatRow } from "../../lib/types";

export default function ChatSidebar({
  chats,
  activeId,
  onSelect,
  onNew,
  onDelete,
}: {
  chats: ChatRow[];
  activeId: number | null;
  onSelect: (chat: ChatRow) => void;
  onNew: () => void;
  onDelete: (chat: ChatRow) => void;
}) {
  const { t } = useTranslation();

  return (
    <aside className="flex w-56 shrink-0 flex-col border-r border-edge bg-panel">
      <div className="p-2">
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
      </div>

      <div className="px-4 pt-2 pb-1 text-[11px] font-medium tracking-wide text-dim uppercase">
        {t("chat.history")}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
        {chats.map((c) => (
          <div
            key={c.id}
            className={`group flex items-center gap-1 rounded-lg ${
              c.id === activeId ? "bg-panel2" : "hover:bg-panel2/60"
            }`}
          >
            <button
              onClick={() => onSelect(c)}
              className={`min-w-0 flex-1 truncate px-3 py-2 text-left text-sm ${
                c.id === activeId ? "text-ink" : "text-dim"
              }`}
              title={c.title}
            >
              {c.title}
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
          </div>
        ))}
      </div>
    </aside>
  );
}
