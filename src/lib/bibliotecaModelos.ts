// A biblioteca de modelos como tabela: ordenar, somar e abrir a pasta.

import type { LocalModel } from "./types";

export type ColunaDaBiblioteca = "name" | "repo" | "quant" | "size";
export type Sentido = "asc" | "desc";

const chave: Record<ColunaDaBiblioteca, (m: LocalModel) => string | number> = {
  name: (m) => m.name.toLocaleLowerCase(),
  repo: (m) => m.repoId.toLocaleLowerCase(),
  quant: (m) => (m.quantLabel ?? "").toLocaleLowerCase(),
  size: (m) => m.totalBytes,
};

/** Ordena sem mexer na lista original; empate desempata pelo nome. */
export function ordenarModelos(
  modelos: readonly LocalModel[],
  coluna: ColunaDaBiblioteca,
  sentido: Sentido,
): LocalModel[] {
  const f = chave[coluna];
  const sinal = sentido === "asc" ? 1 : -1;
  return [...modelos].sort((a, b) => {
    const x = f(a);
    const y = f(b);
    const c = typeof x === "number" && typeof y === "number" ? x - y : String(x).localeCompare(String(y), undefined, { numeric: true });
    return c !== 0 ? c * sinal : a.name.localeCompare(b.name, undefined, { numeric: true });
  });
}

/** Identidade estável de um modelo da lista (o caminho é único). */
export const idDoModelo = (m: LocalModel) => m.primaryPath;

export function somaDeBytes(modelos: readonly LocalModel[]): number {
  return modelos.reduce((s, m) => s + m.totalBytes, 0);
}

/** A pasta e o arquivo de um caminho, com `/` ou `\`. */
export function pastaEArquivo(caminho: string): { pasta: string; arquivo: string } {
  const i = Math.max(caminho.lastIndexOf("/"), caminho.lastIndexOf("\\"));
  return i < 0 ? { pasta: "", arquivo: caminho } : { pasta: caminho.slice(0, i), arquivo: caminho.slice(i + 1) };
}
