// Comunicação DIRETA do webview com o llama-server local via fetch/SSE.
// Decisão de arquitetura: tokens NUNCA passam pelo IPC do Tauri — o CSP já
// permite http://127.0.0.1:*. No navegador (dev sem Tauri) usamos um mock
// que "digita" uma resposta falsa para desenvolver a UI.

import { isTauri } from "./tauri";
import type { ChatParams, EffortLevel } from "./types";

/** Parte de conteúdo multimodal no formato OpenAI (aceito pelo llama-server com --mmproj). */
export type ContentPart =
  | { type: "text"; text: string }
  | { type: "image_url"; image_url: { url: string } };

/** Mensagem enviada ao endpoint /v1/chat/completions. */
export interface ChatMessage {
  role: string;
  content: string | ContentPart[];
}

export interface StreamChatOptions {
  baseUrl: string;
  /**
   * Cabeçalhos extras da requisição (autorização e atribuição dos provedores
   * remotos). Vazio no llama-server local, que não pede autenticação.
   */
  headers?: Record<string, string>;
  model: string;
  messages: ChatMessage[];
  signal: AbortSignal;
  /** Parâmetros de amostragem + system prompt da conversa. */
  params?: ChatParams;
  /**
   * `reasoning_effort` no vocabulário do template do modelo local (lido do
   * GGUF pelo backend: `xhigh` no Qwen3.8/Bonsai 2). Ausente = a tabela
   * padrão, que é o que remotos e modelos sem níveis declarados recebem.
   */
  templateEffort?: string | null;
  /**
   * Se o template recusar o `reasoning_effort` (400/500 falando dele), tenta
   * UMA vez sem o nível. Só o llama-server local: remoto erra como sempre.
   */
  retryWithoutEffort?: boolean;
  /** Chamado a cada pedaço de texto recebido (delta.content). */
  onDelta: (text: string) => void;
  /** Chamado a cada pedaço de raciocínio (reasoning_content ou <think>). */
  onReasoningDelta?: (text: string) => void;
  /** Estimativa de tok/s durante o stream (~4 Hz), para exibição ao vivo. */
  onTokensPerSec?: (tokensPerSec: number) => void;
}

export interface StreamChatResult {
  content: string;
  /** Raciocínio do modelo (vazio quando o modelo não é "thinking"). */
  reasoning: string;
  tokensPerSec: number | null;
  /** Tokens informados pelo servidor; ausente quando não informado. */
  genTokens: number | null;
  /** Duração de geração informada pelo servidor. */
  genMs: number | null;
  /** Tempo entre o primeiro e o último delta de raciocínio, em ms. */
  thinkingMs: number | null;
  /** Tokens do prompt processados de fato (timings do servidor; sem
   *  estimativa — `null` quando o servidor não informou). */
  promptTokens: number | null;
  /** Tokens do prompt reaproveitados do KV cache (timings do servidor). */
  cachedTokens: number | null;
  /** Velocidade de processamento do prompt em tok/s (timings do servidor). */
  promptTps: number | null;
  /**
   * Por que o servidor parou (`finish_reason`): `"length"` é o teto de
   * tokens — num modelo que pensa, pode ter acabado antes de qualquer
   * resposta. `null` quando o servidor não disse.
   */
  finishReason: string | null;
}

// ------------------------------------------------- mini-store de geração ---
// Compatível com useSyncExternalStore: snapshot imutável + subscribe.

export interface GenSnapshot {
  tokensPerSec: number | null;
  generating: boolean;
}

let genSnapshot: GenSnapshot = { tokensPerSec: null, generating: false };
const genListeners = new Set<() => void>();
let genCount = 0;

function setGen(patch: Partial<GenSnapshot>): void {
  genSnapshot = { ...genSnapshot, ...patch };
  for (const fn of genListeners) fn();
}

export const genStats = {
  subscribe(fn: () => void): () => void {
    genListeners.add(fn);
    return () => {
      genListeners.delete(fn);
    };
  },
  get(): GenSnapshot {
    return genSnapshot;
  },
};

// -------------------------------------------------------- <think> inline ---

/**
 * Separa tags <think>...</think> embutidas no texto em streaming.
 * Como uma tag pode chegar dividida entre dois deltas, retemos o maior
 * sufixo do buffer que ainda pode ser o começo de uma tag.
 */
