// Metadados de apresentação das ferramentas: nome amigável e ícone por
// categoria. Fica separado porque a ApprovalBar, o card e o painel de
// execução precisam exatamente do mesmo vocabulário.

import type { TFunction } from "i18next";
import type { ToolCategory, ToolTier } from "../../lib/agent/types";

/** Ferramentas nativas → chave em `agent.tools.*` (i18n já congelado). */
const TOOL_LABEL_KEYS: Record<string, string> = {
  fs_read: "fsRead",
  fs_write: "fsWrite",
  fs_edit: "fsEdit",
  fs_list: "fsList",
  fs_glob: "fsGlob",
  fs_grep: "fsGrep",
  terminal_run: "terminalRun",
  todo_update: "todoUpdate",
  memory_save: "memorySave",
  workspace_search: "workspaceSearch",
};

/**
 * Nome exibido. Ferramentas de MCP chegam como `<server>__<tool>`: mostramos
 * só a parte da ferramenta, com underscores virando espaço.
 */
export function toolLabel(t: TFunction, name: string): string {
  const key = TOOL_LABEL_KEYS[name];
  if (key) return t(`agent.tools.${key}`);
  const short = name.includes("__") ? name.slice(name.indexOf("__") + 2) : name;
  return short.replace(/_/g, " ");
}

/** Cor do ícone/rótulo por categoria (tokens do projeto). */
export function categoryClass(category: ToolCategory): string {
  switch (category) {
    case "edit":
      return "text-warn";
    case "execute":
      return "text-accent";
    case "network":
      return "text-sky-400";
    case "mcp":
      return "text-fuchsia-400";
    case "meta":
      return "text-dim";
    default:
      return "text-dim";
  }
}

/** Cor da borda por risco declarado da ferramenta. */
export function tierClass(tier: ToolTier): string {
  switch (tier) {
    case "danger":
      return "border-bad/40";
    case "caution":
      return "border-warn/30";
    default:
      return "border-edge";
  }
}

const PATHS: Record<ToolCategory, string> = {
  read: "M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8zM14 2v6h6M9 13h6M9 17h4",
  edit: "M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7M18.5 2.5a2.1 2.1 0 013 3L12 15l-4 1 1-4z",
  execute: "M4 17l6-6-6-6M12 19h8",
  network:
    "M12 21a9 9 0 100-18 9 9 0 000 18zM3.6 9h16.8M3.6 15h16.8M12 3a15 15 0 010 18 15 15 0 010-18z",
  mcp: "M9 2v6M15 2v6M6 8h12v4a6 6 0 01-12 0zM12 18v4",
  meta: "M4 6h16M4 12h10M4 18h7",
};

export function ToolIcon({
  category,
  className = "h-3.5 w-3.5",
}: {
  category: ToolCategory;
  className?: string;
}) {
  return (
    <svg
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      viewBox="0 0 24 24"
    >
      <path d={PATHS[category]} />
    </svg>
  );
}

/** JSON dos argumentos formatado; devolve o texto cru se não for JSON. */
export function prettyArgs(argsJson: string): string {
  if (!argsJson.trim()) return "";
  try {
    return JSON.stringify(JSON.parse(argsJson), null, 2);
  } catch {
    return argsJson;
  }
}

/**
 * Heurística de "fora da pasta do projeto" para o aviso da aprovação.
 * Caminho relativo é sempre considerado dentro; absoluto precisa começar
 * pelo workspace (comparação tolerante a `/` × `\` e maiúsculas).
 */
export function isOutsideWorkspace(
  target: string | null,
  workspaceDir: string | null,
): boolean {
  if (!target) return false;
  const isAbsolute = /^([a-zA-Z]:[\\/]|[\\/]|\\\\)/.test(target);
  if (!isAbsolute) return false;
  if (!workspaceDir) return true;
  const norm = (s: string) =>
    s.replace(/[\\/]+/g, "/").replace(/\/+$/, "").toLowerCase();
  return !norm(target).startsWith(norm(workspaceDir));
}
