import { describe, expect, it } from "vitest";
import { escolherSugestoes, type Candidato } from "./primeiroModelo";
import type { FitVerdict, QuantView } from "./types";

const gb = 2 ** 30;

function quant(verdict: FitVerdict, tamanhoGb: number, extra: Partial<QuantView> = {}): QuantView {
  return {
    artifactName: "m.gguf",
    files: ["m.gguf"],
    totalBytes: tamanhoGb * gb,
    filename: "m.gguf",
    label: "Q4_K_M",
    sizeBytes: tamanhoGb * gb,
    bits: 4.8,
    recommended: true,
    verdict,
    estTotalBytes: tamanhoGb * gb,
    kvCacheBytes: 0,
    requiresPrism: false,
    ...extra,
  };
}

const c = (nome: string): Candidato => ({ repoId: `x/${nome}`, nome });

describe("escolherSugestoes", () => {
  it("prefere as que rodam bem, na ordem da lista", () => {
    const s = escolherSugestoes([
      { candidato: c("grande"), quants: [quant({ kind: "partial", ngl: 30, layersTotal: 64 }, 17)] },
      { candidato: c("moe"), quants: [quant({ kind: "moeOffload", ncmoe: 20, layersTotal: 48 }, 20)] },
      { candidato: c("medio"), quants: [quant({ kind: "fullGpu", ngl: 40 }, 7)] },
      { candidato: c("pequeno"), quants: [quant({ kind: "fullGpu", ngl: 36 }, 3)] },
    ]);
    expect(s.map((x) => x.candidato.nome)).toEqual(["moe", "medio", "pequeno"]);
  });

  it("só na CPU, o menor vem antes", () => {
    const s = escolherSugestoes(
      [
        { candidato: c("grande"), quants: [quant({ kind: "cpuOnly" }, 17)] },
        { candidato: c("medio"), quants: [quant({ kind: "cpuOnly" }, 6)] },
        { candidato: c("pequeno"), quants: [quant({ kind: "cpuOnly" }, 2)] },
      ],
      2,
    );
    expect(s.map((x) => x.candidato.nome)).toEqual(["pequeno", "medio"]);
  });

  it("só na CPU, prefere a versão de 4 a 5 bits à recomendada de qualidade máxima", () => {
    const cpu = { kind: "cpuOnly" } as const;
    const [s] = escolherSugestoes([
      {
        candidato: c("modelo"),
        quants: [
          quant(cpu, 16, { label: "BF16", bits: 16, recommended: true }),
          quant(cpu, 8.5, { label: "Q8_0", bits: 8.5, recommended: false }),
          quant(cpu, 5, { label: "Q4_K_M", bits: 4.8, recommended: false }),
          quant(cpu, 4.2, { label: "IQ4_XS", bits: 4.25, recommended: false }),
          quant(cpu, 3, { label: "Q3_K_M", bits: 3.9, recommended: false }),
        ],
      },
    ]);
    expect(s.quant.label).toBe("Q4_K_M");
  });

  it("o candidato padrão vem na frente sempre que cabe", () => {
    const padrao: Candidato = { repoId: "x/leve", nome: "leve", padrao: true };
    const s = escolherSugestoes([
      { candidato: c("grande"), quants: [quant({ kind: "fullGpu", ngl: 60 }, 17)] },
      { candidato: padrao, quants: [quant({ kind: "partial", ngl: 20, layersTotal: 40 }, 6)] },
      { candidato: c("medio"), quants: [quant({ kind: "fullGpu", ngl: 40 }, 7)] },
    ]);
    expect(s.map((x) => x.candidato.nome)).toEqual(["leve", "grande", "medio"]);
    // Se não cabe, não entra.
    const semEle = escolherSugestoes([
      { candidato: padrao, quants: [quant({ kind: "wontFit" }, 60)] },
      { candidato: c("medio"), quants: [quant({ kind: "fullGpu", ngl: 40 }, 7)] },
    ]);
    expect(semEle.map((x) => x.candidato.nome)).toEqual(["medio"]);
  });

  it("deixa de fora o que não cabe, o que não respondeu e o que pede o motor da PrismML", () => {
    const s = escolherSugestoes([
      { candidato: c("nao-cabe"), quants: [quant({ kind: "wontFit" }, 50)] },
      { candidato: c("offline"), quants: null },
      { candidato: c("prism"), quants: [quant({ kind: "fullGpu", ngl: 30 }, 5, { requiresPrism: true })] },
      { candidato: c("sem-recomendado"), quants: [quant({ kind: "fullGpu", ngl: 30 }, 5, { recommended: false })] },
      { candidato: c("ok"), quants: [quant({ kind: "fullGpu", ngl: 30 }, 5)] },
    ]);
    expect(s.map((x) => x.candidato.nome)).toEqual(["ok"]);
  });
});
