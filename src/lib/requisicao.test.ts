import { describe, expect, it, vi } from "vitest";

// `llama.ts` puxa o Tauri e o i18n; aqui só a função pura do corpo interessa.
vi.stubGlobal("localStorage", { getItem: () => "pt-BR", setItem: () => {} });
vi.stubGlobal("document", { documentElement: {} });

const { buildChatBody } = await import("./llama");
const { gerarRequisicao, literalPython, semImagensEmBase64 } = await import("./requisicao");
const { DEFAULT_CHAT_PARAMS } = await import("./types");

const msgs = [{ role: "user" as const, content: "Olá!" }];
const base = { ...DEFAULT_CHAT_PARAMS, effort: undefined as never };

describe("buildChatBody", () => {
  it("por padrão manda a amostragem do app, e nada do que é avançado", () => {
    const b = buildChatBody({ model: "m", messages: msgs, params: { ...DEFAULT_CHAT_PARAMS } });
    expect(b).toMatchObject({ model: "m", stream: true, temperature: 0.8, top_p: 0.95, top_k: 40 });
    for (const k of ["min_p", "repeat_penalty", "seed", "stop", "response_format", "presence_penalty"]) {
      expect(b, k).not.toHaveProperty(k);
    }
  });

  it("padrões do servidor: não manda temperatura, top_p nem top_k", () => {
    const b = buildChatBody({ model: "m", messages: msgs, params: { ...DEFAULT_CHAT_PARAMS, serverDefaults: true } });
    for (const k of ["temperature", "top_p", "top_k"]) expect(b, k).not.toHaveProperty(k);
    expect(b).toHaveProperty("max_tokens");
  });

  it("os parâmetros avançados entram quando definidos", () => {
    const b = buildChatBody({
      model: "m",
      messages: msgs,
      stream: false,
      params: {
        ...DEFAULT_CHAT_PARAMS,
        minP: 0.05,
        repeatPenalty: 1.1,
        presencePenalty: 0.2,
        frequencyPenalty: 0.3,
        seed: 42,
        stop: ["FIM"],
        jsonSchema: '{"type":"object"}',
      },
    });
    expect(b).toMatchObject({
      min_p: 0.05,
      repeat_penalty: 1.1,
      presence_penalty: 0.2,
      frequency_penalty: 0.3,
      seed: 42,
      stop: ["FIM"],
      stream: false,
      response_format: { type: "json_schema", json_schema: { name: "resposta", schema: { type: "object" } } },
    });
    expect(b).not.toHaveProperty("stream_options");
  });

  it("pediu JSON e o esquema não serve: cai em JSON qualquer, em vez de nenhum formato", () => {
    const b = buildChatBody({ model: "m", messages: msgs, params: { ...base, jsonMode: true, jsonSchema: "{ quebrado" } });
    expect(b.response_format).toEqual({ type: "json_object" });
  });

  it("seed 0 é um valor (não some), esquema inválido não vai, e jsonMode pede JSON", () => {
    expect(buildChatBody({ model: "m", messages: msgs, params: { ...base, seed: 0 } })).toHaveProperty("seed", 0);
    const invalido = buildChatBody({ model: "m", messages: msgs, params: { ...base, jsonSchema: "{ não é json" } });
    expect(invalido).not.toHaveProperty("response_format");
    const modo = buildChatBody({ model: "m", messages: msgs, params: { ...base, jsonMode: true } });
    expect(modo.response_format).toEqual({ type: "json_object" });
  });
});

describe("gerarRequisicao", () => {
  const body = buildChatBody({ model: "qwen", messages: msgs, stream: false, params: { ...base, topK: 20, seed: 7 } });

  it("curl: a chave é a variável de ambiente, nunca o valor", () => {
    const com = gerarRequisicao("curl", { baseUrl: "http://127.0.0.1:11711/", body, usaChave: true });
    expect(com).toContain("curl http://127.0.0.1:11711/v1/chat/completions \\");
    expect(com).toContain('-H "Authorization: Bearer $OPENWEIGHTS_API_KEY"');
    expect(com).toContain("-d '{");
    expect(gerarRequisicao("curl", { baseUrl: "http://x", body, usaChave: false })).not.toContain("Authorization");
  });

  it("curl: aspas dentro do texto não quebram o comando", () => {
    const b = buildChatBody({ model: "m", messages: [{ role: "user", content: "it's" }], stream: false });
    expect(gerarRequisicao("curl", { baseUrl: "http://x", body: b, usaChave: false })).toContain(`it'\\''s`);
  });

  it("python: o que o SDK não conhece vai em extra_body, e os literais são de Python", () => {
    const py = gerarRequisicao("python", { baseUrl: "http://127.0.0.1:11711", body, usaChave: false });
    expect(py).toContain('base_url="http://127.0.0.1:11711/v1"');
    expect(py).toContain('api_key="local"');
    expect(py).toContain("seed=7,");
    expect(py).toMatch(/extra_body=\{[\s\S]*"top_k": 20,/);
    expect(py).not.toContain("true");
    expect(gerarRequisicao("python", { baseUrl: "http://x", body, usaChave: true })).toContain('os.environ["OPENWEIGHTS_API_KEY"]');
  });

  it("js: fetch com o corpo e a chave da variável de ambiente", () => {
    const js = gerarRequisicao("js", { baseUrl: "http://127.0.0.1:11711", body, usaChave: true });
    expect(js).toContain('fetch("http://127.0.0.1:11711/v1/chat/completions"');
    expect(js).toContain("process.env.OPENWEIGHTS_API_KEY");
    expect(js).toContain('"model": "qwen"');
  });

  it("imagens em base64 viram marcador", () => {
    const partes = [{ role: "user", content: [{ type: "image_url", image_url: { url: "data:image/png;base64,AAAA" } }] }];
    expect(JSON.stringify(semImagensEmBase64(partes))).toContain("<imagem em base64>");
    expect(JSON.stringify(semImagensEmBase64(partes))).not.toContain("AAAA");
  });

  it("literalPython converte true, false e null", () => {
    expect(literalPython({ a: true, b: null, c: [false] })).toBe('{\n    "a": True,\n    "b": None,\n    "c": [\n        False,\n    ],\n}');
  });
});
