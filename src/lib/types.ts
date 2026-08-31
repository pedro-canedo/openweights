// Espelhos TypeScript dos tipos Rust (serde rename_all = "camelCase").
// Mantenha em sincronia com src-tauri/crates/*/src e src-tauri/src/commands.rs.

export type GpuVendor = "nvidia" | "amd" | "intel" | "apple" | "other";

export interface GpuInfo {
  name: string;
  vendor: GpuVendor;
  vramTotalBytes: number;
  isIntegrated: boolean;
  driverVersion: string | null;
  cudaCompute: [number, number] | null;
}

export interface HardwareProfile {
  os: string;
  arch: string;
  cpuName: string;
  cpuCores: number;
  avx2: boolean;
  avx512: boolean;
  ramTotalBytes: number;
  gpus: GpuInfo[];
}

export interface GpuTelemetry {
  utilPercent: number | null;
  vramUsedBytes: number | null;
  vramTotalBytes: number;
}

export interface Telemetry {
  cpuPercent: number;
  ramUsedBytes: number;
  ramTotalBytes: number;
  gpus: GpuTelemetry[];
  tsMs: number;
  /** Temperaturas em °C — `null` quando o sistema não expõe (comum na CPU
   *  em Windows sem privilégio). Opcionais: build antiga não os manda. */
  cpuTempC?: number | null;
  gpuTempC?: number | null;
  /** Disco onde mora a pasta de dados do app. */
  diskUsedPct?: number | null;
  diskFreeBytes?: number | null;
  /** Taxa agregada de rede desde a última amostra. */
  netRxBytesPerSec?: number | null;
  netTxBytesPerSec?: number | null;
}

// ------------------------------------------------------------- runtime ---

export type BackendVariant =
  | "cuda13"
  | "cuda12"
  | "vulkan"
  | "cpu"
  | "macos-arm64"
  | "macos-x64";

export interface RuntimeState {
  tag: string;
  variant: BackendVariant;
  installed: boolean;
  serverExe: string | null;
  dir?: string | null;
  rpcExe?: string | null;
  rpcReady?: boolean;
}

export type RuntimeEvent =
  | { kind: "progress"; asset: string; receivedBytes: number; totalBytes: number }
  | { kind: "extracting"; asset: string }
  | { kind: "ready" }
  | { kind: "failed"; message: string };

// -------------------------------------------------------------- modelos ---

export interface ModelSummary {
  id: string;
  author: string;
  name: string;
  downloads: number;
  likes: number;
  paramsTotal: number | null;
  architecture: string | null;
  contextLength: number | null;
  gated: boolean;
  updatedAt: string | null;
  /** Licença do cartão do repositório, quando declarada. */
  license: string | null;
  caps: ModelCaps;
}

/** O que o modelo sabe fazer, derivado pelo backend do que o Hub entrega. */
export interface ModelCaps {
  vision: boolean;
  tools: boolean;
  reasoning: boolean;
}

export type FitVerdict =
  | { kind: "fullGpu"; ngl: number }
  | { kind: "partial"; ngl: number; layersTotal: number }
  | { kind: "cpuOnly" }
  | { kind: "wontFit" };

/** QuantView do Rust: QuantOption achatado + info do artefato. */
export interface QuantView {
  artifactName: string;
  files: string[];
  totalBytes: number;
  filename: string;
  label: string;
  sizeBytes: number;
  bits: number | null;
  recommended: boolean;
  verdict: FitVerdict;
  /** Arquivo + KV cache + reserva, com a janela avaliada. */
  estTotalBytes: number;
  kvCacheBytes: number;
}

/** O que a gaveta de quantizações recebe do backend. */
export interface QuantsView {
  quants: QuantView[];
  /** Janela usada na estimativa (já limitada ao teto do modelo). */
  ctxLen: number;
  /** Janela máxima do modelo, quando o repositório publica. */
  modelCtxMax: number | null;
  /** Tamanho do projetor de visão, quando existe. Não é quantização. */
  visionProjectorBytes: number | null;
  /** Fator de correção aplicado, quando há histórico que o sustente. */
  calibrated: number | null;
}

// ------------------------------------------------------------ downloads ---

export type DownloadState = "queued" | "running" | "paused" | "done" | "error";

