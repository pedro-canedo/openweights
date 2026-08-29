import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  getHardwareProfile,
  getServerStatus,
  getSetting,
  onServerLog,
  onServerStatus,
  setSetting,
  startServer,
  stopServer,
} from "../lib/api";
import type { ServerStatus } from "../lib/types";
import { Chips, NumChips, Select } from "../components/form/controls";
import BenchHistoryCard from "../components/server/BenchHistoryCard";
import ClusterPanel from "../components/server/ClusterPanel";
import ConnectCard from "../components/server/ConnectCard";
import EngineConfigSection from "../components/server/EngineConfigSection";
import GlobalFlagsCard from "../components/server/GlobalFlagsCard";

const MAX_LOG_LINES = 500;

export default function LocalServer() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<ServerStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [hasGpu, setHasGpu] = useState(false);
  // A chave vive aqui porque dois cards dependem dela: o Conectar (que a
  // escreve) e os exemplos de uso (que a mostram no código pronto).
  const [apiKey, setApiKey] = useState("");
  // Modelo em foco na configuração do motor — o histórico de benchmark
  // logo abaixo mede e lista exatamente este.
  const [selectedModel, setSelectedModel] = useState("");

  useEffect(() => {
    let un: (() => void) | undefined;
    let cancelled = false;
    getServerStatus().then(setStatus).catch(() => {});
    getSetting("server_api_key")
      .then((v) => v && setApiKey(v))
      .catch(() => {});
    getHardwareProfile()
      .then((p) => setHasGpu(p.gpus.length > 0))
      .catch(() => {});
    onServerStatus(setStatus).then((f) => {
      // Se o cleanup rodou antes de o listen() resolver (StrictMode),
      // desregistra imediatamente para não vazar o listener.
      if (cancelled) f();
      else un = f;
    });
    return () => {
      cancelled = true;
      un?.();
    };
  }, []);

  async function toggle() {
    setBusy(true);
    setError(null);
    try {
      if (status?.running) {
        await stopServer();
        setStatus(await getServerStatus());
      } else {
        setStatus(await startServer());
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mx-auto max-w-4xl px-8 py-8">
      <h1 className="text-xl font-semibold">{t("server.title")}</h1>
      <p className="mt-1 text-sm text-dim">{t("server.subtitle")}</p>

      <div className="mt-6 flex items-center gap-3 rounded-xl border border-edge bg-panel p-5">
        <span
          className={`h-2.5 w-2.5 rounded-full ${status?.running ? "bg-ok" : "bg-dim"}`}
        />
        <span className="text-sm">
          {status?.running ? t("server.running") : t("server.stopped")}
        </span>
        {status?.running && status.baseUrl && (
          <CopyField value={status.baseUrl} />
        )}
        <button
          onClick={() => void toggle()}
          disabled={busy || !status}
          className="ml-auto rounded-lg bg-accent px-4 py-2 text-sm font-medium text-white disabled:opacity-50"
        >
          {busy
            ? t("common.loading")
            : status?.running
              ? t("server.stop")
              : t("server.start")}
        </button>
      </div>
      {error && <div className="mt-2 text-[12px] text-bad">{error}</div>}

      <ConnectCard status={status} apiKey={apiKey} onApiKeyChange={setApiKey} />
      <ServerConfig running={!!status?.running} />
      <EngineConfigSection
        running={!!status?.running}
        hasGpu={hasGpu}
        selected={selectedModel}
        onSelect={setSelectedModel}
      />
      <BenchHistoryCard model={selectedModel} running={!!status?.running} />
      <GlobalFlagsCard running={!!status?.running} />
      <ClusterPanel />
      {status?.baseUrl && <Examples baseUrl={status.baseUrl} apiKey={apiKey} />}
      <Logs />
    </div>
  );
}

function CopyField({ value }: { value: string }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  return (
    <button
      onClick={() => {
        navigator.clipboard.writeText(value).then(() => {
          setCopied(true);
          setTimeout(() => setCopied(false), 1200);
        });
      }}
      className="rounded-lg border border-edge bg-panel2 px-3 py-1.5 font-mono text-[12px] text-dim hover:text-ink"
      title={t("server.copy")}
    >
      {value} {copied ? "✓" : "⧉"}
    </button>
  );
}

function ServerConfig({ running }: { running: boolean }) {
  const { t } = useTranslation();
  const [port, setPort] = useState("11711");
  const [lan, setLan] = useState(false);
  const [modelsMax, setModelsMax] = useState("1");
  const [parallel, setParallel] = useState("1");
  const [saved, setSaved] = useState(false);

  // A chave de API NÃO passa por aqui: quem a escreve é só o card Conectar.
  // Regravá-la no save() sobrescreveria a chave recém-gerada com valor velho.
  useEffect(() => {
    getSetting("server_port").then((v) => v && setPort(v));
    getSetting("server_lan").then((v) => setLan(v === "true"));
    getSetting("server_models_max").then((v) => v && setModelsMax(v));
    getSetting("server_parallel").then((v) => v && setParallel(v));
  }, []);

  async function save() {
    await Promise.all([
      setSetting("server_port", port.trim()),
      setSetting("server_lan", String(lan)),
      setSetting("server_models_max", modelsMax.trim()),
      setSetting("server_parallel", parallel.trim()),
    ]);
    setSaved(true);
    setTimeout(() => setSaved(false), 1500);
  }

  const label = "text-[12px] text-dim";
  const oneToEight = Array.from({ length: 8 }, (_, i) => ({
    value: String(i + 1),
    label: String(i + 1),
  }));

  return (
    <div className="mt-4 rounded-xl border border-edge bg-panel p-5">
      <div className="grid grid-cols-2 gap-4">
        <div className="col-span-2">
          <div className={label}>{t("server.port")}</div>
          <div className="mt-1">
            <NumChips
              value={Number(port) || null}
              suggestions={[11711, 8080, 1234, 11434]}
              min={1024}
              max={65535}
              onCommit={(n) => setPort(String(n ?? 11711))}
            />
          </div>
          {/* 1234 e 11434 estão aqui de propósito: apps já apontados para o
              LM Studio ou o Ollama conectam sem mexer em nada. */}
          <p className="mt-1 text-[11px] leading-relaxed text-dim">
            {t("server.connect.portHint")}
          </p>
        </div>
        <div>
          <div className={label}>{t("server.modelsMax")}</div>
          <div className="mt-1">
            <Select
              value={modelsMax}
              options={oneToEight}
              onChange={setModelsMax}
            />
          </div>
          {/* Sem esta frase o número parece "quantos você tem"; ele é quanto
              a placa vai segurar ao mesmo tempo. */}
          <p className="mt-1 text-[11px] leading-relaxed text-dim">
            {t("server.modelsMaxHint")}
          </p>
        </div>
        <div>
          <div className={label}>{t("server.parallel")}</div>
          <div className="mt-1">
            <Select
              value={parallel}
              options={oneToEight}
              onChange={setParallel}
            />
          </div>
          {/* Sem esta frase o número parece "quantas abas posso abrir"; ele
              divide a janela de contexto entre as conversas. */}
          <p className="mt-1 text-[11px] leading-relaxed text-dim">
            {t("server.parallelHint")}
          </p>
        </div>
        <div className="col-span-2">
          <div className={label}>{t("server.lanAccess")}</div>
          <div className="mt-1">
            <Chips
              value={lan ? "on" : "off"}
              onChange={(v) => setLan(v === "on")}
              options={[
                { id: "off", label: t("server.fields.lanOff") },
                { id: "on", label: t("server.fields.lanOn") },
              ]}
            />
          </div>
          <p className="mt-1 text-[11px] leading-relaxed text-dim">
            {t("server.lanHint")}
          </p>
        </div>
      </div>
      <div className="mt-4 flex items-center gap-3">
        <button
          onClick={() => void save()}
          className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-white"
        >
          {saved ? "✓" : t("common.save")}
        </button>
        {running && (
          <span className="text-[11px] text-warn">{t("server.applyHint")}</span>
        )}
      </div>
    </div>
  );
}

function Examples({ baseUrl, apiKey }: { baseUrl: string; apiKey: string }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState<string | null>(null);

  // Com chave definida, os exemplos já saem prontos para colar — o curl
  // ganha o cabeçalho Bearer e o python usa a chave real.
  const curl = `curl ${baseUrl}/v1/chat/completions \\
  -H "Content-Type: application/json" \\${apiKey ? `\n  -H "Authorization: Bearer ${apiKey}" \\` : ""}
  -d '{"model": "SEU-MODELO", "messages": [{"role": "user", "content": "Olá!"}]}'`;

  const python = `from openai import OpenAI

client = OpenAI(base_url="${baseUrl}/v1", api_key="${apiKey || "local"}")
resp = client.chat.completions.create(
    model="SEU-MODELO",
    messages=[{"role": "user", "content": "Olá!"}],
)
print(resp.choices[0].message.content)`;

  const block = (name: string, code: string) => (
    <div className="relative mt-2">
      <pre className="overflow-x-auto rounded-lg border border-edge bg-panel2 p-3 font-mono text-[11.5px] leading-relaxed text-dim">
        {code}
      </pre>
      <button
        onClick={() => {
          navigator.clipboard.writeText(code).then(() => {
            setCopied(name);
            setTimeout(() => setCopied(null), 1200);
          });
        }}
        className="absolute right-2 top-2 rounded-md border border-edge bg-panel px-2 py-1 text-[11px] text-dim hover:text-ink"
      >
        {copied === name ? t("server.copied") : t("server.copy")}
      </button>
    </div>
  );

  return (
    <div className="mt-4 rounded-xl border border-edge bg-panel p-5">
      <div className="text-sm font-medium">{t("server.exampleTitle")}</div>
      {block("curl", curl)}
      {block("python", python)}
    </div>
  );
}

function Logs() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [lines, setLines] = useState<string[]>([]);
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let un: (() => void) | undefined;
    let cancelled = false;
    onServerLog((line) => {
      setLines((prev) => {
        const next = prev.length >= MAX_LOG_LINES ? prev.slice(1) : prev.slice();
        next.push(line);
        return next;
      });
    }).then((f) => {
      if (cancelled) f();
      else un = f;
    });
    return () => {
      cancelled = true;
      un?.();
    };
  }, []);

  useEffect(() => {
    if (open) bottomRef.current?.scrollIntoView({ block: "end" });
  }, [lines, open]);

  return (
    <div className="mt-4 rounded-xl border border-edge bg-panel">
      <button
        onClick={() => setOpen(!open)}
        className="flex w-full items-center justify-between px-5 py-3 text-sm"
      >
        {t("server.logs")}
        <span className="text-dim">{open ? "▾" : "▸"}</span>
      </button>
      {open && (
        <div className="max-h-64 overflow-y-auto border-t border-edge px-4 py-2 font-mono text-[11px] leading-relaxed text-dim">
          {lines.length ? (
            lines.map((l, i) => <div key={i}>{l}</div>)
          ) : (
            <div>—</div>
          )}
          <div ref={bottomRef} />
        </div>
      )}
    </div>
  );
}
