// O estado do motor, fora de qualquer tela.
//
// A verificação executa o `llama-server --version` de verdade e consulta a
// release do llama.cpp: leva segundos, e o resultado interessa a mais de um
// lugar — a tela de Ajustes mostra o relatório inteiro, e a barra lateral
// precisa saber se há algo a resolver para marcar o item de Ajustes.
//
// Mora num store de módulo pelo mesmo motivo do harness: se vivesse no
// `useState` da tela, cada visita a Ajustes repetiria a execução, e a barra
// lateral — que nunca abre a tela — não teria como saber de nada.
//
// A verificação roda uma vez por sessão, logo depois de o app abrir, e sob
// demanda no botão "Verificar". É o "sempre que abre" pedido: quem atualizou
// o OpenWeights e ficou com a build antiga no disco descobre isso antes de
// tentar carregar um modelo, não depois.

import { runtimeCheck } from "./api";
import type { EngineCheck } from "./types";

export interface EngineState {
  check: EngineCheck | null;
  checking: boolean;
  /** Quando a última verificação terminou (epoch ms); 0 se nunca. */
  checkedAt: number;
}

let state: EngineState = { check: null, checking: false, checkedAt: 0 };
const listeners = new Set<() => void>();

function set(patch: Partial<EngineState>) {
  state = { ...state, ...patch };
  for (const l of listeners) l();
}

export const engineStore = {
  subscribe(l: () => void) {
    listeners.add(l);
    return () => {
      listeners.delete(l);
    };
  },
  get: () => state,
};

/**
 * Verifica o motor. Sem `force`, não repete se já houver resultado — abrir
 * Ajustes três vezes não deve subir o executável três vezes.
 */
export async function verificarMotor(force = false): Promise<void> {
  if (state.checking) return;
  if (!force && state.check) return;
  set({ checking: true });
  try {
    set({ check: await runtimeCheck(), checkedAt: Date.now() });
  } catch {
    // Sem backend (navegador) ou comando indisponível: a tela mostra o que
    // sabe, e o aviso da barra lateral simplesmente não aparece.
  } finally {
    set({ checking: false });
  }
}

/**
 * Há providência a tomar com o motor?
 *
 * `notInstalled` conta: numa instalação nova o motor ainda não foi baixado, e
 * é isso que o primeiro uso do chat vai cobrar.
 */
export function motorPedeAtencao(s: EngineState): boolean {
  return s.check != null && s.check.verdict !== "ready";
}
