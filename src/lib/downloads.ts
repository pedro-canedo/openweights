// Os downloads de modelo como a interface os vê: um mapa por id
// (`repo::artefato`), alimentado por UMA assinatura do evento "download" e
// pela lista inicial. Cada botão deriva o estado dele daqui — antes o
// Descobrir guardava um "começou" local que nunca ouvia o evento, e o botão
// ficava em "Baixando…" para sempre, até depois de o download terminar.
//
// A transição para "pronto" só é anunciada quando esta sessão viu o
// download correr: um artefato que já estava no disco também termina em
// "done", e não é hora de dizer "modelo pronto".

import { useSyncExternalStore } from "react";
import i18n from "../i18n";
import { listDownloads, onDownloadEvent } from "./api";
import { formatBytes } from "./format";
import type { DownloadEvent, DownloadStatus } from "./types";

export function idDoDownload(repoId: string, artifactName: string): string {
  return `${repoId}::${artifactName}`;
}

let porId: ReadonlyMap<string, DownloadStatus> = new Map();
const ouvintes = new Set<() => void>();
const aoTerminar = new Set<(s: DownloadStatus) => void>();
/** Downloads que esta sessão viu na fila ou baixando. */
const vistosCorrendo = new Set<string>();
/** Downloads de quem já cuida do fim sozinho (o primeiro uso, o decisor). */
const silenciosos = new Set<string>();

function publicar(novo: Map<string, DownloadStatus>) {
  porId = novo;
  ouvintes.forEach((f) => f());
}

function aplicar(ev: DownloadEvent) {
  const novo = new Map(porId);
  if (ev.kind === "removed") {
    novo.delete(ev.id);
    vistosCorrendo.delete(ev.id);
    publicar(novo);
    return;
  }
  const s = ev.status;
  novo.set(s.id, s);
  publicar(novo);
  if (s.state === "queued" || s.state === "running") {
    vistosCorrendo.add(s.id);
  } else if (s.state === "done" && vistosCorrendo.delete(s.id) && !silenciosos.has(s.id)) {
    aoTerminar.forEach((f) => f(s));
  }
}

let iniciado: Promise<void> | null = null;

/** Liga o store ao backend uma vez: a lista de agora e o evento daí em diante. */
export function iniciarDownloads(): Promise<void> {
  iniciado ??= (async () => {
    await onDownloadEvent(aplicar);
    const lista = await listDownloads().catch(() => [] as DownloadStatus[]);
    const novo = new Map(porId);
    for (const s of lista) {
      if (!novo.has(s.id)) novo.set(s.id, s);
      if (s.state === "queued" || s.state === "running") vistosCorrendo.add(s.id);
    }
    publicar(novo);
  })();
  return iniciado;
}

export const downloadsStore = {
  subscribe(f: () => void) {
    ouvintes.add(f);
    return () => ouvintes.delete(f);
  },
  get: () => porId,
};

/** O download de um artefato, se houver um nesta sessão. */
export function useDownload(repoId: string, artifactName: string): DownloadStatus | undefined {
  const mapa = useSyncExternalStore(downloadsStore.subscribe, downloadsStore.get);
  return mapa.get(idDoDownload(repoId, artifactName));
}

/** Avisa quando um download que esta sessão viu correr termina. */
export function aoConcluirDownload(f: (s: DownloadStatus) => void): () => void {
  aoTerminar.add(f);
  return () => aoTerminar.delete(f);
}

/** Quem começou o download cuida do fim dele: nada de aviso "modelo pronto". */
export function silenciarDownload(repoId: string, artifactName: string) {
  silenciosos.add(idDoDownload(repoId, artifactName));
}

/**
 * Por que o download não começou, em palavras. O backend recusa por falta de
 * espaço com `disk-space:<precisa>:<livres>` (bytes); o resto vai como veio.
 */
export function motivoDoDownload(erro: unknown): string {
  const texto = String(erro);
  const espaco = /^disk-space:(\d+):(\d+)$/.exec(texto);
  if (espaco) {
    return i18n.t("discover.noSpace", {
      need: formatBytes(Number(espaco[1])),
      free: formatBytes(Number(espaco[2])),
    });
  }
  return i18n.t("discover.downloadFailed", { error: texto });
}

/** Para os testes: começa do zero. */
export function _reiniciarDownloads() {
  porId = new Map();
  iniciado = null;
  vistosCorrendo.clear();
  silenciosos.clear();
  aoTerminar.clear();
}
