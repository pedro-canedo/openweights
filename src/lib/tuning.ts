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

/** Quando carregar o projetor de visão. Ausente = sob demanda. */
export type VisionMode = "off" | "onDemand" | "always";

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
  /** Tokens rascunhados por passada (`spec-draft-n-max`; 2–4 é o equilíbrio). */
  specDraftNMax?: number | null;
  specDraftNMin?: number | null;
  /** Probabilidade mínima para aceitar rascunho (0.75 ajuda em contexto longo). */
  specDraftPMin?: number | null;
  /** Modelo de rascunho externo (caminho GGUF) — sozinho vira `draft-simple`. */
  specDraftModel?: string | null;
  mmproj?: string | null;
  vision?: VisionMode | null;
  kvOffload?: boolean | null;
  mmap?: boolean | null;
  mlock?: boolean | null;
  parallel?: number | null;
  /** Qualquer outra flag do catálogo, como pares [chave, valor] do INI. */
  extras?: [string, string][];
  source: ProfileSource;
}

/** Nada escolhido: o llama.cpp decide tudo. */
export function emptyProfile(): ModelProfile {
  return { source: "manual" };
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

// ---------------------------------------------------------------- medido ---

/** O que uma configuração rendeu de verdade nesta máquina. */
export interface BenchResult {
  /** Tokens por segundo gerando — o número que a pessoa sente. */
  genTps: number;
  /** Tokens por segundo processando o prompt. */
  promptTps: number;
  genStddev: number;
  buildNumber: number;
  cpuInfo: string;
  gpuInfo: string;
  backends: string;
  modelSize: number;
}

/** Progresso da medição, emitido no evento `tune-bench`. */
export interface BenchProgress {
  model: string;
  step: number;
  total: number;
  last?: BenchResult;
}

export interface BenchOutcome {
  model: string;
  results: [ModelProfile, BenchResult][];
  best: number | null;
  /** A placa esquentou durante a corrida: os números servem, com ressalva. */
  suspect: boolean;
}

/**
 * Mede as configurações nesta máquina.
 *
 * Custa minutos e a placa inteira: o motor é derrubado enquanto roda e volta
 * ao fim. Lança `engine-busy:<quem>` quando há execução de agente, automação
 * ou indexação em curso.
 */
export function tuneBench(
  model: string,
  profiles: ModelProfile[],
  force = false,
): Promise<BenchOutcome> {
  if (!isTauri) return Promise.resolve(mockBench(model, profiles));
  return invoke<BenchOutcome>("tune_bench", { model, profiles, force });
}

/** Para a medição: a configuração atual termina, as próximas não começam. */
export function tuneBenchCancel(): Promise<void> {
  if (!isTauri) return Promise.resolve();
  return invoke<void>("tune_bench_cancel");
}

function mockBench(model: string, profiles: ModelProfile[]): BenchOutcome {
  const base = {
    promptTps: 810,
    genStddev: 0.4,
    buildNumber: 10441,
    cpuInfo: "AMD Ryzen 5 4600G",
    gpuInfo: "NVIDIA GeForce RTX 5060 Ti",
    backends: "CUDA",
    modelSize: 6_969_004_032,
  };
  return {
    model,
    results: profiles.map((p, i) => [p, { ...base, genTps: 54 - i * 8 }]),
    best: 0,
    suspect: false,
  };
}

// ---------------------------------------------------------- especulação ---

/** O que um braço da medição de especulação rendeu. */
export interface SpecArm {
  spec: SpecType;
  /** Tokens por segundo por tipo de texto (`code`, `prose`). */
  byPrompt: [string, number][];
  avgTps: number;
}

export interface SpecOutcome {
  model: string;
  arms: SpecArm[];
  best: number | null;
  /** A diferença é pequena demais para valer uma mudança de configuração. */
  inconclusive: boolean;
}

/**
 * Mede especulação (MTP e n-grama) nesta máquina.
 *
 * É o eixo que não dá para responder sem gerar de verdade: cada braço exige o
 * motor de pé com aquela configuração, e um reinício entre eles. Custa mais
 * que o teste comum, por isso é um botão à parte. Não aplica nada — devolve
 * os números.
 */
export function tuneSpecBench(
  model: string,
  profile: ModelProfile,
  force = false,
): Promise<SpecOutcome> {
  if (!isTauri) {
    return Promise.resolve({
      model,
      arms: [
        { spec: "none", byPrompt: [["code", 40], ["prose", 38]], avgTps: 39 },
        { spec: "ngram", byPrompt: [["code", 71], ["prose", 33]], avgTps: 52 },
      ],
      best: 1,
      inconclusive: false,
    });
  }
  return invoke<SpecOutcome>("tune_spec_bench", { model, profile, force });
}