export interface DownloadStatus {
  id: string;
  repoId: string;
  artifactName: string;
  receivedBytes: number;
  totalBytes: number;
  bytesPerSec: number;
  state: DownloadState;
  error: string | null;
}

export type DownloadEvent =
  | { kind: "update"; status: DownloadStatus }
  | { kind: "removed"; id: string };

// ------------------------------------------------------ biblioteca local ---

export interface LocalModel {
  repoId: string;
  name: string;
  primaryPath: string;
  /** Projetor de visão no mesmo repositório, quando há. */
  visionProjector?: string | null;
  totalBytes: number;
  files: string[];
  quantLabel: string;
}

// -------------------------------------------------------------- servidor ---

export interface ServerStatus {
  running: boolean;
  baseUrl: string | null;
  port: number;
  lan: boolean;
  /** Processo de pé com chave de API diferente da gravada no setting —
   *  "reinicie para aplicar" (serde `key_stale`). Sempre `false` parado. */
  keyStale: boolean;
}

/**
 * Agregado de tráfego servido pelo motor (espelho de `ServeAgg` do backend,
 * serde camelCase). Vem dos counters do próprio llama-server (/metrics),
 * então cobre TODOS os clientes: chat interno e apps externos.
 */
export interface ServeAgg {
  /** Tokens de prompt processados de fato (exclui os vindos do cache). */
  promptTokens: number;
  /** Tokens de prompt reaproveitados do KV cache. */
  cachedTokens: number;
  /** Tokens gerados. */
  predictedTokens: number;
  /** prompt + cached + predicted. */
  totalTokens: number;
  /** cached / (prompt + cached); `null` sem dados. */
  cacheEfficiency: number | null;
  /** promptTokens / promptSeconds — média do recorte, não instantânea. */
  avgPromptTps: number | null;
  /** predictedTokens / predictedSeconds — média do recorte. */
  avgGenTps: number | null;
}

/** Estatísticas de serviço (espelho de `ServeStatsDto` do backend). */
export interface ServeStatsDto {
  /** Servidor de pé — senão os números são só históricos. */
  running: boolean;
  /** Modelos com dados (união sessão ∪ desde-sempre). */
  models: string[];
  /** Desde que o app abriu (não desde o boot do servidor). */
  session: ServeAgg;
  /** Acumulado no banco, sobrevive a reinícios. */
  allTime: ServeAgg;
}

// ------------------------------------------------------- props do servidor ---

export interface ChatTemplateCaps {
  supportsTools: boolean;
  supportsParallelToolCalls: boolean;
  supportsSystemRole: boolean;
}

export interface ServerProps {
  modelPath: string | null;
  chatTemplateCaps: ChatTemplateCaps;
  nCtx: number | null;
  modalities: string[];
  /** `"router"` quando quem respondeu foi o roteador, não um modelo. */
  role: string | null;
}

/**
 * Esta resposta fala de um modelo?
 *
 * No modo Router, `/props` sem `?model=` devolve as props do ROTEADOR — e
 * elas parecem um modelo sem capacidade nenhuma. Quem lê os campos precisa
 * passar por aqui antes, senão "ainda não sei" vira "não suporta".
 */
export function describesModel(p: ServerProps | null): p is ServerProps {
  return p != null && p.role !== "router" && p.modelPath != null;
}

// ----------------------------------------------------------------- chat ---

export interface ChatRow {
  id: number;
  title: string;
  modelId: string | null;
  createdAt: number;
  /** JSON de `ChatParams` desta conversa (null = padrões). */
  paramsJson: string | null;
}

export interface MessageRow {
  id: number;
  chatId: number;
  role: string;
  content: string;
  createdAt: number;
  tokensPerSec: number | null;
  /** Tokens gerados na resposta (timings do llama-server). */
  genTokens: number | null;
  /** Duração da geração em ms. */
  genMs: number | null;
  /**
   * Herança do modo agente removido: a coluna `messages.run_id` continua no
   * banco (dados antigos), mas daqui em diante é sempre `null`.
   */
  runId: string | null;
}

export interface PresetRow {
  id: number;
  name: string;
  json: string;
}

