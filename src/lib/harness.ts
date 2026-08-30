// Estado do DeepSeek Harness gerenciado pelo app — fora de qualquer
// componente, de propósito.
//
// A instalação dura minutos (Node portátil + ~190 pacotes npm) e a pessoa
// não pode ficar presa a uma tela enquanto isso: se o estado morasse no
// `useState` da tela, sair para o Chat e voltar perderia o progresso e o
// log, e um segundo clique dispararia uma segunda instalação por cima da
// primeira. Um store de módulo com `useSyncExternalStore` deixa a instalação
// ser do aplicativo, não da tela — e é o mesmo estado que o CTA do Chat lê
// para saber o que dizer no botão.

import {
  dshInstall,
  dshOpenPanel,
  dshStart,
  dshStatus,
  dshStop,
  dshUninstall,
  onProviderEvent,
  type DshStatus,
  type ProviderEvent,
} from "./providers";
import { errorMessage } from "./serverSession";

/** Qual operação está em curso — o rótulo importa para a tela saber o botão. */
export type HarnessBusy = "install" | "start" | "stop" | "uninstall" | null;

export interface HarnessState {
  status: DshStatus | null;
  busy: HarnessBusy;
  progress: ProviderEvent | null;
  log: string[];
  error: string | null;
  /** Segundos desde o início da operação atual (0 quando parada). */
  segundos: number;
}

let state: HarnessState = {
  status: null,
  busy: null,
  progress: null,
  log: [],
  error: null,
  segundos: 0,
};

const listeners = new Set<() => void>();

function set(patch: Partial<HarnessState>) {
  state = { ...state, ...patch };
  for (const l of listeners) l();
}

export const harnessStore = {
  subscribe(l: () => void) {
    listeners.add(l);
    return () => {
      listeners.delete(l);
    };
  },
  get: () => state,
};

/** O cronômetro é do store: ele sobrevive a trocar de tela no meio. */
let cronometro: number | null = null;

function iniciarCronometro() {
  if (cronometro !== null) return;
  set({ segundos: 0 });
  cronometro = window.setInterval(
    () => set({ segundos: state.segundos + 1 }),
    1000,
  );
}

function pararCronometro() {
  if (cronometro !== null) {
    window.clearInterval(cronometro);
    cronometro = null;
  }
  set({ segundos: 0 });
}

let statusPedido = false;

/** Primeira leitura do status. Falha em silêncio (navegador, sem backend). */
export async function refreshStatus(force = false): Promise<void> {
  if (statusPedido && !force) return;
  statusPedido = true;
  try {
    set({ status: await dshStatus() });
  } catch {
    // sem backend ou comando indisponível
  }
}

/**
 * Roda uma operação do harness com progresso e log ao vivo no store.
 *
 * A assinatura de eventos é aberta por operação (e não uma vez para sempre)
 * para não acumular ouvinte a cada montagem; a bandeira `cancelado` é a
 * mesma guarda de StrictMode do NineRouterCard — o `unlisten` pode chegar
 * depois de um segundo registro.
 */
async function comEventos(
  nome: Exclude<HarnessBusy, null>,
  acao: () => Promise<DshStatus>,
): Promise<boolean> {
  if (state.busy) return false;
  set({ busy: nome, error: null, log: [], progress: null });
  iniciarCronometro();
  const sub: { cancelado: boolean; desligar: (() => void) | null } = {
    cancelado: false,
    desligar: null,
  };
  void onProviderEvent((e) => {
    set({
      progress: e,
      log: e.kind === "log" ? [...state.log.slice(-200), e.line] : state.log,
    });
  }).then((f) => {
    if (sub.cancelado) f();
    else sub.desligar = f;
  });
  let ok = false;
  try {
    set({ status: await acao() });
    ok = true;
  } catch (e) {
    set({ error: errorMessage(e) });
  } finally {
    sub.cancelado = true;
    sub.desligar?.();
    pararCronometro();
    set({ busy: null, progress: null });
  }
  return ok;
}

export const instalar = () => comEventos("install", dshInstall);

export const parar = () => comEventos("stop", dshStop);

export const desinstalar = (removerDados: boolean) =>
  comEventos("uninstall", () => dshUninstall(removerDados));

/**
 * Sobe o harness (instalando antes, se preciso). Devolve `true` quando ele
 * ficou no ar — é o gatilho para a tela mostrar o painel.
 */
export async function subir(): Promise<boolean> {
  return comEventos("start", dshStart);
}

/** Janela própria: a saída de emergência de quem prefere fora do app. */
export async function abrirJanela(): Promise<void> {
  try {
    await dshOpenPanel();
  } catch (e) {
    set({ error: errorMessage(e) });
  }
}

export function limparErro(): void {
  set({ error: null });
}
