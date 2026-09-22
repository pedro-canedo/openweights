// Jobs de geração em fundo: o stream vive aqui, não no Chat.
// Trocar de tela/conversa não aborta — só o botão Parar (por chatId).

import { addMessage, listChats, renameChat, getSetting, getModelProfile } from "./api";
import { chatStore } from "./chatStore";
import {
  ensureEndpoint,
  errorMessage,
  listLoadedModels,
  matchServerModel,
  modelsMax,
  visionModelFor,
} from "./serverSession";
import { splitModelRef } from "./providers";
import { jevDecideEffort, summarizeForJev } from "./jev";
import {
  completeOnce,
  streamChat,
  type ChatMessage,
} from "./llama";
import type { ChatParams, EffortLevel } from "./types";

export type JobState = "queued" | "running" | "done" | "error";

export type GenerationPhase = "queued" | "preparing" | "loading" | "waiting" | "generating" | "done" | "error";
export interface GenerationMetrics {
  runId: string; phase: GenerationPhase; queueMs: number | null;
  firstTokenMs: number | null; firstAnswerMs: number | null; totalMs: number | null;
  promptTokens: number | null; cachedTokens: number | null; promptTps: number | null;
  thinkingMs: number | null;
  /** Presente quando um decisor (local ou o Jev remoto) decidiu o esforço desta resposta. */
  jev?: { effort: EffortLevel; confidence: number | null; source?: "local" | "jev" };
}
export interface GenerationJob {
  createdAt: number;
  metrics: GenerationMetrics;
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
  /** A resposta não chegou ao banco: sumiria ao reabrir a conversa. */
  unsaved?: boolean;
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
  published: GenerationJob;
  timer?: ReturnType<typeof setTimeout>;
  retryAt: number;
  executing: boolean;
}

const MAX_RETRIES = 3;
const deletedChats = new Set<number>();
const internals = new Map<number, InternalJob>();

let snap: GenerationSnapshot = { jobs: [] };
const listeners = new Set<() => void>();

const chatListeners = new Map<number, Set<() => void>>();
let summaryKey = "";
let paused = false;
let pumping = false;
let pumpAgain = false;
const titles: Array<() => Promise<void>> = [];
let titleAbort: AbortController | null = null;
function emit(chatId?: number): void {
  if (chatId != null) for (const fn of chatListeners.get(chatId) ?? []) fn();
  const rows = [...internals.values()].map(j => j.published);
  const key = JSON.stringify(rows.map(j => [j.id, j.state, j.model, j.error, j.unsaved]));
  if (key !== summaryKey) {
    summaryKey = key; snap = { jobs: rows };
    for (const fn of listeners) fn();
  }
}
function publish(chatId: number): void {
  const row = internals.get(chatId);
  if (!row) return;
  clearTimeout(row.timer); row.timer = undefined;
  row.published = row.public; emit(chatId);
}
function patch(chatId: number, p: Partial<GenerationJob>, deferred = false): void {
  const row = internals.get(chatId);
  if (!row) return;
  row.public = { ...row.public, ...p };
  if (!deferred) publish(chatId);
  else row.timer ??= setTimeout(() => publish(chatId), 50);
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

function isRetryable(e: unknown): boolean {
  const msg = errorMessage(e);
  return /HTTP 503|HTTP 529|no slot|slots? (full|busy)|unavailable|loading|try again/i.test(
    msg,
  );
}

async function autoTitle(
  chatId: number,
  baseUrl: string,
  model: string,
  history: ChatMessage[],
  assistantText: string,
  headers?: Record<string, string>,
  signal?: AbortSignal,
): Promise<void> {
  try {
    const raw = await completeOnce({
      baseUrl,
      headers,
      model,
      messages: [
        ...history.filter(m => m.role === "user").slice(0, 1).map(m => ({ role: "user", content: typeof m.content === "string" ? m.content.slice(0, 1200) : "Conversa com imagem" })),
        { role: "assistant", content: assistantText.slice(0, 1200) },
        {
          role: "user",
          content:
            "Gere um título curto (máximo 5 palavras) para a conversa acima. Responda apenas o título.",
        },
      ],
      signal,
      maxTokens: 24,
      temperature: 0.3,
    });
    const title = cleanTitle(raw);
    if (!title || signal?.aborted || deletedChats.has(chatId)) return;
    await renameChat(chatId, title);
    void listChats()
      .then(chatStore.setChats)
      .catch(() => {});
  } catch (e) {
    console.warn("auto-título falhou:", e);
  }
}

/**
 * Grava a resposta na conversa — e não desiste calado.
 *
 * Antes, uma falha aqui virava `.catch(() => 0)`: a resposta continuava na
 * tela, e sumia ao reabrir a conversa. Quem viu isso não tinha como saber se
 * o app perdeu ou nunca gravou. Agora tenta de novo e, quando não dá, o job
 * carrega `unsaved` — a tela diz que aquela resposta não foi guardada,
 * enquanto o texto ainda está ali para ser copiado.
 */
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
  if (!text && !reasoning) return undefined;
  if (!canPersist(chatId)) {
    console.warn(`resposta não gravada: conversa ${chatId} foi excluída`);
    patch(chatId, { unsaved: true });
    return undefined;
  }
  const conteudo = toDbContent(text, reasoning);
  for (let tentativa = 1; tentativa <= 3; tentativa++) {
    try {
      const id = await addMessage(
        chatId,
        "assistant",
        conteudo,
        tokensPerSec,
        genTokens,
        genMs,
        model,
        internals.get(chatId)?.public.metrics ?? null,
      );
      if (id > 0) {
        patch(chatId, { unsaved: false });
        return id;
      }
    } catch (e) {
      console.error(`falha ${tentativa}/3 ao gravar a resposta:`, e);
    }
    await new Promise((r) => setTimeout(r, 200 * tentativa));
  }
  patch(chatId, { unsaved: true });
  return undefined;
}

