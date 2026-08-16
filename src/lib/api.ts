// Camada de API tipada sobre os comandos Tauri.
// Todas as telas usam ESTE módulo — nunca `invoke` direto.

import { invoke, isTauri, listen } from "./tauri";
import type { ServerProps } from "./agent/types";
import type {
  ChatRow,
  DownloadEvent,
  DownloadStatus,
  HardwareProfile,
  LocalModel,
  MessageRow,
  ModelSummary,
  PresetRow,
  QuantsView,
  RuntimeEvent,
  RuntimeState,
  ServerStatus,
  Telemetry,
  WorkspaceFile,
} from "./types";
import * as mocks from "./mocks";

// ------------------------------------------------------------ hardware ---

export const getHardwareProfile = () =>
  invoke<HardwareProfile>("hardware_profile");

export const getAppVersion = () => invoke<string>("app_version");

// ------------------------------------------------------------- runtime ---

export const getRuntimeStatus = () =>
  isTauri ? invoke<RuntimeState>("runtime_status") : mocks.runtimeStatus();

export const ensureRuntime = () =>
  isTauri ? invoke<RuntimeState>("runtime_ensure") : mocks.ensureRuntime();

export const onRuntimeEvent = (h: (e: RuntimeEvent) => void) =>
  listen<RuntimeEvent>("runtime", h);

// -------------------------------------------------------------- modelos ---

export type SearchSort = "trending" | "downloads" | "likes" | "updated";

export const searchModels = (query: string, sort: SearchSort = "trending") =>
  isTauri
    ? invoke<ModelSummary[]>("models_search", { query, sort })
    : mocks.searchModels(query);

export const getModelQuants = (
  repoId: string,
  paramsTotal: number | null,
  ctxLen = 8192,
) =>
  isTauri
    ? invoke<QuantsView>("models_quants", { repoId, paramsTotal, ctxLen })
    : mocks.modelQuants(repoId);

// ------------------------------------------------------------ downloads ---

export const startDownload = (repoId: string, artifactName: string) =>
  isTauri
    ? invoke<string>("download_start", { repoId, artifactName })
    : mocks.startDownload(repoId, artifactName);

export const pauseDownload = (id: string) =>
  invoke<void>("download_pause", { id });
export const resumeDownload = (id: string) =>
  invoke<void>("download_resume", { id });
export const cancelDownload = (id: string) =>
  invoke<void>("download_cancel", { id });

export const listDownloads = () =>
  isTauri ? invoke<DownloadStatus[]>("downloads_list") : mocks.listDownloads();

export const onDownloadEvent = (h: (e: DownloadEvent) => void) =>
  listen<DownloadEvent>("download", h);

// ------------------------------------------------------ biblioteca local ---

export const listLocalModels = () =>
  isTauri ? invoke<LocalModel[]>("local_models") : mocks.localModels();

export const deleteModel = (repoId: string, name: string) =>
  invoke<void>("model_delete", { repoId, name });

// -------------------------------------------------------------- servidor ---

export const getServerStatus = () =>
  isTauri ? invoke<ServerStatus>("server_status") : mocks.serverStatus();

export const startServer = () =>
  isTauri ? invoke<ServerStatus>("server_start") : mocks.startServer();

export const stopServer = () =>
  isTauri ? invoke<void>("server_stop") : mocks.stopServer();

/** Quem está usando o motor agora: `agent` (execução ou automação), `rag`. */
export const serverBusy = (): Promise<{ busyWith: string[] }> =>
  isTauri
    ? invoke<{ busyWith: string[] }>("server_busy")
    : Promise.resolve({ busyWith: [] });

/**
 * Para e sobe o motor de uma vez, com a guarda no backend.
 *
 * A tela não enxerga metade de quem usa o servidor — o relógio das automações
 * dispara execuções sozinho e o índice do projeto pede embeddings ao mesmo
 * motor. Sem `force`, o comando recusa devolvendo `engine-busy:<quem>`; quem
 * chama decide se pergunta à pessoa e repete com `force`.
 */
export async function restartServer(force = false): Promise<ServerStatus> {
  if (!isTauri) {
    await mocks.stopServer();
    return mocks.startServer();
  }
  return invoke<ServerStatus>("server_restart", { force });
}

/** `true` quando a recusa veio da guarda de ocupação (e diz quem está usando). */
export function engineBusyReason(e: unknown): string[] | null {
  const msg = e instanceof Error ? e.message : String(e ?? "");
  const hit = msg.match(/engine-busy:([a-z,]+)/);
  return hit ? hit[1].split(",").filter(Boolean) : null;
}

/**
 * `GET /props` do llama-server via backend: capacidades do chat template
 * (ferramentas, tool calls paralelos, papel system) de um modelo.
 *
 * **Passe o modelo.** Sem ele, no modo Router quem responde é o roteador, e a
 * resposta lida cruamente diz "não suporta nada". O backend só pergunta pelo
 * nome quando o modelo já está carregado (perguntar carrega), então a
 * resposta pode ser a do roteador mesmo assim — use `describesModel` antes de
 * concluir qualquer coisa.
 *
 * Rejeita quando o servidor está fora do ar — quem chama decide o fallback.
 */
export const getServerProps = (model?: string | null) =>
  isTauri
    ? invoke<ServerProps>("server_props", { model: model ?? null })
    : mocks.serverProps();

