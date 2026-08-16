// Ponte tipada com o "Ajustar para esta máquina".
//
// A memória mostrada aqui não é estimativa nossa: vem do `llama-fit-params`,
// que roda no pacote do próprio llama.cpp e responde por dispositivo em menos
// de dois segundos, sem carregar o modelo. Por isso a tela pode dizer
// "10,7 de 16 GB" sem estar chutando.

import { invoke, isTauri } from "./tauri";

/** Tipo do KV cache. Comprimir troca um pouco de qualidade por memória. */
export type KvType = "f16" | "q8_0" | "q4_0";

/** Especulação: prevê tokens à frente. Só é ligada depois de medida. */
export type SpecType = "none" | "mtp" | "ngram";

/** De onde veio a configuração. */
export type ProfileSource = "manual" | "recommended" | "tested";

/** Configuração de carga de um modelo. Campo ausente = o llama.cpp decide. */
export interface ModelProfile {
  ctx?: number | null;
  /** Camadas na GPU. É resultado exibido, não botão. */
  ngl?: number | null;
  ncmoe?: number | null;
  kvK?: KvType | null;
  kvV?: KvType | null;
  flashAttn?: boolean | null;
  batch?: number | null;
  ubatch?: number | null;
  threads?: number | null;
  spec?: SpecType | null;
  mmproj?: string | null;
  source: ProfileSource;
}

/** O que a pessoa quer desta máquina — a tela mostra as duas respostas. */
export type Intent = "balanced" | "moreContext";

export interface DeviceMemory {
  modelBytes: number;
  contextBytes: number;
  computeBytes: number;
}

export interface ProbeReport {
  /** Por dispositivo, com o nome que o llama.cpp dá (`CUDA0`, `Host`). */
  devices: [string, DeviceMemory][];
}

export interface TuneOption {
  profile: ModelProfile;
  intent: Intent;
  report: ProbeReport;
  fitsGpu: boolean;
  gpuBytes: number;
  hostBytes: number;
}

/** Um motivo: a chave traduz em `tune.reason.*`, os valores entram nela. */
export interface Reason {
  key: string;
  values: [string, string][];
}

export interface TuneAdvice {
  model: string;
  recommended: number;
  options: TuneOption[];
  reasons: Reason[];
  current: ModelProfile | null;
  vramBytes: number;
  /** Fatos detectados que ainda não viram decisão: `mtp`, `vision`. */
  facts: string[];
}

export interface TuneApplied {
  ok: boolean;
  error: string | null;
  profile: ModelProfile | null;
}

/** Calcula a configuração recomendada (uma sonda por candidato, ~2 s cada). */
export function tuneAdvise(model: string): Promise<TuneAdvice> {
  if (!isTauri) return Promise.resolve(mockAdvice(model));
  return invoke<TuneAdvice>("tune_advise", { model });
}

/**
 * Grava a configuração, reinicia o motor e confere que o modelo carrega.
 *
 * Nunca lança por causa da configuração: quando o modelo não carrega, o
 * backend restaura o perfil anterior sozinho e devolve `ok: false` com o
 * motivo. Só lança quando o motor está ocupado (`engine-busy:…`), e aí a
 * escolha continua gravada — é só repetir com `force`.
 */
export function tuneApply(
  model: string,
  profile: ModelProfile,
  force = false,
): Promise<TuneApplied> {
  if (!isTauri) {
    return Promise.resolve({ ok: true, error: null, profile });
  }
  return invoke<TuneApplied>("tune_apply", { model, profile, force });
}

// ------------------------------------------------------------- simulação ---

function mockAdvice(model: string): TuneAdvice {
  const gib = 1024 ** 3;
  const dev = (m: number, c: number): DeviceMemory => ({
    modelBytes: m * gib,
    contextBytes: c * gib,
    computeBytes: 0.5 * gib,
  });
  return {
    model,
    recommended: 0,
    options: [
      {
        profile: {
          ctx: 32768,
          ngl: 36,
          flashAttn: true,
          source: "recommended",
        },
        intent: "balanced",
        report: { devices: [["CUDA0", dev(5.8, 1.1)]] },
        fitsGpu: true,
        gpuBytes: 7.4 * gib,
        hostBytes: 0.5 * gib,
      },
      {
        profile: {
          ctx: 131072,
          ngl: 36,
          kvK: "q8_0",
          kvV: "q8_0",
          flashAttn: true,
          source: "recommended",
        },
        intent: "moreContext",
        report: { devices: [["CUDA0", dev(5.8, 2.3)]] },
        fitsGpu: true,
        gpuBytes: 8.6 * gib,
        hostBytes: 0.5 * gib,
      },
    ],
    reasons: [
      { key: "ctx", values: [["ctx", "32768"], ["kv", String(1.1 * gib)]] },
      { key: "flashAttn", values: [] },
      {
        key: "fitsGpu",
        values: [
          ["used", String(7.4 * gib)],
          ["total", String(16 * gib)],
        ],
      },
      {
        key: "alternative",
        values: [
          ["ctx", "131072"],
          ["used", String(8.6 * gib)],
        ],
      },
    ],
    current: null,
    vramBytes: 16 * gib,
    facts: ["mtp"],
  };
}
