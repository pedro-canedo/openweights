import { describe, expect, it } from "vitest";
import { contarCategorias, filtrarFlags, type FlagSpec } from "./flags";

const flag = (key: string, category: string, helpText: string | null = null, aliases: string[] = []): FlagSpec => ({
  key,
  aliases,
  category,
  kind: { kind: "bool" } as unknown as FlagSpec["kind"],
  default: null,
  scope: "both" as FlagSpec["scope"],
  curated: false,
  helpText,
  requires: [],
  conflicts: [],
  dependsOn: null,
  typedField: null,
});

const flags = [
  flag("n-gpu-layers", "memory", null, ["-ngl"]),
  flag("ctx-size", "context", "size of the prompt context"),
  flag("mlock", "memory", "force system to keep model in RAM"),
  flag("rope-scaling", "rope", "RoPE frequency scaling method"),
];

describe("filtrarFlags", () => {
  it("acha pelo texto de ajuda, não só pela chave", () => {
    expect(filtrarFlags(flags, "keep model ram", null, () => "").map((f) => f.key)).toEqual(["mlock"]);
  });

  it("acha pelo apelido e pelo rótulo traduzido, sem acento", () => {
    expect(filtrarFlags(flags, "-ngl", null, () => "").map((f) => f.key)).toEqual(["n-gpu-layers"]);
    const rotulo = (f: FlagSpec) => (f.key === "n-gpu-layers" ? "Camadas na GPU" : "");
    expect(filtrarFlags(flags, "camadas", null, rotulo).map((f) => f.key)).toEqual(["n-gpu-layers"]);
  });

  it("a categoria sozinha lista todas as dela, sem teto", () => {
    expect(filtrarFlags(flags, "", "memory", () => "").map((f) => f.key)).toEqual(["n-gpu-layers", "mlock"]);
    expect(filtrarFlags(flags, "mlock", "rope", () => "")).toEqual([]);
  });
});

describe("contarCategorias", () => {
  it("conta na ordem em que aparecem", () => {
    expect(contarCategorias(flags)).toEqual([["memory", 2], ["context", 1], ["rope", 1]]);
  });
});
