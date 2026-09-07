import { afterEach, describe, expect, it, vi } from "vitest";
vi.mock("./tauri", () => ({ isTauri: true }));
import { streamChat } from "./llama";
const event = (value: unknown) => `data: ${JSON.stringify(value)}\n\n`;
function response(text: string) {
  const data = new TextEncoder().encode(text);
  // Um byte por pacote: testa UTF-8 e tags quebradas entre chunks de rede.
  return new Response(new ReadableStream({ start(c) { for (const byte of data) c.enqueue(Uint8Array.of(byte)); c.close(); } }));
}
afterEach(() => vi.unstubAllGlobals());
describe("stream protocol", () => {
  it("keeps unicode and split thinking tags intact and uses reported tokens", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => response(
      event({ choices: [{ delta: { content: "<thi" } }] }) +
      event({ choices: [{ delta: { content: "nk>razão</think>olá" } }] }) +
      event({ choices: [{ delta: {}, finish_reason: "stop" }], timings: { predicted_n: 7, predicted_per_second: 3, predicted_ms: 2000 } }) + "data: [DONE]\n\n")));
    let content = "", reasoning = "";
    const result = await streamChat({ baseUrl: "http://test", model: "m", messages: [], signal: new AbortController().signal, onDelta: d => { content += d; }, onReasoningDelta: d => { reasoning += d; } });
    expect(content).toBe("olá"); expect(reasoning).toBe("razão"); expect(result.genTokens).toBe(7);
  });
  it("does not label network fragments as tokens when timings are absent", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => response(event({ choices: [{ delta: { content: "many words" } }] }) + "data: [DONE]\n\n")));
    const result = await streamChat({ baseUrl: "http://test", model: "m", messages: [], signal: new AbortController().signal, onDelta: () => {} });
    expect(result.genTokens).toBeNull(); expect(result.tokensPerSec).toBeNull();
  });
  it("reports truncated streams as errors after delivering partial content", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => response(event({ choices: [{ delta: { content: "partial" } }] }))));
    const delta = vi.fn();
    await expect(streamChat({ baseUrl: "http://test", model: "m", messages: [], signal: new AbortController().signal, onDelta: delta })).rejects.toThrow("before completion");
    expect(delta).toHaveBeenCalledWith("partial");
  });
  it("accepts provider usage without inventing speed", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => response(event({ usage: { prompt_tokens: 12, completion_tokens: 3, prompt_tokens_details: { cached_tokens: 2 } } }) + "data: [DONE]\n\n")));
    const result = await streamChat({ baseUrl: "http://test", model: "m", messages: [], signal: new AbortController().signal, onDelta: () => {} });
    expect(result).toMatchObject({ genTokens: 3, cachedTokens: 2, promptTokens: 12, tokensPerSec: null });
  });
});
