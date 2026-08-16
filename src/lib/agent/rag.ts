// Ponte tipada com os comandos do índice do projeto (RAG, H4).
// Nenhuma tela chama `invoke` direto — tudo passa por aqui, como em `api.ts`.
//
// O progresso da indexação chega por um `Channel` do Tauri (não por `listen`),
// pelo mesmo motivo do harness: o canal nasce junto com a chamada e morre com
// ela, então não sobra assinatura pendurada quando a tela desmonta.
//
// No navegador (npm run dev sem Tauri) tudo é simulado: um índice em memória
// com busca por substring. É assim que o painel é desenvolvido sem o Rust.

import { invoke, isTauri } from "../tauri";

/** O que este SQLite oferece. `vector: false` = só busca textual. */
export interface RagCapabilities {
  vector: boolean;
  fts: boolean;
}

export interface RagStatus {
  capabilities: RagCapabilities;
  indexed: boolean;
  files: number;
  chunks: number;
  /** Trechos com vetor. 0 = índice só textual. */
  vectors: number;
  embedModel: string | null;
  embedDim: number | null;
  /** Há modelo de embedding configurado agora? */
  embedModelConfigured: boolean;
  updatedAt: number | null;
  indexing: boolean;
}

export type RagPhase =
  | "scanning"
  | "indexing"
  | "embedding"
  | "done"
  | "cancelled";

export interface RagProgress {
  phase: RagPhase;
  done: number;
  total: number;
  path: string;
}

export interface RagReport {
  scanned: number;
  indexed: number;
  skipped: number;
  removed: number;
  chunks: number;
  embedded: number;
  vector: boolean;
  cancelled: boolean;
}

export type RagHitSource = "text" | "vector" | "both";

export interface RagHit {
  chunkId: number;
  path: string;
  startLine: number;
  endLine: number;
  snippet: string;
  score: number;
  source: RagHitSource;
}

/** `@tauri-apps/api/core` é carregado sob demanda (igual a `tauri.ts`). */
async function core() {
  return import("@tauri-apps/api/core");
}

export function ragStatus(workspaceDir: string): Promise<RagStatus> {
  return isTauri
    ? invoke<RagStatus>("rag_status", { workspaceDir })
    : Promise.resolve(mockStatus());
}

/**
 * Indexa (ou atualiza) o projeto. A promessa só resolve no fim, com o
 * relatório; o andamento chega por `onProgress`.
 *
 * `embedModel`: `undefined` mantém o que já estava escolhido; string vazia
 * desliga a parte vetorial.
 */
export async function ragIndexStart(
  workspaceDir: string,
  onProgress: (p: RagProgress) => void,
  embedModel?: string,
): Promise<RagReport> {
  if (!isTauri) return mockIndex(onProgress);
  const { Channel, invoke: rawInvoke } = await core();
  const ch = new Channel<RagProgress>();
  ch.onmessage = onProgress;
  return rawInvoke<RagReport>("rag_index_start", {
    workspaceDir,
    embedModel,
    onProgress: ch,
  });
}

export function ragIndexCancel(): Promise<void> {
  return isTauri ? invoke<void>("rag_index_cancel") : Promise.resolve();
}

export function ragSearch(
  workspaceDir: string,
  query: string,
  limit = 10,
): Promise<RagHit[]> {
  return isTauri
    ? invoke<RagHit[]>("rag_search", { workspaceDir, query, limit })
    : Promise.resolve(mockSearch(query, limit));
}

export function ragClear(workspaceDir: string): Promise<void> {
  if (!isTauri) {
    mock.indexed = false;
    mock.files = 0;
    mock.chunks = 0;
    return Promise.resolve();
  }
  return invoke<void>("rag_clear", { workspaceDir });
}

// ----------------------------------------------------------------- mocks ---

const mock = { indexed: false, files: 0, chunks: 0 };

const MOCK_CHUNKS: RagHit[] = [
  {
    chunkId: 1,
    path: "src/lib/agent/runStore.ts",
    startLine: 42,
    endLine: 78,
    snippet:
      "// Reata a interface a um run em andamento, repetindo o que ela perdeu.\nexport function attach(runId: string) {\n  // ...\n}",
    score: 0.032,
    source: "both",
  },
  {
    chunkId: 2,
    path: "src-tauri/crates/policy/src/lib.rs",
    startLine: 12,
    endLine: 40,
    snippet:
      "/// Decide se uma ferramenta pode rodar sem perguntar ao usuário.\npub fn decide(spec: &ToolSpec) -> Decision { /* ... */ }",
    score: 0.028,
    source: "vector",
  },
  {
    chunkId: 3,
    path: "src/components/chat/Composer.tsx",
    startLine: 90,
    endLine: 130,
    snippet:
      "const send = async () => {\n  if (!draft.trim()) return;\n  await start(draft);\n};",
    score: 0.021,
    source: "text",
  },
];

function mockStatus(): RagStatus {
  return {
    capabilities: { vector: true, fts: true },
    indexed: mock.indexed,
    files: mock.files,
    chunks: mock.chunks,
    vectors: mock.indexed ? mock.chunks : 0,
    embedModel: mock.indexed ? "nomic-embed-text" : null,
    embedDim: mock.indexed ? 768 : null,
    embedModelConfigured: true,
    updatedAt: mock.indexed ? Math.floor(Date.now() / 1000) : null,
    indexing: false,
  };
}

async function mockIndex(
  onProgress: (p: RagProgress) => void,
): Promise<RagReport> {
  const total = 120;
  onProgress({ phase: "scanning", done: 0, total: 0, path: "" });
  for (let done = 10; done <= total; done += 10) {
    await new Promise((r) => setTimeout(r, 90));
    onProgress({
      phase: done < total ? "indexing" : "embedding",
      done,
      total,
      path: `src/arquivo-${done}.ts`,
    });
  }
  onProgress({ phase: "done", done: total, total, path: "" });
  mock.indexed = true;
  mock.files = total;
  mock.chunks = total * 3;
  return {
    scanned: total,
    indexed: total,
    skipped: 0,
    removed: 0,
    chunks: total * 3,
    embedded: total * 3,
    vector: true,
    cancelled: false,
  };
}

function mockSearch(query: string, limit: number): RagHit[] {
  const q = query.trim().toLowerCase();
  if (!q || !mock.indexed) return [];
  return MOCK_CHUNKS.slice(0, limit);
}
