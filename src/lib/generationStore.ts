// Jobs de geração em fundo: o stream vive aqui, não no Chat.
// Trocar de tela/conversa não aborta — só o botão Parar (por chatId).

import {
  addMessage,
  getServerStatus,
  getSetting,
  listChats,
  renameChat,
  startServer,
} from "./api";
import { chatStore } from "./chatStore";
import {
  completeOnce,
  streamChat,
  type ChatMessage,
} from "./llama";
import type { ChatParams } from "./types";

export type JobState = "queued" | "running" | "done" | "error";

export interface GenerationJob {
  id: string;
  chatId: number;
  model: string;
  state: JobState;
  content: string;
  reasoning: string;
  thinkingMs: number | null;
  tokensPerSec: number | null;
  genTokens: number | null;
  genMs: number | null;
  loadingModel: boolean;
  error?: string;
  rowId?: number;
}

export interface GenerationSnapshot {
  jobs: GenerationJob[];
}

export interface StartGenerationOpts {
  chatId: number;
  messages: ChatMessage[];
  model: string;
  params: ChatParams;
  autoTitle?: boolean;
}

interface InternalJob {
  public: GenerationJob;
  abort: AbortController;
  opts: StartGenerationOpts;
  retries: number;
}

const MAX_RETRIES = 8;
const deletedChats = new Set<number>();
const internals = new Map<number, InternalJob>();

let snap: GenerationSnapshot = { jobs: [] };
const listeners = new Set<() => void>();

function emit(): void {
  snap = { jobs: [...internals.values()].map((j) => j.public) };
  for (const fn of listeners) fn();
}

function patch(chatId: number, p: Partial<GenerationJob>): void {
  const row = internals.get(chatId);
  if (!row) return;
  row.public = { ...row.public, ...p };
  emit();
}

function canPersist(chatId: number): boolean {
  return !deletedChats.has(chatId);
}

function toDbContent(text: string, reasoning: string): string {
  return reasoning ? `<think>${reasoning}</think>\n${text}` : text;
}

