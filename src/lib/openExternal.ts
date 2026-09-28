// Abrir link no navegador do sistema, com o mesmo fallback que o resto do app
// já usava solto em duas telas.
//
// Passa pelo comando `open_external` do backend, e não pelo
// `@tauri-apps/plugin-opener`: no Linux o plugin chama o `xdg-open` de dentro
// do AppImage, que herda a glib embutida e não abre nada (sem erro nenhum).

import { invoke, isTauri } from "./tauri";

export async function openUrl(url: string): Promise<void> {
  if (!isTauri) {
    window.open(url, "_blank", "noopener");
    return;
  }
  try {
    await invoke("open_external", { url });
  } catch {
    window.open(url, "_blank", "noopener");
  }
}

const ESQUEMAS = ["http:", "https:", "mailto:", "tel:"];

/**
 * O clique em link que sai do app: `target="_blank"`, ou Ctrl/Shift+clique.
 *
 * É a mesma regra do script que o `tauri-plugin-opener` injetava (desligado
 * em `main.rs`), trocando só o destino: o `open_external` em vez do plugin.
 * Devolve a URL a abrir, ou `null` quando o clique não é com a gente.
 */
export function externalLinkOf(e: MouseEvent): string | null {
  if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.altKey) return null;
  const a = e
    .composedPath()
    .find((n) => (n as Partial<Node>).nodeName?.toUpperCase() === "A") as
    | HTMLAnchorElement
    | undefined;
  if (!a || !a.href) return null;
  if (a.target !== "_blank" && !e.ctrlKey && !e.shiftKey) return null;
  let url: URL;
  try {
    url = new URL(a.href);
  } catch {
    return null;
  }
  return ESQUEMAS.includes(url.protocol) ? url.href : null;
}

/** Liga a interceptação dos links externos. Uma vez, no boot da interface. */
export function installExternalLinks(): void {
  if (!isTauri) return;
  window.addEventListener("click", (e) => {
    const url = externalLinkOf(e);
    if (!url) return;
    e.preventDefault();
    void openUrl(url);
  });
}