export const onServerStatus = (h: (s: ServerStatus) => void) =>
  listen<ServerStatus>("server-status", h);

export const onServerLog = (h: (line: string) => void) =>
  listen<string>("server-log", h);

// ----------------------------------------------------------------- chat ---

export const listChats = () =>
  isTauri ? invoke<ChatRow[]>("chats_list") : mocks.listChats();

export const createChat = (title: string, modelId: string | null) =>
  isTauri
    ? invoke<number>("chat_create", { title, modelId })
    : mocks.createChat(title);

export const deleteChat = (chatId: number) =>
  invoke<void>("chat_delete", { chatId });

export const listMessages = (chatId: number) =>
  isTauri
    ? invoke<MessageRow[]>("messages_list", { chatId })
    : mocks.listMessages(chatId);

export const addMessage = (
  chatId: number,
  role: string,
  content: string,
  tokensPerSec: number | null = null,
  genTokens: number | null = null,
  genMs: number | null = null,
  /** Modelo que gerou a resposta: o backend usa para carimbar a
   *  configuração vigente, e é o que torna os tokens/s comparáveis depois. */
  model: string | null = null,
) =>
  isTauri
    ? invoke<number>("message_add", {
        chatId,
        role,
        content,
        tokensPerSec,
        genTokens,
        genMs,
        model,
      })
    : Promise.resolve(mocks.nextMessageId());

export const renameChat = (chatId: number, title: string) =>
  isTauri
    ? invoke<void>("chat_rename", { chatId, title })
    : mocks.renameChat(chatId, title);

export const setChatParams = (chatId: number, paramsJson: string) =>
  isTauri
    ? invoke<void>("chat_set_params", { chatId, paramsJson })
    : mocks.setChatParams(chatId, paramsJson);

export const deleteMessage = (messageId: number) =>
  invoke<void>("message_delete", { messageId });

export const updateMessage = (messageId: number, content: string) =>
  invoke<void>("message_update", { messageId, content });

export const listPresets = () =>
  isTauri ? invoke<PresetRow[]>("presets_list") : mocks.listPresets();

export const savePreset = (name: string, json: string) =>
  isTauri
    ? invoke<number>("preset_save", { name, json })
    : mocks.savePreset(name, json);

export const deletePreset = (id: number) =>
  isTauri ? invoke<void>("preset_delete", { id }) : mocks.deletePreset(id);

// ------------------------------------------------------------- settings ---

export const getSetting = (key: string) =>
  isTauri
    ? invoke<string | null>("settings_get", { key })
    : Promise.resolve(localStorage.getItem(`mock:${key}`));

export const setSetting = (key: string, value: string) =>
  isTauri
    ? invoke<void>("settings_set", { key, value })
    : Promise.resolve(void localStorage.setItem(`mock:${key}`, value));

/** Mapa modelo → janela de contexto (`null` no comando = automático / --fit). */
export const MODEL_CTX_SETTING = "model_ctx_sizes";

export function lookupModelCtx(
  map: Record<string, number>,
  model: string,
): number | null {
  if (!model) return null;
  const stem = model.replace(/\.gguf$/i, "");
  const n = map[model] ?? map[stem] ?? map[`${stem}.gguf`];
  return typeof n === "number" && n > 0 ? n : null;
}

export async function getModelCtxMap(): Promise<Record<string, number>> {
  const raw = await getSetting(MODEL_CTX_SETTING).catch(() => null);
  if (!raw) return {};
  try {
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    const out: Record<string, number> = {};
    for (const [k, v] of Object.entries(parsed)) {
      const n = typeof v === "number" ? v : Number(v);
      if (Number.isFinite(n) && n > 0) out[k] = Math.round(n);
    }
    return out;
  } catch {
    return {};
  }
}

export const setModelCtx = (model: string, ctxLen: number | null) =>
  isTauri
    ? invoke<number | null>("model_set_ctx", { model, ctxLen })
    : (async () => {
        const map = await getModelCtxMap();
        if (ctxLen == null) delete map[model];
        else map[model] = ctxLen;
        await setSetting(MODEL_CTX_SETTING, JSON.stringify(map));
        return ctxLen;
      })();

// ------------------------------------------------------------ telemetria ---

export const onTelemetry = (h: (t: Telemetry) => void) =>
  listen<Telemetry>("telemetry", h);

// ----------------------------------------------------------- workspace ---

export const pickWorkspace = () =>
  isTauri ? invoke<string | null>("workspace_pick") : Promise.resolve(null);

export const listWorkspace = (root: string) =>
  isTauri
    ? invoke<WorkspaceFile[]>("workspace_list", { root })
    : Promise.resolve([]);

export const readWorkspaceFile = (root: string, rel: string) =>
  isTauri
    ? invoke<string>("workspace_read", { root, rel })
    : Promise.reject(new Error("indisponível no navegador"));

export const writeWorkspaceFile = (root: string, rel: string, content: string) =>
  isTauri
    ? invoke<void>("workspace_write", { root, rel, content })
    : Promise.reject(new Error("indisponível no navegador"));

export const revealWorkspace = (root: string, rel?: string | null) =>
  isTauri
    ? invoke<void>("workspace_reveal", { root, rel: rel ?? null })
    : Promise.reject(new Error("indisponível no navegador"));
