// Jev (TypeSafe) — a camada de decisão barata na frente dos modelos locais.
//
// O Jev não gera texto: recebe a conversa resumida e devolve, em menos de
// meio segundo, quanto raciocínio a mensagem pede. O chat usa isso para
// trocar o `effort` da requisição ANTES de mandá-la ao llama-server — o
// streaming continua direto do webview, como sempre. Quem fala com o
// OpenRouter é o backend (a chave fica lá); aqui só o resumo e o resultado.
//
// Mesmo contrato dos outros wrappers: nenhuma tela chama `invoke` direto.

import { invoke, isTauri } from "./tauri";
import type { ChatMessage } from "./llama";
import type { EffortLevel } from "./types";

export type JevSource = "jev" | "padrao";

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
}

export interface JevLastDecision {
  superficie: "chat" | "proxy";
  decisao: {
    origem: JevSource;
    nivel: "nenhum" | "medio" | "alto" | null;
    confianca: number | null;
    custo: number | null;
    motivo: string | null;
  };
}

export interface JevStatus {
  enabled: boolean;
  keyPresent: boolean;
  gateChat: boolean;
  gateHarness: boolean;
  minConfidence: number;
  model: string;
  proxyRunning: boolean;
  proxyPort: number | null;
  proxyBaseUrl: string | null;
  contadores: {
    chamadas: number;
    aplicadas: number;
    falhas: number;
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

export const defaultJevStatus = (): JevStatus => ({
  enabled: false,
  keyPresent: false,
  gateChat: true,
  gateHarness: true,
  minConfidence: 0.6,
  model: "typesafe/jev-1.13",
  proxyRunning: false,
  proxyPort: null,
  proxyBaseUrl: null,
  contadores: { chamadas: 0, aplicadas: 0, falhas: 0, custo: 0, ultima: null },
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
  isTauri
    ? invoke<JevConfig>("jev_config_get")
    : Promise.resolve({ enabled: false, model: "typesafe/jev-1.13", gateChat: true, gateHarness: true, minConfidence: 0.6 });

export const jevConfigSet = (config: JevConfig): Promise<JevStatus> =>
  isTauri ? invoke<JevStatus>("jev_config_set", { config }) : Promise.resolve(defaultJevStatus());

export const jevTest = (): Promise<JevLastDecision["decisao"]> =>
  invoke<JevLastDecision["decisao"]>("jev_testar");
