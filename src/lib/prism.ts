// O motor da PrismML (modelos Bonsai 2): estado, instalação e o erro tipado
// que o backend devolve quando um modelo precisa dele e ele não está lá.
//
// Store fora do React, como `engine.ts`: a instalação começa num lugar (o
// Descobrir, o chat, a biblioteca) e o progresso tem de aparecer em todos
// eles, inclusive depois de trocar de tela. Uma única assinatura do evento
// `runtime-prism` alimenta todo mundo.
//
// O estado é por ARQUIVO além de por máquina: no Linux com NVIDIA, um Bonsai
// PQ2_0 pede o motor CUDA 12.8 mesmo com o Vulkan instalado (no Vulkan esse
// formato roda na CPU), enquanto o PTQ1_0 abre no Vulkan sem baixar nada.

import { cancelPrism, ensurePrism, getPrismStatus, onPrismEvent, retryPrismCuda } from "./api";
import { isTauri } from "./tauri";
import type { BackendVariant, PrismStatus, RuntimeEvent, RuntimeState } from "./types";

export interface PrismSnapshot {
  /** O motor da máquina, sem arquivo em vista. */
  state: PrismStatus | null;
  /** O mesmo, respondido para o arquivo de cada modelo que perguntou. */
  porModelo: Readonly<Record<string, PrismStatus>>;
  /**
   * O último modelo que o chat tentou abrir e ouviu `prism-required` — o
   * cartão do chat não recebe o modelo, e é por ele que sabe o que oferecer.
   */
  modeloExigido: string | null;
  installing: boolean;
  progress: RuntimeEvent | null;
  error: string | null;
}

/** O erro tipado do backend: o modelo exige o motor e ele não está instalado. */
export const PRISM_REQUIRED = "prism-required";

/**
 * O código de uma instalação cancelada (no evento `failed` e no erro do
 * comando): a tela o trata como "parou", não como erro.
 */
export const PRISM_CANCELLED = "prism-install-cancelled";

function mensagem(err: unknown): string {
  return err instanceof Error ? err.message : typeof err === "string" ? err : "";
}

/** `true` quando a falha (Error, string ou texto de bolha) é a falta do motor. */
export function isPrismRequired(err: unknown): boolean {
  return mensagem(err).includes(PRISM_REQUIRED);
}

/** `true` quando a falha é a instalação cancelada pela pessoa. */
export function isPrismCancelled(err: unknown): boolean {
  return mensagem(err).includes(PRISM_CANCELLED);
}

/** Nome curto da variante, para rótulos ("PrismML · CUDA 12.8"). */
export function variantLabel(v: BackendVariant): string {
  switch (v) {
    case "cuda13":
      return "CUDA 13.3";
    case "cuda12":
      return "CUDA 12.4";
    case "cuda128":
      return "CUDA 12.8";
    case "vulkan":
      return "Vulkan";
    case "cpu":
      return "CPU";
    case "macos-arm64":
    case "macos-x64":
      return "Metal";
  }
}

/** O que o cartão do motor mostra para um estado. */
export type PrismCardMode =
  | { kind: "hidden" }
  /**
   * Falta instalar. `upgradeFrom`: o motor da PrismML já está aqui nesta
   * variante (o Vulkan, ou a CPU de uma máquina onde o Vulkan também
   * reprovou), mas o arquivo é PQ2_0 e a instalação é a versão CUDA.
   */
  | {
      kind: "install";
      bytes: number;
      diskBytes: number | null;
      upgradeFrom: BackendVariant | null;
    }
  /** Instalado, mas o PQ2_0 vai rodar na CPU (Vulkan). Aviso, não bloqueio. */
  | { kind: "pq2OnCpu"; cudaFailed: boolean };

export function prismCardMode(st: PrismStatus | null): PrismCardMode {
  if (!st) return { kind: "hidden" };
  if (!st.installed) {
    return {
      kind: "install",
      bytes: st.downloadBytes,
      diskBytes: st.diskBytes,
      upgradeFrom: st.pq2 && st.variant === "cuda128" ? st.installedVariant : null,
    };
  }
  if (st.pq2OnCpu) return { kind: "pq2OnCpu", cudaFailed: st.cudaFailed };
  return { kind: "hidden" };
}

/**
 * Para o Descobrir, antes do download (sem cabeçalho, pelo nome): quanto o
 * motor baixa junto deste arquivo (`null` = nada a baixar) e se, NESTA
 * máquina, um PQ2_0 vai rodar na CPU — o caso de sugerir o PTQ1_0.
 */
export function prismParaDownload(
  st: PrismStatus | null,
  artifactName: string,
): { bytes: number | null; pq2NaCpu: boolean } {
  if (!st) return { bytes: null, pq2NaCpu: false };
  const pq2 = /PQ2_0/i.test(artifactName);
  if (pq2 && st.cudaDownloadBytes != null) return { bytes: st.cudaDownloadBytes, pq2NaCpu: false };
  const motor = st.installedVariant ?? st.variant;
  return {
    bytes: st.installed ? null : st.downloadBytes,
    pq2NaCpu: pq2 && motor === "vulkan",
  };
}

