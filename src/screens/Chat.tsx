// Tela de Chat: conversas persistidas (SQLite via api), seletor de modelo
// (Router mode carrega sob demanda), streaming SSE direto do llama-server,
// parâmetros por conversa, raciocínio (thinking), anexos, regenerar/editar
// e auto-título em background.

import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import {
  addMessage,
  createChat,
  deleteMessage,
  getServerStatus,
  listChats,
  listLocalModels,
  listMessages,
  readWorkspaceFile,
  setChatParams,
  updateMessage,
} from "../lib/api";
import { type ChatMessage, type ContentPart } from "../lib/llama";
import { chatStore } from "../lib/chatStore";
import { generationStore } from "../lib/generationStore";
import { navigate, takePendingChatModel } from "../lib/nav";
import {
  DEFAULT_CHAT_PARAMS,
  type ChatParams,
  type ChatRow,
  type MessageRow,
  type WorkspaceFile,
} from "../lib/types";
import {
  classifyFile,
  readAttachment,
  type Attachment,
} from "../components/chat/AttachmentChips";
import ChatHero from "../components/chat/ChatHero";
import { ApprovalSelect } from "../components/chat/ChatProperties";
import Composer from "../components/chat/Composer";
import {
  WorkspaceExplorer,
  WorkspaceHost,
  WorkspaceToggle,
  WorkspaceTrigger,
} from "../components/chat/WorkspacePanel";
import MessageList, { type UiMessage } from "../components/chat/MessageList";
import ModelSelect from "../components/chat/ModelSelect";
import ParamsPanel from "../components/chat/ParamsPanel";

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

/** Extrai o prefixo <think>...</think> salvo no DB (raciocínio persistido). */
function parseThinkPrefix(content: string): {
  reasoning: string | null;
  content: string;
} {
  const m = /^\s*<think>([\s\S]*?)<\/think>\s*/.exec(content);
  if (!m) return { reasoning: null, content };
  return { reasoning: m[1].trim(), content: content.slice(m[0].length) };
}

/** Linha de mensagem do DB → mensagem da UI (com parse do raciocínio). */
function rowToUi(r: MessageRow): UiMessage {
  if (r.role === "user") {
    return { role: "user", content: r.content, tokensPerSec: null, rowId: r.id };
  }
  const { reasoning, content } = parseThinkPrefix(r.content);
  return {
    role: "assistant",
    content,
    tokensPerSec: r.tokensPerSec,
    rowId: r.id,
    reasoning: reasoning ?? undefined,
    genTokens: r.genTokens,
    genMs: r.genMs,
  };
}

const IMAGE_MARKER_RE = /\n*\[imagem: [^\]\n]+\]/g;

/** Mensagens da UI → formato da API (multimodal quando há imagens). */
function toApiMessages(history: UiMessage[]): ChatMessage[] {
  return history.map((m) => {
    if (m.role === "user" && m.images && m.images.length > 0) {
      const text = m.content.replace(IMAGE_MARKER_RE, "").trim();
      const parts: ContentPart[] = [];
      if (text) parts.push({ type: "text", text });
      for (const img of m.images) {
        parts.push({ type: "image_url", image_url: { url: img.dataUrl } });
      }
      return { role: m.role, content: parts };
    }
    return { role: m.role, content: m.content };
  });
}