function createThinkSplitter(
  emitText: (t: string) => void,
  emitReasoning: (t: string) => void,
) {
  const OPEN = "<think>";
  const CLOSE = "</think>";
  let pending = "";
  let inThink = false;

  /** Tamanho do maior sufixo de `s` que é prefixo próprio de `tag`. */
  const partialSuffix = (s: string, tag: string): number => {
    const max = Math.min(s.length, tag.length - 1);
    for (let k = max; k > 0; k--) {
      if (s.endsWith(tag.slice(0, k))) return k;
    }
    return 0;
  };

  const feed = (delta: string) => {
    pending += delta;
    for (;;) {
      const tag = inThink ? CLOSE : OPEN;
      const idx = pending.indexOf(tag);
      if (idx >= 0) {
        const before = pending.slice(0, idx);
        if (before) (inThink ? emitReasoning : emitText)(before);
        pending = pending.slice(idx + tag.length);
        inThink = !inThink;
        continue;
      }
      const hold = partialSuffix(pending, tag);
      const out = pending.slice(0, pending.length - hold);
      if (out) (inThink ? emitReasoning : emitText)(out);
      pending = pending.slice(pending.length - hold);
      break;
    }
  };

  const flush = () => {
    if (pending) (inThink ? emitReasoning : emitText)(pending);
    pending = "";
  };

  return { feed, flush };
}

// ------------------------------------------------------------- streaming ---

/** Delta de um chunk SSE. `content` pode vir `null` em chunks de tool call. */
interface SseDelta {
  content?: string | null;
  reasoning_content?: string | null;
  /**
   * Deltas de chamada de ferramenta. O chat normal não envia `tools`, mas um
   * modelo com template de tools pode emitir mesmo assim (e o llama-server
   * tira o trecho do `content`). Ignoramos de propósito: nada aqui entra no
   * texto e um chunk só com `tool_calls` não conta como token gerado.
   */
  tool_calls?: unknown;
}

/** Forma mínima de um chunk SSE do endpoint OpenAI-compatible. */
interface SseChunk {
  choices?: { delta?: SseDelta; finish_reason?: string | null }[];
  error?: { message?: string };
  usage?: { prompt_tokens?: number; completion_tokens?: number; prompt_tokens_details?: { cached_tokens?: number } };
  timings?: {
    predicted_n?: number;
    predicted_ms?: number;
    predicted_per_second?: number;
    /** Tokens do prompt reaproveitados do KV cache. */
    cache_n?: number;
    /** Tokens do prompt processados de fato (exclui o cache). */
    prompt_n?: number;
    prompt_per_second?: number;
  };
}

const REASONING_EFFORT: Record<EffortLevel, "low" | "medium" | "high"> = {
  low: "low",
  medium: "medium",
  high: "high",
  extra: "high",
  max: "high",
};

/**
 * Esforço → campos que o llama-server / modelos thinking reconhecem.
 * `templateEffort` é o nível já traduzido para o que o template aceita; sem
 * ele, a tabela padrão (que o Qwen3.8 recusa em `high`).
 */
function applyEffort(
  body: Record<string, unknown>,
  effort?: EffortLevel,
  templateEffort?: string | null,
) {
  if (!effort) return;
  body.reasoning_effort = templateEffort ?? REASONING_EFFORT[effort];
  body.chat_template_kwargs = { enable_thinking: effort !== "low" };
}

/**
 * Tira o `reasoning_effort` do corpo (topo e `chat_template_kwargs`) para a
 * segunda tentativa. Devolve `false` quando não havia nada a tirar — aí o
 * erro não é nosso e não se repete.
 */
function dropReasoningEffort(body: Record<string, unknown>): boolean {
  let had = "reasoning_effort" in body;
  delete body.reasoning_effort;
  const kwargs = body.chat_template_kwargs;
  if (kwargs && typeof kwargs === "object" && "reasoning_effort" in kwargs) {
    had = true;
    delete (kwargs as Record<string, unknown>).reasoning_effort;
  }
  return had;
}

/** Injeta o system prompt (se houver) como primeira mensagem. */
function withSystemPrompt(
  messages: ChatMessage[],
  params?: ChatParams,
): ChatMessage[] {
  const sys = params?.systemPrompt.trim();
  if (!sys) return messages;
  return [{ role: "system", content: sys }, ...messages];
}

