import { describe, expect, it } from "vitest";
import { ordenarModelos, pastaEArquivo, somaDeBytes } from "./bibliotecaModelos";
import type { LocalModel } from "./types";

const m = (name: string, quant: string, gb: number, repo = "a/b"): LocalModel => ({
  repoId: repo,
  name,
  primaryPath: `/m/${name}`,
  totalBytes: gb * 2 ** 30,
  files: [],
  quantLabel: quant,
  requiresPrism: false,
});
const lista = [m("Zeta-Q4.gguf", "Q4_K_M", 4), m("alfa-Q8.gguf", "Q8_0", 8), m("Beta-Q2.gguf", "Q2_K", 2, "c/d")];

describe("ordenarModelos", () => {
  it("por nome, sem diferenciar caixa, nos dois sentidos", () => {
    expect(ordenarModelos(lista, "name", "asc").map((x) => x.name)).toEqual(["alfa-Q8.gguf", "Beta-Q2.gguf", "Zeta-Q4.gguf"]);
    expect(ordenarModelos(lista, "name", "desc")[0].name).toBe("Zeta-Q4.gguf");
  });
  it("por tamanho é numérico", () => {
    expect(ordenarModelos(lista, "size", "desc").map((x) => x.quantLabel)).toEqual(["Q8_0", "Q4_K_M", "Q2_K"]);
  });
  it("por origem e por quantização", () => {
    expect(ordenarModelos(lista, "repo", "desc")[0].repoId).toBe("c/d");
    expect(ordenarModelos(lista, "quant", "asc")[0].quantLabel).toBe("Q2_K");
  });
  it("não mexe na lista original e desempata pelo nome", () => {
    const copia = [...lista];
    ordenarModelos(lista, "size", "asc");
    expect(lista).toEqual(copia);
    const iguais = [m("b.gguf", "Q4", 1), m("a.gguf", "Q4", 1)];
    expect(ordenarModelos(iguais, "size", "asc").map((x) => x.name)).toEqual(["a.gguf", "b.gguf"]);
  });
});

describe("somaDeBytes e pastaEArquivo", () => {
  it("soma e separa o caminho, com / ou \\", () => {
    expect(somaDeBytes(lista)).toBe(14 * 2 ** 30);
    expect(somaDeBytes([])).toBe(0);
    expect(pastaEArquivo("/home/x/m/a.gguf")).toEqual({ pasta: "/home/x/m", arquivo: "a.gguf" });
    expect(pastaEArquivo("C:\\fake\\m\\a.gguf")).toEqual({ pasta: "C:\\fake\\m", arquivo: "a.gguf" });
    expect(pastaEArquivo("a.gguf")).toEqual({ pasta: "", arquivo: "a.gguf" });
  });
});
