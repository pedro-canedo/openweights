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

/**
 * Junta linhas novas às que já há: sem repetir um `seq`, em ordem de `seq`
 * (dois leitores — stdout e stderr — podem entregar fora de ordem) e sem passar
 * de `maximo`.
 */
export function fundirLinhas(
  atuais: readonly LinhaDeLog[],
  novas: readonly LinhaDeLog[],
  maximo: number,
): readonly LinhaDeLog[] {
  if (novas.length === 0) return atuais;
  const vistos = new Set(atuais.map((l) => l.seq));
  const bem = novas.filter((l) => !vistos.has(l.seq) && vistos.add(l.seq));
  if (bem.length === 0) return atuais;
  const junto = [...atuais, ...bem];
  // Quase sempre já está em ordem; só ordena quando não está.
  for (let i = 1; i < junto.length; i++) {
    if (junto[i].seq < junto[i - 1].seq) {
      junto.sort((a, b) => a.seq - b.seq);
      break;
    }
  }
  return junto.length > maximo ? junto.slice(junto.length - maximo) : junto;
}

// As linhas chegam em rajada (um motor verboso): juntam-se por quadro em vez de
// refazer a lista inteira a cada linha.
let fila: LinhaDeLog[] = [];
let agendado = false;
function descarregar() {
  agendado = false;
  const lote = fila;
  fila = [];
  publicar(fundirLinhas(linhas, lote, MAXIMO));
}
function acrescentar(l: LinhaDeLog) {
  fila.push(l);
  if (agendado) return;
  agendado = true;
  if (typeof requestAnimationFrame === "function") requestAnimationFrame(descarregar);
  else setTimeout(descarregar, 16);
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
    publicar(fundirLinhas([], [...antigas, ...espera], MAXIMO));
    pronto = true;
  })().catch((e) => {
    // Não fica preso na falha: a próxima chamada tenta de novo.
    iniciado = null;
    throw e;
  });
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
  fila = [];
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
  fila = [];
  agendado = false;
  iniciado = null;
  aberto = false;
}