/**
 * Envia a conversa ao llama-server e consome a resposta em streaming SSE.
 * Retorna texto, raciocínio e métricas (timings do servidor quando
 * disponíveis, senão estimadas por nº de chunks / tempo desde o 1º token).
 */
export async function streamChat(
  opts: StreamChatOptions,
): Promise<StreamChatResult> {
  genCount += 1;
  setGen({ generating: true, tokensPerSec: null });
  try {
    return isTauri ? await streamReal(opts) : await streamMock(opts);
  } finally {
    genCount = Math.max(0, genCount - 1);
    setGen({ generating: genCount > 0 });
  }
}

async function streamReal({
  baseUrl,
  headers,
  model,
  messages,
  signal,
  params,
  templateEffort,
  retryWithoutEffort,
  onDelta,
  onReasoningDelta,
}: StreamChatOptions): Promise<StreamChatResult> {
  const body: Record<string, unknown> = {
    model,
    messages: withSystemPrompt(messages, params),
    stream: true,
    stream_options: { include_usage: true },
  };
  if (params) {
    body.temperature = params.temperature;
    body.top_p = params.topP;
    body.top_k = params.topK;
    if (params.maxTokens != null) body.max_tokens = params.maxTokens;
    applyEffort(body, params.effort, templateEffort);
  }

  const send = () =>
    fetch(`${baseUrl}/v1/chat/completions`, {
      method: "POST",
      headers: { "Content-Type": "application/json", ...headers },
      body: JSON.stringify(body),
      signal,
    });
  let res = await send();
  if (!res.ok) {
    let errBody = await res.text().catch(() => "");
    // Rede de segurança: o template recusou o nível ("Unexpected reasoning
    // effort high") — template trocado por flag no perfil, ou uma validação
    // que o backend não soube ler. Uma tentativa sem o nível, antes de
    // qualquer token: o interruptor `enable_thinking` continua valendo. Só
    // no local e só em 400/500 — o que o llama-server devolve para isso; um
    // 401/429 que cite o campo não é o template.
    if (retryWithoutEffort && (res.status === 400 || res.status === 500)
      && /reasoning[ _]effort/i.test(errBody) && dropReasoningEffort(body)) {
      res = await send();
      if (!res.ok) errBody = await res.text().catch(() => "");
    }
    if (!res.ok) throw new Error(`HTTP ${res.status}: ${errBody.slice(0, 300)}`);
  }
  if (!res.body) throw new Error("resposta sem corpo (stream indisponível)");

  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  let content = "";
  let reasoning = "";
  let firstReasonAt = 0;
  let lastReasonAt = 0;
  let serverTps: number | null = null;
  let serverTokens: number | null = null;
  let serverMs: number | null = null;
  // Lado do prompt: só existe quando o servidor informa (chunk final com
  // timings) — nunca estimamos estes três.
  let serverPromptTokens: number | null = null;
  let serverCachedTokens: number | null = null;
  let serverPromptTps: number | null = null;
  let done = false;
  let finished = false;
  let finishReason: string | null = null;

  const emitText = (t: string) => {
    content += t;
    onDelta(t);
  };
  const emitReasoning = (t: string) => {
    const now = performance.now();
    if (firstReasonAt === 0) firstReasonAt = now;
    lastReasonAt = now;
    reasoning += t;
    onReasoningDelta?.(t);
  };
  const splitter = createThinkSplitter(emitText, emitReasoning);

  const handleLine = (line: string) => {
    const trimmed = line.trim();
    if (!trimmed.startsWith("data:")) return;
    const payload = trimmed.slice(5).trim();
    if (payload === "[DONE]") {
      done = true;
      return;
    }
    let chunk: SseChunk;
    try {
      chunk = JSON.parse(payload) as SseChunk;
    } catch {
      return; // linha parcial/ruído — ignora
    }
    if (chunk.error) throw new Error(chunk.error.message ?? "stream error");
    const motivo = chunk.choices?.find(c => c.finish_reason != null)?.finish_reason;
    if (motivo != null) {
      finished = true;
      finishReason ??= motivo;
    }
    if (chunk.usage) {
      const u = chunk.usage;
      if (typeof u.completion_tokens === "number") serverTokens = u.completion_tokens;
      if (typeof u.prompt_tokens === "number") serverPromptTokens = u.prompt_tokens;
      if (typeof u.prompt_tokens_details?.cached_tokens === "number") serverCachedTokens = u.prompt_tokens_details.cached_tokens;
    }
    const timings = chunk.timings;
    if (timings) {
      const {
        predicted_per_second: tps,
        predicted_n: n,
        predicted_ms: ms,
        cache_n: cacheN,
        prompt_n: promptN,
        prompt_per_second: promptTps,
      } = timings;
      if (typeof tps === "number" && Number.isFinite(tps)) serverTps = tps;
      if (typeof n === "number" && Number.isFinite(n)) serverTokens = n;
      if (typeof ms === "number" && Number.isFinite(ms)) serverMs = ms;
      if (typeof cacheN === "number" && Number.isFinite(cacheN))
        serverCachedTokens = cacheN;
      if (typeof promptN === "number" && Number.isFinite(promptN))
        serverPromptTokens = promptN;
      if (typeof promptTps === "number" && Number.isFinite(promptTps))
        serverPromptTps = promptTps;
    }
    const delta = chunk.choices?.[0]?.delta;
    // Modelos thinking com --jinja: raciocínio chega em campo separado.
    const reasonDelta = delta?.reasoning_content;
    const textDelta = delta?.content;
    const gotReason = typeof reasonDelta === "string" && reasonDelta.length > 0;
    const gotText = typeof textDelta === "string" && textDelta.length > 0;
    if (gotReason) emitReasoning(reasonDelta);
    if (gotText) splitter.feed(textDelta);
  };

  try {
    while (!done) {
      const { value, done: eof } = await reader.read();
      if (eof) break;
      buffer += decoder.decode(value, { stream: true });
      let nl: number;
      while (!done && (nl = buffer.indexOf("\n")) >= 0) {
        const line = buffer.slice(0, nl);
        buffer = buffer.slice(nl + 1);
        handleLine(line);
      }
    }
    if (!done && buffer) handleLine(buffer);
  } finally {
    // Sem isto o corpo fica pendurado depois do `[DONE]` (saímos do laço sem
    // ler o EOF): o WebView2 mantém a conexão ocupada e, passadas ~6
    // gerações, os fetches seguintes ficam presos na fila do pool.
    try {
      await reader.cancel();
    } catch {
      // stream já abortado/errado — nada a liberar
    }
  }
  splitter.flush();
  if (!done && !finished) throw new Error("stream ended before completion");

  const tokensPerSec = serverTps;
  const genTokens = serverTokens;
  const genMs = serverMs;
  const thinkingMs =
    firstReasonAt > 0 ? Math.max(0, Math.round(lastReasonAt - firstReasonAt)) : null;

  setGen({ tokensPerSec });
  return {
    content,
    reasoning,
    tokensPerSec,
    genTokens,
    genMs,
    thinkingMs,
    promptTokens: serverPromptTokens,
    cachedTokens: serverCachedTokens,
    promptTps: serverPromptTps,
    finishReason,
  };
}

