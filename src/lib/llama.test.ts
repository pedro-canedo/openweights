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
describe("reasoning effort in the request body", () => {
  const done = () => response(event({ choices: [{ delta: { content: "ok" }, finish_reason: "stop" }] }) + "data: [DONE]\n\n");
  const params = (effort: string) => ({ temperature: 0.7, topP: 0.9, topK: 40, maxTokens: null, systemPrompt: "", effort }) as any;
  async function sentBody(effort: string, templateEffort?: string | null) {
    const fetch = vi.fn(async (..._: unknown[]) => done());
    vi.stubGlobal("fetch", fetch);
    await streamChat({ baseUrl: "http://test", model: "m", messages: [], params: params(effort), templateEffort, signal: new AbortController().signal, onDelta: () => {} });
    return JSON.parse((fetch.mock.calls[0][1] as RequestInit).body as string);
  }
  it("sends the level the template accepts (Bonsai 2 / Qwen3.8 only knows xhigh)", async () => {
    for (const effort of ["high", "extra", "max"]) {
      expect(await sentBody(effort, "xhigh")).toMatchObject({ reasoning_effort: "xhigh", chat_template_kwargs: { enable_thinking: true } });
    }
    expect(await sentBody("medium", "medium")).toMatchObject({ reasoning_effort: "medium", chat_template_kwargs: { enable_thinking: true } });
    expect(await sentBody("low", "low")).toMatchObject({ reasoning_effort: "low", chat_template_kwargs: { enable_thinking: false } });
  });
  it("keeps the usual table without a translated level (remote or unknown template)", async () => {
    expect(await sentBody("low")).toMatchObject({ reasoning_effort: "low", chat_template_kwargs: { enable_thinking: false } });
    expect(await sentBody("medium", null)).toMatchObject({ reasoning_effort: "medium" });
    for (const effort of ["high", "extra", "max"]) {
      expect(await sentBody(effort)).toMatchObject({ reasoning_effort: "high", chat_template_kwargs: { enable_thinking: true } });
    }
  });
  it("retries once without the level when the template rejects it", async () => {
    const fetch = vi.fn()
      .mockResolvedValueOnce(new Response("Jinja Exception: Unexpected reasoning effort high. Supported types are xhigh (default), medium, and low.", { status: 500 }))
      .mockResolvedValueOnce(done());
    vi.stubGlobal("fetch", fetch);
    let content = "";
    await streamChat({ baseUrl: "http://test", model: "m", messages: [], params: params("high"), retryWithoutEffort: true, signal: new AbortController().signal, onDelta: d => { content += d; } });
    expect(fetch).toHaveBeenCalledTimes(2);
    expect(JSON.parse(fetch.mock.calls[0][1].body).reasoning_effort).toBe("high");
    const retry = JSON.parse(fetch.mock.calls[1][1].body);
    expect(retry).not.toHaveProperty("reasoning_effort");
    expect(retry.chat_template_kwargs).toEqual({ enable_thinking: true });
    expect(content).toBe("ok");
  });
  it("does not retry other errors, a body without a level, or a second rejection", async () => {
    const retry = { retryWithoutEffort: true, signal: new AbortController().signal, onDelta: () => {} };
    let fetch = vi.fn(async () => new Response("model not found", { status: 400 }));
    vi.stubGlobal("fetch", fetch);
    await expect(streamChat({ baseUrl: "http://test", model: "m", messages: [], params: params("high"), ...retry })).rejects.toThrow("HTTP 400: model not found");
    expect(fetch).toHaveBeenCalledTimes(1);
    fetch = vi.fn(async () => new Response("invalid reasoning_effort", { status: 400 }));
    vi.stubGlobal("fetch", fetch);
    await expect(streamChat({ baseUrl: "http://test", model: "m", messages: [], ...retry })).rejects.toThrow("HTTP 400");
    expect(fetch).toHaveBeenCalledTimes(1);
    fetch = vi.fn(async () => new Response("Unexpected reasoning effort", { status: 500 }));
    vi.stubGlobal("fetch", fetch);
    await expect(streamChat({ baseUrl: "http://test", model: "m", messages: [], params: params("max"), ...retry })).rejects.toThrow("HTTP 500: Unexpected reasoning effort");
    expect(fetch).toHaveBeenCalledTimes(2);
  });
  it("does not retry a remote model or a status that is not the template's", async () => {
    let fetch = vi.fn(async () => new Response("reasoning_effort is not supported for this model", { status: 400 }));
    vi.stubGlobal("fetch", fetch);
    await expect(streamChat({ baseUrl: "http://test", model: "m", messages: [], params: params("high"), signal: new AbortController().signal, onDelta: () => {} })).rejects.toThrow("HTTP 400: reasoning_effort");
    expect(fetch).toHaveBeenCalledTimes(1);
    fetch = vi.fn(async () => new Response("rate limited on reasoning_effort=high", { status: 429 }));
    vi.stubGlobal("fetch", fetch);
    await expect(streamChat({ baseUrl: "http://test", model: "m", messages: [], params: params("high"), retryWithoutEffort: true, signal: new AbortController().signal, onDelta: () => {} })).rejects.toThrow("HTTP 429");
    expect(fetch).toHaveBeenCalledTimes(1);
  });
});
