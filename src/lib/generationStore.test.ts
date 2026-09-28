import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  stream: vi.fn(), add: vi.fn(), title: vi.fn(), endpoint: vi.fn(), setting: vi.fn(), profile: vi.fn(), jev: vi.fn(), templateEffort: vi.fn(), vision: vi.fn(),
}));
vi.mock("./jev", () => ({ jevDecideEffort: mocks.jev, chatReasoningEffort: mocks.templateEffort, summarizeForJev: (m: unknown) => m }));
vi.mock("./api", () => ({ addMessage: mocks.add, listChats: vi.fn(async () => []), renameChat: vi.fn(), getSetting: mocks.setting, getModelProfile: mocks.profile }));
vi.mock("./llama", () => ({ streamChat: mocks.stream, completeOnce: mocks.title }));
vi.mock("./serverSession", () => ({ ensureEndpoint: mocks.endpoint, listLoadedModels: vi.fn(async () => ["m"]), modelsMax: vi.fn(async () => 1), matchServerModel: (m: string) => m, visionModelFor: mocks.vision, errorMessage: String }));
vi.mock("./chatStore", () => ({ chatStore: { setChats: vi.fn() } }));
const result = { content: "ok", reasoning: "", tokensPerSec: 10, genTokens: 1, genMs: 100, thinkingMs: null, promptTokens: 8, cachedTokens: 4, promptTps: 100, finishReason: "stop" };
const opts = (chatId: number, model = "m") => ({ chatId, model, messages: [{ role: "user", content: "hi" }], params: {} as any });
let store: typeof import("./generationStore").generationStore;
beforeEach(async () => {
  vi.useFakeTimers(); vi.resetModules(); vi.clearAllMocks();
  mocks.add.mockResolvedValue(1); mocks.title.mockResolvedValue("Title");
  mocks.setting.mockResolvedValue("1"); mocks.profile.mockResolvedValue(null);
  mocks.endpoint.mockImplementation(async (m: string) => ({ provider: m.startsWith("openrouter:") ? "openrouter" : "local", baseUrl: "http://localhost", headers: {} }));
  mocks.stream.mockResolvedValue(result);
  mocks.jev.mockResolvedValue(null);
  mocks.templateEffort.mockResolvedValue(null);
  mocks.vision.mockImplementation((m: string) => m);
  store = (await import("./generationStore")).generationStore;
});
afterEach(() => { vi.clearAllTimers(); vi.useRealTimers(); });
describe("generation coordination", () => {
  it("buffers fragments without losing text when cancelled before publication", async () => {
    let stream: any;
    mocks.stream.mockImplementation((o) => new Promise((_, reject) => { stream = o; o.signal.addEventListener("abort", () => reject(new Error("aborted"))); }));
    store.start(opts(1)); await vi.advanceTimersByTimeAsync(1);
    stream.onDelta("hello"); stream.onDelta(" world");
    expect(store.jobFor(1)?.content).toBe("");
    store.cancel(1); await vi.advanceTimersByTimeAsync(1);
    expect(mocks.add.mock.calls[0][2]).toBe("hello world");
    expect(store.jobFor(1)?.state).toBe("done");
  });
  it("publishes at most twenty times a second and does not notify unrelated chats", async () => {
    let stream: any;
    mocks.stream.mockImplementation((o) => { stream = o; return new Promise(() => {}); });
    store.start(opts(1)); await vi.advanceTimersByTimeAsync(1);
    const active = vi.fn(), other = vi.fn(), navigation = vi.fn();
    store.subscribeChat(1, active); store.subscribeChat(2, other); store.subscribe(navigation);
    for (let i = 0; i < 100; i++) { stream.onDelta("x"); await vi.advanceTimersByTimeAsync(10); }
    expect(active.mock.calls.length).toBeLessThanOrEqual(20);
    expect(store.jobFor(1)?.content).toBe("x".repeat(100));
    expect(other).not.toHaveBeenCalled(); expect(navigation).not.toHaveBeenCalled();
  });
  it("serializes local work while remote work bypasses local capacity", async () => {
    mocks.stream.mockImplementation(() => new Promise(() => {}));
    store.start(opts(1)); store.start(opts(2)); store.start(opts(3, "openrouter:x"));
    await vi.advanceTimersByTimeAsync(10);
    expect(mocks.stream).toHaveBeenCalledTimes(2);
    expect(store.jobFor(2)?.state).toBe("queued");
  });
  it("does not replay a failed response after receiving content", async () => {
    mocks.stream.mockImplementation(async o => { o.onDelta("partial"); throw new Error("HTTP 503"); });
    store.start(opts(1)); await vi.advanceTimersByTimeAsync(15000);
    expect(mocks.stream).toHaveBeenCalledTimes(1);
    expect(mocks.add.mock.calls[0][2]).toBe("partial");
    expect(store.jobFor(1)?.state).toBe("error");
  });
  it("limits transient retries and finishes with an error", async () => {
    mocks.stream.mockRejectedValue(new Error("HTTP 503"));
    store.start(opts(1)); await vi.advanceTimersByTimeAsync(15000);
    expect(mocks.stream).toHaveBeenCalledTimes(4);
    expect(store.jobFor(1)?.state).toBe("error");
  });
  it("blocks new work until the benchmark releases its lease", async () => {
    const release = store.acquireBenchmark(); store.start(opts(1));
    await vi.advanceTimersByTimeAsync(10); expect(mocks.stream).not.toHaveBeenCalled();
    release(); await vi.advanceTimersByTimeAsync(10); expect(mocks.stream).toHaveBeenCalledOnce();
  });
  it("refuses a benchmark while a generation is preparing", async () => {
    mocks.endpoint.mockImplementation(() => new Promise(() => {}));
    store.start(opts(1)); await vi.advanceTimersByTimeAsync(1);
    expect(() => store.acquireBenchmark()).toThrow("engine-busy");
  });
  it("never sends a cancelled queued request", async () => {
    const release = store.acquireBenchmark(); store.start(opts(1)); store.cancel(1); release();
    await vi.advanceTimersByTimeAsync(10); expect(mocks.stream).not.toHaveBeenCalled();
  });
  it("bounds automatic title input and records run metrics", async () => {
    store.start({ ...opts(1), autoTitle: true, messages: [{ role: "user", content: "x".repeat(20000) }] });
    await vi.advanceTimersByTimeAsync(10);
    expect(mocks.title.mock.calls[0][0].messages[0].content.length).toBe(1200);
    expect(mocks.add.mock.calls[0][7]).toMatchObject({ promptTokens: 8, cachedTokens: 4, phase: "done" });
  });
  it("applies the effort Jev decided and records it in the metrics", async () => {
    mocks.jev.mockResolvedValue({ effort: "low", source: "jev", confidence: 0.82, cost: 1e-6, reason: null });
    store.start({ ...opts(1), params: { effort: "high" } as any }); await vi.advanceTimersByTimeAsync(10);
    expect(mocks.jev).toHaveBeenCalledWith("m", [{ role: "user", content: "hi" }], "high");
    expect(mocks.stream.mock.calls[0][0].params.effort).toBe("low");
    expect(store.jobFor(1)?.metrics.jev).toEqual({ effort: "low", confidence: 0.82, source: "jev" });
  });
  it("applies a decision from the local decider and remembers the source", async () => {
    mocks.jev.mockResolvedValue({ effort: "high", source: "local", confidence: 0.91, cost: null, reason: null });
    store.start({ ...opts(1), params: { effort: "low" } as any }); await vi.advanceTimersByTimeAsync(10);
    expect(mocks.stream.mock.calls[0][0].params.effort).toBe("high");
    expect(store.jobFor(1)?.metrics.jev).toEqual({ effort: "high", confidence: 0.91, source: "local" });
  });
  it("keeps the conversation effort when Jev falls back to the default", async () => {
    mocks.jev.mockResolvedValue({ effort: "high", source: "padrao", confidence: 0.41, cost: 1e-6, reason: "confiança baixa" });
    mocks.templateEffort.mockResolvedValue("xhigh");
    store.start({ ...opts(1), params: { effort: "high" } as any }); await vi.advanceTimersByTimeAsync(10);
    expect(mocks.templateEffort).toHaveBeenCalledWith("m", "high");
    expect(mocks.stream.mock.calls[0][0].params.effort).toBe("high");
    expect(mocks.stream.mock.calls[0][0].templateEffort).toBe("xhigh");
    expect(store.jobFor(1)?.metrics.jev).toBeUndefined();
  });
  it("translates the conversation effort when Jev is off", async () => {
    mocks.templateEffort.mockResolvedValue("xhigh");
    store.start({ ...opts(1, "bonsai.gguf"), params: { effort: "max" } as any }); await vi.advanceTimersByTimeAsync(10);
    expect(mocks.templateEffort).toHaveBeenCalledWith("bonsai.gguf", "max");
    const sent = mocks.stream.mock.calls[0][0];
    expect(sent).toMatchObject({ templateEffort: "xhigh", retryWithoutEffort: true });
    expect(sent.params.effort).toBe("max");
    expect(store.jobFor(1)?.metrics.jev).toBeUndefined();
  });
  it("asks the template level by the file name when the message carries an image", async () => {
    mocks.vision.mockImplementation((m: string) => `${m} (visão)`);
    mocks.templateEffort.mockResolvedValue("xhigh");
    const messages = [{ role: "user", content: [{ type: "image_url", image_url: { url: "x" } }] }];
    store.start({ ...opts(1, "bonsai.gguf"), messages, params: { effort: "high" } as any } as any); await vi.advanceTimersByTimeAsync(10);
    expect(mocks.templateEffort).toHaveBeenCalledWith("bonsai.gguf", "high");
    const sent = mocks.stream.mock.calls[0][0];
    expect(sent.model).toBe("bonsai.gguf (visão)");
    expect(sent.templateEffort).toBe("xhigh");
  });
  it("never asks Jev about a remote model", async () => {
    mocks.jev.mockResolvedValue({ effort: "low", source: "jev", confidence: 0.9, cost: null, reason: null });
    store.start({ ...opts(1, "openrouter:x"), params: { effort: "high" } as any }); await vi.advanceTimersByTimeAsync(10);
    expect(mocks.jev).not.toHaveBeenCalled();
    expect(mocks.stream.mock.calls[0][0].params.effort).toBe("high");
  });
  it("translates the final effort to the template's level after Jev decides", async () => {
    mocks.jev.mockResolvedValue({ effort: "max", source: "local", confidence: 0.9, cost: null, reason: null });
    mocks.templateEffort.mockResolvedValue("xhigh");
    store.start({ ...opts(1, "bonsai.gguf"), params: { effort: "low" } as any }); await vi.advanceTimersByTimeAsync(10);
    expect(mocks.templateEffort).toHaveBeenCalledWith("bonsai.gguf", "max");
    const sent = mocks.stream.mock.calls[0][0];
    expect(sent.templateEffort).toBe("xhigh");
    expect(sent.params.effort).toBe("max");
    expect(store.jobFor(1)?.metrics.jev).toEqual({ effort: "max", confidence: 0.9, source: "local" });
  });
  it("records when the token limit ran out while the model was still thinking", async () => {
    mocks.stream.mockResolvedValue({ ...result, content: "", reasoning: "pensando…", finishReason: "length" });
    store.start({ ...opts(1), params: { effort: "high", maxTokens: 4096 } as any }); await vi.advanceTimersByTimeAsync(10);
    expect(store.jobFor(1)?.metrics).toMatchObject({ cutOff: "reasoning", maxTokens: 4096 });
    // Vai para o banco com as métricas: sobrevive a reabrir a conversa.
    expect(mocks.add.mock.calls[0][7]).toMatchObject({ cutOff: "reasoning", maxTokens: 4096 });
  });
  it("tells a cut answer from a cut thought, and a normal stop from both", async () => {
    mocks.stream.mockResolvedValue({ ...result, content: "metade da", finishReason: "length" });
    store.start({ ...opts(1), params: { maxTokens: null } as any }); await vi.advanceTimersByTimeAsync(10);
    expect(store.jobFor(1)?.metrics).toMatchObject({ cutOff: "answer", maxTokens: null });

    mocks.stream.mockResolvedValue(result);
    store.start(opts(2)); await vi.advanceTimersByTimeAsync(10);
    expect(store.jobFor(2)?.metrics.cutOff).toBeUndefined();
  });
  it("never translates the effort of a remote model", async () => {
    mocks.templateEffort.mockResolvedValue("xhigh");
    store.start({ ...opts(1, "openrouter:x"), params: { effort: "high" } as any }); await vi.advanceTimersByTimeAsync(10);
    expect(mocks.templateEffort).not.toHaveBeenCalled();
    expect(mocks.stream.mock.calls[0][0].templateEffort).toBeNull();
    expect(mocks.stream.mock.calls[0][0].retryWithoutEffort).toBe(false);
  });
});
