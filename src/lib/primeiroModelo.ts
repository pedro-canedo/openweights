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
  /** O ponto de partida: leve para começar, vem na frente sempre que couber. */
  padrao?: boolean;
}

export const CANDIDATOS: Candidato[] = [
  { repoId: "unsloth/Ornith-1.0-9B-GGUF", nome: "Ornith 9B", padrao: true },
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
 * A versão do primeiro modelo. O advisor recomenda a de melhor qualidade que
 * cabe — o Ornith 9B em BF16, 17 GB, numa placa de 24 GB —, mas o primeiro
 * modelo é para começar: o download tem de ser curto, e na CPU o tamanho ainda
 * é a velocidade. Fica a maior entre 4 e 5 bits por peso (a faixa do Q4_K_M)
 * que roda tão bem quanto a recomendada; a recomendada fica se já é leve ou se
 * não há versão nessa faixa. A de qualidade máxima continua no Descobrir.
 */
function versaoLeve(quants: QuantView[], recomendada: QuantView, f: number): QuantView {
  if (recomendada.bits != null && recomendada.bits <= 5) return recomendada;
  const leves = quants.filter((q) => {
    if (q.requiresPrism || q.bits == null || q.bits < 4 || q.bits > 5) return false;
    const fq = faixa(q);
    return fq != null && fq <= f;
  });
  // A tabela de bits empata a família Q4 inteira em 4,5: no empate, o arquivo
  // maior é o de melhor qualidade (Q4_K_M antes de Q4_K_S e Q4_0).
  const melhor = (a: QuantView, q: QuantView) => a.bits! - q.bits! || a.totalBytes - q.totalBytes;
  return leves.reduce<QuantView | null>((a, q) => (a && melhor(a, q) >= 0 ? a : q), null) ?? recomendada;
}

/**
 * As `n` melhores para esta máquina. O candidato padrão, se cabe, vem na
 * frente; depois, as que rodam bem, na ordem da lista (a mais capaz antes).
 * Só na CPU, a menor vem antes — ali o tamanho é a velocidade. Candidato sem resposta do Hub (`quants: null`) fica de fora,
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
    const quant = versaoLeve(a.quants, recomendada, f);
    return [{ sugestao: { candidato: a.candidato, quant }, f, ordem }];
  });
  aptos.sort(
    (x, y) =>
      Number(!!y.sugestao.candidato.padrao) - Number(!!x.sugestao.candidato.padrao) ||
      x.f - y.f ||
      (x.f === 2
        ? x.sugestao.quant.totalBytes - y.sugestao.quant.totalBytes
        : x.ordem - y.ordem),
  );
  return aptos.slice(0, n).map((a) => a.sugestao);
}