function cleanTitle(raw: string): string {
  return raw
    .replace(/\s+/g, " ")
    .replace(/^["'“”‘’«»\s]+/, "")
    .replace(/["'“”‘’«»\s.!?…:;,]+$/, "")
    .trim()
    .slice(0, 80);
}

function isAbortError(e: unknown): boolean {
  return (
    (e instanceof DOMException && e.name === "AbortError") ||
    (e instanceof Error && e.name === "AbortError")
  );
}

function isRetryable(e: unknown): boolean {
  const msg = e instanceof Error ? e.message : String(e);
  return /HTTP 503|HTTP 529|no slot|slots? (full|busy)|unavailable|loading|try again/i.test(
    msg,
  );
}

function matchServerModel(wanted: string, ids: string[]): string {
  if (ids.includes(wanted)) return wanted;
  const stem = wanted.replace(/\.gguf$/i, "");
  const hit = ids.find((id) => {
    const idStem = id.replace(/\.gguf$/i, "");
    return (
      id === stem ||
      idStem === stem ||
      id.endsWith(`/${wanted}`) ||
      id.endsWith(`/${stem}`) ||
      idStem.endsWith(`/${stem}`)
    );
  });
  return hit ?? wanted;
}

async function listLoadedModels(baseUrl: string): Promise<string[]> {
  try {
    const res = await fetch(`${baseUrl}/v1/models`);
    if (!res.ok) return [];
    const json = (await res.json()) as { data?: { id: string }[] };
    return (json.data ?? []).map((d) => d.id);
  } catch {
    return [];
  }
}

async function modelsMax(): Promise<number> {
  const raw = await getSetting("server_models_max").catch(() => null);
  const n = raw ? Number(raw) : 2;
  return Number.isFinite(n) && n >= 1 ? n : 2;
}

function runningModels(): Set<string> {
  const out = new Set<string>();
  for (const j of internals.values()) {
    if (j.public.state === "running") out.add(j.public.model);
  }
  return out;
}

/** Fila só se o modelo é novo e já estamos no teto de modelos distintos. */
function mustQueue(model: string, loaded: string[], max: number): boolean {
  if (loaded.some((id) => id === model || matchServerModel(model, [id]) === id)) {
    return false;
  }
  if ([...runningModels()].some((m) => matchServerModel(model, [m]) === m)) {
    return false;
  }
  const distinct = new Set<string>([...runningModels(), ...loaded]);
  return distinct.size >= max;
}

async function autoTitle(
  chatId: number,
  baseUrl: string,
  model: string,
  history: ChatMessage[],
  assistantText: string,
): Promise<void> {
  try {
    const raw = await completeOnce({
      baseUrl,
      model,
      messages: [
        ...history,
        { role: "assistant", content: assistantText },
        {
          role: "user",
          content:
            "Gere um título curto (máximo 5 palavras) para a conversa acima. Responda apenas o título.",
        },
      ],
      maxTokens: 24,
      temperature: 0.3,
    });
    const title = cleanTitle(raw);
    if (!title || deletedChats.has(chatId)) return;
    await renameChat(chatId, title);
    void listChats()
      .then(chatStore.setChats)
      .catch(() => {});
  } catch (e) {
    console.warn("auto-título falhou:", e);
  }
}

async function persistAssistant(
  chatId: number,
  text: string,
  reasoning: string,
  tokensPerSec: number | null,
  genTokens: number | null,
  genMs: number | null,
): Promise<number | undefined> {
  if (!canPersist(chatId) || (!text && !reasoning)) return undefined;
  const id = await addMessage(
    chatId,
    "assistant",
    toDbContent(text, reasoning),
    tokensPerSec,
    genTokens,
    genMs,
  ).catch(() => 0);
  return id > 0 ? id : undefined;
}

async function runJob(chatId: number): Promise<void> {
  const row = internals.get(chatId);
  if (!row || row.public.state === "running") return;
  if (row.abort.signal.aborted) {
    internals.delete(chatId);
    emit();
    return;
  }

  patch(chatId, { state: "running", loadingModel: true, error: undefined });

  let status = await getServerStatus();
  if (!status.running || !status.baseUrl) {
    status = await startServer();
  }
  if (!status.baseUrl) {
    patch(chatId, {
      state: "error",
      loadingModel: false,
      error: "Servidor local indisponível.",
    });
    pump();
    return;
  }
  const baseUrl = status.baseUrl;

  const loaded = await listLoadedModels(baseUrl);
  const resolved = matchServerModel(row.opts.model, loaded);
  const max = await modelsMax();
  const othersRunning = [...internals.values()].some(
    (j) => j.public.chatId !== chatId && j.public.state === "running",
  );
  if (othersRunning && mustQueue(resolved, loaded, max)) {
    patch(chatId, { state: "queued", model: resolved, loadingModel: false });
    return;
  }

  patch(chatId, { model: resolved });

  let firstReasonAt = 0;
  let lastReasonAt = 0;
  let slowTimer = window.setTimeout(
    () => patch(chatId, { loadingModel: true }),
    2000,
  );

  try {
    const result = await streamChat({
      baseUrl,
      model: resolved,
      messages: row.opts.messages,
      params: row.opts.params,
      signal: row.abort.signal,
      onDelta: (delta) => {
        window.clearTimeout(slowTimer);
        const cur = internals.get(chatId);
        if (!cur) return;
        patch(chatId, {
          content: cur.public.content + delta,
          loadingModel: false,
        });
      },
      onReasoningDelta: (delta) => {
        const now = performance.now();
        if (firstReasonAt === 0) firstReasonAt = now;
        lastReasonAt = now;
        window.clearTimeout(slowTimer);
        const cur = internals.get(chatId);
        if (!cur) return;
        patch(chatId, {
          reasoning: cur.public.reasoning + delta,
          loadingModel: false,
          thinkingMs: Math.round(lastReasonAt - firstReasonAt),
        });
      },
    });

    window.clearTimeout(slowTimer);
    const thinkingMs =
      result.thinkingMs ??
      (firstReasonAt > 0 ? Math.round(lastReasonAt - firstReasonAt) : null);
    const rowId = await persistAssistant(
      chatId,
      result.content,
      result.reasoning,
      result.tokensPerSec,
      result.genTokens,
      result.genMs,
    );
    patch(chatId, {
      state: "done",
      content: result.content,
      reasoning: result.reasoning,
      thinkingMs,
      tokensPerSec: result.tokensPerSec,
      genTokens: result.genTokens,
      genMs: result.genMs,
      loadingModel: false,
      rowId,
    });
    if (row.opts.autoTitle && canPersist(chatId)) {
      void autoTitle(chatId, baseUrl, resolved, row.opts.messages, result.content);
    }
  } catch (err) {
    window.clearTimeout(slowTimer);
    if (isAbortError(err)) {
      const cur = internals.get(chatId);
      const text = cur?.public.content ?? "";
      const reasoning = cur?.public.reasoning ?? "";
      const thinkingMs =
        firstReasonAt > 0 ? Math.round(lastReasonAt - firstReasonAt) : null;
      const rowId = await persistAssistant(chatId, text, reasoning, null, null, null);
      if (cur) {
        patch(chatId, {
          state: "done",
          thinkingMs,
          loadingModel: false,
          rowId,
        });
      }
    } else if (isRetryable(err) && row.retries < MAX_RETRIES) {
      row.retries += 1;
      patch(chatId, { state: "queued", loadingModel: false });
      const others = [...internals.values()].some(
        (j) => j.public.chatId !== chatId && j.public.state === "running",
      );
      if (!others) {
        window.setTimeout(() => void pump(), 2000);
      }
    } else {
      const detail =
        err instanceof Error ? err.message : typeof err === "string" ? err : String(err);
      const cur = internals.get(chatId);
      if (cur && (cur.public.content || cur.public.reasoning)) {
        const rowId = await persistAssistant(
          chatId,
          cur.public.content,
          cur.public.reasoning,
          null,
          null,
          null,
        );
        patch(chatId, { state: "error", error: detail, loadingModel: false, rowId });
      } else {
        patch(chatId, { state: "error", error: detail, loadingModel: false });
      }
    }
  } finally {
    window.clearTimeout(slowTimer);
    pump();
  }
}

function pickNext(): InternalJob | null {
  const queued = [...internals.values()].filter((j) => j.public.state === "queued");
  if (queued.length === 0) return null;
  const running = runningModels();
  const same = queued.find((j) => running.has(j.public.model));
  return same ?? queued[0];
}

function pump(): void {
  const next = pickNext();
  if (!next) return;
  void (async () => {
    let status = await getServerStatus();
    if (!status.baseUrl && !status.running) {
      status = await startServer().catch(() => status);
    }
    const loaded = status.baseUrl ? await listLoadedModels(status.baseUrl) : [];
    const max = await modelsMax();
    const model = matchServerModel(next.opts.model, loaded);
    if (mustQueue(model, loaded, max)) {
      const hasRunning = [...internals.values()].some(
        (j) => j.public.state === "running",
      );
      if (hasRunning) return;
    }
    void runJob(next.public.chatId);
  })();
}

export const generationStore = {
  subscribe(fn: () => void): () => void {
    listeners.add(fn);
    return () => {
      listeners.delete(fn);
    };
  },
  get(): GenerationSnapshot {
    return snap;
  },
  jobFor(chatId: number | null): GenerationJob | undefined {
    if (chatId == null) return undefined;
    return internals.get(chatId)?.public;
  },
  activeJobs(): GenerationJob[] {
    return snap.jobs.filter((j) => j.state === "queued" || j.state === "running");
  },
  isBusy(chatId: number | null): boolean {
    const j = this.jobFor(chatId);
    return j != null && (j.state === "queued" || j.state === "running");
  },
  start(opts: StartGenerationOpts): boolean {
    if (deletedChats.has(opts.chatId)) return false;
    const existing = internals.get(opts.chatId);
    if (existing && (existing.public.state === "queued" || existing.public.state === "running")) {
      return false;
    }
    const id = `${opts.chatId}-${Date.now()}`;
    const row: InternalJob = {
      abort: new AbortController(),
      retries: 0,
      opts,
      public: {
        id,
        chatId: opts.chatId,
        model: opts.model,
        state: "queued",
        content: "",
        reasoning: "",
        thinkingMs: null,
        tokensPerSec: null,
        genTokens: null,
        genMs: null,
        loadingModel: false,
      },
    };
    internals.set(opts.chatId, row);
    emit();
    void runJob(opts.chatId);
    return true;
  },
  cancel(chatId: number | null): void {
    if (chatId == null) return;
    const row = internals.get(chatId);
    if (!row) return;
    if (row.public.state === "queued") {
      internals.delete(chatId);
      emit();
      pump();
      return;
    }
    row.abort.abort();
  },
  dismiss(chatId: number): void {
    const row = internals.get(chatId);
    if (!row) return;
    if (row.public.state === "queued" || row.public.state === "running") return;
    internals.delete(chatId);
    emit();
  },
  markDeleted(chatId: number): void {
    deletedChats.add(chatId);
    const row = internals.get(chatId);
    if (!row) return;
    row.abort.abort();
    internals.delete(chatId);
    emit();
  },
};