// ------------------------------------------------------ resposta única ---

/**
 * Remove o raciocínio de uma resposta não-streaming.
 * Cobre o bloco fechado e também um `<think>` que ficou ABERTO — o caso do
 * auto-título, em que o teto de tokens corta o modelo no meio do raciocínio
 * e o título viraria "<think>Ok, o usuário quer…".
 */
function stripThink(raw: string): string {
  return raw
    .replace(/<think>[\s\S]*?<\/think>/g, "")
    .replace(/<think>[\s\S]*$/, "")
    .replace(/^[\s\S]*?<\/think>/, "")
    .trim();
}

/**
 * Uma completude única (stream: false) — usada para tarefas auxiliares
 * como o auto-título. Retorna o texto sem tags <think> e sem espaços
 * nas pontas. No navegador (mock) retorna um título simulado.
 */
export async function completeOnce({
  baseUrl,
  headers,
  model,
  messages,
  maxTokens,
  temperature,
  signal,
}: {
  baseUrl: string;
  headers?: Record<string, string>;
  model: string;
  messages: ChatMessage[];
  maxTokens: number;
  temperature: number;
  signal?: AbortSignal;
}): Promise<string> {
  if (!isTauri) {
    await sleep(500);
    return "Título simulado";
  }
  const res = await fetch(`${baseUrl}/v1/chat/completions`, {
    method: "POST",
    headers: { "Content-Type": "application/json", ...headers },
    body: JSON.stringify({
      model,
      messages,
      stream: false,
      max_tokens: maxTokens,
      temperature,
      // Tarefa auxiliar: raciocinar aqui só gasta o orçamento de tokens e
      // devolve um "título" que na verdade é o começo do pensamento.
      reasoning_effort: "low",
      chat_template_kwargs: { enable_thinking: false },
    }),
    signal,
  });
  if (!res.ok) {
    const body = await res.text().catch(() => "");
    throw new Error(`llama-server HTTP ${res.status}: ${body.slice(0, 300)}`);
  }
  const json = (await res.json()) as {
    choices?: { message?: { content?: string | null } }[];
  };
  return stripThink(json.choices?.[0]?.message?.content ?? "");
}

