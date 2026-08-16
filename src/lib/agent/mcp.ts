// Ponte tipada com os comandos dos conectores MCP.
//
// Mesma regra do resto do harness: nenhuma tela chama `invoke` direto. Fora
// do Tauri (npm run dev no navegador) tudo é simulado em memória, com um
// conector de exemplo já configurado — é assim que a tela de conectores é
// desenvolvida sem precisar de um servidor MCP de verdade.
//
// O fluxo que a interface precisa respeitar:
//   adicionar → testar conexão (lê o catálogo e grava o hash)
//   → revisar as ferramentas → aprovar.
// Enquanto a aprovação não vem, o conector não entrega nada ao agente.

import { invoke, isTauri } from "../tauri";
import type { ToolCategory, ToolTier } from "./types";

/** Situação de um conector. Espelha `lr_mcp::McpStatus`. */
export type McpStatus =
  | "disabled"
  | "disconnected"
  | "connected"
  | "error"
  | "needsApproval";

/** Linha da lista de conectores. Espelha `lr_mcp::McpServerView`. */
export interface McpServerView {
  id: string;
  name: string;
  /** `stdio` (programa local) ou `http` (servidor remoto). */
  transport: string;
  /** Comando ou URL, para a pessoa reconhecer o conector. */
  summary: string;
  enabled: boolean;
  status: McpStatus;
  needsApproval: boolean;
  /** Hash das definições atuais — é o que `mcpApproveTools` recebe. */
  toolsHash: string | null;
  toolCount: number;
  lastError: string | null;
}

/** Uma ferramenta do conector, com os selos da interface. */
export interface McpToolView {
  name: string;
  description: string;
  category: ToolCategory;
  tier: ToolTier;
  readOnly: boolean;
  destructive: boolean;
  openWorld: boolean;
  idempotent: boolean;
}

/** Formulário "preencher manualmente" da tela de conectores. */
export interface McpManualForm {
  transport: "stdio" | "http";
  command: string;
  args: string;
  env: string;
  url: string;
  headers: string;
}

// ------------------------------------------------------------- comandos ---

export function mcpServersList(): Promise<McpServerView[]> {
  return isTauri ? invoke<McpServerView[]>("mcp_servers_list") : mockList();
}

export function mcpServerAdd(
  name: string,
  configJson: string,
): Promise<McpServerView[]> {
  return isTauri
    ? invoke<McpServerView[]>("mcp_server_add", { name, configJson })
    : mockAdd(name, configJson);
}

export function mcpServerRemove(id: string): Promise<void> {
  return isTauri ? invoke<void>("mcp_server_remove", { id }) : mockRemove(id);
}

export function mcpServerEnable(id: string, enabled: boolean): Promise<void> {
  return isTauri
    ? invoke<void>("mcp_server_enable", { id, enabled })
    : mockEnable(id, enabled);
}

/** Conecta, lê o catálogo e grava o hash atual. */
export function mcpServerTest(id: string): Promise<McpToolView[]> {
  return isTauri
    ? invoke<McpToolView[]>("mcp_server_test", { id })
    : mockTest(id);
}

/** Ferramentas já conhecidas, direto do cache — não conecta. */
export function mcpServerTools(id: string): Promise<McpToolView[]> {
  return isTauri
    ? invoke<McpToolView[]>("mcp_server_tools", { id })
    : mockTools(id);
}

export function mcpApproveTools(id: string, hash: string): Promise<void> {
  return isTauri
    ? invoke<void>("mcp_server_approve_tools", { id, hash })
    : mockApprove(id, hash);
}

// -------------------------------------------------------------- ajudas ---

/**
 * Monta o JSON de uma entrada `mcpServers` a partir do formulário.
 *
 * O backend aceita o bloco inteiro colado; aqui só produzimos a forma de uma
 * entrada só, que é o que o formulário sabe descrever.
 */
export function formToConfigJson(form: McpManualForm): string {
  if (form.transport === "http") {
    return JSON.stringify({
      url: form.url.trim(),
      headers: parsePairs(form.headers),
    });
  }
  return JSON.stringify({
    command: form.command.trim(),
    args: splitArgs(form.args),
    env: parsePairs(form.env),
  });
}

/** Quebra a linha de argumentos respeitando aspas simples e duplas. */
export function splitArgs(text: string): string[] {
  const out: string[] = [];
  let current = "";
  let quote: string | null = null;
  let open = false;
  for (const ch of text) {
    if (quote) {
      if (ch === quote) quote = null;
      else current += ch;
      continue;
    }
    if (ch === '"' || ch === "'") {
      quote = ch;
      open = true;
      continue;
    }
    if (ch === " " || ch === "\t" || ch === "\n") {
      if (current || open) out.push(current);
      current = "";
      open = false;
      continue;
    }
    current += ch;
  }
  if (current || open) out.push(current);
  return out;
}

