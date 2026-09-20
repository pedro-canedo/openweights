// O motor da PrismML (modelos Bonsai 2): estado, instalação e o erro tipado
// que o backend devolve quando um modelo precisa dele e ele não está lá.
//
// Store fora do React, como `engine.ts`: a instalação começa num lugar (o
// Descobrir, o chat, a biblioteca) e o progresso tem de aparecer em todos
// eles, inclusive depois de trocar de tela. Uma única assinatura do evento
// `runtime-prism` alimenta todo mundo.

import { ensurePrism, getPrismStatus, onPrismEvent } from "./api";
import { isTauri } from "./tauri";
import type { RuntimeEvent, RuntimeState } from "./types";

export interface PrismSnapshot {
  state: RuntimeState | null;
  installing: boolean;
  progress: RuntimeEvent | null;
  error: string | null;
}

/** Tamanho aproximado do pacote, para a tela dizer antes do clique. */
export const PRISM_APPROX_BYTES = 150 * 1024 * 1024;

/** O erro tipado do backend: o modelo exige o motor e ele não está instalado. */
export const PRISM_REQUIRED = "prism-required";

/** `true` quando a falha (Error, string ou texto de bolha) é a falta do motor. */
export function isPrismRequired(err: unknown): boolean {
  const msg = err instanceof Error ? err.message : typeof err === "string" ? err : "";
  return msg.includes(PRISM_REQUIRED);
}

let snap: PrismSnapshot = { state: null, installing: false, progress: null, error: null };
const listeners = new Set<() => void>();
let assinado = false;

function emit(next: Partial<PrismSnapshot>) {
  snap = { ...snap, ...next };
  for (const l of listeners) l();
}

async function assinar() {
  if (assinado || !isTauri) return;
  assinado = true;
  await onPrismEvent((e) => {
    if (e.kind === "ready") emit({ progress: null, installing: false, error: null });
    else if (e.kind === "failed") emit({ progress: null, installing: false, error: e.message });
    else emit({ progress: e, installing: true });
  });
}

/** Lê o estado no backend (barato: é um `is_file`). */
export async function carregarPrism(): Promise<RuntimeState | null> {
  void assinar();
  try {
    const state = await getPrismStatus();
    emit({ state });
    return state;
  } catch {
    return snap.state;
  }
}

let instalando: Promise<RuntimeState> | null = null;

/** Instala o motor (uma instalação por vez; quem chama de novo espera a mesma). */
export function instalarPrism(): Promise<RuntimeState> {
  void assinar();
  return (instalando ??= (async () => {
    emit({ installing: true, error: null });
    try {
      const state = await ensurePrism();
      emit({ state, installing: false, progress: null });
      return state;
    } catch (e) {
      const error = e instanceof Error ? e.message : String(e);
      emit({ installing: false, progress: null, error });
      throw e;
    } finally {
      instalando = null;
    }
  })());
}

export const prismStore = {
  subscribe(fn: () => void) {
    listeners.add(fn);
    return () => {
      listeners.delete(fn);
    };
  },
  get: () => snap,
};