let pumpTimer: ReturnType<typeof setTimeout> | undefined;
function schedulePump(delay = 0): void {
  if (pumpTimer != null) return;
  pumpTimer = setTimeout(() => { pumpTimer = undefined; void pump(); }, delay);
}
async function pump(): Promise<void> {
  if (paused) return;
  if (pumping) { pumpAgain = true; return; }
  pumping = true;
  try {
    const queued = [...internals.values()].filter(j => j.public.state === "queued" && !j.executing && !j.abort.signal.aborted && j.retryAt <= performance.now());
    const localQueued = queued.some(j => splitModelRef(j.opts.model).provider === "local");
    const configured = localQueued ? Number(await getSetting("server_parallel").catch(() => null)) : 1;
    const max = localQueued ? await modelsMax() : 1;
    const slots = Number.isFinite(configured) && configured >= 1 ? configured : 1;
    if (paused) return;
    for (const row of queued) {
      if (internals.get(row.opts.chatId) !== row || row.abort.signal.aborted) continue;
      if (splitModelRef(row.opts.model).provider === "local") {
        const running = [...internals.values()].filter(j => j.executing && splitModelRef(j.opts.model).provider === "local");
        const names = new Set(running.map(j => j.opts.model));
        const profile = await getModelProfile(row.opts.model).catch(() => null);
        const capacity = profile?.parallel ?? slots;
        if (paused || row.abort.signal.aborted) continue;
        if (titleAbort || running.filter(j => j.opts.model === row.opts.model).length >= Math.max(1, capacity)) continue;
        if (!names.has(row.opts.model) && names.size >= max) continue;
      }
      row.executing = true;
      void runJob(row);
    }
    if (!paused && !titleAbort && ![...internals.values()].some(j => j.executing || j.public.state === "queued")) {
      const title = titles.shift();
      if (title) {
        titleAbort = new AbortController();
        void title().finally(() => { titleAbort = null; schedulePump(); });
      }
    }
  } finally { pumping = false; if (pumpAgain) { pumpAgain = false; schedulePump(); } }
}
async function runJob(row: InternalJob): Promise<void> {
  const chatId = row.opts.chatId;
  const now = performance.now();
  patch(chatId, { state: "running", startedAt: now, metrics: { ...row.public.metrics, phase: "preparing", queueMs: now - row.public.createdAt } });
  const cancelled = () => row.abort.signal.aborted || internals.get(chatId) !== row;
  let stopStatus = () => {};
  try {
    const { provider, baseUrl, headers } = await ensureEndpoint(row.opts.model);
    if (cancelled()) return;
    let resolved = splitModelRef(row.opts.model).model;
    if (provider === "local") {
      const loaded = await listLoadedModels(baseUrl, headers, row.abort.signal);
      if (cancelled()) return;
      const image = row.opts.messages.some(m => Array.isArray(m.content) && m.content.some(p => p.type === "image_url"));
      resolved = matchServerModel(image ? visionModelFor(row.opts.model, loaded) : row.opts.model, loaded);
    }
    // O Jev decide o esforço desta requisição antes de ela sair — só para
    // modelo local (é o único cujo raciocínio o app controla) e só quando o
    // backend diz que decidiu; qualquer outra coisa mantém o da conversa.
    let params = row.opts.params;
    if (provider === "local" && params?.effort) {
      const d = await jevDecideEffort(resolved, summarizeForJev(row.opts.messages), params.effort);
      if (cancelled()) return;
      if (d && d.source !== "padrao") {
        params = { ...params, effort: d.effort };
        patch(chatId, { metrics: { ...row.public.metrics, jev: { effort: d.effort, confidence: d.confidence, source: d.source } } });
      }
    }
    // Sem evento confirmado do motor, espera não significa carregamento.
    patch(chatId, { metrics: { ...row.public.metrics, phase: "waiting" } });
    if (provider === "local") {
      let stopped = false;
      let timer: ReturnType<typeof setTimeout>;
      const poll = async () => {
        if (stopped || cancelled() || row.public.metrics.firstTokenMs != null) return;
        try {
          const response = await fetch(`${baseUrl}/models`, { headers, signal: AbortSignal.any([row.abort.signal, AbortSignal.timeout(2000)]) });
          if (response.ok) {
            const data = await response.json() as { data?: { id: string; status?: { value?: string } }[] };
            const state = data.data?.find(m => m.id === resolved)?.status?.value;
            if (!stopped && !cancelled() && row.public.metrics.firstTokenMs == null) {
              const phase = state === "loading" ? "loading" : "waiting";
              if (row.public.metrics.phase !== phase) patch(chatId, { loadingModel: phase === "loading", metrics: { ...row.public.metrics, phase } });
            }
          }
        } catch { /* A observação não pode derrubar o stream. */ }
        if (!stopped) timer = setTimeout(() => void poll(), 500);
      };
      timer = setTimeout(() => void poll(), 500);
      stopStatus = () => { stopped = true; clearTimeout(timer); };
    }
    const delta = (text: string, reasoning: boolean) => {
      if (cancelled()) return;
      const at = performance.now();
      const cur = row.public;
      const metrics = { ...cur.metrics, phase: "generating" as const, firstTokenMs: cur.metrics.firstTokenMs ?? at - cur.createdAt };
      if (!reasoning) metrics.firstAnswerMs ??= at - cur.createdAt;
      patch(chatId, {
        content: reasoning ? cur.content : cur.content + text,
        reasoning: reasoning ? cur.reasoning + text : cur.reasoning,
        thinkStartedAt: reasoning ? cur.thinkStartedAt ?? at : cur.thinkStartedAt,
        answerStartedAt: reasoning ? cur.answerStartedAt : cur.answerStartedAt ?? at,
        thinkingMs: cur.thinkStartedAt != null && cur.answerStartedAt == null ? at - cur.thinkStartedAt : cur.thinkingMs,
        genMs: !reasoning ? at - (cur.answerStartedAt ?? at) : cur.genMs,
        metrics,
        loadingModel: false,
      }, true);
    };
    const result = await streamChat({ baseUrl, headers, model: resolved, messages: row.opts.messages, params,
      signal: row.abort.signal, onDelta: d => delta(d, false), onReasoningDelta: d => delta(d, true) });
    if (cancelled()) return;
    patch(chatId, { content: result.content, reasoning: result.reasoning, thinkingMs: result.thinkingMs,
      tokensPerSec: result.tokensPerSec, genTokens: result.genTokens, genMs: result.genMs,
      metrics: { ...row.public.metrics, promptTokens: result.promptTokens, cachedTokens: result.cachedTokens, promptTps: result.promptTps, thinkingMs: result.thinkingMs } });
    if (row.opts.autoTitle) titles.push(async () => {
      if (deletedChats.has(chatId)) return;
      await autoTitle(chatId, baseUrl, resolved, row.opts.messages, result.content, headers, titleAbort?.signal);
    });
  } catch (err) {
    if (!cancelled()) {
      if (!row.public.content && !row.public.reasoning && isRetryable(err) && row.retries < MAX_RETRIES) {
        row.retries++;
        row.retryAt = performance.now() + 1000 * row.retries;
        patch(chatId, { state: "queued", metrics: { ...row.public.metrics, phase: "queued" } });
        return;
      }
      patch(chatId, { error: errorMessage(err) });
    }
  } finally {
    stopStatus();
    if (internals.get(chatId) === row && row.public.state !== "queued") {
      const metrics = { ...row.public.metrics, phase: row.public.error ? "error" as const : "done" as const, totalMs: performance.now() - row.public.createdAt };
      patch(chatId, { metrics });
      const cur = row.public;
      const rowId = await persistAssistant(chatId, cur.content, cur.reasoning, cur.tokensPerSec, cur.genTokens, cur.genMs, row.opts.model);
      patch(chatId, { state: cur.error ? "error" : "done", loadingModel: false, rowId });
    }
    row.executing = false;
    schedulePump();
    if (row.public.state === "queued") setTimeout(() => schedulePump(), Math.max(0, row.retryAt - performance.now()));
  }
}
export const generationStore = {
  subscribe(fn: () => void) { listeners.add(fn); return () => { listeners.delete(fn); }; },
  get: () => snap,
  subscribeChat(chatId: number | null, fn: () => void) {
    if (chatId == null) return () => {};
    const set = chatListeners.get(chatId) ?? new Set<() => void>();
    chatListeners.set(chatId, set); set.add(fn);
    return () => { set.delete(fn); if (!set.size) chatListeners.delete(chatId); };
  },
  jobFor(chatId: number | null) { return chatId == null ? undefined : internals.get(chatId)?.published; },
  activeJobs() { return [...internals.values()].map(j => j.published).filter(j => j.state === "queued" || j.state === "running"); },
  isBusy(chatId: number | null) { const j = this.jobFor(chatId); return j?.state === "queued" || j?.state === "running"; },
  // Trava síncrona antes do primeiro await da comparação.
  acquireBenchmark() {
    if (paused || titleAbort || [...internals.values()].some(j => j.executing || j.public.state === "queued")) throw new Error("engine-busy:chat");
    paused = true;
    return () => { paused = false; schedulePump(); };
  },
  start(opts: StartGenerationOpts): boolean {
    if (deletedChats.has(opts.chatId) || this.isBusy(opts.chatId)) return false;
    titleAbort?.abort();
    const id = crypto.randomUUID();
    const job: GenerationJob = { id, chatId: opts.chatId, model: opts.model, state: "queued", content: "", reasoning: "", thinkingMs: null,
      tokensPerSec: null, genTokens: null, genMs: null, loadingModel: false, startedAt: null, thinkStartedAt: null, answerStartedAt: null,
      createdAt: performance.now(), metrics: { runId: id, phase: "queued", queueMs: null, firstTokenMs: null, firstAnswerMs: null, totalMs: null, promptTokens: null, cachedTokens: null, promptTps: null, thinkingMs: null } };
    internals.set(opts.chatId, { public: job, published: job, opts, abort: new AbortController(), retries: 0, retryAt: 0, executing: false });
    emit(opts.chatId); schedulePump(); return true;
  },
  cancel(chatId: number | null) {
    const row = chatId == null ? undefined : internals.get(chatId);
    if (!row) return;
    row.abort.abort(); publish(row.opts.chatId);
    if (!row.executing) {
      patch(row.opts.chatId, { state: "done", metrics: { ...row.public.metrics, phase: "done", totalMs: performance.now() - row.public.createdAt } });
      schedulePump();
    }
  },
  dismiss(chatId: number) {
    const row = internals.get(chatId);
    if (!row || row.executing || this.isBusy(chatId)) return;
    clearTimeout(row.timer); internals.delete(chatId); emit(chatId);
  },
  markDeleted(chatId: number) {
    deletedChats.add(chatId);
    const row = internals.get(chatId);
    row?.abort.abort(); clearTimeout(row?.timer);
    internals.delete(chatId); emit(chatId); schedulePump();
  },
};
