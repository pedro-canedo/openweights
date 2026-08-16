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