export default function Chat() {
  const { t } = useTranslation();
  const { chats, openId, openNew } = useSyncExternalStore(
    chatStore.subscribe,
    chatStore.get,
  );
  const genSnap = useSyncExternalStore(
    generationStore.subscribe,
    generationStore.get,
  );

  const [activeChatId, setActiveChatId] = useState<number | null>(null);
  const [messages, setMessages] = useState<UiMessage[]>([]);
  const [models, setModels] = useState<string[] | null>(null);
  const [selectedModel, setSelectedModel] = useState("");
  const [draft, setDraft] = useState("");
  const [params, setParams] = useState<ChatParams>(DEFAULT_CHAT_PARAMS);
  const [paramsOpen, setParamsOpen] = useState(false);
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [attachError, setAttachError] = useState<string | null>(null);
  const [dragOver, setDragOver] = useState(false);
  const [editingIdx, setEditingIdx] = useState<number | null>(null);
  const [workspaceFiles, setWorkspaceFiles] = useState<WorkspaceFile[]>([]);

  const busyRef = useRef(false);
  // "Época" da conversa exibida: troca/criação invalida listMessages stale.
  const convEpochRef = useRef(0);
  // Conversas excluídas na sessão: não persistir mensagens nelas (FK).
  const deletedChatsRef = useRef<Set<number>>(new Set());
  // Ids já vistos na lista — um id novo (ainda não listado) não é exclusão.
  const seenChatIdsRef = useRef<Set<number>>(new Set());
  // Pula o persist com debounce quando os params acabaram de ser CARREGADOS.
  const paramsSkipRef = useRef(false);
  const attachErrTimerRef = useRef(0);

  const canPersistId = (id: number | null): id is number =>
    id != null && !deletedChatsRef.current.has(id);

  useEffect(() => {
    let cancelled = false;
    void listChats()
      .then((rows) => {
        if (!cancelled) chatStore.setChats(rows);
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
    };
  }, []);

  useEffect(() => {
    chatStore.setActive(activeChatId);
  }, [activeChatId]);

  // Persistência dos parâmetros da conversa ativa (debounce ~500 ms).
  useEffect(() => {
    if (paramsSkipRef.current) {
      paramsSkipRef.current = false;
      return;
    }
    const id = activeChatId;
    if (!canPersistId(id)) return;
    const timer = window.setTimeout(() => {
      void setChatParams(id, JSON.stringify(params)).catch(() => {});
    }, 500);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [params]);

  const resetComposer = () => {
    setDraft("");
    setAttachments([]);
    setEditingIdx(null);
  };

  const selectChat = async (chat: ChatRow) => {
    convEpochRef.current++;
    const epoch = convEpochRef.current;
    setActiveChatId(chat.id);
    resetComposer();
    if (chat.modelId) setSelectedModel(chat.modelId);

    // Parâmetros da conversa (paramsJson) — carregar não deve re-persistir.
    paramsSkipRef.current = true;
    let loaded = DEFAULT_CHAT_PARAMS;
    if (chat.paramsJson) {
      try {
        loaded = {
          ...DEFAULT_CHAT_PARAMS,
          ...(JSON.parse(chat.paramsJson) as Partial<ChatParams>),
        };
      } catch {
        // JSON corrompido — usa padrões
      }
    }
    setParams(loaded);

    try {
      const rows = await listMessages(chat.id);
      if (convEpochRef.current === epoch) setMessages(rows.map(rowToUi));
    } catch {
      if (convEpochRef.current === epoch) setMessages([]);
    }
  };

  const newChat = () => {
    convEpochRef.current++;
    setActiveChatId(null);
    setMessages([]);
    resetComposer();
    // Conversa nova usa o estado atual do painel de parâmetros.
  };

  useEffect(() => {
    if (openNew) {
      chatStore.clearPending();
      newChat();
      return;
    }
    if (openId == null) return;
    const row = chats.find((c) => c.id === openId);
    if (!row) return;
    chatStore.clearPending();
    void selectChat(row);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openId, openNew, chats]);

  useEffect(() => {
    for (const c of chats) seenChatIdsRef.current.add(c.id);
  }, [chats]);

  useEffect(() => {
    if (activeChatId == null) return;
    if (chats.some((c) => c.id === activeChatId)) return;
    // Conversa recém-criada ainda não entrou em `chats` — não abortar o job.
    if (!seenChatIdsRef.current.has(activeChatId)) return;
    deletedChatsRef.current.add(activeChatId);
    generationStore.markDeleted(activeChatId);
  }, [chats, activeChatId]);

  // ------------------------------------------------------------- anexos ---

  const showAttachError = (msg: string) => {
    setAttachError(msg);
    window.clearTimeout(attachErrTimerRef.current);
    attachErrTimerRef.current = window.setTimeout(
      () => setAttachError(null),
      4000,
    );
  };

  const addFiles = async (files: File[]) => {
    for (const file of files) {
      try {
        const att = await readAttachment(file);
        setAttachments((prev) => [...prev, att]);
      } catch (e) {
        if (e instanceof Error && e.message === "tooBig") {
          const max = classifyFile(file) === "image" ? "8 MB" : "300 KB";
          showAttachError(t("chat.attachTooBig", { max }));
        } else {
          showAttachError(t("chat.attachUnsupported"));
        }
      }
    }
  };

  const job = genSnap.jobs.find((j) => j.chatId === activeChatId);
  const generating =
    job != null && (job.state === "running" || job.state === "queued");
  const loadingModel = job?.loadingModel ?? false;

  const displayMessages: UiMessage[] = (() => {
    if (!job || (job.state !== "running" && job.state !== "queued")) {
      return messages;
    }
    const base =
      messages[messages.length - 1]?.role === "assistant"
        ? messages.slice(0, -1)
        : messages;
    return [
      ...base,
      {
        role: "assistant" as const,
        content: job.content,
        tokensPerSec: job.tokensPerSec,
        reasoning: job.reasoning || undefined,
        thinkingMs: job.thinkingMs,
        genTokens: job.genTokens,
        genMs: job.genMs,
        startedAt: job.startedAt,
        thinkStartedAt: job.thinkStartedAt,
        answerStartedAt: job.answerStartedAt,
      },
    ];
  })();

  useEffect(() => {
    if (!job || !activeChatId) return;
    if (job.state !== "done" && job.state !== "error") return;
    const epoch = convEpochRef.current;
    const errorOnly = Boolean(job.error && !job.content && !job.rowId);
    const errorText = job.error;
    const thinkingMs = job.thinkingMs;
    void (async () => {
      try {
        const rows = await listMessages(activeChatId);
        if (convEpochRef.current !== epoch) return;
        const ui = rows.map(rowToUi);
        if (thinkingMs != null) {
          for (let i = ui.length - 1; i >= 0; i--) {
            if (ui[i].role === "assistant") {
              ui[i] = { ...ui[i], thinkingMs };
              break;
            }
          }
        }
        if (errorOnly && errorText) {
          setMessages([
            ...ui,
            {
              role: "assistant",
              content: `${t("common.error")}: ${errorText}`,
              tokensPerSec: null,
              error: true,
            },
          ]);
        } else {
          setMessages(ui);
        }
      } catch {
        // listMessages falhou — o overlay já mostrou o snapshot
      }
      if (convEpochRef.current === epoch) generationStore.dismiss(activeChatId);
    })();
  }, [job?.id, job?.state, activeChatId, t]);

  // --------------------------------------------------------------- envio ---

  const handleSend = async () => {
    if (busyRef.current || !selectedModel) return;
    if (generationStore.isBusy(activeChatId)) return;
    const text = draft.trim();
    if (!text && attachments.length === 0) return;

    // Monta o conteúdo final: texto + blocos de arquivo + marcadores de imagem.
    const textAtts = attachments.filter((a) => a.kind === "text");
    const imageAtts = attachments.filter((a) => a.kind === "image");
    let composed = text;
    if (params.workspaceDir) {
      const seen = new Set<string>();
      for (const m of text.matchAll(/@([^\s@]+)/g)) {
        const rel = m[1];
        if (seen.has(rel)) continue;
        seen.add(rel);
        try {
          const body = await readWorkspaceFile(params.workspaceDir, rel);
          composed += `\n\n\`\`\`\n[arquivo: ${rel}]\n${body}\n\`\`\``;
        } catch {
          // @sem arquivo correspondente — envia só a menção
        }
      }
    }
    for (const a of textAtts) {
      composed += `\n\n\`\`\`\n[arquivo: ${a.name}]\n${a.data}\n\`\`\``;
    }
    const markers = imageAtts.map((a) => `\n\n[imagem: ${a.name}]`).join("");
    const content = (composed + markers).trim();
    if (!content) return;

    busyRef.current = true;
    const epoch = convEpochRef.current;
    const wasEditing = editingIdx;
    const images =
      imageAtts.length > 0
        ? imageAtts.map((a) => ({ name: a.name, dataUrl: a.data }))
        : undefined;
    resetComposer();

    try {
      let chatId = activeChatId;
      let history: UiMessage[];
      let created = false;

      if (wasEditing != null) {
        // Editar-e-reenviar: substitui a última user e descarta a resposta.
        const prev = messages;
        const original = prev[wasEditing];
        if (!original || original.role !== "user") {
          return;
        }
        const edited: UiMessage = {
          role: "user",
          content,
          tokensPerSec: null,
          rowId: original.rowId,
          images: images ?? original.images,
        };
        if (canPersistId(chatId) && original.rowId != null) {
          await updateMessage(original.rowId, content).catch(() => {});
        }
        // Descarta do DB tudo que vinha depois da mensagem editada
        // (normalmente só a resposta do assistente).
        for (const m of prev.slice(wasEditing + 1)) {
          if (m.rowId != null && canPersistId(chatId)) {
            await deleteMessage(m.rowId).catch(() => {});
          }
        }
        history = [...prev.slice(0, wasEditing), edited];
        if (convEpochRef.current === epoch) setMessages(history);
      } else {
        const userMsg: UiMessage = {
          role: "user",
          content,
          tokensPerSec: null,
          images,
        };
        history = [...messages, userMsg];
        if (convEpochRef.current === epoch) setMessages(history);

        // Conversa é criada na primeira mensagem; título = início do texto.
        if (chatId == null) {
          const title =
            content.length > 40 ? `${content.slice(0, 40)}…` : content;
          chatId = await createChat(title, selectedModel);
          created = true;
          chatStore.setChats([
            {
              id: chatId,
              title,
              modelId: selectedModel,
              createdAt: Date.now(),
              paramsJson: JSON.stringify(params),
            },
            ...chats.filter((c) => c.id !== chatId),
          ]);
          if (convEpochRef.current === epoch) setActiveChatId(chatId);
          void setChatParams(chatId, JSON.stringify(params)).catch(() => {});
          void listChats()
            .then(chatStore.setChats)
            .catch(() => {});
        }
        if (canPersistId(chatId)) {
          const rowId = await addMessage(chatId, "user", content).catch(
            () => 0,
          );
          if (rowId > 0) {
            history = [...messages, { ...userMsg, rowId }];
          }
        }
      }

      if (chatId == null) return;
      generationStore.start({
        chatId,
        messages: toApiMessages(history),
        model: selectedModel,
        params,
        autoTitle: created,
      });
    } catch (err) {
      const detail = err instanceof Error ? err.message : String(err);
      if (convEpochRef.current === epoch) {
        setMessages((prev) => [
          ...prev.filter((m) => !(m.role === "assistant" && !m.rowId && !m.content)),
          {
            role: "assistant",
            content: `${t("common.error")}: ${detail}`,
            tokensPerSec: null,
            error: true,
          },
        ]);
      }
    } finally {
      busyRef.current = false;
    }
  };

  // ------------------------------------------------- regenerar / editar ---

  const regenerate = async () => {
    if (busyRef.current || generating) return;
    const prev = messages;
    const last = prev[prev.length - 1];
    if (!last || last.role !== "assistant") return;
    const history = prev.slice(0, -1);
    if (history[history.length - 1]?.role !== "user") return;

    busyRef.current = true;
    const epoch = convEpochRef.current;
    const chatId = activeChatId;
    try {
      if (last.rowId != null && canPersistId(chatId)) {
        await deleteMessage(last.rowId).catch(() => {});
      }
      if (convEpochRef.current === epoch) setMessages(history);
      if (!canPersistId(chatId)) return;
      generationStore.start({
        chatId,
        messages: toApiMessages(history),
        model: selectedModel,
        params,
      });
    } finally {
      busyRef.current = false;
    }
  };

  const startEdit = (index: number) => {
    if (busyRef.current || generating) return;
    const msg = messages[index];
    if (!msg || msg.role !== "user") return;
    setEditingIdx(index);
    setDraft(msg.content);
    setAttachments([]);
  };

  const removeMessage = async (index: number) => {
    if (busyRef.current || generating) return;
    const epoch = convEpochRef.current;
    const msg = messages[index];
    if (!msg) return;
    if (msg.rowId != null && canPersistId(activeChatId)) {
      await deleteMessage(msg.rowId).catch(() => {});
    }
    if (convEpochRef.current === epoch) {
      setMessages((prev) => prev.filter((_, i) => i !== index));
      setEditingIdx((cur) => (cur === index ? null : cur));
    }
  };

  // ---------------------------------------------------------------- UI ---

  const noModels =
    models !== null && models.length === 0 && selectedModel === "";

  return (
    <div className="flex h-full">
      <div className="flex min-w-0 flex-1 flex-col">
        <WorkspaceHost
          dir={params.workspaceDir}
          onDirChange={(workspaceDir) =>
            setParams((p) => ({ ...p, workspaceDir }))
          }
          files={workspaceFiles}
          onFiles={setWorkspaceFiles}
          disabled={generating}
        >
        <div className="flex min-h-0 flex-1">
          <div
            className="relative flex min-w-0 flex-1 flex-col"
            onDragOver={(e) => {
              if (e.dataTransfer.types.includes("Files")) {
                e.preventDefault();
                setDragOver(true);
              }
            }}
            onDragLeave={(e) => {
              if (!e.currentTarget.contains(e.relatedTarget as Node)) {
                setDragOver(false);
              }
            }}
            onDrop={(e) => {
              e.preventDefault();
              setDragOver(false);
              const files = Array.from(e.dataTransfer.files);
              if (files.length > 0) void addFiles(files);
            }}
          >
            {dragOver && (
              <div className="pointer-events-none absolute inset-2 z-20 flex items-center justify-center rounded-2xl border-2 border-dashed border-accent bg-accent/10 text-sm font-medium text-accent">
                {t("chat.attach")}
              </div>
            )}

            <div className="absolute top-3 right-4 z-10 flex items-center gap-2">
              <WorkspaceToggle />
              <button
                onClick={() => setParamsOpen((o) => !o)}
                title={t("chat.params")}
                className={`flex h-8 w-8 items-center justify-center rounded-full border transition-colors ${
                  paramsOpen
                    ? "border-accent text-accent"
                    : "border-edge text-dim hover:border-accent hover:text-ink"
                }`}
              >
                <svg
                  className="h-4 w-4"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.8"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  viewBox="0 0 24 24"
                >
                  <circle cx="12" cy="12" r="3" />
                  <path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 11-2.83 2.83l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 11-4 0v-.09a1.65 1.65 0 00-1-1.51 1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 11-2.83-2.83l.06-.06a1.65 1.65 0 00.33-1.82 1.65 1.65 0 00-1.51-1H3a2 2 0 110-4h.09a1.65 1.65 0 001.51-1 1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 112.83-2.83l.06.06a1.65 1.65 0 001.82.33h0a1.65 1.65 0 001-1.51V3a2 2 0 114 0v.09a1.65 1.65 0 001 1.51h0a1.65 1.65 0 001.82-.33l.06-.06a2 2 0 112.83 2.83l-.06.06a1.65 1.65 0 00-.33 1.82v0a1.65 1.65 0 001.51 1H21a2 2 0 110 4h-.09a1.65 1.65 0 00-1.51 1z" />
                </svg>
              </button>
            </div>

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
            ) : displayMessages.length === 0 && !generating && !loadingModel ? (
              <ChatHero onPick={setDraft} />
            ) : (
              <MessageList
                messages={displayMessages}
                generating={generating}
                loadingModel={loadingModel}
                onRegenerate={() => void regenerate()}
                onEditResend={startEdit}
                onDeleteMsg={(i) => void removeMessage(i)}
              />
            )}

            {!noModels && (
              <div className="shrink-0">
                <Composer
                  draft={draft}
                  onDraftChange={setDraft}
                  onSend={() => void handleSend()}
                  onStop={() => generationStore.cancel(activeChatId)}
                  generating={generating}
                  disabled={!selectedModel}
                  attachments={attachments}
                  onAttachFiles={(files) => void addFiles(files)}
                  onRemoveAttachment={(id) =>
                    setAttachments((prev) => prev.filter((a) => a.id !== id))
                  }
                  attachError={attachError}
                  editing={editingIdx != null}
                  onCancelEdit={resetComposer}
                  workspaceFiles={workspaceFiles}
                  startActions={<WorkspaceTrigger />}
                  leftActions={
                    <ApprovalSelect
                      params={params}
                      onChange={setParams}
                      disabled={generating}
                    />
                  }
                  rightActions={
                    <ModelSelect
                      models={models ?? []}
                      value={selectedModel}
                      onChange={setSelectedModel}
                      params={params}
                      onParamsChange={setParams}
                      disabled={generating}
                    />
                  }
                />
              </div>
            )}
          </div>

          <WorkspaceExplorer />
          {paramsOpen && <ParamsPanel params={params} onChange={setParams} />}
        </div>
        </WorkspaceHost>
      </div>
    </div>
  );
}