/** Lê `CHAVE=valor` por linha (o formato que as pessoas já colam). */
export function parsePairs(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const eq = line.indexOf("=");
    const colon = line.indexOf(":");
    const at = eq >= 0 && (colon < 0 || eq < colon) ? eq : colon;
    if (at <= 0) continue;
    out[line.slice(0, at).trim()] = line.slice(at + 1).trim();
  }
  return out;
}

// --------------------------------------------------------------- mocks ---
//
// Roteiro do navegador: um conector já aprovado e outro aguardando revisão,
// que é o estado mais difícil de desenhar e o que mais precisa ser visto.

let mockServers: McpServerView[] = [
  {
    id: "github",
    name: "GitHub",
    transport: "stdio",
    summary: "npx -y @modelcontextprotocol/server-github",
    enabled: true,
    status: "connected",
    needsApproval: false,
    toolsHash: "hash-github-1",
    toolCount: 2,
    lastError: null,
  },
  {
    id: "docs",
    name: "Documentação",
    transport: "http",
    summary: "https://exemplo.com/mcp",
    enabled: true,
    status: "needsApproval",
    needsApproval: true,
    toolsHash: "hash-docs-2",
    toolCount: 1,
    lastError: null,
  },
];

const mockCatalog: Record<string, McpToolView[]> = {
  github: [
    {
      name: "search_issues",
      description: "Busca issues no repositório.",
      category: "mcp",
      tier: "safe",
      readOnly: true,
      destructive: false,
      openWorld: false,
      idempotent: true,
    },
    {
      name: "create_issue",
      description: "Cria uma issue nova.",
      category: "mcp",
      tier: "caution",
      readOnly: false,
      destructive: false,
      openWorld: false,
      idempotent: false,
    },
  ],
  docs: [
    {
      name: "fetch_page",
      description: "Baixa uma página da documentação.",
      category: "network",
      tier: "danger",
      readOnly: true,
      destructive: false,
      openWorld: true,
      idempotent: true,
    },
  ],
};

function mockList(): Promise<McpServerView[]> {
  return Promise.resolve(mockServers.map((s) => ({ ...s })));
}

function mockAdd(name: string, configJson: string): Promise<McpServerView[]> {
  let parsed: Record<string, unknown>;
  try {
    parsed = JSON.parse(configJson) as Record<string, unknown>;
  } catch {
    return Promise.reject(new Error("configuração inválida: não é um JSON válido"));
  }
  const label = name.trim() || "conector";
  const id = label.toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_|_$/g, "");
  const url = typeof parsed.url === "string" ? parsed.url : null;
  mockServers = mockServers.filter((s) => s.id !== id);
  mockServers.push({
    id,
    name: label,
    transport: url ? "http" : "stdio",
    summary: url ?? String(parsed.command ?? "?"),
    enabled: true,
    status: "disconnected",
    needsApproval: false,
    toolsHash: null,
    toolCount: 0,
    lastError: null,
  });
  mockCatalog[id] = [];
  return mockList();
}

function mockRemove(id: string): Promise<void> {
  mockServers = mockServers.filter((s) => s.id !== id);
  delete mockCatalog[id];
  return Promise.resolve();
}

function mockEnable(id: string, enabled: boolean): Promise<void> {
  const server = mockServers.find((s) => s.id === id);
  if (server) {
    server.enabled = enabled;
    server.status = enabled
      ? server.needsApproval
        ? "needsApproval"
        : "disconnected"
      : "disabled";
  }
  return Promise.resolve();
}

function mockTest(id: string): Promise<McpToolView[]> {
  const server = mockServers.find((s) => s.id === id);
  if (!server) return Promise.reject(new Error(`conector \`${id}\` não existe`));
  const tools = mockCatalog[id] ?? [];
  server.toolsHash = `hash-${id}-${tools.length}`;
  server.toolCount = tools.length;
  server.needsApproval = true;
  server.status = "needsApproval";
  server.lastError = null;
  return Promise.resolve(tools.map((t) => ({ ...t })));
}

function mockTools(id: string): Promise<McpToolView[]> {
  return Promise.resolve((mockCatalog[id] ?? []).map((t) => ({ ...t })));
}

function mockApprove(id: string, hash: string): Promise<void> {
  const server = mockServers.find((s) => s.id === id);
  if (!server) return Promise.reject(new Error(`conector \`${id}\` não existe`));
  if (server.toolsHash !== hash) {
    return Promise.reject(
      new Error("as ferramentas mudaram enquanto você revisava"),
    );
  }
  server.needsApproval = false;
  server.status = server.enabled ? "connected" : "disabled";
  return Promise.resolve();
}
