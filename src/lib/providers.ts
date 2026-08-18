// Fontes de LLM além do llama.cpp local (OpenRouter, 9router).
//
// Mesmo contrato dos outros wrappers do projeto: nenhuma tela chama `invoke`
// direto. A autoridade sobre o formato da referência de modelo é o Rust
// (`lr_providers::ModelRef`); `splitModelRef`/`joinModelRef` abaixo são o
// espelho síncrono que o seletor do chat precisa no render, e os casos que
// eles reproduzem estão travados nos testes daquele módulo.

import { invoke, isTauri, listen } from "./tauri";

export type ProviderId = "local" | "9router" | "openrouter";

/** Prefixos que identificam provedor numa referência de modelo. */
const PREFIXES: Record<string, ProviderId> = {
  "9router": "9router",
  openrouter: "openrouter",
};

export interface ResolvedEndpoint {
  provider: ProviderId;
  baseUrl: string;
  apiKey: string | null;
  /** Pares nome/valor a anexar na requisição (atribuição do OpenRouter). */
  headers: [string, string][];
}

export interface ProviderView {
  id: ProviderId;
  ready: boolean;
  reason: string | null;
  baseUrl: string | null;
}

export interface OpenRouterModel {
  id: string;
  name: string;
  contextLength: number | null;
  promptPrice: number | null;
  completionPrice: number | null;
  isFree: boolean;
  supportsTools: boolean;
}

export interface KeyInfo {
  label: string;
  usage: number;
  limit: number | null;
  isFreeTier: boolean;
}

export interface OpenRouterConfig {
  enabled: boolean;
  apiKey: string;
  favorites: string[];
}

export interface NineRouterConfig {
  installed: boolean;
  version: string;
  port: number;
  password: string;
  jwtSecret: string;
}

export interface ProvidersConfig {
  openRouter: OpenRouterConfig;
  nineRouter: NineRouterConfig;
}

export const defaultProvidersConfig = (): ProvidersConfig => ({
  openRouter: { enabled: false, apiKey: "", favorites: [] },
  nineRouter: {
    installed: false,
    version: "",
    port: 20128,
    password: "",
    jwtSecret: "",
  },
});

/**
 * Separa o provedor do nome do modelo.
 *
 * Corta no PRIMEIRO `:` e só aceita prefixo conhecido: ids do OpenRouter têm
 * `/` e podem ter `:` (`anthropic/claude-sonnet-4.5:beta`), e nomes de GGUF
 * também — tratar qualquer coisa antes de `:` como provedor quebraria os
 * dois. Sem prefixo reconhecido é modelo local, que é como toda conversa
 * anterior a esta funcionalidade está gravada.
 */
export function splitModelRef(raw: string): { provider: ProviderId; model: string } {
  const texto = raw.trim();
  const corte = texto.indexOf(":");
  if (corte > 0) {
    const prefixo = PREFIXES[texto.slice(0, corte)];
    const resto = texto.slice(corte + 1);
    if (prefixo && resto) return { provider: prefixo, model: resto };
  }
  return { provider: "local", model: texto };
}

/** Inverso de `splitModelRef`. O local não ganha prefixo. */
export function joinModelRef(provider: ProviderId, model: string): string {
  return provider === "local" ? model : `${provider}:${model}`;
}

// ---------------------------------------------------------------- comandos ---

export const providersConfigGet = async (): Promise<ProvidersConfig> => {
  if (!isTauri) return defaultProvidersConfig();
  const raw = await invoke<string | null>("providers_config_get");
  if (!raw) return defaultProvidersConfig();
  // Parse defensivo: o setting pode ter sido editado à mão. JSON estragado
  // deixa a tela nos padrões — igual ao backend.
  try {
    const cfg = JSON.parse(raw) as Partial<ProvidersConfig>;
    const padrao = defaultProvidersConfig();
    return {
      openRouter: { ...padrao.openRouter, ...(cfg.openRouter ?? {}) },
      nineRouter: { ...padrao.nineRouter, ...(cfg.nineRouter ?? {}) },
    };
  } catch {
    return defaultProvidersConfig();
  }
};

export const providersConfigSet = (cfg: ProvidersConfig): Promise<void> =>
  isTauri
    ? invoke<void>("providers_config_set", { json: JSON.stringify(cfg) })
    : Promise.resolve();

export const providersList = (): Promise<ProviderView[]> =>
  isTauri ? invoke<ProviderView[]>("providers_list") : Promise.resolve([]);

export const providerEndpoint = (modelRef: string): Promise<ResolvedEndpoint> =>
  invoke<ResolvedEndpoint>("provider_endpoint", { modelRef });

export const openRouterModels = (): Promise<OpenRouterModel[]> =>
  isTauri ? invoke<OpenRouterModel[]>("openrouter_models") : Promise.resolve([]);

export const openRouterKeyInfo = (): Promise<KeyInfo> =>
  invoke<KeyInfo>("openrouter_key_info");

// ----------------------------------------------------------------- 9router ---

export interface NineRouterStatus {
  nodeInstalled: boolean;
  installed: boolean;
  running: boolean;
  port: number;
  /** URL do painel quando no ar — é o que o iframe carrega. */
  dashboardUrl: string | null;
  password: string;
  version: string;
}

/** Progresso e log da instalação/execução dos provedores gerenciados. */
export type ProviderEvent =
  | { kind: "progress"; asset: string; receivedBytes: number; totalBytes: number }
  | { kind: "extracting"; asset: string }
  | { kind: "installing"; phase: string }
  | { kind: "log"; line: string }
  | { kind: "ready" }
  | { kind: "failed"; message: string };

export const onProviderEvent = (h: (e: ProviderEvent) => void) =>
  listen<ProviderEvent>("provider", h);

export const nineRouterStatus = (): Promise<NineRouterStatus> =>
  invoke<NineRouterStatus>("ninerouter_status");

export const nineRouterInstall = (): Promise<NineRouterStatus> =>
  invoke<NineRouterStatus>("ninerouter_install");

export const nineRouterStart = (): Promise<NineRouterStatus> =>
  invoke<NineRouterStatus>("ninerouter_start");

export const nineRouterStop = (): Promise<NineRouterStatus> =>
  invoke<NineRouterStatus>("ninerouter_stop");

export const nineRouterUninstall = (removeData: boolean): Promise<NineRouterStatus> =>
  invoke<NineRouterStatus>("ninerouter_uninstall", { removeData });

// ----------------------------------------------------------------- gateway ---

export interface GatewayStatus {
  installed: boolean;
  running: boolean;
  port: number;
  exposeLan: boolean;
  baseUrl: string | null;
  /** Prefixos ativos agora (`/local`, `/9router`). */
  routes: string[];
}

export const gatewayStatus = (): Promise<GatewayStatus> =>
  invoke<GatewayStatus>("gateway_status");

export const gatewayConfigSet = (port: number, exposeLan: boolean): Promise<void> =>
  invoke<void>("gateway_config_set", { port, exposeLan });

export const gatewayInstall = (): Promise<GatewayStatus> =>
  invoke<GatewayStatus>("gateway_install");

export const gatewayStart = (): Promise<GatewayStatus> =>
  invoke<GatewayStatus>("gateway_start");

export const gatewayStop = (): Promise<GatewayStatus> =>
  invoke<GatewayStatus>("gateway_stop");

export const gatewayRefreshRoutes = (): Promise<GatewayStatus> =>
  invoke<GatewayStatus>("gateway_refresh_routes");

export const gatewayUninstall = (): Promise<GatewayStatus> =>
  invoke<GatewayStatus>("gateway_uninstall");
