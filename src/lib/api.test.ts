import { describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn(async () => 7));
vi.mock("./tauri", () => ({ isTauri: true, invoke, listen: vi.fn() }));
import { addMessage, inteiro } from "./api";

describe("message_add payload", () => {
  it("sends the counts the backend stores as integers as integers", async () => {
    // O que o chat manda de fato: `predicted_ms` do llama-server com casas
    // decimais. Com o valor cru, o serde recusava e a resposta não era gravada.
    await addMessage(3, "assistant", "Olá, 2+2 é 4.", 66.6, 71, 1069.234, "bonsai.gguf", {
      runId: "r", phase: "done", queueMs: 1.5, firstTokenMs: 30.2, firstAnswerMs: 900.7, totalMs: 1100.1,
      promptTokens: 24, cachedTokens: 0, promptTps: 1221.3, thinkingMs: 857.9, cutOff: "answer", maxTokens: 4096,
    });
    expect(invoke).toHaveBeenCalledWith("message_add", expect.objectContaining({
      genTokens: 71,
      genMs: 1069,
      metrics: expect.objectContaining({ promptTokens: 24, cachedTokens: 0, cutOff: "answer", maxTokens: 4096 }),
    }));
  });

  it("rounds, never goes negative and keeps unknown as null", () => {
    expect(inteiro(1069.6)).toBe(1070);
    expect(inteiro(-3)).toBe(0);
    expect(inteiro(null)).toBeNull();
    expect(inteiro(undefined)).toBeNull();
    expect(inteiro(Number.NaN)).toBeNull();
  });
});
