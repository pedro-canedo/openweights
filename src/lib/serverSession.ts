// Sessão do servidor local (llama-server em Router mode): garantir que está
// no ar, listar os modelos carregados e resolver o nome pedido para o id que
// o Router conhece.
//
// Este módulo é deliberadamente independente do chat: o `generationStore`
// (chat normal) e o `runStore` do agente (H1) precisam exatamente da mesma
// preparação antes de abrir um stream, e duplicar essa lógica é como se
// perde a paridade entre os dois caminhos.

import { getServerStatus, getSetting, startServer } from "./api";

export interface ServerSession {
  /** URL conectável pela UI (nunca 0.0.0.0, mesmo em modo LAN). */
  baseUrl: string;
}

/** Texto de erro exibível: o `invoke` do Tauri rejeita com string crua. */
export function errorMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  return String(e);
}

/**
 * Garante o servidor local no ar e devolve a `baseUrl`.
 *
 * Rejeita com `Error` de mensagem pronta para exibição quando o servidor não
 * sobe (runtime ausente, `/health` sem resposta em 30 s, porta ocupada...).
 * Quem chama PRECISA tratar: sem isso o job de geração fica preso em
 * "running" para sempre.
 */
export async function ensureServer(): Promise<ServerSession> {
  let status = await getServerStatus().catch(() => null);
  if (!status || !status.running || !status.baseUrl) {
    status = await startServer().catch((e) => {
      throw new Error(errorMessage(e) || "Servidor local indisponível.");
    });
  }
  if (!status.baseUrl) {
    throw new Error("Servidor local indisponível.");
  }
  return { baseUrl: status.baseUrl };
}

/** Ids atualmente servidos pelo Router (`GET /v1/models`). Nunca lança. */
export async function listLoadedModels(baseUrl: string): Promise<string[]> {
  try {
    const res = await fetch(`${baseUrl}/v1/models`);
    if (!res.ok) return [];
    const json = (await res.json()) as { data?: { id: string }[] };
    return (json.data ?? []).map((d) => d.id);
  } catch {
    return [];
  }
}

/**
 * Casa o nome escolhido na UI (nome do artefato, com ou sem `.gguf`) com o id
 * publicado pelo Router. Devolve `wanted` quando não há correspondência —
 * deixar o servidor decidir dá uma mensagem de erro melhor que adivinhar.
 */
export function matchServerModel(wanted: string, ids: string[]): string {
  if (ids.includes(wanted)) return wanted;
  const stem = wanted.replace(/\.gguf$/i, "");
  const hit = ids.find((id) => {
    const idStem = id.replace(/\.gguf$/i, "");
    return (
      id === stem ||
      idStem === stem ||
      id.endsWith(`/${wanted}`) ||
      id.endsWith(`/${stem}`) ||
      idStem.endsWith(`/${stem}`)
    );
  });
  return hit ?? wanted;
}

/** Teto de modelos simultâneos do Router (`--models-max`; padrão 2). */
export async function modelsMax(): Promise<number> {
  const raw = await getSetting("server_models_max").catch(() => null);
  const n = raw ? Number(raw) : 2;
  return Number.isFinite(n) && n >= 1 ? n : 2;
}
