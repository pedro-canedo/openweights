import { describe, expect, it } from "vitest";
import { filtrarComandos, normalizar, type Comando } from "./comandos";

const c = (id: string, rotulo: string, palavras?: string): Comando => ({
  id,
  grupo: "actions",
  rotulo,
  palavras,
  executar: () => {},
});

const lista = [
  c("a", "Ir para Configurações", "settings"),
  c("b", "Ir para Servidor Local › Desempenho"),
  c("c", "Nova conversa", "new chat"),
  c("d", "Iniciar o Servidor Local", "motor engine start"),
];

describe("filtrarComandos", () => {
  it("sem busca, devolve tudo na ordem do registro", () => {
    expect(filtrarComandos(lista, "  ").map((x) => x.id)).toEqual(["a", "b", "c", "d"]);
  });

  it("ignora acento e maiúscula", () => {
    expect(normalizar("Configurações")).toBe("configuracoes");
    expect(filtrarComandos(lista, "CONFIG").map((x) => x.id)).toEqual(["a"]);
  });

  it("cada palavra tem de aparecer, no rótulo ou nas palavras extras", () => {
    expect(filtrarComandos(lista, "servidor desemp").map((x) => x.id)).toEqual(["b"]);
    expect(filtrarComandos(lista, "engine").map((x) => x.id)).toEqual(["d"]);
    expect(filtrarComandos(lista, "chat").map((x) => x.id)).toEqual(["c"]);
  });

  it("quem tem uma palavra começando pela busca vem antes", () => {
    // "ser" está em "Servidor" nos dois; em "d" também está em "Iniciar o Servidor".
    const ids = filtrarComandos([c("x", "Observar a fila"), c("y", "Servidor")], "ser").map((x) => x.id);
    expect(ids).toEqual(["y", "x"]);
  });
});
