// Jobs de geração em fundo: o stream vive aqui, não no Chat.
// Trocar de tela/conversa não aborta — só o botão Parar (por chatId).

import { addMessage, listChats, renameChat } from "./api";
import { chatStore } from "./chatStore";
import {
  ensureServer,
  errorMessage,
  listLoadedModels,
  matchServerModel,
  modelsMax,
  visionModelFor,
} from "./serverSession";
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
  /** performance.now() quando o job entra em running (relógio da UI). */
  startedAt: number | null;
  thinkStartedAt: number | null;
  answerStartedAt: number | null;
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
  const msg = errorMessage(e);
  return /HTTP 503|HTTP 529|no slot|slots? (full|busy)|unavailable|loading|try again/i.test(
    msg,
  );
}

function runningJobs(chatId?: number): boolean {
  for (const j of internals.values()) {
    if (j.public.chatId === chatId) continue;
    if (j.public.state === "running") return true;
  }
  return false;
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
  /** Modelo que gerou — o backend carimba a configuração vigente dele. */
  model: string | null = null,
): Promise<number | undefined> {
  if (!canPersist(chatId) || (!text && !reasoning)) return undefined;
  const id = await addMessage(
    chatId,
    "assistant",
    toDbContent(text, reasoning),
    tokensPerSec,
    genTokens,
    genMs,
    model,
  ).catch(() => 0);
  return id > 0 ? id : undefined;
}

/**
 * Parar durante a preparação (subir servidor / listar modelos): não há fetch
 * em voo para o AbortController cancelar, então encerramos o job na mão —
 * senão ele fica preso em "running" e a conversa nunca destrava.
 */
function finishIfAborted(chatId: number): boolean {
  const row = internals.get(chatId);
  if (!row || !row.abort.signal.aborted) return false;
  patch(chatId, { state: "done", loadingModel: false });
  return true;
}

