// Ponte tipada com o "Ajustar para esta máquina".
//
// A memória mostrada aqui não é estimativa nossa: vem do `llama-fit-params`,
// que roda no pacote do próprio llama.cpp e responde por dispositivo em menos
// de dois segundos, sem carregar o modelo. Por isso a tela pode dizer
// "10,7 de 16 GB" sem estar chutando.

import { invoke, isTauri, listen } from "./tauri";

/** Tipo do KV cache. Comprimir troca um pouco de qualidade por memória. */
export type KvType = "f16" | "q8_0" | "q4_0";

/** Especulação: prevê tokens à frente. Só é ligada depois de medida. */
export type SpecType =
  | "none"
  | "draftSimple"
  | "draftEagle3"
  | "draftMtp"
  | "draftDflash"
  | "draftDspark"
  | "ngramSimple"
  | "ngramMapK"
  | "ngramMapK4v"
  | "ngramMod"
  | "ngramCache";

/** Quando carregar o projetor de visão. Ausente = sob demanda. */
export type VisionMode = "off" | "onDemand" | "always";

/** De onde veio a configuração. */
export type ProfileSource = "manual" | "recommended" | "tested";

/** Configuração de carga de um modelo. Campo ausente = o llama.cpp decide. */
/**
 * Como o arquivo do modelo vai para a memória (`--load-mode`).
 *
 * `none` lê tudo para a RAM antes de responder — rende quando sobra memória e
 * cobra caro quando não sobra. Com o modelo maior que a RAM, o mapeamento é o
 * que faz o trabalho.
 */
export type LoadMode = "auto" | "none" | "mmap" | "mlock" | "mmapMlock" | "dio";

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
  /**
   * Os tipos ligados ao mesmo tempo — o `--spec-type` do llama.cpp aceita
   * lista, e um rascunho (MTP) com um n-grama são complementares: um adivinha
   * texto novo, o outro adivinha o que já está no prompt.
   */
  spec?: SpecType[] | null;
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
  /** Como o arquivo vai para a memória (`--load-mode`). */
  loadMode?: LoadMode | null;
  /** @deprecated obsoleto no llama.cpp; lido de perfis antigos e convertido. */
  mmap?: boolean | null;
  /** @deprecated obsoleto no llama.cpp; lido de perfis antigos e convertido. */
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
 * ao fim. Lança `engine-busy:<quem>` quando outra tarefa está usando o motor.
 */
export function tuneBench(
  model: string,
  profiles: ModelProfile[],
  force = false,
): Promise<BenchOutcome> {
  if (!isTauri) return Promise.resolve(mockBench(model, profiles));
  return invoke<BenchOutcome>("tune_bench", { model, profiles, force });
}

/** Um ponto medido da curva de um botão. */
export interface SweepPoint {
  /** O valor testado nesta dimensão. */
  value: number;
  genTps: number;
  promptTps: number;
}

export interface SweepOutcome {
  dim: "ncmoe" | "ubatch";
  points: SweepPoint[];
  /** Com que tamanho de prompt a curva foi medida. */
  nPrompt: number;
}

/**
 * Mede a curva de um botão numa invocação só do `llama-bench`.
 *
 * `ncmoe` responde "onde parar de empurrar especialista para a CPU" e
 * `ubatch`, "de que tamanho compensa a passada de prompt". As duas perguntas
 * não têm resposta calculável — só medível.
 */
export function tuneSweep(
  model: string,
  dim: "ncmoe" | "ubatch",
  force = false,
): Promise<SweepOutcome> {
  if (!isTauri) return Promise.resolve(mockSweep(dim));
  return invoke<SweepOutcome>("tune_sweep", { model, dim, force });
}

