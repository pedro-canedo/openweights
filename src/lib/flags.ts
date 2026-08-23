// Ponte tipada com o catálogo de flags do llama.cpp.
//
// O catálogo vem do backend: ~55 flags curadas (rótulo em `flags.catalog.*`
// no i18n) + todas as demais extraídas do `--help` do binário pinado. O
// preview do INI/argumentos também é renderizado no Rust, pelas MESMAS
// funções do boot — esta camada nunca monta comando nem INI.

import { invoke, isTauri } from "./tauri";
import type { ModelProfile } from "./tuning";

/** Que controle a interface desenha (espelho de `lr_types::flags::FlagKind`). */
export type FlagKind =
  | { type: "bool" }
  | { type: "tri" }
  | { type: "int"; min: number; max: number; step: number }
  | { type: "float"; min: number; max: number; step: number }
  | { type: "enum"; options: string[] }
  | { type: "text" }
  | { type: "path" }
  | { type: "list" };

export type FlagScope = "global" | "perModel" | "both" | "routerOnly" | "managed";

export type FlagRequirement =
  | "gpu"
  | "multiGpu"
  | "moeModel"
  | "mtpModel"
  | "mmprojPresent"
  | "specEnabled"
  | "ropeYarn"
  | "flashAttnOn";

export interface FlagSpec {
  key: string;
  aliases: string[];
  category: string;
  kind: FlagKind;
  default: string | null;
  scope: FlagScope;
  curated: boolean;
  helpText: string | null;
  requires: FlagRequirement[];
  conflicts: string[];
  dependsOn: string | null;
  /** Campo do ModelProfile que já cobre esta flag (a UI redireciona). */
  typedField: string | null;
}

export interface FlagCatalog {
  tag: string;
  variant: string;
  /** O `--help` do binário não pôde ser lido; só as curadas estão aqui. */
  degraded: boolean;
  flags: FlagSpec[];
}

export type IssueCode =
  | "unknown"
  | "managed"
  | "wrongScope"
  | "badValue"
  | "duplicateOfTyped"
  | "conflict"
  | "missingDependency"
  | "duplicate";

export interface FlagIssue {
  key: string;
  code: IssueCode;
  detail: string;
}

export interface EnginePreview {
  args: string[];
  ini: string;
  iniPath: string;
}

export interface RouterModelView {
  id: string;
  state: "unloaded" | "loading" | "loaded" | "unknown" | string;
}

export interface ModelCaps {
  moe: boolean | null;
  mtpHead: boolean | null;
  hasMmproj: boolean;
  nLayers: number | null;
  trainCtx: number | null;
  busyWith: string[];
}

export interface EnginePresetView {
  /** `builtin.<slug>` (rótulo via i18n) ou o id numérico dos salvos. */
  id: string;
  name: string;
  builtin: boolean;
  profile: ModelProfile;
}

/** Uma flag global gravada: o destino (args × INI `[*]`) e a natureza
 * (switch × com valor) são decididos no salvamento, com o catálogo em mãos. */
export interface GlobalFlag {
  key: string;
  value: string;
  place: "args" | "ini";
  switch: boolean;
}

export interface HarnessStatus {
  id: string;
  name: string;
  installed: boolean;
  path: string | null;
  launchable: boolean;
  installCmd: string;
  commandPreview: string;
  docsUrl: string;
}

export interface ServerStatusLike {
  running: boolean;
  baseUrl: string | null;
  port: number;
  lan: boolean;
}

export function flagsCatalog(): Promise<FlagCatalog> {
  if (!isTauri) return Promise.resolve(mockCatalog());
  return invoke<FlagCatalog>("flags_catalog");
}

export function flagsValidate(
  scope: "perModel" | "global",
  extras: [string, string][],
  model?: string | null,
): Promise<FlagIssue[]> {
  if (!isTauri) return Promise.resolve([]);
  return invoke<FlagIssue[]>("flags_validate", { scope, extras, model: model ?? null });
}

export function enginePreview(model?: string | null): Promise<EnginePreview> {
  if (!isTauri) {
    return Promise.resolve({
      args: ["--models-dir", "/dados/models", "--host", "127.0.0.1", "--port", "11711"],
      ini: `; gerado automaticamente — não edite\nversion = 1\n\n[${model ?? "modelo"}.gguf]\nmodel = /dados/models/m.gguf\nctx-size = 32768\n\n`,
      iniPath: "/dados/router-models.ini",
    });
  }
  return invoke<EnginePreview>("engine_preview", { model: model ?? null });
}

export function routerModels(): Promise<RouterModelView[]> {
  if (!isTauri) {
    return Promise.resolve([
      { id: "Qwen3.6-27B-MTP-Q4_K_M.gguf", state: "loaded" },
      { id: "gemma-3-4b-it-Q4_K_M.gguf", state: "unloaded" },
    ]);
  }
  return invoke<RouterModelView[]>("router_models");
}

