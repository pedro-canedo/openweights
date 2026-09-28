// Modo Simples/Avançado: uma preferência de dois níveis.
//
// O Simples esconde o que só faz sentido para quem já conhece o motor: o grupo
// "Avançado" da barra lateral (Servidor Local, Fontes, Treinar) e os controles
// extras do chat. Nada fica inalcançável: as telas continuam abrindo pelos
// botões que levam a elas, e a troca de modo está em Configurações.
//
// Instalação nova começa no Simples; quem atualiza fica no Avançado. Quem já
// tem o motor instalado ou um modelo na biblioteca usava o app antes de este
// modo existir, e não pode abrir a nova versão com metade das telas sumidas.

import { useSyncExternalStore } from "react";
import { getRuntimeStatus, listLocalModels } from "./api";

export type Modo = "simples" | "avancado";

const CHAVE = "ow.mode";

function guardado(): Modo | null {
  try {
    const v = localStorage.getItem(CHAVE);
    return v === "simples" || v === "avancado" ? v : null;
  } catch {
    return null;
  }
}

let modo: Modo | null = guardado();
const ouvintes = new Set<() => void>();

export const modoStore = {
  /** Enquanto a primeira decisão não sai, nada some da tela. */
  get: (): Modo => modo ?? "avancado",
  subscribe(f: () => void) {
    ouvintes.add(f);
    return () => ouvintes.delete(f);
  },
};

export function useModo(): Modo {
  return useSyncExternalStore(modoStore.subscribe, modoStore.get);
}

export function definirModo(novo: Modo) {
  modo = novo;
  try {
    localStorage.setItem(CHAVE, novo);
  } catch {
    // sem armazenamento: a escolha vale só nesta sessão
  }
  ouvintes.forEach((f) => f());
}

/** O modo de quem abre esta versão pela primeira vez. */
export function modoPelaHistoria(sinais: {
  motorInstalado: boolean;
  modelosNaBiblioteca: number;
}): Modo {
  return sinais.motorInstalado || sinais.modelosNaBiblioteca > 0 ? "avancado" : "simples";
}

/** Decide uma vez, na primeira abertura com o modo, e grava. */
export async function decidirModoInicial(): Promise<void> {
  if (modo) return;
  const [motor, modelos] = await Promise.all([
    getRuntimeStatus().catch(() => null),
    listLocalModels().catch(() => []),
  ]);
  // Uma resposta que chegou depois de a pessoa escolher não desfaz a escolha.
  if (modo) return;
  definirModo(
    modoPelaHistoria({
      motorInstalado: motor?.installed ?? false,
      modelosNaBiblioteca: modelos.length,
    }),
  );
}
