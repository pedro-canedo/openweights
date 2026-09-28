// O primeiro modelo: o que o onboarding sugere a quem acabou de instalar.
//
// Uma lista curta de famílias atuais, da mais capaz para a mais leve. Para cada
// uma, o advisor diz a quantização recomendada nesta máquina e se ela cabe;
// daqui saem as três que rodam melhor. É o único lugar do app que escolhe
// modelo pela pessoa, e os modelos envelhecem rápido: a lista é revista a cada
// release.

import type { QuantView } from "./types";

export interface Candidato {
  repoId: string;
  nome: string;
}

export const CANDIDATOS: Candidato[] = [
  { repoId: "unsloth/Qwen3.6-35B-A3B-GGUF", nome: "Qwen3.6 35B-A3B" },
  { repoId: "unsloth/Qwen3.8-27B-GGUF", nome: "Qwen3.8 27B" },
  { repoId: "unsloth/gemma-4-12b-it-GGUF", nome: "Gemma 4 12B" },
  { repoId: "unsloth/Qwen3.5-9B-GGUF", nome: "Qwen3.5 9B" },
  { repoId: "unsloth/gemma-4-E4B-it-GGUF", nome: "Gemma 4 E4B" },
  { repoId: "unsloth/Qwen3.5-4B-GGUF", nome: "Qwen3.5 4B" },
  { repoId: "unsloth/Qwen3.5-2B-GGUF", nome: "Qwen3.5 2B" },
];

export interface Sugestao {
  candidato: Candidato;
  quant: QuantView;
}

/**
 * 0: roda bem (tudo na GPU, ou MoE com os especialistas na RAM); 1: parte
 * na GPU; 2: só na CPU. `null`: não cabe.
 */
function faixa(q: QuantView): number | null {
  switch (q.verdict.kind) {
    case "fullGpu":
    case "moeOffload":
      return 0;
    case "partial":
      return 1;
    case "cpuOnly":
      return 2;
    default:
      return null;
  }
}

/**
 * A versão para rodar só na CPU. O advisor recomenda a de melhor qualidade que
 * cabe na RAM — um BF16, numa máquina com memória sobrando —, mas na CPU o
 * tamanho é a velocidade, e um primeiro modelo lento é um mau começo. Fica a
 * maior entre 4 e 5 bits por peso (a faixa do Q4_K_M), se houver.
 */
function versaoParaCpu(quants: QuantView[], recomendada: QuantView): QuantView {
  const leves = quants.filter(
    (q) =>
      q.verdict.kind === "cpuOnly" &&
      !q.requiresPrism &&
      q.bits != null &&
      q.bits >= 4 &&
      q.bits <= 5,
  );
  return leves.reduce<QuantView | null>((a, q) => (a && a.bits! >= q.bits! ? a : q), null) ?? recomendada;
}

/**
 * As `n` melhores para esta máquina: primeiro as que rodam bem, na ordem da
 * lista (a mais capaz antes). Só na CPU, a menor vem antes — ali o tamanho é
 * a velocidade. Candidato sem resposta do Hub (`quants: null`) fica de fora,
 * e o que só abre no motor da PrismML também: primeiro modelo não é hora de
 * baixar um segundo motor.
 */
export function escolherSugestoes(
  avaliados: { candidato: Candidato; quants: QuantView[] | null }[],
  n = 3,
): Sugestao[] {
  const aptos = avaliados.flatMap((a, ordem) => {
    const recomendada = a.quants?.find((q) => q.recommended);
    const f = recomendada && !recomendada.requiresPrism ? faixa(recomendada) : null;
    if (!a.quants || !recomendada || f == null) return [];
    const quant = f === 2 ? versaoParaCpu(a.quants, recomendada) : recomendada;
    return [{ sugestao: { candidato: a.candidato, quant }, f, ordem }];
  });
  aptos.sort(
    (x, y) =>
      x.f - y.f ||
      (x.f === 2
        ? x.sugestao.quant.totalBytes - y.sugestao.quant.totalBytes
        : x.ordem - y.ordem),
  );
  return aptos.slice(0, n).map((a) => a.sugestao);
}
