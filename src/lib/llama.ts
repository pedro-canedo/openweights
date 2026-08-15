// Comunicação DIRETA do webview com o llama-server local via fetch/SSE.
// Decisão de arquitetura: tokens NUNCA passam pelo IPC do Tauri — o CSP já
// permite http://127.0.0.1:*. No navegador (dev sem Tauri) usamos um mock
// que "digita" uma resposta falsa para desenvolver a UI.

import { isTauri } from "./tauri";

export interface StreamChatOptions {
  baseUrl: string;
  model: string;
  messages: { role: string; content: string }[];
  signal: AbortSignal;
  /** Chamado a cada pedaço de texto recebido (delta.content). */
  onDelta: (text: string) => void;
}

export interface StreamChatResult {
  content: string;
  tokensPerSec: number | null;
}

// ------------------------------------------------- mini-store de geração ---
// Compatível com useSyncExternalStore: snapshot imutável + subscribe.

export interface GenSnapshot {
  tokensPerSec: number | null;
  generating: boolean;
}

let genSnapshot: GenSnapshot = { tokensPerSec: null, generating: false };
const genListeners = new Set<() => void>();

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

// ------------------------------------------------------------- streaming ---

/** Forma mínima de um chunk SSE do endpoint OpenAI-compatible. */
interface SseChunk {
  choices?: { delta?: { content?: string } }[];
  timings?: { predicted_per_second?: number };
}

/**
 * Envia a conversa ao llama-server e consome a resposta em streaming SSE.
 * Retorna o texto completo e o tok/s (medido pelo servidor quando disponível,
 * senão estimado por nº de chunks / tempo desde o primeiro token).
 */
export async function streamChat(
  opts: StreamChatOptions,
): Promise<StreamChatResult> {
  setGen({ generating: true, tokensPerSec: null });
  try {
    return isTauri ? await streamReal(opts) : await streamMock(opts);
  } finally {
    setGen({ generating: false });
  }
}

async function streamReal({
  baseUrl,
  model,
  messages,
  signal,
  onDelta,
}: StreamChatOptions): Promise<StreamChatResult> {
  const res = await fetch(`${baseUrl}/v1/chat/completions`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ model, messages, stream: true }),
    signal,
  });
  if (!res.ok) {
    const body = await res.text().catch(() => "");
    throw new Error(`llama-server HTTP ${res.status}: ${body.slice(0, 300)}`);
  }
  if (!res.body) throw new Error("resposta sem corpo (stream indisponível)");

  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  let content = "";
  let chunks = 0;
  let firstTokenAt = 0;
  let lastStatsAt = 0;
  let serverTps: number | null = null;
  let done = false;

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
    const tps = chunk.timings?.predicted_per_second;
    if (typeof tps === "number" && Number.isFinite(tps)) serverTps = tps;
    const delta = chunk.choices?.[0]?.delta?.content;
    if (typeof delta === "string" && delta.length > 0) {
      const now = performance.now();
      if (chunks === 0) firstTokenAt = now;
      chunks += 1;
      content += delta;
      onDelta(delta);
      // Estimativa ao vivo para a StatusBar (limitada a ~4 Hz).
      if (chunks > 1 && now - lastStatsAt > 250) {
        lastStatsAt = now;
        const elapsed = (now - firstTokenAt) / 1000;
        if (elapsed > 0) setGen({ tokensPerSec: (chunks - 1) / elapsed });
      }
    }
  };

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

  let tokensPerSec: number | null = serverTps;
  if (tokensPerSec == null && chunks > 1 && firstTokenAt > 0) {
    const elapsed = (performance.now() - firstTokenAt) / 1000;
    tokensPerSec = elapsed > 0 ? (chunks - 1) / elapsed : null;
  }
  setGen({ tokensPerSec });
  return { content, tokensPerSec };
}

// ------------------------------------------------------- mock (navegador) ---

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
  signal,
  onDelta,
}: StreamChatOptions): Promise<StreamChatResult> {
  await sleep(400); // latência fake de "carregar o modelo"
  const parts = MOCK_REPLY.match(/\S+\s*/g) ?? [];
  const start = performance.now();
  let content = "";
  let emitted = 0;
  for (const part of parts) {
    if (signal.aborted) {
      throw new DOMException("Geração cancelada", "AbortError");
    }
    await sleep(15 + Math.random() * 35);
    content += part;
    emitted += 1;
    onDelta(part);
    if (emitted % 8 === 0) {
      const elapsed = (performance.now() - start) / 1000;
      if (elapsed > 0) setGen({ tokensPerSec: emitted / elapsed });
    }
  }
  const elapsed = (performance.now() - start) / 1000;
  const tokensPerSec = elapsed > 0 ? emitted / elapsed : null;
  setGen({ tokensPerSec });
  return { content, tokensPerSec };
}
