// O `owcli` no terminal do sistema: o estado e os dois gestos (ligar e
// desligar). O backend é `lr_owcli::noterminal`; aqui só o contrato.
//
// Os erros chegam com prefixo (`owcli-path-conflict:<caminho>`,
// `owcli-not-installed:`), e `motivoDoTerminal` os põe em palavras.

import i18n from "../i18n";
import { invoke, isTauri } from "./tauri";

/** Espelho do `commands_owcli::OwcliNoTerminal`. */
export interface OwcliNoTerminal {
  /** Este sistema tem como pôr o comando no terminal. */
  supported: boolean;
  /** O agente está instalado: sem ele não há o que apontar. */
  installed: boolean;
  /** O app tem um ponto seu no terminal (link ou entrada no Path). */
  ativo: boolean;
  apontaParaOAtual: boolean;
  /** O link (Linux, macOS) ou a pasta no Path (Windows). */
  local: string | null;
  /** A pasta do link está no PATH do app; no Windows, se está ativo. */
  noPath: boolean;
  /** Um arquivo que não é do app ocupa o lugar do link. */
  conflito: string | null;
  /** O comando chega pelo Path do usuário (Windows), e não por um link. */
  viaPath: boolean;
}

export const owcliNoTerminal = {
  status: () => (isTauri ? invoke<OwcliNoTerminal>("owcli_terminal_status") : simulado.status()),
  ativar: () => (isTauri ? invoke<OwcliNoTerminal>("owcli_terminal_ativar") : simulado.ativar()),
  desativar: () =>
    isTauri ? invoke<OwcliNoTerminal>("owcli_terminal_desativar") : simulado.desativar(),
};

/** Por que não deu, em palavras. */
export function motivoDoTerminal(erro: unknown): string {
  const texto = String(erro);
  const conflito = /^owcli-path-conflict:(.+)$/s.exec(texto);
  if (conflito) return i18n.t("owcliTerminal.conflict", { path: conflito[1] });
  if (texto.startsWith("owcli-not-installed")) return i18n.t("owcliTerminal.notInstalled");
  if (texto.startsWith("owcli-path-unsupported")) return i18n.t("owcliTerminal.unsupported");
  // O motivo do sistema (em inglês, às vezes) vem depois do prefixo do contrato.
  return i18n.t("owcliTerminal.failed", { error: texto.replace(/^owcli-[a-z-]+:/, "").trim() });
}

// --------------------------------------------------- navegador (Playwright) ---
// Os testes ligam, antes de a página carregar: `__owcliSemTerminal` (o agente
// não está instalado), `__owcliTerminalConflito` (um arquivo alheio ocupa o
// lugar), `__owcliTerminalForaDoPath` (a pasta do link não está no PATH),
// `__owcliTerminalWindows` (o comando chega pelo Path) e
// `__owcliTerminalDesatualizado` (o link aponta para uma versão antiga).

const simulado = (() => {
  let ativo = false;
  const g = () =>
    globalThis as {
      __owcliSemTerminal?: boolean;
      __owcliTerminalConflito?: boolean;
      __owcliTerminalForaDoPath?: boolean;
      __owcliTerminalWindows?: boolean;
      __owcliTerminalDesatualizado?: boolean;
    };
  const estado = (): OwcliNoTerminal => {
    const windows = !!g().__owcliTerminalWindows;
    return {
      supported: true,
      installed: !g().__owcliSemTerminal,
      ativo,
      apontaParaOAtual: ativo && !g().__owcliTerminalDesatualizado,
      local: windows
        ? "C:\\Users\\voce\\AppData\\Roaming\\dev.openweights.app\\runtimes\\owcli\\owcli-runtime-6621daf3-v1\\bin"
        : "/home/voce/.local/bin/owcli",
      noPath: windows ? ativo : !g().__owcliTerminalForaDoPath,
      conflito: g().__owcliTerminalConflito && !ativo ? "/home/voce/.local/bin/owcli" : null,
      viaPath: windows,
    };
  };
  return {
    status: async () => estado(),
    async ativar() {
      if (g().__owcliTerminalConflito) throw "owcli-path-conflict:/home/voce/.local/bin/owcli";
      ativo = true;
      return estado();
    },
    async desativar() {
      ativo = false;
      return estado();
    },
  };
})();