async function runJob(chatId: number): Promise<void> {
  const row = internals.get(chatId);
  if (!row || row.public.state === "running") return;
  if (row.abort.signal.aborted) {
    internals.delete(chatId);
    emit();
    pump();
    return;
  }

  patch(chatId, {
    state: "running",
    loadingModel: true,
    error: undefined,
    startedAt: performance.now(),
    thinkStartedAt: null,
    answerStartedAt: null,
    thinkingMs: null,
    genMs: null,
  });

  let firstReasonAt = 0;
  let lastReasonAt = 0;
  let slowTimer = 0;
  // Voltou para a fila: quem acorda este job é o término do job concorrente
  // (ou o `schedulePump` abaixo) — chamar `pump()` aqui seria um ping-pong.
  let requeued = false;

  try {
    const { baseUrl } = await ensureServer();
    if (finishIfAborted(chatId)) return;

    const loaded = await listLoadedModels(baseUrl);
    const max = await modelsMax();
    if (finishIfAborted(chatId)) return;

    // Mensagem com imagem vai para a entrada que carrega o projetor de
    // visão, quando ela existe. É o próprio roteador que troca entre as
    // duas, então o projetor só ocupa a placa quando há imagem.
    const temImagem = row.opts.messages.some(
      (m) =>
        Array.isArray(m.content) &&
        m.content.some((parte) => parte.type === "image_url"),
    );
    const pedido = temImagem
      ? visionModelFor(row.opts.model, loaded)
      : row.opts.model;
    const resolved = matchServerModel(pedido, loaded);
    if (runningJobs(chatId) && mustQueue(resolved, loaded, max)) {
      requeued = true;
      patch(chatId, {
        state: "queued",
        model: resolved,
        loadingModel: false,
        startedAt: null,
      });
      // Corrida: o job concorrente pode ter terminado (e chamado `pump()`)
      // entre o teste acima e este patch — sem re-agendar, este job ficaria
      // órfão na fila para sempre.
      if (!runningJobs()) schedulePump();
      return;
    }

    patch(chatId, { model: resolved });

    slowTimer = window.setTimeout(
      () => patch(chatId, { loadingModel: true }),
      2000,
    );

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
        const now = performance.now();
        const answerStartedAt = cur.public.answerStartedAt ?? now;
        const thinkOrigin =
          cur.public.thinkStartedAt ?? cur.public.startedAt ?? firstReasonAt;
        patch(chatId, {
          content: cur.public.content + delta,
          loadingModel: false,
          answerStartedAt,
          genMs: Math.round(now - answerStartedAt),
          thinkingMs:
            thinkOrigin > 0
              ? Math.round(answerStartedAt - thinkOrigin)
              : cur.public.thinkingMs,
        });
      },
      onReasoningDelta: (delta) => {
        const now = performance.now();
        if (firstReasonAt === 0) firstReasonAt = now;
        lastReasonAt = now;
        window.clearTimeout(slowTimer);
        const cur = internals.get(chatId);
        if (!cur) return;
        const thinkStartedAt = cur.public.thinkStartedAt ?? now;
        patch(chatId, {
          reasoning: cur.public.reasoning + delta,
          loadingModel: false,
          thinkStartedAt,
          thinkingMs: Math.round(now - thinkStartedAt),
        });
      },
      // tok/s ao vivo: sem isto a métrica só aparecia no fim do stream.
      onTokensPerSec: (tps) => patch(chatId, { tokensPerSec: tps }),
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
      resolved,
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
    // `signal.aborted` cobre o Parar que chegou durante uma etapa que não é
    // o fetch (subir servidor, listar modelos): o erro que borbulha ali não
    // é AbortError, mas o usuário cancelou — não é falha para exibir.
    if (isAbortError(err) || row.abort.signal.aborted) {
      const cur = internals.get(chatId);
      const text = cur?.public.content ?? "";
      const reasoning = cur?.public.reasoning ?? "";
      const now = performance.now();
      const thinkingMs =
        cur?.public.thinkingMs ??
        (firstReasonAt > 0 ? Math.round(lastReasonAt - firstReasonAt) : null);
      const genMs =
        cur?.public.genMs ??
        (cur?.public.answerStartedAt != null
          ? Math.round(now - cur.public.answerStartedAt)
          : null);
      const rowId = await persistAssistant(
        chatId,
        text,
        reasoning,
        cur?.public.tokensPerSec ?? null,
        null,
        genMs,
        cur?.public.model ?? null,
      );
      if (cur) {
        patch(chatId, {
          state: "done",
          thinkingMs,
          genMs,
          loadingModel: false,
          rowId,
        });
      }
    } else if (isRetryable(err) && row.retries < MAX_RETRIES) {
      row.retries += 1;
      requeued = true;
      patch(chatId, { state: "queued", loadingModel: false, startedAt: null });
      if (!runningJobs(chatId)) schedulePump(2000);
    } else {
      const detail = errorMessage(err);
      const cur = internals.get(chatId);
      if (cur && (cur.public.content || cur.public.reasoning)) {
        const rowId = await persistAssistant(
          chatId,
          cur.public.content,
          cur.public.reasoning,
          cur.public.tokensPerSec,
          null,
          null,
          cur.public.model ?? null,
        );
        patch(chatId, { state: "error", error: detail, loadingModel: false, rowId });
      } else {
        patch(chatId, { state: "error", error: detail, loadingModel: false });
      }
    }
  } finally {
    window.clearTimeout(slowTimer);
    if (!requeued) pump();
  }
}

function pickNext(): InternalJob | null {
  const queued = [...internals.values()].filter((j) => j.public.state === "queued");
  if (queued.length === 0) return null;
  const running = runningModels();
  const same = queued.find((j) => running.has(j.public.model));
  return same ?? queued[0];
}

let pumpTimer = 0;

/**
 * Um único `pump()` atrasado por vez. Serve às corridas em que ninguém mais
 * vai acordar a fila (job concorrente terminou cedo demais, retry de 503).
 */
function schedulePump(delayMs = 250): void {
  if (pumpTimer) return;
  pumpTimer = window.setTimeout(() => {
    pumpTimer = 0;
    pump();
  }, delayMs);
}

function pump(): void {
  const next = pickNext();
  if (!next) return;
  void (async () => {
    const session = await ensureServer().catch(() => null);
    const loaded = session ? await listLoadedModels(session.baseUrl) : [];
    const max = await modelsMax();
    const model = matchServerModel(next.opts.model, loaded);
    // Ainda no teto de modelos: só arrisca se não há mais nada rodando
    // (senão o job espera o término do concorrente, que chama `pump()`).
    if (mustQueue(model, loaded, max) && runningJobs()) return;
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
        startedAt: null,
        thinkStartedAt: null,
        answerStartedAt: null,
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
    // Em "running" o abort corta o fetch; se o stream ainda nem começou
    // (subindo servidor/modelo), `finishIfAborted` fecha o job na sequência.
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
    // Excluir a conversa some com o job sem passar pelo `finally` do runJob:
    // sem este pump, um job em fila esperando por ele ficaria parado.
    pump();
  },
};
