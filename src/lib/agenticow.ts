// Estado do AgenticOw — fora de qualquer componente, de propósito.
//
// A primeira subida baixa e verifica o runtime, e a pessoa não pode ficar
// presa a uma tela enquanto isso: sair para o Chat e voltar não pode perder o
// progresso, e um segundo clique não pode disparar uma segunda subida. Um
// store de módulo com `useSyncExternalStore` deixa a subida ser do aplicativo,
// não da tela — e é o mesmo estado que o convite do Chat lê para saber o que
// dizer.
//
// A interface do AgenticOw não mora no React: é uma webview FILHA da janela
// principal, posicionada sobre a área de conteúdo da tela (`mostrar`,
// `posicionar`, `esconder`). Ela nasce uma vez e sobrevive à troca de tela;
// esconder não derruba a sessão aberta.

import { invoke, listen } from "./tauri";
import { errorMessage } from "./serverSession";

export interface AgenticowStatus {
  /** Há pacote do AgenticOw para esta máquina. */
  supported: boolean;
  installed: boolean;
  running: boolean;
  ready: boolean;
  port: number | null;
  /** Release do runtime que este app fixa. */
  tag: string;
  /** Revisão do fork que este app fixa. */
  revision: string;
  /** Tag do DeepSeek Harness em que a revisão se baseia (depois da saudação). */
  upstreamTag: string | null;
  lastError: string | null;
  /** Modelos no último catálogo entregue (null antes do primeiro). */
  models: number | null;
  /** O que cada fonte do OpenWeights tem agora. */
  sources: AgenticowSources;
}

export interface AgenticowSources {
  localModels: number;
  serverRunning: boolean;
  openrouterKey: boolean;
  openrouterFavorites: number;
  ninerouterInstalled: boolean;
  ninerouterRunning: boolean;
}

export type AgenticowEvent =
  | { kind: "phase"; phase: string }
  | { kind: "progress"; receivedBytes: number; totalBytes: number }
  | { kind: "ready" }
  | { kind: "failed"; message: string }
  | { kind: "stopped" }
  | { kind: "log"; line: string }
  | { kind: "catalog"; revision: number | null; ok: boolean; message: string | null };

/** Retângulo da área de conteúdo, em pixels do CSS. */
export interface Area {
  x: number;
  y: number;
  width: number;
  height: number;
}

export const agenticowStatus = () => invoke<AgenticowStatus>("agenticow_status");
const agenticowStart = (locale: string) => invoke<AgenticowStatus>("agenticow_start", { locale });
const agenticowStop = () => invoke<AgenticowStatus>("agenticow_stop");
const agenticowUninstall = (removeData: boolean) =>
  invoke<AgenticowStatus>("agenticow_uninstall", { removeData });

export type AgenticowBusy = "start" | "stop" | "uninstall" | null;

export interface AgenticowState {
  status: AgenticowStatus | null;
  busy: AgenticowBusy;
  /** Fase da preparação em curso (`node`, `migrating`, `download`, …). */
  phase: string | null;
  /** Download com tamanho conhecido: vira barra de verdade. */
  progress: { receivedBytes: number; totalBytes: number } | null;
  log: string[];
  error: string | null;
  /** Último catálogo recusado pelo AgenticOw (o aceito não precisa de aviso). */
  catalogError: string | null;
  /** Segundos desde o início da operação atual (0 quando parada). */
  segundos: number;
}

let state: AgenticowState = {
  status: null,
  busy: null,
  phase: null,
  progress: null,
  log: [],
  error: null,
  catalogError: null,
  segundos: 0,
};

const listeners = new Set<() => void>();

function set(patch: Partial<AgenticowState>) {
  state = { ...state, ...patch };
  for (const l of listeners) l();
}

export const agenticowStore = {
  subscribe(l: () => void) {
    listeners.add(l);
    return () => {
      listeners.delete(l);
    };
  },
  get: () => state,
};

let cronometro: ReturnType<typeof setInterval> | null = null;
function iniciarCronometro() {
  pararCronometro();
  set({ segundos: 0 });
  cronometro = setInterval(() => set({ segundos: state.segundos + 1 }), 1000);
}
function pararCronometro() {
  if (cronometro) clearInterval(cronometro);
  cronometro = null;
}

// Um ouvinte só, aberto na primeira necessidade: a subida pode ter começado
// pelo convite do Chat, e a tela tem de ver o progresso do mesmo jeito.
let ouvindo = false;
function garantirOuvinte() {
  if (ouvindo) return;
  ouvindo = true;
  void listen<AgenticowEvent>("agenticow", (e) => {
    switch (e.kind) {
      case "phase":
        set({ phase: e.phase, progress: e.phase === "download" ? state.progress : null });
        break;
      case "progress":
        set({ progress: { receivedBytes: e.receivedBytes, totalBytes: e.totalBytes } });
        break;
      case "log":
        set({ log: [...state.log.slice(-200), e.line] });
        break;
      case "catalog":
        set({ catalogError: e.ok ? null : (e.message ?? "") });
        // A contagem de modelos mudou junto: o aviso de "sem modelos" segue.
        void refreshStatus();
        break;
      case "ready":
      case "failed":
      case "stopped":
        void refreshStatus();
        break;
    }
  });
}

export async function refreshStatus(): Promise<AgenticowStatus | null> {
  garantirOuvinte();
  try {
    const status = await agenticowStatus();
    set({ status });
    return status;
  } catch {
    return state.status;
  }
}

async function operacao(
  nome: Exclude<AgenticowBusy, null>,
  acao: () => Promise<AgenticowStatus>,
): Promise<boolean> {
  garantirOuvinte();
  if (state.busy) return false;
  set({ busy: nome, error: null, phase: null, progress: null, log: [] });
  iniciarCronometro();
  let ok = false;
  try {
    set({ status: await acao() });
    ok = true;
  } catch (e) {
    set({ error: errorMessage(e) });
    await refreshStatus();
  } finally {
    pararCronometro();
    set({ busy: null, phase: null, progress: null });
  }
  return ok;
}

/** O app fala pt-BR e inglês; o AgenticOw também. */
export function idiomaDoAgenticow(language: string | undefined): string {
  return language?.toLowerCase().startsWith("en") ? "en" : "pt-BR";
}

/** Sobe o AgenticOw (instalando o runtime antes, se preciso). */
export const iniciar = (language: string | undefined) =>
  operacao("start", () => agenticowStart(idiomaDoAgenticow(language)));

export const parar = () => operacao("stop", agenticowStop);

export const desinstalar = (removerDados: boolean) =>
  operacao("uninstall", () => agenticowUninstall(removerDados));

/** O idioma do app mudou: o AgenticOw acompanha, sem reiniciar. */
export async function definirIdioma(language: string | undefined): Promise<void> {
  try {
    await invoke("agenticow_set_locale", { locale: idiomaDoAgenticow(language) });
  } catch {
    // Sem Host no ar: o idioma vai na próxima subida.
  }
}

export const mostrar = (area: Area) => invoke<void>("agenticow_show", { area });
export const posicionar = (area: Area) => invoke<void>("agenticow_set_bounds", { area });
export const esconder = () => invoke<void>("agenticow_hide");

export function limparErro(): void {
  set({ error: null });
}
