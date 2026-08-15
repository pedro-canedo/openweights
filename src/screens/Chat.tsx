// Tela de Chat: conversas persistidas (SQLite via api), seletor de modelo
// (Router mode carrega sob demanda) e streaming SSE direto do llama-server.

import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  addMessage,
  createChat,
  deleteChat,
  getServerStatus,
  listChats,
  listLocalModels,
  listMessages,
  startServer,
} from "../lib/api";
import { streamChat } from "../lib/llama";
import { navigate, takePendingChatModel } from "../lib/nav";
import type { ChatRow } from "../lib/types";
import ChatSidebar from "../components/chat/ChatSidebar";
import Composer from "../components/chat/Composer";
import MessageList, { type UiMessage } from "../components/chat/MessageList";
import ModelSelect from "../components/chat/ModelSelect";

/**
 * Opções do seletor: se o servidor está rodando, usa GET /v1/models
 * (fonte da verdade do Router mode); senão, os nomes da biblioteca local.
 */
async function loadModelOptions(): Promise<string[]> {
  try {
    const status = await getServerStatus();
    if (status.running && status.baseUrl) {
      const res = await fetch(`${status.baseUrl}/v1/models`);
      if (res.ok) {
        const json = (await res.json()) as { data?: { id: string }[] };
        const ids = (json.data ?? []).map((d) => d.id);
        if (ids.length > 0) return ids;
      }
    }
  } catch {
    // servidor inacessível — cai para a biblioteca local
  }
  try {
    return (await listLocalModels()).map((m) => m.name);
  } catch {
    return [];
  }
}

function isAbortError(e: unknown): boolean {
  return (
    (e instanceof DOMException && e.name === "AbortError") ||
    (e instanceof Error && e.name === "AbortError")
  );
}

