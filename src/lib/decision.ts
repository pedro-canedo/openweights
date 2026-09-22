// O motor de decisão (o llama-server do fork `parallel-decision`, que serve
// `/v1/decision`): a instalação começa no cartão do Jev e o progresso tem de
// aparecer no painel de downloads, inclusive depois de trocar de tela. Uma
// única assinatura do evento `runtime-decision` alimenta todo mundo — mesmo
// desenho do `prism.ts`.

import { onDecisionEvent } from "./api";
import { jevLocalInstall, type JevLocalStatus } from "./jev";
import { isTauri } from "./tauri";
import type { RuntimeEvent } from "./types";

export interface DecisionSnapshot {
  installing: boolean;
  progress: RuntimeEvent | null;
  error: string | null;
}

/** O erro tipado do backend quando esta máquina não roda o motor. */
export const DECISION_UNSUPPORTED = "decision-unsupported";

let snap: DecisionSnapshot = { installing: false, progress: null, error: null };
const listeners = new Set<() => void>();
let assinado = false;

function emit(next: Partial<DecisionSnapshot>) {
  snap = { ...snap, ...next };
  for (const l of listeners) l();
}

async function assinar() {
  if (assinado || !isTauri) return;
  assinado = true;
  await onDecisionEvent((e) => {
    if (e.kind === "ready") emit({ progress: null, installing: false, error: null });
    else if (e.kind === "failed") emit({ progress: null, installing: false, error: e.message });
    else emit({ progress: e, installing: true });
  });
}

let instalando: Promise<JevLocalStatus> | null = null;

/** Pede a instalação (uma por vez; quem chama de novo espera a mesma). O
 * motor chega pelo evento; o modelo, pelo painel de downloads. */
export function instalarDecisor(): Promise<JevLocalStatus> {
  void assinar();
  return (instalando ??= (async () => {
    emit({ error: null });
    try {
      const st = await jevLocalInstall();
      if (!st.runtimeInstalled) emit({ installing: true });
      return st;
    } catch (e) {
      const error = e instanceof Error ? e.message : String(e);
      emit({ installing: false, progress: null, error });
      throw e;
    } finally {
      instalando = null;
    }
  })());
}

export function assinarDecisor() {
  void assinar();
}

export const decisionStore = {
  subscribe(fn: () => void) {
    listeners.add(fn);
    return () => {
      listeners.delete(fn);
    };
  },
  get: () => snap,
};
