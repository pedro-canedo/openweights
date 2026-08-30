// Sessão do servidor local (llama-server em Router mode): garantir que está
// no ar, listar os modelos carregados e resolver o nome pedido para o id que
// o Router conhece.
//
// Este módulo é deliberadamente independente do chat: qualquer caminho que
// abra um stream contra o servidor local precisa exatamente da mesma
// preparação, e mantê-la aqui evita duplicá-la.

import { getServerStatus, getSetting, startServer } from "./api";
import { providerEndpoint, splitModelRef, type ProviderId } from "./providers";
import { isTauri } from "./tauri";

export interface ServerSession {
  /** URL conectável pela UI (nunca 0.0.0.0, mesmo em modo LAN). */
  baseUrl: string;
}

/** Para onde mandar a conversa, já com o que anexar no cabeçalho. */
export interface EndpointSession {
  provider: ProviderId;
  baseUrl: string;
  /** Cabeçalhos prontos (auth + atribuição), vazio no provedor local. */
  headers: Record<string, string>;
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

/**
 * Garante o endpoint da conversa e devolve para onde mandar.
 *
 * Generaliza o `ensureServer` sem substituí-lo: um modelo sem prefixo é
 * local, e nesse caso o caminho é exatamente o de sempre — subir o
 * llama-server se preciso. Provedor remoto não tem nada a subir; o que ele
 * precisa é da chave, que vem do backend junto da URL.
 */
export async function ensureEndpoint(modelRef: string): Promise<EndpointSession> {
  const { provider } = splitModelRef(modelRef);
  if (provider === "local") {
    // O auto-start vem PRIMEIRO — subir o llama-server se preciso é o
    // comportamento de sempre, e a chave só existe com o processo de pé.
    const { baseUrl } = await ensureServer();
    const headers: Record<string, string> = {};
    if (isTauri) {
      // A chave ATIVA do processo vem do backend junto do endpoint; sem
      // chave configurada o servidor aceita tudo e o cabeçalho fica vazio.
      const ep = await providerEndpoint("local").catch(() => null);
      if (ep?.apiKey) headers.Authorization = `Bearer ${ep.apiKey}`;
    }
    return { provider, baseUrl, headers };
  }

  const ep = await providerEndpoint(modelRef).catch((e) => {
    throw new Error(errorMessage(e) || "Provedor indisponível.");
  });
  const headers: Record<string, string> = {};
  for (const [nome, valor] of ep.headers) headers[nome] = valor;
  if (ep.apiKey) headers.Authorization = `Bearer ${ep.apiKey}`;
  return { provider: ep.provider, baseUrl: ep.baseUrl, headers };
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

/** Sufixo do id da entrada que carrega o projetor de visão. */
export const VISION_SUFFIX = " (visão)";

/**
 * Id a usar quando a mensagem tem imagem.
 *
 * O motor recebe duas entradas do mesmo arquivo — uma leve e outra com o
 * projetor — e trocar de uma para a outra é a mesma troca de modelo que ele
 * já sabe fazer. Assim o projetor só ocupa memória de vídeo quando há imagem,
 * sem recarregar nada na hora de enviar.
 *
 * Sem a entrada com visão (modelo sem projetor, ou visão desligada), devolve
 * o próprio modelo: mandar a imagem para um modelo que não a lê dá um erro
 * claro do motor, melhor do que a tela inventar um id que não existe.
 */
export function visionModelFor(model: string, loaded: string[]): string {
  const alvo = `${model}${VISION_SUFFIX}`;
  return loaded.includes(alvo) ? alvo : matchServerModel(alvo, loaded) === alvo ? alvo : model;
}

/**
 * Teto de modelos carregados ao mesmo tempo (`--models-max`; padrão 1).
 *
 * Um. Trocar de modelo é o caso comum, e o segundo não cabe na placa com o
 * primeiro ainda lá — o roteador descarrega o menos usado ao bater no teto,
 * então 1 significa "descarregue o anterior para carregar o novo".
 */
export async function modelsMax(): Promise<number> {
  const raw = await getSetting("server_models_max").catch(() => null);
  const n = raw ? Number(raw) : 1;
  return Number.isFinite(n) && n >= 1 ? n : 1;
}
