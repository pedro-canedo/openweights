import { describe, expect, it, vi } from "vitest";

vi.stubGlobal("localStorage", { getItem: () => "pt-BR", setItem: () => {} });
vi.stubGlobal("document", { documentElement: {} });

const { filtrarLogs, textoDosLogs } = await import("./logs");

const l = (seq: number, origem: string, texto: string) => ({ seq, ts: Date.UTC(2026, 0, 1, 12, 0, seq), origem, texto });
const linhas = [l(1, "servidor", "Carregando o modelo"), l(2, "decisor", "pronto"), l(3, "servidor", "ERRO: sem memória")];

describe("filtrarLogs", () => {
  it("respeita as origens ligadas", () => {
    expect(filtrarLogs(linhas, new Set(["decisor"]), "").map((x) => x.seq)).toEqual([2]);
    expect(filtrarLogs(linhas, new Set(), "")).toEqual([]);
  });
  it("busca sem diferenciar caixa, só nas origens ligadas", () => {
    const todas = new Set(["servidor", "decisor"]);
    expect(filtrarLogs(linhas, todas, "erro").map((x) => x.seq)).toEqual([3]);
    expect(filtrarLogs(linhas, new Set(["decisor"]), "erro")).toEqual([]);
    expect(filtrarLogs(linhas, todas, "  ").length).toBe(3);
  });
});

describe("textoDosLogs", () => {
  it("uma linha por linha do log, com hora e origem", () => {
    const texto = textoDosLogs(linhas.slice(0, 2));
    const partes = texto.split("\n");
    expect(partes).toHaveLength(2);
    expect(partes[0]).toMatch(/^\d\d:\d\d:\d\d\.\d{3} \[servidor\] Carregando o modelo$/);
    expect(partes[1]).toMatch(/\[decisor\] pronto$/);
  });
});
