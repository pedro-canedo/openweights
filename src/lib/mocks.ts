// Mocks para desenvolvimento da UI no navegador (sem Tauri).
// Dados plausíveis para uma máquina com GPU de 16 GB.

import type {
  ChatRow,
  DownloadStatus,
  LocalModel,
  MessageRow,
  ModelSummary,
  PresetRow,
  QuantsView,
  QuantView,
  RuntimeState,
  ServeAgg,
  ServerProps,
  ServerStatus,
  ServeStatsDto,
  ClusterSnapshot,
} from "./types";

const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));

export async function runtimeStatus(): Promise<RuntimeState> {
  return {
    tag: "b10441",
    variant: "cuda13",
    installed: true,
    serverExe: "C:/fake/llama-server.exe",
    rpcReady: true,
    rpcExe: "C:/fake/ggml-rpc-server.exe",
  };
}

export async function ensureRuntime(): Promise<RuntimeState> {
  await delay(800);
  return runtimeStatus();
}

const MODELS: ModelSummary[] = [
  {
    id: "unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF",
    author: "unsloth",
    name: "Qwen3-Coder-30B-A3B-Instruct-GGUF",
    downloads: 9_300_000,
    likes: 1_542,
    paramsTotal: 30_500_000_000,
    architecture: "qwen3moe",
    contextLength: 262_144,
    gated: false,
    updatedAt: "2026-07-30T10:00:00.000Z",
    license: "apache-2.0",
    caps: { vision: false, tools: true, reasoning: true },
  },
  {
    id: "unsloth/Qwen3-8B-GGUF",
    author: "unsloth",
    name: "Qwen3-8B-GGUF",
    downloads: 4_100_000,
    likes: 987,
    paramsTotal: 8_190_000_000,
    architecture: "qwen3",
    contextLength: 40_960,
    gated: false,
    updatedAt: "2026-06-12T10:00:00.000Z",
    license: "apache-2.0",
    caps: { vision: false, tools: true, reasoning: true },
  },
  {
    id: "bartowski/gemma-3-27b-it-GGUF",
    author: "bartowski",
    name: "gemma-3-27b-it-GGUF",
    downloads: 2_800_000,
    likes: 640,
    paramsTotal: 27_000_000_000,
    architecture: "gemma3",
    contextLength: 131_072,
    gated: false,
    updatedAt: "2026-05-02T10:00:00.000Z",
    license: "apache-2.0",
    caps: { vision: false, tools: true, reasoning: true },
  },
  {
    id: "meta-llama/Llama-3.1-8B-Instruct",
    author: "meta-llama",
    name: "Llama-3.1-8B-Instruct",
    downloads: 1_900_000,
    likes: 4_312,
    paramsTotal: 8_030_000_000,
    architecture: "llama",
    contextLength: 131_072,
    gated: true,
    updatedAt: "2025-11-20T10:00:00.000Z",
    license: "apache-2.0",
    caps: { vision: false, tools: true, reasoning: true },
  },
];

export async function searchModels(query: string): Promise<ModelSummary[]> {
  await delay(350);
  const q = query.toLowerCase();
  return q
    ? MODELS.filter((m) => m.id.toLowerCase().includes(q))
    : MODELS;
}

