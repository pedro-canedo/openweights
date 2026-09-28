// Área de transferência do sistema, lida e escrita pelo app.
//
// No app vai pelo backend (arboard): no WebKitGTK a API do navegador não é
// confiável para LER, e colar num terminal é ler. No navegador comum
// (npm run dev, Playwright) cai na API do navegador.

import { invoke, isTauri } from "./tauri";

export async function copiarTexto(texto: string): Promise<void> {
  if (isTauri) {
    await invoke("area_de_transferencia_escrever", { texto });
    return;
  }
  await navigator.clipboard.writeText(texto);
}

export async function lerTexto(): Promise<string> {
  if (isTauri) return invoke<string>("area_de_transferencia_ler");
  return navigator.clipboard.readText();
}