/** Como o chat trata ações que pedem confirmação (ferramentas, no futuro). */
export type ApprovalMode = "manual" | "auto" | "ignore";

/** Orçamento de geração / raciocínio. `high` é o padrão. */
export type EffortLevel = "low" | "medium" | "high" | "extra" | "max";

/** Teto de tokens sugerido por nível de esforço (`null` = sem limite). */
export const EFFORT_MAX_TOKENS: Record<EffortLevel, number | null> = {
  low: 1024,
  medium: 2048,
  high: 4096,
  extra: 8192,
  max: null,
};

/** Parâmetros de amostragem + system prompt de uma conversa. */
export interface ChatParams {
  systemPrompt: string;
  temperature: number;
  topP: number;
  topK: number;
  /** null = sem limite. */
  maxTokens: number | null;
  approval: ApprovalMode;
  effort: EffortLevel;
  /** Pasta anexada à sessão (leitura/edição e @arquivo). */
  workspaceDir: string | null;
  /**
   * Modelo escolhido nesta conversa. `chats.model_id` só é gravado na
   * criação (não há `chat_set_model` ainda), então trocar de modelo no meio
   * da conversa se perdia ao reabrir; aqui o valor acompanha os params.
   * Ausente/`null` = cair para `ChatRow.modelId`.
   */
  model?: string | null;
}

export interface WorkspaceFile {
  path: string;
  name: string;
  bytes: number;
}

export type ClusterRole = "idle" | "host" | "worker" | "pending";

export interface ClusterPeer {
  id: string;
  hostname: string;
  os: string;
  gpuName: string;
  deviceId: string;
  advertisedBytes: number;
  llamaTag: string;
  ip: string;
  controlPort: number;
  tagOk: boolean;
  paired: boolean;
}

export interface ClusterConnected {
  peerId: string;
  hostname: string;
  gpuName: string;
  devices: string;
  tensorSplit: string;
  rpcAddr: string;
}

export interface ClusterSnapshot {
  instanceId: string;
  hostname: string;
  llamaTag: string;
  rpcReady: boolean;
  deviceId: string | null;
  advertisedBytes: number;
  role: ClusterRole;
  peers: ClusterPeer[];
  pendingFrom: ClusterPeer | null;
  connected: ClusterConnected | null;
  warning: string | null;
  enabled: boolean;
}

export const DEFAULT_CHAT_PARAMS: ChatParams = {
  systemPrompt: "",
  temperature: 0.8,
  topP: 0.95,
  topK: 40,
  maxTokens: EFFORT_MAX_TOKENS.high,
  approval: "auto",
  effort: "high",
  workspaceDir: null,
  model: null,
};

/** Campos que um `ChatParams` persistido pode trazer de volta à tela. */
const CHAT_PARAM_KEYS = [
  "systemPrompt",
  "temperature",
  "topP",
  "topK",
  "maxTokens",
  "approval",
  "effort",
  "workspaceDir",
  "model",
] as const satisfies readonly (keyof ChatParams)[];

/**
 * Saneia um `ChatParams` vindo de JSON persistido (conversa ou preset):
 * aplica os padrões por baixo e DESCARTA campos que não existem mais.
 *
 * O modo agente foi removido — `paramsJson` antigos ainda trazem
 * `agent`/`mode`/`workMode`/`codeMode` e esses campos precisam ser
 * ignorados sem erro (e sem voltar a ser gravados no próximo persist).
 */
export function sanitizeChatParams(raw: unknown): ChatParams {
  const out: ChatParams = { ...DEFAULT_CHAT_PARAMS };
  if (raw != null && typeof raw === "object") {
    const r = raw as Record<string, unknown>;
    for (const key of CHAT_PARAM_KEYS) {
      if (r[key] !== undefined) {
        (out as unknown as Record<string, unknown>)[key] = r[key];
      }
    }
  }
  return out;
}

/** `sanitizeChatParams` direto do JSON — inválido/corrompido cai nos padrões. */
export function parseChatParams(json: string | null): ChatParams {
  if (!json) return { ...DEFAULT_CHAT_PARAMS };
  try {
    return sanitizeChatParams(JSON.parse(json));
  } catch {
    return { ...DEFAULT_CHAT_PARAMS };
  }
}