export async function modelQuants(_repoId: string): Promise<QuantsView> {
  await delay(300);
  const gb = 2 ** 30;
  const mk = (
    label: string,
    sizeGb: number,
    verdict: QuantView["verdict"],
    recommended = false,
  ): QuantView => ({
    artifactName: `model-${label}.gguf`,
    files: [`model-${label}.gguf`],
    totalBytes: sizeGb * gb,
    filename: `model-${label}.gguf`,
    label,
    sizeBytes: sizeGb * gb,
    bits: null,
    recommended,
    verdict,
    // Arquivo + KV da janela avaliada + reserva de runtime.
    estTotalBytes: sizeGb * gb + 1.4 * gb,
    kvCacheBytes: 0.4 * gb,
  });
  return {
    quants: [
      mk("UD-Q2_K_XL", 10.3, { kind: "fullGpu", ngl: 64 }),
      mk("Q3_K_M", 13.8, { kind: "fullGpu", ngl: 64 }),
      mk("UD-Q4_K_XL", 17.9, { kind: "partial", ngl: 52, layersTotal: 64 }, true),
      mk("Q5_K_M", 19.8, { kind: "partial", ngl: 46, layersTotal: 64 }),
      mk("Q8_0", 29, { kind: "cpuOnly" }),
      mk("BF16", 54.7, { kind: "wontFit" }),
    ],
    ctxLen: 8192,
    modelCtxMax: 262144,
    visionProjectorBytes: 931 * 2 ** 20,
    calibrated: null,
  };
}

const downloads = new Map<string, DownloadStatus>();

export async function startDownload(
  repoId: string,
  artifactName: string,
): Promise<string> {
  const id = `${repoId}::${artifactName}`;
  downloads.set(id, {
    id,
    repoId,
    artifactName,
    receivedBytes: 1.2 * 2 ** 30,
    totalBytes: 5 * 2 ** 30,
    bytesPerSec: 48 * 2 ** 20,
    state: "running",
    error: null,
  });
  return id;
}

export async function listDownloads(): Promise<DownloadStatus[]> {
  if (downloads.size === 0) {
    downloads.set("unsloth/Qwen3-8B-GGUF::Qwen3-8B-Q4_K_M.gguf", {
      id: "unsloth/Qwen3-8B-GGUF::Qwen3-8B-Q4_K_M.gguf",
      repoId: "unsloth/Qwen3-8B-GGUF",
      artifactName: "Qwen3-8B-Q4_K_M.gguf",
      receivedBytes: 2.1 * 2 ** 30,
      totalBytes: 5.1 * 2 ** 30,
      bytesPerSec: 0,
      state: "paused",
      error: null,
    });
  }
  return [...downloads.values()];
}

export async function localModels(): Promise<LocalModel[]> {
  return [
    {
      repoId: "unsloth/Qwen3-8B-GGUF",
      name: "Qwen3-8B-UD-Q4_K_XL.gguf",
      primaryPath: "C:/fake/models/unsloth/Qwen3-8B-GGUF/Qwen3-8B-UD-Q4_K_XL.gguf",
      totalBytes: 5.1 * 2 ** 30,
      files: ["C:/fake/models/unsloth/Qwen3-8B-GGUF/Qwen3-8B-UD-Q4_K_XL.gguf"],
      quantLabel: "UD-Q4_K_XL",
    },
  ];
}

/// As fotos dos autores no navegador: consulta o Hub de verdade.
///
/// É a única exceção ao "mock não faz rede", e por um motivo: o avatar é
/// público, o Hub libera CORS e a foto é justamente o que se está tentando
/// ver na tela. Um mock devolvendo nada faria a UI parecer certa no navegador
/// e diferente no app — o oposto do que estes dados existem para fazer.
export async function authorAvatars(
  authors: string[],
): Promise<Record<string, string>> {
  const fotos: Record<string, string> = {};
  await Promise.all(
    [...new Set(authors.filter(Boolean))].map(async (autor) => {
      for (const rota of ["users", "organizations"]) {
        try {
          const r = await fetch(
            `https://huggingface.co/api/${rota}/${autor}/overview`,
          );
          if (!r.ok) continue;
          const url: string | undefined = (await r.json())?.avatarUrl;
          // `/avatars/*.svg` é o identicon gerado — as iniciais dizem mais.
          if (url?.startsWith("https://")) fotos[autor] = url;
          return;
        } catch {
          return;
        }
      }
    }),
  );
  return fotos;
}

let mockServer: ServerStatus = {
  running: false,
  baseUrl: null,
  port: 11711,
  lan: false,
  keyStale: false,
};