/** Sobe o servidor se preciso e carrega o modelo no Router. Pode demorar
 * minutos num modelo grande — quem chama mostra progresso via `routerModels`. */
export function routerLoadModel(model: string): Promise<ServerStatusLike> {
  if (!isTauri) {
    return Promise.resolve({ running: true, baseUrl: "http://127.0.0.1:11711", port: 11711, lan: false });
  }
  return invoke<ServerStatusLike>("router_load_model", { model });
}

export function routerUnloadModel(model: string): Promise<void> {
  if (!isTauri) return Promise.resolve();
  return invoke<void>("router_unload_model", { model });
}

export function modelCapabilities(model: string): Promise<ModelCaps> {
  if (!isTauri) {
    return Promise.resolve({
      moe: model.toLowerCase().includes("a3b"),
      mtpHead: model.toLowerCase().includes("mtp"),
      hasMmproj: false,
      nLayers: 48,
      trainCtx: 262144,
      busyWith: [],
    });
  }
  return invoke<ModelCaps>("model_capabilities", { model });
}

export function enginePresetsList(): Promise<EnginePresetView[]> {
  if (!isTauri) {
    return Promise.resolve([
      { id: "builtin.default", name: "", builtin: true, profile: { source: "manual" } },
      {
        id: "builtin.mtpTurbo",
        name: "",
        builtin: true,
        profile: { spec: "mtp", specDraftNMax: 4, specDraftPMin: 0.75, flashAttn: true, source: "manual" },
      },
    ]);
  }
  return invoke<EnginePresetView[]>("engine_presets_list");
}

export function enginePresetSave(name: string, profile: ModelProfile): Promise<number> {
  if (!isTauri) return Promise.resolve(1);
  return invoke<number>("engine_preset_save", { name, profile });
}

export function enginePresetDelete(id: number): Promise<void> {
  if (!isTauri) return Promise.resolve();
  return invoke<void>("engine_preset_delete", { id });
}

/** Aplica (merge) um preset sobre o perfil do modelo e devolve o resultado. */
export function enginePresetApply(model: string, presetId: string): Promise<ModelProfile> {
  if (!isTauri) return Promise.resolve({ source: "manual" });
  return invoke<ModelProfile>("engine_preset_apply", { model, presetId });
}

export function harnessList(model: string): Promise<HarnessStatus[]> {
  if (!isTauri) {
    return Promise.resolve([
      {
        id: "dsh",
        name: "DeepSeek Harness",
        installed: false,
        path: null,
        launchable: true,
        installCmd: "npm install -g @deepseek-ai/dsh",
        commandPreview: "DSH_HOME=/dados/dsh-home npx -y @deepseek-ai/dsh web",
        docsUrl: "https://github.com/deepseek-ai/deepseek-harness",
      },
      {
        id: "aider",
        name: "Aider",
        installed: true,
        path: "/usr/local/bin/aider",
        launchable: true,
        installCmd: "python -m pip install aider-install && aider-install",
        commandPreview: `OPENAI_API_KEY=local aider --openai-api-base http://127.0.0.1:11711/v1 --model openai/${model}`,
        docsUrl: "https://aider.chat/docs/llms/openai-compat.html",
      },
    ]);
  }
  return invoke<HarnessStatus[]>("harness_list", { model });
}

export function harnessLaunch(id: string, model: string, workdir?: string | null): Promise<void> {
  if (!isTauri) return Promise.resolve();
  return invoke<void>("harness_launch", { id, model, workdir: workdir ?? null });
}

// ------------------------------------------------------------- simulação ---

function mockCatalog(): FlagCatalog {
  const f = (
    key: string,
    category: string,
    kind: FlagKind,
    scope: FlagScope,
    extra?: Partial<FlagSpec>,
  ): FlagSpec => ({
    key,
    aliases: [],
    category,
    kind,
    default: null,
    scope,
    curated: true,
    helpText: null,
    requires: [],
    conflicts: [],
    dependsOn: null,
    typedField: null,
    ...extra,
  });
  return {
    tag: "b10441",
    variant: "cuda13",
    degraded: false,
    flags: [
      f("ctx-size", "context", { type: "int", min: 512, max: 262144, step: 1 }, "perModel", {
        aliases: ["c"],
        typedField: "ctx",
      }),
      f("cache-reuse", "context", { type: "int", min: 0, max: 262144, step: 1 }, "perModel", {
        default: "0",
      }),
      f("spec-draft-n-max", "spec", { type: "int", min: 1, max: 16, step: 1 }, "perModel", {
        default: "3",
        typedField: "specDraftNMax",
        dependsOn: "spec-type",
        requires: ["specEnabled"],
      }),
      f("jinja", "usage", { type: "bool" }, "perModel"),
      f("metrics", "server", { type: "bool" }, "global"),
      f(
        "swa-full",
        "dynamic",
        { type: "bool" },
        "both",
        { curated: false, helpText: "use full-size SWA cache (default: false)" },
      ),
    ],
  };
}