export default function Chat() {
  const { t } = useTranslation();

  const [chats, setChats] = useState<ChatRow[]>([]);
  const [activeChatId, setActiveChatId] = useState<number | null>(null);
  const [messages, setMessages] = useState<UiMessage[]>([]);
  const [models, setModels] = useState<string[] | null>(null);
  const [selectedModel, setSelectedModel] = useState("");
  const [draft, setDraft] = useState("");
  const [generating, setGenerating] = useState(false);
  const [loadingModel, setLoadingModel] = useState(false);

  const abortRef = useRef<AbortController | null>(null);
  const busyRef = useRef(false);
  // "Época" da conversa exibida: trocar/criar/excluir conversa invalida
  // qualquer setMessages pendente do streaming anterior (o catch do abort
  // roda DEPOIS da troca e restauraria a conversa antiga por cima da nova).
  const convEpochRef = useRef(0);
  // Conversas excluídas na sessão: não persistir mensagens nelas (FK).
  const deletedChatsRef = useRef<Set<number>>(new Set());

  useEffect(() => {
    let cancelled = false;
    void listChats()
      .then((rows) => {
        if (!cancelled) setChats(rows);
      })
      .catch(() => {});

    const pending = takePendingChatModel();
    void loadModelOptions().then((list) => {
      if (cancelled) return;
      const options =
        pending && !list.includes(pending) ? [pending, ...list] : list;
      setModels(options);
      setSelectedModel((cur) => cur || pending || options[0] || "");
    });

    return () => {
      cancelled = true;
      abortRef.current?.abort();
    };
  }, []);

  const selectChat = async (chat: ChatRow) => {
    convEpochRef.current++;
    abortRef.current?.abort();
    setActiveChatId(chat.id);
    if (chat.modelId) setSelectedModel(chat.modelId);
    try {
      const rows = await listMessages(chat.id);
      setMessages(
        rows.map((r) => ({
          role: r.role === "user" ? ("user" as const) : ("assistant" as const),
          content: r.content,
          tokensPerSec: r.tokensPerSec,
        })),
      );
    } catch {
      setMessages([]);
    }
  };

  const newChat = () => {
    convEpochRef.current++;
    abortRef.current?.abort();
    setActiveChatId(null);
    setMessages([]);
  };

  const removeChat = async (chat: ChatRow) => {
    // Sem chave i18n dedicada para confirmação de exclusão de conversa.
    if (!window.confirm(`Excluir "${chat.title}"?`)) return;
    try {
      await deleteChat(chat.id);
    } catch {
      // mock do navegador não implementa — remove só da UI
    }
    deletedChatsRef.current.add(chat.id);
    setChats((prev) => prev.filter((c) => c.id !== chat.id));
    if (chat.id === activeChatId) {
      convEpochRef.current++;
      abortRef.current?.abort();
      setActiveChatId(null);
      setMessages([]);
    }
  };

  const handleSend = async () => {
    const text = draft.trim();
    if (!text || !selectedModel || busyRef.current) return;
    busyRef.current = true;
    setDraft("");
    setGenerating(true);

    const history: UiMessage[] = [
      ...messages,
      { role: "user", content: text, tokensPerSec: null },
    ];
    setMessages([
      ...history,
      { role: "assistant", content: "", tokensPerSec: null },
    ]);

    const ac = new AbortController();
    abortRef.current = ac;
    let chatId = activeChatId;
    let assistantText = "";
    let slowTimer = 0;

    // Qualquer troca de conversa invalida os setMessages deste envio.
    const epoch = convEpochRef.current;
    const safeSetMessages = (m: UiMessage[]) => {
      if (convEpochRef.current === epoch) setMessages(m);
    };
    const canPersist = (id: number | null): id is number =>
      id != null && !deletedChatsRef.current.has(id);

    try {
      // Conversa é criada na primeira mensagem; título = início do texto.
      if (chatId == null) {
        const title = text.length > 40 ? `${text.slice(0, 40)}…` : text;
        chatId = await createChat(title, selectedModel);
        if (convEpochRef.current === epoch) setActiveChatId(chatId);
        void listChats()
          .then(setChats)
          .catch(() => {});
      }
      if (canPersist(chatId)) await addMessage(chatId, "user", text);

      // Servidor parado? Inicia automaticamente antes de conversar.
      let status = await getServerStatus();
      if (!status.running || !status.baseUrl) {
        setLoadingModel(true);
        status = await startServer();
      }
      if (!status.baseUrl) throw new Error(t("server.needRuntime"));

      // Router mode: a 1ª resposta de um modelo recém-selecionado demora
      // (carga na memória) — avisa quando passar de 2 s sem nenhum delta.
      slowTimer = window.setTimeout(() => setLoadingModel(true), 2000);

      const { tokensPerSec } = await streamChat({
        baseUrl: status.baseUrl,
        model: selectedModel,
        messages: history.map(({ role, content }) => ({ role, content })),
        signal: ac.signal,
        onDelta: (delta) => {
          assistantText += delta;
          window.clearTimeout(slowTimer);
          setLoadingModel(false);
          safeSetMessages([
            ...history,
            { role: "assistant", content: assistantText, tokensPerSec: null },
          ]);
        },
      });

      safeSetMessages([
        ...history,
        { role: "assistant", content: assistantText, tokensPerSec },
      ]);
      if (canPersist(chatId)) {
        await addMessage(chatId, "assistant", assistantText, tokensPerSec);
      }
    } catch (err) {
      if (isAbortError(err)) {
        // Parada manual: preserva o texto parcial (se houver).
        if (assistantText) {
          safeSetMessages([
            ...history,
            { role: "assistant", content: assistantText, tokensPerSec: null },
          ]);
          if (canPersist(chatId)) {
            await addMessage(chatId, "assistant", assistantText, null).catch(
              () => {},
            );
          }
        } else {
          safeSetMessages(history);
        }
      } else {
        const detail =
          typeof err === "string"
            ? err
            : err instanceof Error
              ? err.message
              : String(err);
        safeSetMessages([
          ...history,
          {
            role: "assistant",
            content: `${t("common.error")}: ${detail}`,
            tokensPerSec: null,
            error: true,
          },
        ]);
      }
    } finally {
      window.clearTimeout(slowTimer);
      setLoadingModel(false);
      setGenerating(false);
      abortRef.current = null;
      busyRef.current = false;
    }
  };

  const noModels =
    models !== null && models.length === 0 && selectedModel === "";

  return (
    <div className="flex h-full">
      <ChatSidebar
        chats={chats}
        activeId={activeChatId}
        onSelect={(c) => void selectChat(c)}
        onNew={newChat}
        onDelete={(c) => void removeChat(c)}
      />

      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex shrink-0 items-center gap-3 border-b border-edge px-4 py-2">
          <ModelSelect
            models={models ?? []}
            value={selectedModel}
            onChange={setSelectedModel}
            disabled={generating}
          />
        </header>

        {noModels ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 text-center text-sm text-dim">
            <span>{t("chat.noModels")}</span>
            <button
              onClick={() => navigate("discover")}
              className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-white"
            >
              {t("nav.discover")}
            </button>
          </div>
        ) : messages.length === 0 && !generating && !loadingModel ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-1 px-6 text-center text-sm text-dim">
            <span>{t("chat.empty")}</span>
            <span className="text-xs">{t("chat.noServer")}</span>
          </div>
        ) : (
          <MessageList
            messages={messages}
            generating={generating}
            loadingModel={loadingModel}
          />
        )}

        <Composer
          draft={draft}
          onDraftChange={setDraft}
          onSend={() => void handleSend()}
          onStop={() => abortRef.current?.abort()}
          generating={generating}
          disabled={!selectedModel}
        />
      </div>
    </div>
  );
}
