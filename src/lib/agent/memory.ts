// Ponte tipada com os comandos da memória de longo prazo (H3).
// Fonte da verdade dos tipos: src-tauri/crates/memory/src/lib.rs e
// src-tauri/crates/store/src/memory.rs (serde camelCase).
//
// Mesma regra do resto do app: nenhuma tela chama `invoke` direto, e no
// navegador (npm run dev sem Tauri) tudo é simulado em memória — é assim que
// o painel é desenvolvido sem o backend Rust.

import { invoke, isTauri } from "../tauri";

/** Onde o fato vale. `workspace` = só na pasta aberta. */
export type FactScope = "global" | "workspace";

/** Um fato memorizado, como vem do banco. */
export interface MemoryFact {
  id: number;
  /** `null` = fato global (vale em qualquer projeto). */
  workspaceDir: string | null;
  content: string;
  sourceRunId: string | null;
  /** Epoch em segundos. */
  createdAt: number;
}

/** O que voltou ao guardar um fato. */
export interface SavedFact {
  id: number;
  content: string;
  scope: FactScope;
  /** Arquivo de assunto em `.openweights/memory/` (ex.: `build`). */
  topic: string;
  file: string | null;
}

/** Resultado de uma arrumação. */
export interface ConsolidateReport {
  episodes: number;
  added: string[];
  skipped: number;
}

export function isGlobalFact(fact: MemoryFact): boolean {
  return fact.workspaceDir == null || fact.workspaceDir === "";
}

// ------------------------------------------------------------- comandos ---

export async function memoryFacts(
  workspaceDir: string | null,
): Promise<MemoryFact[]> {
  if (!isTauri) return mockList();
  return invoke<MemoryFact[]>("memory_facts_list", { workspaceDir });
}

export async function memoryAddFact(
  workspaceDir: string | null,
  content: string,
): Promise<SavedFact> {
  if (!isTauri) return mockAdd(workspaceDir, content);
  return invoke<SavedFact>("memory_fact_add", { workspaceDir, content });
}

export async function memoryForgetFact(
  id: number,
  workspaceDir: string | null,
): Promise<void> {
  if (!isTauri) return mockForget(id);
  return invoke<void>("memory_fact_delete", { id, workspaceDir });
}

/**
 * Arruma a memória agora. O backend recusa enquanto houver run em andamento
 * — a consolidação usa o mesmo modelo do agente.
 */
export async function memoryConsolidateNow(
  workspaceDir: string | null,
  model?: string,
): Promise<ConsolidateReport> {
  if (!isTauri) return mockConsolidate();
  return invoke<ConsolidateReport>("memory_consolidate_now", {
    workspaceDir,
    model: model ?? null,
  });
}

export async function memoryOpenFolder(workspaceDir: string): Promise<void> {
  if (!isTauri) return;
  return invoke<void>("memory_open_folder", { workspaceDir });
}

// ---------------------------------------------------------------- mocks ---

let mockFacts: MemoryFact[] = [
  {
    id: 1,
    workspaceDir: null,
    content: "prefiro respostas curtas e diretas",
    sourceRunId: null,
    createdAt: Math.floor(Date.now() / 1000) - 86_400,
  },
  {
    id: 2,
    workspaceDir: "/projeto",
    content: "este projeto usa pnpm, não npm",
    sourceRunId: "run-1",
    createdAt: Math.floor(Date.now() / 1000) - 3_600,
  },
];
let mockNextId = 3;

async function mockList(): Promise<MemoryFact[]> {
  return mockFacts.slice();
}

async function mockAdd(
  workspaceDir: string | null,
  content: string,
): Promise<SavedFact> {
  const clean = content.trim();
  if (!clean) throw new Error("o fato está vazio");
  if (mockFacts.some((f) => f.content.toLowerCase() === clean.toLowerCase())) {
    throw new Error(`já sabemos disso: ${clean}`);
  }
  const fact: MemoryFact = {
    id: mockNextId++,
    workspaceDir,
    content: clean,
    sourceRunId: null,
    createdAt: Math.floor(Date.now() / 1000),
  };
  mockFacts = [...mockFacts, fact];
  return {
    id: fact.id,
    content: clean,
    scope: workspaceDir ? "workspace" : "global",
    topic: "geral",
    file: workspaceDir ? `${workspaceDir}/.openweights/memory/geral.md` : null,
  };
}

async function mockForget(id: number): Promise<void> {
  mockFacts = mockFacts.filter((f) => f.id !== id);
}

async function mockConsolidate(): Promise<ConsolidateReport> {
  await new Promise((r) => setTimeout(r, 600));
  const added = "os testes rodam com pnpm test";
  if (!mockFacts.some((f) => f.content === added)) {
    mockFacts = [
      ...mockFacts,
      {
        id: mockNextId++,
        workspaceDir: "/projeto",
        content: added,
        sourceRunId: null,
        createdAt: Math.floor(Date.now() / 1000),
      },
    ];
  }
  return { episodes: 4, added: [added], skipped: 2 };
}
