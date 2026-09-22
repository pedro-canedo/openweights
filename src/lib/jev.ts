// Jev — a camada de decisão barata na frente dos modelos locais.
//
// O Jev não gera texto: recebe a conversa resumida e devolve, em
// milissegundos, quanto raciocínio a mensagem pede. O chat usa isso para
// trocar o `effort` da requisição ANTES de mandá-la ao llama-server — o
// streaming continua direto do webview, como sempre.
//
// Quem decide é uma cadeia no backend: o decisor LOCAL (um segundo
// llama-server, do fork `parallel-decision`, com um modelo pequeno só para
// isso) primeiro; o Jev da TypeSafe via OpenRouter como reserva; e o padrão
// da pessoa quando nenhum responde. Aqui só o resumo e o resultado.
//
// Mesmo contrato dos outros wrappers: nenhuma tela chama `invoke` direto.

import { invoke, isTauri } from "./tauri";
import { listen } from "@tauri-apps/api/event";
import type { ChatMessage } from "./llama";
import type { EffortLevel } from "./types";

/** Quem decidiu: o decisor local, o Jev remoto, ou o padrão da pessoa. */
export type JevSource = "local" | "jev" | "padrao";

export interface JevDecision {
  /** O `effort` a usar — o próprio padrão quando nada foi decidido. */
  effort: EffortLevel;
  source: JevSource;
  confidence: number | null;
  /** Em dólares, quando o OpenRouter informou. */
  cost: number | null;
  /** Por que caiu no padrão. */
  reason: string | null;
}

export interface JevConfig {
  enabled: boolean;
  model: string;
  gateChat: boolean;
  gateHarness: boolean;
  minConfidence: number;
  /** `null` = a regra da VRAM decide; `true`/`false` é a escolha da pessoa. */
  localEnabled: boolean | null;
  /** O GGUF da biblioteca que o decisor local carrega. */
  localModel: string;
  /** Cair no Jev remoto quando o local não responde. */
  remoteFallback: boolean;
}

export interface JevDecisionSample {
  origem: JevSource;
  fonte: "local" | "jev" | null;
  nivel: "nenhum" | "medio" | "alto" | null;
  confianca: number | null;
  custo: number | null;
  motivo: string | null;
}

export interface JevLastDecision {
  superficie: "chat" | "proxy";
  decisao: JevDecisionSample;
}

export interface JevTestResult extends JevDecisionSample {
  ms: number;
}

/** O decisor local, como o backend o vê. */
export interface JevLocalStatus {
  supported: boolean;
  enabled: boolean;
  enabledByUser: boolean | null;
  vramOk: boolean;
  runtimeInstalled: boolean;
  runtimeTag: string;
  runtimeApproxBytes: number;
  model: string;
  modelPresent: boolean;
  modelRepo: string;
  modelBytes: number;
  running: boolean;
  ready: boolean;
  baseUrl: string | null;
  port: number | null;
}

export interface JevStatus {
  enabled: boolean;
  keyPresent: boolean;
  remoteFallback: boolean;
  gateChat: boolean;
  gateHarness: boolean;
  minConfidence: number;
  model: string;
  proxyRunning: boolean;
  proxyPort: number | null;
  proxyBaseUrl: string | null;
  local: JevLocalStatus;
  contadores: {
    chamadas: number;
    aplicadas: number;
    falhas: number;
    locais: number;
    remotas: number;
    custo: number;
    ultima: JevLastDecision | null;
  };
}

/** Uma mensagem como o Jev a vê: papel e texto, sem anexos. */
export interface JevMessage {
  papel: string;
  texto: string;
}

/** Teto por mensagem no resumo. O backend corta de novo; isto só evita
 * atravessar o IPC com um contexto inteiro que ele vai jogar fora. */
const MAX_CHARS = 4_000;

/** Reduz a conversa ao que o Jev precisa. Imagens viram `[imagem]`. */
export function summarizeForJev(messages: ChatMessage[]): JevMessage[] {
  return messages.map((m) => {
    const texto =
      typeof m.content === "string"
        ? m.content
        : m.content
            .map((p) => (p.type === "text" ? p.text : "[imagem]"))
            .join("\n");
    return { papel: m.role, texto: texto.length > MAX_CHARS ? `${texto.slice(0, MAX_CHARS - 1)}…` : texto };
  });
}

export const DEFAULT_LOCAL_MODEL = "qwen2.5-1.5b-instruct-q8_0.gguf";

export const defaultJevLocalStatus = (): JevLocalStatus => ({
  supported: false,
  enabled: false,
  enabledByUser: null,
  vramOk: false,
  runtimeInstalled: false,
  runtimeTag: "decision-runtime-14d04e75-v1",
  runtimeApproxBytes: 200 * 1024 * 1024,
  model: DEFAULT_LOCAL_MODEL,
  modelPresent: false,
  modelRepo: "Qwen/Qwen2.5-1.5B-Instruct-GGUF",
  modelBytes: 1_894_532_128,
  running: false,
  ready: false,
  baseUrl: null,
  port: null,
});

export const defaultJevStatus = (): JevStatus => ({
  enabled: false,
  keyPresent: false,
  remoteFallback: true,
  gateChat: true,
  gateHarness: true,
  minConfidence: 0.6,
  model: "typesafe/jev-1.13",
  proxyRunning: false,
  proxyPort: null,
  proxyBaseUrl: null,
  local: defaultJevLocalStatus(),
  contadores: { chamadas: 0, aplicadas: 0, falhas: 0, locais: 0, remotas: 0, custo: 0, ultima: null },
});

export const defaultJevConfig = (): JevConfig => ({
  enabled: false,
  model: "typesafe/jev-1.13",
  gateChat: true,
  gateHarness: true,
  minConfidence: 0.6,
  localEnabled: null,
  localModel: DEFAULT_LOCAL_MODEL,
  remoteFallback: true,
});

/**
 * Pede ao backend o esforço desta requisição. `null` fora do Tauri e em
 * qualquer erro: a decisão é um extra, nunca pode segurar a resposta.
 */
export async function jevDecideEffort(
  model: string,
  messages: JevMessage[],
  defaultEffort: EffortLevel,
): Promise<JevDecision | null> {
  if (!isTauri) return null;
  try {
    return await invoke<JevDecision>("jev_decidir_esforco", { model, messages, defaultEffort });
  } catch {
    return null;
  }
}

export const jevStatus = (): Promise<JevStatus> =>
  isTauri ? invoke<JevStatus>("jev_status") : Promise.resolve(defaultJevStatus());

export const jevConfigGet = (): Promise<JevConfig> =>
  isTauri ? invoke<JevConfig>("jev_config_get") : Promise.resolve(defaultJevConfig());

export const jevConfigSet = (config: JevConfig): Promise<JevStatus> =>
  isTauri ? invoke<JevStatus>("jev_config_set", { config }) : Promise.resolve(defaultJevStatus());

export const jevTest = (): Promise<JevTestResult> => invoke<JevTestResult>("jev_testar");

/** Instala o decisor local (motor + modelo); o progresso chega pelo painel
 * de downloads e o processo sobe sozinho quando as duas partes chegam. */
export const jevLocalInstall = (): Promise<JevLocalStatus> =>
  isTauri ? invoke<JevLocalStatus>("jev_local_install") : Promise.resolve(defaultJevLocalStatus());

/** O backend avisa quando o decisor local muda de estado (subiu, caiu,
 * ficou pronto, instalou). */
export const onJevStatus = (h: (s: JevStatus) => void) =>
  isTauri ? listen<JevStatus>("jev-status", (e) => h(e.payload)) : Promise.resolve(() => {});