// ------------------------------------------------------- mock (navegador) ---

const MOCK_REASONING =
  "O usuário quer ver a interface do chat funcionando. Vou montar uma " +
  "resposta com vários elementos de Markdown para exercitar a renderização. " +
  "Também simulo este raciocínio para testar o bloco de pensamento da UI.";

const MOCK_REPLY = `Claro! Esta é uma **resposta simulada** do modo navegador (sem Tauri), útil para desenvolver a UI do chat.

Alguns elementos de Markdown para testar a renderização:

1. Listas numeradas funcionam;
2. \`código inline\` também;
3. E blocos de código com destaque de sintaxe:

\`\`\`python
from openai import OpenAI

client = OpenAI(base_url="http://127.0.0.1:11711/v1", api_key="local")
resp = client.chat.completions.create(
    model="qwen3-8b",
    messages=[{"role": "user", "content": "Olá!"}],
)
print(resp.choices[0].message.content)
\`\`\`

| Recurso | Status |
| --- | --- |
| Streaming SSE | ok |
| Markdown + GFM | ok |

> No app de verdade, os tokens vêm direto do llama-server local.`;

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function streamMock({
  onTokensPerSec,
  signal,
  onDelta,
  onReasoningDelta,
}: StreamChatOptions): Promise<StreamChatResult> {
  await sleep(400); // latência fake de "carregar o modelo"

  // Os testes ligam `__chatFalha` com a mensagem do erro; ele acontece uma vez
  // só, e a tentativa seguinte (o "Tentar de novo") responde normalmente.
  const g = globalThis as { __chatFalha?: string };
  if (g.__chatFalha) {
    const mensagem = g.__chatFalha;
    delete g.__chatFalha;
    throw new Error(mensagem);
  }

  const throwIfAborted = () => {
    if (signal.aborted) {
      throw new DOMException("Geração cancelada", "AbortError");
    }
  };

  // Fase de raciocínio simulado (2–3 frases) antes do texto.
  let reasoning = "";
  let thinkingMs: number | null = null;
  if (onReasoningDelta) {
    const thinkStart = performance.now();
    for (const part of MOCK_REASONING.match(/\S+\s*/g) ?? []) {
      throwIfAborted();
      await sleep(10 + Math.random() * 25);
      reasoning += part;
      onReasoningDelta(part);
    }
    thinkingMs = Math.round(performance.now() - thinkStart);
  }

  const parts = MOCK_REPLY.match(/\S+\s*/g) ?? [];
  const start = performance.now();
  let content = "";
  let emitted = 0;
  for (const part of parts) {
    throwIfAborted();
    await sleep(15 + Math.random() * 35);
    content += part;
    emitted += 1;
    onDelta(part);
    if (emitted % 8 === 0) {
      const elapsed = (performance.now() - start) / 1000;
      if (elapsed > 0) {
        setGen({ tokensPerSec: emitted / elapsed });
        onTokensPerSec?.(emitted / elapsed);
      }
    }
  }
  const genMs = Math.round(performance.now() - start);
  const tokensPerSec = genMs > 0 ? emitted / (genMs / 1000) : null;
  setGen({ tokensPerSec });
  return {
    content,
    reasoning,
    tokensPerSec,
    genTokens: emitted,
    genMs,
    thinkingMs,
    // Timings de prompt plausíveis, como o chunk final do servidor traria.
    promptTokens: 24,
    cachedTokens: 8,
    promptTps: 640,
    finishReason: "stop",
  };
}
