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
import { cancelDownload, listDownloads, onDownloadEvent } from "./api";
import { confirmar } from "../components/ui/Dialog";
import { toast } from "../components/ui/Toast";
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

/**
 * Cancela ou descarta um download. O que já foi baixado se perde, então, com
 * bytes no disco, pergunta antes e diz quanto; sem nada baixado, cancela
 * direto. Devolve se o download foi mesmo cancelado (para a tela se atualizar).
 */
export async function descartarDownload(s: DownloadStatus): Promise<boolean> {
  if (s.receivedBytes > 0) {
    const sim = await confirmar({
      title: i18n.t("downloadsPanel.discardTitle", { name: s.artifactName }),
      message: i18n.t("downloadsPanel.discardMessage", {
        size: formatBytes(s.receivedBytes),
      }),
      confirmLabel: i18n.t("models.discard"),
      tone: "danger",
    });
    if (!sim) return false;
  }
  // O diálogo pode ter ficado aberto até o download terminar: cancelar um que
  // já acabou apagaria o modelo inteiro.
  if (porId.get(s.id)?.state === "done") return false;
  try {
    await cancelDownload(s.id);
    return true;
  } catch (e) {
    toast({
      tone: "bad",
      message: i18n.t("downloadsPanel.cancelFailed", { error: String(e) }),
      duration: 0,
    });
    return false;
  }
}

/** Pausar e retomar: se falhar, diz em vez de só registrar no console. */
export function agirNoDownload(p: Promise<void>): void {
  void p.catch((e) =>
    toast({
      tone: "bad",
      message: i18n.t("downloadsPanel.actionFailed", { error: String(e) }),
      duration: 0,
    }),
  );
}

/** Para os testes: começa do zero. */
export function _reiniciarDownloads() {
  porId = new Map();
  iniciado = null;
  vistosCorrendo.clear();
  silenciosos.clear();
  aoTerminar.clear();
}
