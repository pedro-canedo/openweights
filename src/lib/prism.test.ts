import { describe, expect, it } from "vitest";
import { isPrismRequired } from "./prism";

describe("isPrismRequired", () => {
  it("recognises the typed error however it arrives", () => {
    expect(isPrismRequired("prism-required")).toBe(true);
    expect(isPrismRequired(new Error("prism-required"))).toBe(true);
    expect(isPrismRequired("Erro: prism-required")).toBe(true);
  });
  it("does not mistake other failures for a missing engine", () => {
    expect(isPrismRequired("HTTP 500: failed to load model")).toBe(false);
    expect(isPrismRequired(new Error("engine-busy:chat"))).toBe(false);
    expect(isPrismRequired(null)).toBe(false);
    expect(isPrismRequired(undefined)).toBe(false);
  });
});
