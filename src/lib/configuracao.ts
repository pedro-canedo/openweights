// Exportar e importar a configuração (`src-tauri/src/config_io.rs`).

import { invoke, isTauri } from "./tauri";

/** O que um import aplicou (espelho de `config_io::Resumo`). */
export interface ResumoDoImport {
  /** Itens do arquivo que não foram aplicados por não serem seguros ou válidos. */
  ignored: number;
  settings: number;
  profiles: number;
  presets: number;
  enginePresets: number;
  providers: boolean;
  secrets: number;
}

/** O que o arquivo diz ter, lido do texto sem aplicar nada. */
export interface ConteudoDoArquivo {
  appVersion: string | null;
  profiles: number;
  presets: number;
  enginePresets: number;
  settings: number;
  providers: boolean;
  secrets: number;
}

export async function exportarConfiguracao(incluirSegredos: boolean): Promise<string> {
  if (isTauri) return invoke<string>("config_export", { includeSecrets: incluirSegredos });
  return JSON.stringify(
    {
      format: 1,
      app: "OpenWeights",
      appVersion: "0.1.0-dev",
      server: { server_port: "11711", server_parallel: "1" },
      profiles: [{ model: "Qwen3-8B-UD-Q4_K_XL.gguf", profile: { ctx: 32768 } }],
      presets: [],
      enginePresets: [],
      providers: {},
      includesSecrets: incluirSegredos,
      ...(incluirSegredos ? { secrets: { server_api_key: "segredo-de-exemplo" } } : {}),
    },
    null,
    2,
  );
}

/** Lê o texto e diz o que há nele; joga erro com o motivo se não serve. */
export function lerArquivo(texto: string): ConteudoDoArquivo {
  let doc: Record<string, unknown>;
  try {
    doc = JSON.parse(texto) as Record<string, unknown>;
  } catch (e) {
    throw new Error(`invalid-json:${e instanceof Error ? e.message : String(e)}`);
  }
  if (!doc || typeof doc !== "object" || doc.app !== "OpenWeights") throw new Error("not-openweights");
  if (doc.format !== 1) throw new Error(`format:${String(doc.format)}`);
  const conta = (v: unknown) => (Array.isArray(v) ? v.length : 0);
  const chaves = (v: unknown) => (v && typeof v === "object" ? Object.keys(v as object).length : 0);
  return {
    appVersion: typeof doc.appVersion === "string" ? doc.appVersion : null,
    profiles: conta(doc.profiles),
    presets: conta(doc.presets),
    enginePresets: conta(doc.enginePresets),
    settings: chaves(doc.server),
    providers: chaves(doc.providers) > 0,
    secrets: chaves(doc.secrets),
  };
}

export async function importarConfiguracao(
  texto: string,
  incluirSegredos: boolean,
): Promise<ResumoDoImport> {
  if (isTauri) return invoke<ResumoDoImport>("config_import", { text: texto, includeSecrets: incluirSegredos });
  const c = lerArquivo(texto);
  return {
    ignored: 0,
    settings: c.settings,
    profiles: c.profiles,
    presets: c.presets,
    enginePresets: c.enginePresets,
    providers: c.providers,
    secrets: incluirSegredos ? c.secrets : 0,
  };
}