let snap: PrismSnapshot = {
  state: null,
  porModelo: {},
  modeloExigido: null,
  installing: false,
  progress: null,
  error: null,
};
const listeners = new Set<() => void>();
let assinado = false;

function emit(next: Partial<PrismSnapshot>) {
  snap = { ...snap, ...next };
  for (const l of listeners) l();
}

/**
 * O fim de uma instalação, venha de onde vier. A disparada pelo download de
 * um Bonsai não passa por `instalarPrism`: sem reler aqui, o Descobrir
 * seguiria oferecendo o motor que acabou de chegar (ou escondendo a dica do
 * PTQ1_0 quando a versão CUDA reprovou), e o cartão do chat voltaria ao botão.
 */
function aoTerminar(error: string | null) {
  emit({ progress: null, installing: false, error });
  void recarregarTudo();
}

async function assinar() {
  if (assinado || !isTauri) return;
  assinado = true;
  await onPrismEvent((e) => {
    if (e.kind === "ready") aoTerminar(null);
    else if (e.kind === "failed") aoTerminar(isPrismCancelled(e.message) ? null : e.message);
    else emit({ progress: e, installing: true });
  });
}

/**
 * Lê o estado no backend (barato: `is_file` e, com modelo, o cabeçalho do
 * GGUF). Sem `model`, o da máquina; com, o do arquivo daquele modelo.
 */
export async function carregarPrism(model?: string): Promise<PrismStatus | null> {
  void assinar();
  try {
    const st = await getPrismStatus(model);
    if (model) emit({ porModelo: { ...snap.porModelo, [model]: st } });
    else emit({ state: st });
    return st;
  } catch {
    return model ? (snap.porModelo[model] ?? null) : snap.state;
  }
}

let recarga: Promise<void> | null = null;

/**
 * Relê tudo o que a tela já perguntou — depois de uma instalação, o disco
 * mudou. Quem chama durante uma releitura espera a mesma: o evento `ready` e
 * o fim do comando chegam juntos, depois da mesma mudança.
 */
function recarregarTudo(): Promise<void> {
  return (recarga ??= (async () => {
    try {
      await Promise.all([
        carregarPrism(),
        ...Object.keys(snap.porModelo).map((m) => carregarPrism(m)),
      ]);
    } finally {
      recarga = null;
    }
  })());
}

/**
 * Registra o modelo que ouviu `prism-required` (quem chama é a preparação do
 * endpoint do chat). O cartão do chat pergunta pelo arquivo dele.
 */
export function marcarModeloExigido(model: string): void {
  emit({ modeloExigido: model });
  void carregarPrism(model);
}

let instalando: Promise<RuntimeState> | null = null;

/** Instala o motor (uma instalação por vez; quem chama de novo espera a mesma). */
export function instalarPrism(): Promise<RuntimeState> {
  void assinar();
  return (instalando ??= (async () => {
    emit({ installing: true, error: null });
    try {
      const state = await ensurePrism();
      // Terminou bem: um erro de uma tentativa anterior não fica na tela.
      emit({ installing: false, progress: null, error: null });
      await recarregarTudo();
      return state;
    } catch (e) {
      const error = e instanceof Error ? e.message : String(e);
      emit({ installing: false, progress: null, error: isPrismCancelled(error) ? null : error });
      throw e;
    } finally {
      instalando = null;
    }
  })());
}

/** Desiste da instalação em curso — a desta tela ou a que o download disparou. */
export function cancelarPrism(): Promise<void> {
  return cancelPrism().catch(() => {});
}

/**
 * "Tentar a versão CUDA de novo": esquece a reprovação desta máquina, relê
 * (o arquivo PQ2_0 volta a pedir o CUDA) e instala.
 */
export async function tentarCudaDeNovo(): Promise<RuntimeState> {
  await retryPrismCuda();
  await recarregarTudo();
  return instalarPrism();
}

// Os cartões do chat montados agora, o mais recente por último. Quando o
// motor chega, só ele avisa o chat: a conversa pode ter várias bolhas de
// `prism-required`, e cada uma reenviando seria uma geração por bolha.
const cartoesDoChat: symbol[] = [];

/** Registra um cartão do chat; devolve a função que o tira. */
export function registrarCartaoDoChat(id: symbol): () => void {
  cartoesDoChat.push(id);
  return () => {
    const i = cartoesDoChat.lastIndexOf(id);
    if (i >= 0) cartoesDoChat.splice(i, 1);
  };
}

/** `true` para o cartão do chat que avisa quando o motor chega. */
export function ehCartaoDoChatAtivo(id: symbol): boolean {
  return cartoesDoChat[cartoesDoChat.length - 1] === id;
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
