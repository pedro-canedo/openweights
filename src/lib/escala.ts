// O tamanho da interface: um zoom da janela inteira, guardado entre sessões.
//
// No app é o zoom da webview (o mesmo do Ctrl+= e Ctrl+-); no navegador, o
// `zoom` do CSS, que é o que o Playwright enxerga.

import { isTauri } from "./tauri";

export const ESCALAS = [0.9, 1, 1.1, 1.25, 1.5] as const;

const CHAVE = "ow.zoom";

export function escalaGuardada(): number {
  try {
    const v = Number(localStorage.getItem(CHAVE));
    return (ESCALAS as readonly number[]).includes(v) ? v : 1;
  } catch {
    return 1;
  }
}

export async function aplicarEscala(fator: number, gravar = true): Promise<void> {
  if (gravar) {
    try {
      localStorage.setItem(CHAVE, String(fator));
    } catch {
      // sem armazenamento: vale só nesta sessão
    }
  }
  if (isTauri) {
    const { getCurrentWebview } = await import("@tauri-apps/api/webview");
    await getCurrentWebview().setZoom(fator);
  } else {
    document.documentElement.style.setProperty("zoom", String(fator));
  }
}
