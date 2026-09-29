import { describe, expect, it } from "vitest";
import { lerArquivo } from "./configuracao";

const base = { app: "OpenWeights", format: 1, appVersion: "0.27.0" };

describe("lerArquivo", () => {
  it("conta o que o arquivo traz, sem aplicar", () => {
    const c = lerArquivo(
      JSON.stringify({
        ...base,
        server: { a: "1", b: "2" },
        profiles: [{}, {}, {}],
        presets: [{}],
        enginePresets: [],
        providers: { openRouter: {} },
        secrets: { k: "v" },
      }),
    );
    expect(c).toEqual({ appVersion: "0.27.0", settings: 2, profiles: 3, presets: 1, enginePresets: 0, providers: true, secrets: 1 });
  });

  it("recusa com o motivo o que não serve", () => {
    expect(() => lerArquivo("{ não é json")).toThrow(/^invalid-json:/);
    expect(() => lerArquivo("{}")).toThrow("not-openweights");
    expect(() => lerArquivo(JSON.stringify({ ...base, format: 2 }))).toThrow("format:2");
    expect(() => lerArquivo("null")).toThrow("not-openweights");
  });

  it("arquivo mínimo é válido e vazio", () => {
    expect(lerArquivo(JSON.stringify(base)).profiles).toBe(0);
  });
});
