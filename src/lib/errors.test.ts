import { beforeAll, describe, expect, it, vi } from "vitest";

// O i18n de verdade lê o localStorage e o <html> ao carregar; aqui ele responde pt-BR, e os
// textos que se conferem são os reais.
vi.stubGlobal("localStorage", { getItem: () => "pt-BR", setItem: () => {} });
vi.stubGlobal("document", { documentElement: {} });

let classificarErro: typeof import("./errors").classificarErro;
let textoDoErro: typeof import("./errors").textoDoErro;
beforeAll(async () => {
  ({ classificarErro, textoDoErro } = await import("./errors"));
});

const tipo = (e: unknown) => classificarErro(e).tipo;

describe("classificarErro", () => {
  it("reconhece o motor ausente pela mensagem do backend", () => {
    expect(tipo("o runtime do llama.cpp ainda não está instalado")).toBe("motor-ausente");
    expect(classificarErro("o runtime do llama.cpp ainda não está instalado").acoes).toEqual(["motor"]);
  });

  it("navegador sem conexão com o servidor: o servidor não está de pé", () => {
    for (const m of ["Failed to fetch", "NetworkError when attempting to fetch resource.", "Load failed", "engine não está rodando"]) {
      expect(tipo(new TypeError(m))).toBe("sem-servidor");
    }
  });

  it("memória de vídeo esgotada, vista de três lados", () => {
    expect(tipo("HTTP 500: CUDA error: out of memory")).toBe("sem-memoria");
    expect(tipo("HTTP 500: cudaMalloc failed")).toBe("sem-memoria");
    expect(tipo("não consegui carregar o modelo `x`. Em geral é memória de vídeo: feche o que estiver usando a GPU")).toBe("sem-memoria");
  });

  it("contexto estourado", () => {
    expect(tipo('HTTP 400: {"error":{"type":"exceed_context_size_error"}}')).toBe("contexto");
    expect(tipo("HTTP 400: the request exceeds the available context size")).toBe("contexto");
  });

  it("o modelo que emudeceu", () => {
    expect(tipo("o modelo ficou 120s sem emitir nada (antes do primeiro token)")).toBe("travou");
  });

  it("provedor remoto: chave, créditos, limite e modelo", () => {
    expect(tipo("HTTP 401: Missing API key")).toBe("chave");
    expect(tipo("HTTP 403: forbidden")).toBe("chave");
    expect(tipo("HTTP 402: Insufficient credits")).toBe("creditos");
    expect(tipo("HTTP 429: rate limited")).toBe("limite");
    expect(tipo("HTTP 404: model not found")).toBe("modelo-inexistente");
    // 404 que não é de modelo não vira "sem o modelo".
    expect(tipo("HTTP 404: not found")).toBe("desconhecido");
  });

  it("um 500 de carga de modelo vem do template, e não da memória", () => {
    expect(tipo("HTTP 500: failed to load model")).toBe("modelo-nao-carregou");
  });

  it("disco: bytes do backend viram tamanhos", () => {
    const c = classificarErro("disk-space:5368709120:1073741824");
    expect(c.tipo).toBe("disco");
    expect(c.explicacao).toContain("5,0 GB");
    expect(c.explicacao).toContain("1,0 GB");
  });

  it("ignora o prefixo 'Algo deu errado:' que a bolha do Chat põe", () => {
    expect(tipo("Algo deu errado: HTTP 401: Missing API key")).toBe("chave");
    expect(classificarErro("Algo deu errado: HTTP 401: x").detalhe).toBe("Algo deu errado: HTTP 401: x");
  });

  it("o que não conhece vira 'algo deu errado' e guarda o texto original", () => {
    const c = classificarErro(new Error("ninguém previu isto"));
    expect(c.tipo).toBe("desconhecido");
    expect(c.detalhe).toBe("ninguém previu isto");
    expect(c.acoes).toEqual(["tentar"]);
  });

  it("todo tipo tem título e explicação nos dois idiomas (sem chave solta)", () => {
    const amostras = [
      "o runtime do llama.cpp ainda não está instalado", "Failed to fetch", "out of memory", "exceed_context_size_error",
      "failed to load model", "o modelo ficou 9s sem emitir nada (x)", "HTTP 401: x", "HTTP 402: x", "HTTP 429: x",
      "HTTP 404: model x", "disk-space:1:1", "falha de rede", "???",
    ];
    for (const a of amostras) {
      const c = classificarErro(a);
      expect(c.titulo, a).not.toMatch(/^actionableError\./);
      expect(c.explicacao, a).not.toMatch(/^actionableError\./);
    }
  });
});

describe("textoDoErro", () => {
  it("aceita Error, string e objeto", () => {
    expect(textoDoErro(new Error("a"))).toBe("a");
    expect(textoDoErro("b")).toBe("b");
    expect(textoDoErro({ c: 1 })).toBe('{"c":1}');
  });
});
