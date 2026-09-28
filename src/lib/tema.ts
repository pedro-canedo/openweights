// O tema da interface: escuro (padrão) ou claro, guardado entre sessões.
// Configurações e a paleta de comandos trocam por aqui; o `main.tsx` aplica o
// guardado antes da primeira pintura.

import { useSyncExternalStore } from "react";

export type Tema = "dark" | "light";

const CHAVE = "theme";

function guardado(): Tema {
  try {
    return localStorage.getItem(CHAVE) === "light" ? "light" : "dark";
  } catch {
    return "dark";
  }
}

let tema: Tema = guardado();
const ouvintes = new Set<() => void>();

export function aplicarTema(novo: Tema) {
  tema = novo;
  try {
    localStorage.setItem(CHAVE, novo);
  } catch {
    // sem armazenamento: vale só nesta sessão
  }
  if (novo === "light") document.documentElement.dataset.theme = "light";
  else delete document.documentElement.dataset.theme;
  ouvintes.forEach((f) => f());
}

export function useTema(): Tema {
  return useSyncExternalStore(
    (f) => {
      ouvintes.add(f);
      return () => ouvintes.delete(f);
    },
    () => tema,
  );
}
