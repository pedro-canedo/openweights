// O log de tudo que roda por baixo do app (`src-tauri/src/logs.rs`): o
// llama-server, o decisor, o 9router, o gateway do OwCLI e o próprio
// OpenWeights. Uma assinatura só, para o painel e para a tela do servidor; o
// que passou antes de a tela existir vem de `server_logs(since)`.

import { useSyncExternalStore } from "react";
import { invoke, isTauri, listen } from "./tauri";
import * as mocks from "./mocks";

export interface LinhaDeLog {
  seq: number;
  /** Milissegundos desde a época Unix. */
  ts: number;
  origem: string;
  texto: string;
}

/** As origens na ordem em que os filtros aparecem. */
export const ORIGENS = ["servidor", "decisor", "9router", "gateway", "app"] as const;

/** Linhas guardadas na tela (o backend guarda 2000 por origem). */
const MAXIMO = 5000;

let linhas: readonly LinhaDeLog[] = [];
const ouvintes = new Set<() => void>();
let iniciado: Promise<void> | null = null;

function publicar(novas: readonly LinhaDeLog[]) {
  linhas = novas;
  ouvintes.forEach((f) => f());
}

function acrescentar(l: LinhaDeLog) {
  const ultimo = linhas[linhas.length - 1];
  if (ultimo && l.seq <= ultimo.seq) return;
  publicar(linhas.length >= MAXIMO ? [...linhas.slice(1), l] : [...linhas, l]);
}

const lerDoBackend = (since: number) =>
  isTauri
    ? invoke<LinhaDeLog[]>("server_logs", { since, source: null })
    : mocks.serverLogs(since);

/** Liga o store uma vez: o evento primeiro, depois o que já tinha passado. */
export function iniciarLogs(): Promise<void> {
  iniciado ??= (async () => {
    const espera: LinhaDeLog[] = [];
    let pronto = false;
    await listen<LinhaDeLog>("log-line", (l) => (pronto ? acrescentar(l) : espera.push(l)));
    const antigas = await lerDoBackend(0).catch(() => [] as LinhaDeLog[]);
    // O evento que chegou durante a leitura pode já estar na lista: o `seq`
    // decide, sem duplicar nem perder.
    publicar([]);
    [...antigas, ...espera].sort((a, b) => a.seq - b.seq).forEach(acrescentar);
    pronto = true;
  })();
  return iniciado;
}

export const logsStore = {
  subscribe(f: () => void) {
    ouvintes.add(f);
    return () => ouvintes.delete(f);
  },
  get: () => linhas,
};

export function useLogs(): readonly LinhaDeLog[] {
  return useSyncExternalStore(logsStore.subscribe, logsStore.get);
}

export async function limparLogs(): Promise<void> {
  if (isTauri) await invoke<void>("logs_clear", { source: null });
  else await mocks.logsClear();
  publicar([]);
}

// ---------------------------------------------------------- o painel ---

let aberto = false;
const ouvintesDoPainel = new Set<() => void>();

export const painelDeLogs = {
  subscribe(f: () => void) {
    ouvintesDoPainel.add(f);
    return () => ouvintesDoPainel.delete(f);
  },
  get: () => aberto,
  definir(v: boolean) {
    if (aberto === v) return;
    aberto = v;
    ouvintesDoPainel.forEach((f) => f());
  },
  alternar() {
    painelDeLogs.definir(!aberto);
  },
};

export function usePainelDeLogs(): boolean {
  return useSyncExternalStore(painelDeLogs.subscribe, painelDeLogs.get);
}

// ---------------------------------------------------- texto para levar ---

/** `HH:MM:SS.mmm` no relógio local. */
export function horaDoLog(ts: number): string {
  const d = new Date(ts);
  const p = (n: number, w = 2) => String(n).padStart(w, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}.${p(d.getMilliseconds(), 3)}`;
}

/** O que copiar e salvar: uma linha por linha, com hora e origem. */
export function textoDosLogs(ls: readonly LinhaDeLog[]): string {
  return ls.map((l) => `${horaDoLog(l.ts)} [${l.origem}] ${l.texto}`).join("\n");
}

/** Filtro do painel: origens ligadas e um pedaço de texto (sem caixa). */
export function filtrarLogs(
  ls: readonly LinhaDeLog[],
  origens: ReadonlySet<string>,
  busca: string,
): LinhaDeLog[] {
  const q = busca.trim().toLocaleLowerCase();
  return ls.filter(
    (l) => origens.has(l.origem) && (q === "" || l.texto.toLocaleLowerCase().includes(q)),
  );
}

/** Para os testes: começa do zero. */
export function _reiniciarLogs() {
  linhas = [];
  iniciado = null;
  aberto = false;
}
