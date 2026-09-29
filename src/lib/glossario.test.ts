import { describe, expect, it } from "vitest";
import pt from "../i18n/pt-BR.json";
import en from "../i18n/en.json";
import { TERMOS } from "./glossario";

describe("glossário", () => {
  for (const [idioma, dic] of [["pt-BR", pt], ["en", en]] as const) {
    it(`todo termo tem nome e explicação em ${idioma}`, () => {
      const g = dic.glossary as Record<string, { name?: string; text?: string } | string>;
      expect(g.ask).toContain("{{term}}");
      for (const id of TERMOS) {
        const termo = g[id] as { name?: string; text?: string };
        expect(termo?.name, id).toBeTruthy();
        expect(termo?.text?.length ?? 0, id).toBeGreaterThan(30);
      }
    });
  }

  it("o dicionário não guarda termo que ninguém declarou", () => {
    const declarados = new Set<string>(TERMOS);
    const guardados = Object.keys(pt.glossary).filter((k) => k !== "ask");
    expect(guardados.filter((k) => !declarados.has(k))).toEqual([]);
  });
});