export async function serverStatus(): Promise<ServerStatus> {
  return mockServer;
}

export async function clusterStatus(): Promise<ClusterSnapshot> {
  return {
    instanceId: "mock",
    hostname: "Dev-PC",
    llamaTag: "b10441",
    rpcReady: true,
    deviceId: "CUDA0",
    advertisedBytes: 12 * 2 ** 30 * 0.75,
    role: "idle",
    peers: [
      {
        id: "mac",
        hostname: "MacBook-de-Pedro",
        os: "macos",
        gpuName: "Apple GPU",
        deviceId: "MTL0",
        advertisedBytes: 18 * 2 ** 30 * 0.75,
        llamaTag: "b10441",
        ip: "192.168.1.8",
        controlPort: 17890,
        tagOk: true,
        paired: false,
      },
    ],
    pendingFrom: null,
    connected: null,
    warning: null,
    enabled: true,
  };
}

export async function startServer(): Promise<ServerStatus> {
  await delay(600);
  mockServer = {
    running: true,
    baseUrl: "http://127.0.0.1:11711",
    port: 11711,
    lan: false,
    keyStale: false,
  };
  return mockServer;
}

export async function stopServer(): Promise<void> {
  mockServer = {
    running: false,
    baseUrl: null,
    port: 11711,
    lan: false,
    keyStale: false,
  };
}

// ------------------------------------------------ estatísticas de serviço ---

/** Counters crus por modelo — o mock deriva o ServeAgg como o backend faz. */
type ServeCounters = {
  promptTokens: number;
  cachedTokens: number;
  predictedTokens: number;
  promptSeconds: number;
  predictedSeconds: number;
};

const zeroCounters = (): ServeCounters => ({
  promptTokens: 0,
  cachedTokens: 0,
  predictedTokens: 0,
  promptSeconds: 0,
  predictedSeconds: 0,
});

const serveSession = new Map<string, ServeCounters>();
const serveAllTime = new Map<string, ServeCounters>();
let serveSeeded = false;

/** Sessão menor que o desde-sempre, como numa máquina usada há semanas. */
function seedServe(): void {
  serveSeeded = true;
  serveAllTime.set("Qwen3.6-27B-MTP-Q4_K_M.gguf", {
    promptTokens: 182_400,
    cachedTokens: 96_800,
    predictedTokens: 74_300,
    promptSeconds: 228.4,
    predictedSeconds: 1_486.2,
  });
  serveAllTime.set("gemma-3-4b-it-Q4_K_M.gguf", {
    promptTokens: 22_100,
    cachedTokens: 4_800,
    predictedTokens: 9_650,
    promptSeconds: 18.9,
    predictedSeconds: 120.5,
  });
  serveSession.set("Qwen3.6-27B-MTP-Q4_K_M.gguf", {
    promptTokens: 12_300,
    cachedTokens: 8_400,
    predictedTokens: 5_120,
    promptSeconds: 15.2,
    predictedSeconds: 102.7,
  });
}

/** Um pouco de tráfego novo a cada consulta, para a tela "viver" no dev. */
function growServe(): void {
  if (!mockServer.running) return;
  const id = "Qwen3.6-27B-MTP-Q4_K_M.gguf";
  for (const map of [serveSession, serveAllTime]) {
    const c = map.get(id) ?? zeroCounters();
    c.promptTokens += 220;
    c.cachedTokens += 340;
    c.predictedTokens += 180;
    c.promptSeconds += 0.3;
    c.predictedSeconds += 3.4;
    map.set(id, c);
  }
}

