import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  stream: vi.fn(), add: vi.fn(), title: vi.fn(), endpoint: vi.fn(), setting: vi.fn(), profile: vi.fn(),
}));
vi.mock("./api", () => ({ addMessage: mocks.add, listChats: vi.fn(async () => []), renameChat: vi.fn(), getSetting: mocks.setting, getModelProfile: mocks.profile }));
vi.mock("./llama", () => ({ streamChat: mocks.stream, completeOnce: mocks.title }));
vi.mock("./serverSession", () => ({ ensureEndpoint: mocks.endpoint, listLoadedModels: vi.fn(async () => ["m"]), modelsMax: vi.fn(async () => 1), matchServerModel: (m: string) => m, visionModelFor: (m: string) => m, errorMessage: String }));
vi.mock("./chatStore", () => ({ chatStore: { setChats: vi.fn() } }));
const result = { content: "ok", reasoning: "", tokensPerSec: 10, genTokens: 1, genMs: 100, thinkingMs: null, promptTokens: 8, cachedTokens: 4, promptTps: 100 };
const opts = (chatId: number, model = "m") => ({ chatId, model, messages: [{ role: "user", content: "hi" }], params: {} as any });
let store: typeof import("./generationStore").generationStore;
beforeEach(async () => {
  vi.useFakeTimers(); vi.resetModules(); vi.clearAllMocks();
  mocks.add.mockResolvedValue(1); mocks.title.mockResolvedValue("Title");
  mocks.setting.mockResolvedValue("1"); mocks.profile.mockResolvedValue(null);
  mocks.endpoint.mockImplementation(async (m: string) => ({ provider: m.startsWith("openrouter:") ? "openrouter" : "local", baseUrl: "http://localhost", headers: {} }));
  mocks.stream.mockResolvedValue(result);
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
});
