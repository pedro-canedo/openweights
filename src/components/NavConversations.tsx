// Conversas na sidebar principal: busca, nova, lista, ações no hover.

import { useEffect, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { deleteChat, listChats, listMessages, renameChat } from "../lib/api";
import { chatStore } from "../lib/chatStore";
import { navigate } from "../lib/nav";
import type { ChatRow } from "../lib/types";

export default function NavConversations() {
  const { t } = useTranslation();
  const { chats, activeId } = useSyncExternalStore(
    chatStore.subscribe,
    chatStore.get,
  );
  const [query, setQuery] = useState("");
  const [renamingId, setRenamingId] = useState<number | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const [exportedId, setExportedId] = useState<number | null>(null);

  useEffect(() => {
    void listChats()
      .then(chatStore.setChats)
      .catch(() => {});
  }, []);

  const q = query.trim().toLowerCase();
  const visible = q
    ? chats.filter((c) => c.title.toLowerCase().includes(q))
    : chats;

  const openChat = (chat: ChatRow) => {
    chatStore.requestOpen(chat.id);
    navigate("chat");
  };

  const newChat = () => {
    chatStore.requestNew();
    navigate("chat");
  };

  const commitRename = (chat: ChatRow) => {
    const title = renameDraft.trim();
    setRenamingId(null);
    if (!title || title === chat.title) return;
    void renameChat(chat.id, title)
      .then(() => listChats())
      .then(chatStore.setChats)
      .catch(() => {});
  };

  const remove = (chat: ChatRow) => {
    if (!window.confirm(t("chat.deleteChatConfirm", { title: chat.title }))) {
      return;
    }
    void deleteChat(chat.id).catch(() => {});
    chatStore.setChats(chats.filter((c) => c.id !== chat.id));
    if (chat.id === activeId) chatStore.requestNew();
  };

  const exportMd = async (chat: ChatRow) => {
    try {
      const rows = await listMessages(chat.id);
      const md = [
        `# ${chat.title}`,
        ...rows.map(
          (m) =>
            `### ${m.role === "user" ? "Você" : "Assistente"}\n\n${m.content}`,
        ),
      ].join("\n\n");
      await navigator.clipboard.writeText(md);
      setExportedId(chat.id);
      window.setTimeout(
        () => setExportedId((cur) => (cur === chat.id ? null : cur)),
        1500,
      );
    } catch {
      // clipboard indisponível
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col px-2 pt-3">
      <div className="mb-2 flex items-center justify-between px-1">
        <span className="text-[11px] font-medium tracking-wide text-dim uppercase">
          {t("chat.history")}
        </span>
        <button
          type="button"
          onClick={newChat}
          title={t("chat.newChat")}
          className="flex h-6 w-6 items-center justify-center rounded-md text-dim transition-colors hover:bg-panel2 hover:text-ink"
        >
          <svg
            className="h-3.5 w-3.5"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            viewBox="0 0 24 24"
          >
            <path d="M12 5v14M5 12h14" />
          </svg>
        </button>
      </div>

      <label className="relative mb-2 block">
        <svg
          className="pointer-events-none absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-dim"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          strokeLinecap="round"
          strokeLinejoin="round"
          viewBox="0 0 24 24"
        >
          <path d="M21 21l-4.35-4.35M17 10a7 7 0 11-14 0 7 7 0 0114 0z" />
        </svg>
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t("chat.search")}
          className="w-full rounded-lg border border-edge bg-panel2 py-1.5 pr-2 pl-8 text-xs outline-none placeholder:text-dim focus:border-accent"
        />
      </label>

      <div className="min-h-0 flex-1 overflow-y-auto pb-1">
        {visible.length === 0 ? (
          <p className="px-2 py-3 text-center text-[11px] text-dim">
            {chats.length === 0 ? t("chat.historyEmpty") : t("chat.searchEmpty")}
          </p>
        ) : (
          visible.map((c) => (
            <div
              key={c.id}
              className={`group mb-0.5 flex items-center rounded-lg transition-colors ${
                c.id === activeId
                  ? "bg-panel2 text-ink"
                  : "text-dim hover:bg-panel2/60 hover:text-ink"
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
                  className="mx-1 my-1 min-w-0 flex-1 rounded border border-accent bg-panel px-2 py-1 text-xs outline-none"
                />
              ) : (
                <>
                  <button
                    type="button"
                    onClick={() => openChat(c)}
                    title={c.title}
                    className="flex min-w-0 flex-1 items-center gap-2 px-2.5 py-1.5 text-left"
                  >
                    <span
                      className={`h-1.5 w-1.5 shrink-0 rounded-full ${
                        c.id === activeId ? "bg-accent" : "bg-edge"
                      }`}
                    />
                    <span className="truncate text-[13px]">{c.title}</span>
                  </button>
                  <div className="mr-0.5 hidden shrink-0 items-center group-hover:flex">
                    <button
                      type="button"
                      onClick={() => void exportMd(c)}
                      title={
                        exportedId === c.id
                          ? t("chat.exported")
                          : t("chat.exportMd")
                      }
                      className="flex h-6 w-6 items-center justify-center rounded text-dim hover:bg-panel hover:text-ink"
                    >
                      {exportedId === c.id ? (
                        <svg
                          className="h-3 w-3 text-ok"
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
                          className="h-3 w-3"
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
                    <button
                      type="button"
                      onClick={() => {
                        setRenamingId(c.id);
                        setRenameDraft(c.title);
                      }}
                      title={t("chat.rename")}
                      className="flex h-6 w-6 items-center justify-center rounded text-dim hover:bg-panel hover:text-ink"
                    >
                      <svg
                        className="h-3 w-3"
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
                      type="button"
                      onClick={() => remove(c)}
                      title={t("chat.deleteChat")}
                      className="flex h-6 w-6 items-center justify-center rounded text-dim hover:bg-bad/20 hover:text-bad"
                    >
                      <svg
                        className="h-3 w-3"
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
                </>
              )}
            </div>
          ))
        )}
      </div>
    </div>
  );
}
