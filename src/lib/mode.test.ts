import { describe, expect, it } from "vitest";
import { modoPelaHistoria } from "./mode";

describe("modoPelaHistoria", () => {
  it("instalação nova começa no Simples", () => {
    expect(modoPelaHistoria({ motorInstalado: false, modelosNaBiblioteca: 0 })).toBe("simples");
  });

  it("quem já tinha o motor fica no Avançado", () => {
    expect(modoPelaHistoria({ motorInstalado: true, modelosNaBiblioteca: 0 })).toBe("avancado");
  });

  it("quem já tinha modelo fica no Avançado, mesmo sem o motor", () => {
    expect(modoPelaHistoria({ motorInstalado: false, modelosNaBiblioteca: 2 })).toBe("avancado");
  });
});
