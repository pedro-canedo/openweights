// Tela de conectores MCP.
//
// Um conector é um servidor externo que empresta ferramentas ao agente, e
// isso é poder demais para conceder no escuro. Por isso a tela é organizada
// em torno da revisão, não do cadastro:
//
//  1. adicionar (formulário OU colando o bloco `mcpServers` do README);
//  2. testar a conexão — é o passo que lê o catálogo e grava a impressão
//     digital das definições;
//  3. ver ferramenta por ferramenta, com os selos que o servidor declarou;
//  4. aprovar.
//
// Se o servidor trocar as ferramentas depois (o ataque de "rug pull"), o
// banner de re-aprovação volta e o backend suspende o conector sozinho — a
// tela apenas mostra o que já está bloqueado.

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  formToConfigJson,
  mcpApproveTools,
  mcpServerAdd,
  mcpServerEnable,
  mcpServerRemove,
  mcpServerTest,
  mcpServerTools,
  mcpServersList,
  type McpManualForm,
  type McpServerView,
  type McpStatus,
  type McpToolView,
} from "../../lib/agent/mcp";

const EMPTY_FORM: McpManualForm = {
  transport: "stdio",
  command: "",
  args: "",
  env: "",
  url: "",
  headers: "",
};

const input =
  "w-full rounded-lg border border-edge bg-panel2 px-3 py-2 text-sm outline-none placeholder:text-dim focus:border-accent";
const ghostButton =
  "rounded-lg border border-edge px-3 py-1.5 text-[12px] text-dim transition-colors hover:border-accent hover:text-ink disabled:opacity-40";

