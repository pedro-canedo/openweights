import { describe, expect, it } from "vitest";
import { aspas, comandoShell } from "./comandoShell";

describe("aspas", () => {
  it("deixa como está o que é seguro e protege o resto", () => {
    expect(aspas("--port")).toBe("--port");
    expect(aspas("/home/voce/modelos/a.gguf")).toBe("/home/voce/modelos/a.gguf");
    expect(aspas("com espaço")).toBe("'com espaço'");
    expect(aspas("")).toBe("''");
    expect(aspas("it's")).toBe("'it'\\''s'");
    expect(aspas("$(rm -rf ~)")).toBe("'$(rm -rf ~)'");
  });
});

describe("comandoShell", () => {
  it("põe as variáveis na frente, uma por linha, e o comando por último", () => {
    expect(
      comandoShell("llama-server", ["--port", "11711", "--api-key", "•••"], ["LLAMA_ARG_X=a b", "OUTRA=1"]),
    ).toBe("LLAMA_ARG_X='a b' \\\nOUTRA=1 \\\nllama-server --port 11711 --api-key '•••'");
  });
  it("sem variáveis é uma linha só", () => {
    expect(comandoShell("llama-server", ["--models-dir", "/m d"], [])).toBe("llama-server --models-dir '/m d'");
  });
});
