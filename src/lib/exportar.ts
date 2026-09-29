// Levar um texto para fora do app (os logs, a configuração): grava em
// `<dados>/exports/` e mostra a pasta. Ver `src-tauri/src/exportar.rs`.

import { invoke, isTauri } from "./tauri";

/** Grava e devolve o caminho onde ficou. */
export async function salvarExportacao(nome: string, conteudo: string): Promise<string> {
  if (isTauri) return invoke<string>("export_save", { name: nome, content: conteudo });
  // Navegador simulado: nada em disco, só o caminho que o app real devolveria.
  return `/home/voce/.local/share/dev.openweights.app/exports/${nome}`;
}

export async function mostrarExportacao(caminho: string | null): Promise<void> {
  if (isTauri) await invoke<void>("export_reveal", { path: caminho });
}