export default function McpConnectors() {
  const { t } = useTranslation();
  const [servers, setServers] = useState<McpServerView[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showForm, setShowForm] = useState(false);

  const reload = useCallback(async () => {
    try {
      setServers(await mcpServersList());
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoaded(true);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  return (
    <div
      id="mcp-connectors"
      className="rounded-xl border border-edge bg-panel px-5 py-4"
    >
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="text-sm">{t("agent.mcp.title")}</div>
          <div className="mt-1 text-[12px] text-dim">
            {t("agent.mcp.subtitle")}
          </div>
        </div>
        <button
          type="button"
          onClick={() => setShowForm((v) => !v)}
          className="shrink-0 rounded-lg bg-accent px-3 py-1.5 text-[12px] font-medium text-white"
        >
          {t("agent.mcp.add")}
        </button>
      </div>

      {showForm && (
        <AddForm
          onCancel={() => setShowForm(false)}
          onAdded={async () => {
            setShowForm(false);
            await reload();
          }}
        />
      )}

      {error && (
        <p className="mt-3 rounded-lg border border-bad/40 bg-bad/10 px-3 py-2 text-[12px] text-bad">
          {error}
        </p>
      )}

      <div className="mt-4 flex flex-col gap-2">
        {!loaded && (
          <p className="text-[12px] text-dim">{t("common.loading")}</p>
        )}
        {loaded && servers.length === 0 && (
          <p className="text-[12px] text-dim">{t("agent.mcp.empty")}</p>
        )}
        {servers.map((server) => (
          <ServerCard key={server.id} server={server} onChanged={reload} />
        ))}
      </div>
    </div>
  );
}

// ------------------------------------------------------------ cadastro ---

function AddForm({
  onCancel,
  onAdded,
}: {
  onCancel: () => void;
  onAdded: () => void | Promise<void>;
}) {
  const { t } = useTranslation();
  const [mode, setMode] = useState<"manual" | "json">("manual");
  const [name, setName] = useState("");
  const [form, setForm] = useState<McpManualForm>(EMPTY_FORM);
  const [json, setJson] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function patch(part: Partial<McpManualForm>) {
    setForm((prev) => ({ ...prev, ...part }));
  }

  async function submit() {
    setBusy(true);
    setError(null);
    try {
      const config = mode === "json" ? json : formToConfigJson(form);
      await mcpServerAdd(name.trim(), config);
      setName("");
      setForm(EMPTY_FORM);
      setJson("");
      await onAdded();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  const canSubmit =
    !busy &&
    (mode === "json"
      ? json.trim().length > 0
      : form.transport === "http"
        ? form.url.trim().length > 0
        : form.command.trim().length > 0);

  return (
    <div className="mt-4 rounded-xl border border-edge bg-panel2 p-4">
      <div className="flex gap-1.5">
        {(["manual", "json"] as const).map((m) => (
          <button
            key={m}
            type="button"
            onClick={() => setMode(m)}
            className={`rounded-lg px-3 py-1.5 text-[12px] transition-colors ${
              mode === m
                ? "bg-accent/15 text-ink"
                : "text-dim hover:bg-panel hover:text-ink"
            }`}
          >
            {t(m === "manual" ? "agent.mcp.addManual" : "agent.mcp.addJson")}
          </button>
        ))}
      </div>

      <label className="mt-3 block text-[11px] text-dim">
        {t("agent.mcp.name")}
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder={t("agent.mcp.namePlaceholder")}
          className={`mt-1 ${input}`}
        />
      </label>

      {mode === "json" ? (
        <label className="mt-3 block text-[11px] text-dim">
          {t("agent.mcp.addJson")}
          <textarea
            value={json}
            onChange={(e) => setJson(e.target.value)}
            rows={7}
            spellCheck={false}
            placeholder={t("agent.mcp.jsonPlaceholder")}
            className={`mt-1 font-mono text-[12px] ${input}`}
          />
        </label>
      ) : (
        <>
          <label className="mt-3 block text-[11px] text-dim">
            {t("agent.mcp.transport")}
            <select
              value={form.transport}
              onChange={(e) =>
                patch({ transport: e.target.value as "stdio" | "http" })
              }
              className={`mt-1 ${input}`}
            >
              <option value="stdio">{t("agent.mcp.stdio")}</option>
              <option value="http">{t("agent.mcp.http")}</option>
            </select>
          </label>

          {form.transport === "stdio" ? (
            <>
              <label className="mt-3 block text-[11px] text-dim">
                {t("agent.mcp.command")}
                <input
                  value={form.command}
                  onChange={(e) => patch({ command: e.target.value })}
                  placeholder="npx"
                  spellCheck={false}
                  className={`mt-1 font-mono ${input}`}
                />
              </label>
              <label className="mt-3 block text-[11px] text-dim">
                {t("agent.mcp.args")}
                <input
                  value={form.args}
                  onChange={(e) => patch({ args: e.target.value })}
                  placeholder="-y @modelcontextprotocol/server-github"
                  spellCheck={false}
                  className={`mt-1 font-mono ${input}`}
                />
                <span className="mt-1 block text-[11px] text-dim">
                  {t("agent.mcp.argsHint")}
                </span>
              </label>
              <label className="mt-3 block text-[11px] text-dim">
                {t("agent.mcp.env")}
                <textarea
                  value={form.env}
                  onChange={(e) => patch({ env: e.target.value })}
                  rows={3}
                  spellCheck={false}
                  placeholder="GITHUB_TOKEN=..."
                  className={`mt-1 font-mono text-[12px] ${input}`}
                />
                <span className="mt-1 block text-[11px] text-dim">
                  {t("agent.mcp.envHint")}
                </span>
              </label>
            </>
          ) : (
            <>
              <label className="mt-3 block text-[11px] text-dim">
                {t("agent.mcp.url")}
                <input
                  value={form.url}
                  onChange={(e) => patch({ url: e.target.value })}
                  placeholder="https://exemplo.com/mcp"
                  spellCheck={false}
                  className={`mt-1 font-mono ${input}`}
                />
              </label>
              <label className="mt-3 block text-[11px] text-dim">
                {t("agent.mcp.headers")}
                <textarea
                  value={form.headers}
                  onChange={(e) => patch({ headers: e.target.value })}
                  rows={3}
                  spellCheck={false}
                  placeholder="Authorization: Bearer ..."
                  className={`mt-1 font-mono text-[12px] ${input}`}
                />
                <span className="mt-1 block text-[11px] text-dim">
                  {t("agent.mcp.headersHint")}
                </span>
              </label>
            </>
          )}
        </>
      )}

      {error && (
        <p className="mt-3 text-[12px] text-bad" role="alert">
          {error}
        </p>
      )}

      <div className="mt-4 flex justify-end gap-2">
        <button type="button" onClick={onCancel} className={ghostButton}>
          {t("common.cancel")}
        </button>
        <button
          type="button"
          disabled={!canSubmit}
          onClick={() => void submit()}
          className="rounded-lg bg-accent px-4 py-1.5 text-[12px] font-medium text-white disabled:opacity-40"
        >
          {t("common.save")}
        </button>
      </div>
    </div>
  );
}

// -------------------------------------------------------------- conector ---

function ServerCard({
  server,
  onChanged,
}: {
  server: McpServerView;
  onChanged: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const [tools, setTools] = useState<McpToolView[] | null>(null);
  const [open, setOpen] = useState(server.needsApproval);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // O banner de re-aprovação só é útil com a lista à vista: a pessoa precisa
  // ver O QUE mudou, e o cache responde mesmo com o servidor fora do ar.
  useEffect(() => {
    if (!open || tools) return;
    mcpServerTools(server.id)
      .then(setTools)
      .catch(() => setTools([]));
  }, [open, tools, server.id]);

  async function run(action: () => Promise<unknown>) {
    setBusy(true);
    setError(null);
    try {
      await action();
      await onChanged();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="rounded-xl border border-edge bg-panel2 px-4 py-3">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <StatusDot status={server.status} />
            <span className="truncate text-sm text-ink">{server.name}</span>
            <span className="shrink-0 rounded-md bg-panel px-1.5 py-0.5 text-[10px] text-dim">
              {t(server.transport === "http" ? "agent.mcp.http" : "agent.mcp.stdio")}
            </span>
          </div>
          <div className="mt-1 truncate font-mono text-[11px] text-dim">
            {server.summary}
          </div>
          <div className="mt-1 text-[11px] text-dim">
            {statusLabel(t, server.status)}
            {server.toolCount > 0 && (
              <> · {t("agent.mcp.toolsCount", { count: server.toolCount })}</>
            )}
          </div>
        </div>

        <div className="flex shrink-0 flex-col items-end gap-1.5">
          <button
            type="button"
            disabled={busy}
            onClick={() =>
              void run(async () => {
                setTools(await mcpServerTest(server.id));
                setOpen(true);
              })
            }
            className={ghostButton}
          >
            {busy ? t("agent.mcp.testing") : t("agent.mcp.test")}
          </button>
          <div className="flex gap-1.5">
            <button
              type="button"
              disabled={busy}
              onClick={() =>
                void run(() => mcpServerEnable(server.id, !server.enabled))
              }
              className={ghostButton}
            >
              {t(server.enabled ? "agent.mcp.disable" : "agent.mcp.enable")}
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => {
                if (!confirm(t("agent.mcp.removeConfirm", { name: server.name })))
                  return;
                void run(() => mcpServerRemove(server.id));
              }}
              className={ghostButton}
            >
              {t("agent.mcp.remove")}
            </button>
          </div>
        </div>
      </div>

      {server.lastError && (
        <p className="mt-2 text-[11px] text-bad">
          {t("agent.mcp.testFailed", { error: server.lastError })}
        </p>
      )}
      {error && <p className="mt-2 text-[11px] text-bad">{error}</p>}

      {server.needsApproval && (
        <div className="mt-3 rounded-lg border border-warn/40 bg-warn/10 px-3 py-2.5">
          <div className="text-[12px] font-medium text-ink">
            {t("agent.mcp.changedTitle")}
          </div>
          <p className="mt-1 text-[11px] text-dim">
            {t("agent.mcp.changedBody", { name: server.name })}
          </p>
          <button
            type="button"
            disabled={busy || !server.toolsHash}
            title={server.toolsHash ? undefined : t("agent.mcp.testFirst")}
            onClick={() =>
              void run(() =>
                mcpApproveTools(server.id, server.toolsHash as string),
              )
            }
            className="mt-2 rounded-lg bg-accent px-3 py-1.5 text-[12px] font-medium text-white disabled:opacity-40"
          >
            {t("agent.mcp.approveTools")}
          </button>
          {!server.toolsHash && (
            <p className="mt-1.5 text-[11px] text-dim">
              {t("agent.mcp.testFirst")}
            </p>
          )}
        </div>
      )}

      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="mt-2 text-[11px] text-dim underline-offset-2 hover:text-ink hover:underline"
      >
        {t(open ? "agent.mcp.hideTools" : "agent.mcp.showTools")}
      </button>

      {open && (
        <div className="mt-2 flex flex-col gap-1.5">
          {tools === null && (
            <p className="text-[11px] text-dim">{t("common.loading")}</p>
          )}
          {tools?.length === 0 && (
            <p className="text-[11px] text-dim">{t("agent.mcp.noTools")}</p>
          )}
          {tools?.map((tool) => <ToolRow key={tool.name} tool={tool} />)}
        </div>
      )}
    </div>
  );
}

function ToolRow({ tool }: { tool: McpToolView }) {
  const { t } = useTranslation();
  return (
    <div className="rounded-lg border border-edge bg-panel px-3 py-2">
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="font-mono text-[12px] text-ink">{tool.name}</span>
        {tool.readOnly && <Badge tone="ok">{t("agent.mcp.badgeReadOnly")}</Badge>}
        {tool.destructive && (
          <Badge tone="bad">{t("agent.mcp.badgeDestructive")}</Badge>
        )}
        {tool.openWorld && (
          <Badge tone="warn">{t("agent.mcp.badgeOpenWorld")}</Badge>
        )}
      </div>
      {tool.description && (
        <p className="mt-1 text-[11px] text-dim">{tool.description}</p>
      )}
    </div>
  );
}

function Badge({
  tone,
  children,
}: {
  tone: "ok" | "warn" | "bad";
  children: React.ReactNode;
}) {
  const tones = {
    ok: "border-ok/40 text-ok",
    warn: "border-warn/40 text-warn",
    bad: "border-bad/40 text-bad",
  } as const;
  return (
    <span
      className={`rounded-md border px-1.5 py-0.5 text-[10px] ${tones[tone]}`}
    >
      {children}
    </span>
  );
}

function StatusDot({ status }: { status: McpStatus }) {
  const color =
    status === "connected"
      ? "bg-ok"
      : status === "error"
        ? "bg-bad"
        : status === "needsApproval"
          ? "bg-warn"
          : "bg-dim";
  return <span className={`h-2 w-2 shrink-0 rounded-full ${color}`} />;
}

function statusLabel(t: (key: string) => string, status: McpStatus): string {
  switch (status) {
    case "connected":
      return t("agent.mcp.connected");
    case "disabled":
      return t("agent.mcp.statusDisabled");
    case "error":
      return t("agent.mcp.statusError");
    case "needsApproval":
      return t("agent.mcp.statusPending");
    default:
      return t("agent.mcp.disconnected");
  }
}