function aggServe(
  map: Map<string, ServeCounters>,
  model: string | null,
): ServeAgg {
  const total = zeroCounters();
  for (const [id, c] of map) {
    if (model && id !== model) continue;
    total.promptTokens += c.promptTokens;
    total.cachedTokens += c.cachedTokens;
    total.predictedTokens += c.predictedTokens;
    total.promptSeconds += c.promptSeconds;
    total.predictedSeconds += c.predictedSeconds;
  }
  const promptEntrada = total.promptTokens + total.cachedTokens;
  return {
    promptTokens: total.promptTokens,
    cachedTokens: total.cachedTokens,
    predictedTokens: total.predictedTokens,
    totalTokens: promptEntrada + total.predictedTokens,
    cacheEfficiency:
      promptEntrada > 0 ? total.cachedTokens / promptEntrada : null,
    avgPromptTps:
      total.promptSeconds > 0
        ? total.promptTokens / total.promptSeconds
        : null,
    avgGenTps:
      total.predictedSeconds > 0
        ? total.predictedTokens / total.predictedSeconds
        : null,
  };
}

export async function serveStats(
  model: string | null,
): Promise<ServeStatsDto> {
  if (!serveSeeded) seedServe();
  growServe();
  const models = [
    ...new Set([...serveSession.keys(), ...serveAllTime.keys()]),
  ].sort();
  return {
    running: mockServer.running,
    models,
    session: aggServe(serveSession, model),
    allTime: aggServe(serveAllTime, model),
  };
}

export async function serveStatsClear(): Promise<void> {
  // Limpo é limpo: marcar como semeado impede a re-semeadura no próximo poll.
  serveSeeded = true;
  serveSession.clear();
  serveAllTime.clear();
}

/** IPs plausíveis de uma máquina Windows com WSL (o card gera o resto). */
export async function serverLanUrls(): Promise<string[]> {
  return [
    `http://192.168.1.7:${mockServer.port}`,
    `http://172.20.32.1:${mockServer.port}`,
  ];
}

/** No navegador o modelo "suporta ferramentas": não trava a UI de dev. */
export async function serverProps(): Promise<ServerProps> {
  return {
    modelPath: "C:/fake/models/unsloth/Qwen3-8B-GGUF/Qwen3-8B-UD-Q4_K_XL.gguf",
    chatTemplateCaps: {
      supportsTools: true,
      supportsParallelToolCalls: true,
      supportsSystemRole: true,
    },
    nCtx: 40_960,
    modalities: [],
    role: null,
  };
}

/** Id crescente para o `addMessage` do navegador (0 significaria falha). */
let messageSeq = 0;
export function nextMessageId(): number {
  messageSeq += 1;
  return messageSeq;
}

const chats: ChatRow[] = [];
const messages: MessageRow[] = [];
const presets: PresetRow[] = [
  {
    id: 1,
    name: "Padrão",
    json: '{"systemPrompt":"","temperature":0.8,"topP":0.95,"topK":40,"maxTokens":null}',
  },
];

export async function listChats(): Promise<ChatRow[]> {
  return [...chats].reverse();
}

export async function createChat(title: string): Promise<number> {
  const id = chats.length + 1;
  chats.push({
    id,
    title,
    modelId: null,
    createdAt: Date.now() / 1000,
    paramsJson: null,
  });
  return id;
}

export async function listMessages(chatId: number): Promise<MessageRow[]> {
  return messages.filter((m) => m.chatId === chatId);
}

export async function renameChat(chatId: number, title: string): Promise<void> {
  const c = chats.find((c) => c.id === chatId);
  if (c) c.title = title;
}

export async function setChatParams(
  chatId: number,
  paramsJson: string,
): Promise<void> {
  const c = chats.find((c) => c.id === chatId);
  if (c) c.paramsJson = paramsJson;
}

export async function listPresets(): Promise<PresetRow[]> {
  return [...presets];
}

export async function savePreset(name: string, json: string): Promise<number> {
  const existing = presets.find((p) => p.name === name);
  if (existing) {
    existing.json = json;
    return existing.id;
  }
  const id = presets.length + 1;
  presets.push({ id, name, json });
  return id;
}

export async function deletePreset(id: number): Promise<void> {
  const i = presets.findIndex((p) => p.id === id);
  if (i >= 0) presets.splice(i, 1);
}
