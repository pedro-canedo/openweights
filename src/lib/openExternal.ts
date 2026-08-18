// Abrir link no navegador do sistema, com o mesmo fallback que o resto do app
// já usava solto em duas telas.

import { isTauri } from "./tauri";

export async function openUrl(url: string): Promise<void> {
  if (!isTauri) {
    window.open(url, "_blank", "noopener");
    return;
  }
  try {
    const { openUrl: abrir } = await import("@tauri-apps/plugin-opener");
    await abrir(url);
  } catch {
    window.open(url, "_blank", "noopener");
  }
}