function mockSweep(dim: "ncmoe" | "ubatch"): SweepOutcome {
  const points =
    dim === "ncmoe"
      ? [30, 32, 34, 38, 46].map((value, i) => ({
          value,
          genTps: 17.4 - i * 0.9,
          promptTps: 340 - i * 12,
        }))
      : [128, 256, 512, 1024, 2048].map((value, i) => ({
          value,
          genTps: 17.1,
          promptTps: [22, 61, 148, 291, 345][i],
        }));
  return { dim, points, nPrompt: 4096 };
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

// ---------------------------------------------------------- histórico ---

/**
 * Uma medição gravada — espelho do `PerfRowDto` do backend (serde camelCase).
 */
export interface PerfRowDto {
  /** Como gravado no banco (epoch em milissegundos). */
  measuredAt: number;
  genTps: number;
  promptTps: number | null;
  genStddev: number | null;
  suspect: boolean;
  source: string;
  buildNumber: number;
  gpuName: string | null;
  profileKey: string;
  /**
   * Pares do INI da configuração medida; `null` em linhas antigas (antes da
   * migração) — exibir o profileKey encurtado ou "—", NUNCA rotular de "uso".
   */
  profileSummary: Record<string, string> | null;
  /** Limite de energia da placa nesta medição, em watts. */
  powerLimitW: number | null;
  deltaPct: number | null;
  /**
   * `powerChanged` compara normalmente, mas avisa: a diferença veio dos
   * watts, não da configuração. Os demais motivos anulam o percentual.
   */
  deltaReason:
    | "ok"
    | "first"
    | "buildChange"
    | "suspect"
    | "promptChanged"
    | "powerChanged";
}

/** Uso real: média de tokens/s das respostas do chat, por configuração. */
export interface UsageRowDto {
  profileKey: string;
  avgTps: number;
  samples: number;
}

export interface PerfHistoryDto {
  /** `best_gpu().name` do hardware atual; `null` => "CPU". */
  gpuName: string | null;
  /** `""` para perfil vazio (normalização ""≡None é do backend). */
  currentProfileKey: string;
  bestProfileKey: string | null;
  rows: PerfRowDto[];
  usage: UsageRowDto[];
}

/**
 * Histórico de medições desta máquina para um modelo, mais novo primeiro.
 *
 * A série é recortada pelo machine_key atual: trocar de GPU ou de driver
 * "aposenta" as medições antigas por design — elas não são comparáveis.
 */
export function perfHistory(modelId: string): Promise<PerfHistoryDto> {
  if (!isTauri) return Promise.resolve(mockPerfHistory());
  return invoke<PerfHistoryDto>("perf_history", { modelId });
}

function mockPerfHistory(): PerfHistoryDto {
  // measuredAt em milissegundos, como o backend grava (scheduler::now_ms()).
  const agora = Date.now();
  const dia = 24 * 60 * 60 * 1000;
  const chave = "a1b2c3d4e5f60718";
  const resumo: Record<string, string> = {
    "gpu-layers": "99",
    "ctx-size": "16384",
    "flash-attn": "on",
    "batch-size": "1024",
  };
  const base = {
    promptTps: 812,
    genStddev: 0.4,
    source: "tune_bench",
    gpuName: "GPU simulada 16 GB",
    profileKey: chave,
    profileSummary: resumo,
  };
  return {
    gpuName: "GPU simulada 16 GB",
    currentProfileKey: chave,
    bestProfileKey: chave,
    rows: [
      {
        ...base,
        measuredAt: agora - dia,
        genTps: 54.2,
        suspect: false,
        buildNumber: 10441,
        deltaPct: 3.1,
        powerLimitW: 370,
        deltaReason: "ok",
      },
      {
        ...base,
        measuredAt: agora - 3 * dia,
        genTps: 52.6,
        suspect: false,
        buildNumber: 10441,
        deltaPct: null,
        powerLimitW: 370,
        deltaReason: "buildChange",
      },
      {
        ...base,
        measuredAt: agora - 9 * dia,
        genTps: 49.8,
        suspect: true,
        buildNumber: 10380,
        deltaPct: null,
        powerLimitW: 250,
        deltaReason: "powerChanged",
      },
      {
        ...base,
        measuredAt: agora - 15 * dia,
        genTps: 50.8,
        suspect: false,
        buildNumber: 10380,
        profileKey: "",
        profileSummary: null,
        deltaPct: null,
        powerLimitW: 370,
        deltaReason: "first",
      },
    ],
    usage: [
      { profileKey: chave, avgTps: 48.7, samples: 132 },
      { profileKey: "9f8e7d6c5b4a3921", avgTps: 44.1, samples: 18 },
    ],
  };
}

// ---------------------------------------------------------- especulação ---

/** O que um braço da medição de especulação rendeu. */
/**
 * A resposta continuou a mesma?
 *
 * Especulação é *lossless* — o modelo grande confere cada rascunho —, então
 * com temperatura zero a saída tem de bater com a de quem não especula. Se
 * mudou, não é otimização: é defeito, e o braço é recusado.
 */
export type SpecQuality = "match" | "truncated" | "diverged" | "unverifiable";

/** Onde os textos passaram a discordar, para a tela mostrar o trecho. */
export interface Divergence {
  prompt: string;
  atChar: number;
  expected: string;
  got: string;
}

export interface SpecArm {
  spec: SpecType[];
  /** Tokens por segundo por tipo de texto (`code`, `prose`). */
  byPrompt: [string, number][];
  avgTps: number;
  quality: SpecQuality;
  divergence: Divergence | null;
}

export interface SpecOutcome {
  model: string;
  arms: SpecArm[];
  best: number | null;
  /** A diferença é pequena demais para valer uma mudança de configuração. */
  inconclusive: boolean;
  /** Índice do braço sem especulação — a régua de tudo. */
  reference: number;
  /** `unverifiable`: a máquina não repete a si mesma; nada é recusado. */
  qualityGate: "ok" | "unverifiable";
  /** Braços recusados por mudarem a resposta. */
  rejected: number[];
}

/**
 * Mede especulação (MTP e n-grama) nesta máquina.
 *
 * É o eixo que não dá para responder sem gerar de verdade: cada braço exige o
 * motor de pé com aquela configuração, e um reinício entre eles. Custa mais
 * que o teste comum, por isso é um botão à parte. Não aplica nada — devolve
 * os números.
 */
/** O que a medição automática decidiu para um modelo. */
export interface SpecDecision {
  key: string;
  spec: SpecType[];
  gainPct: number | null;
  verdict: "applied" | "inconclusive" | "rejected" | "deferred" | "unverifiable";
  at: number;
}

/** Progresso da medição automática, no evento `tune-spec`. */
export interface SpecProgress {
  phase: "start" | "done";
  model: string;
  outcome?: SpecOutcome;
  verdict?: string;
}

export function onTuneSpec(h: (p: SpecProgress) => void) {
  return listen<SpecProgress>("tune-spec", h);
}

/** Interrompe a bateria automática — quem chegou usar tem precedência. */
export function tuneSpecCancel(): Promise<void> {
  return isTauri ? invoke<void>("tune_spec_cancel") : Promise.resolve();
}

export function tuneSpecBench(
  model: string,
  profile: ModelProfile,
  force = false,
): Promise<SpecOutcome> {
  if (!isTauri) {
    return Promise.resolve({
      model,
      arms: [
        {
          spec: ["none"],
          byPrompt: [["code", 40], ["prose", 38]],
          avgTps: 39,
          quality: "match",
          divergence: null,
        },
        {
          spec: ["ngramMod"],
          byPrompt: [["code", 71], ["prose", 33]],
          avgTps: 52,
          quality: "match",
          divergence: null,
        },
        {
          spec: ["draftMtp", "ngramMod"],
          byPrompt: [["code", 88], ["prose", 41]],
          avgTps: 64.5,
          quality: "match",
          divergence: null,
        },
        {
          spec: ["draftDflash"],
          byPrompt: [["code", 120], ["prose", 96]],
          avgTps: 108,
          quality: "diverged",
          divergence: {
            prompt: "code",
            atChar: 42,
            expected: "t += x; } t }",
            got: "t -= x; } t }",
          },
        },
      ],
      best: 2,
      inconclusive: false,
      reference: 0,
      qualityGate: "ok",
      rejected: [3],
    });
  }
  return invoke<SpecOutcome>("tune_spec_bench", { model, profile, force });
}
