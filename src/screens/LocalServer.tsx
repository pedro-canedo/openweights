// Servidor Local: uma faixa de estado que não rola, e quatro abas.
//
// A tela nasceu como uma pilha de onze cards de peso idêntico — status,
// conectar, estatísticas, porta, motor, especulação, benchmark, energia,
// flags, cluster, exemplos, logs. Tudo estava lá, e era exatamente esse o
// problema: quem só quer ligar o servidor e colar o endereço no Cursor
// atravessava cinco cards de ajuste fino para chegar ao terceiro; quem veio
// mexer numa flag rolava a tela inteira toda vez.
//
// A divisão segue a pergunta que a pessoa traz:
//
// - **Visão geral** — "ligou? como eu uso isto?"
// - **Desempenho** — "está rápido? dá para melhorar?"
// - **Rede** — "quem mais alcança este servidor?"
// - **Avançado** — "preciso mexer no que o app decidiu por mim."
//
// O estado do servidor fica fora das abas, grudado no topo: ele é premissa
// das quatro.

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  getHardwareProfile,
  getServerStatus,
  getSetting,
  onServerLog,
  onServerStatus,
  serveStats,
  setSetting,
  startServer,
  stopServer,
} from "../lib/api";
import { routerModels } from "../lib/flags";
import { takePendingServerTab } from "../lib/nav";
import type { ServerStatus } from "../lib/types";
import { Chips, NumChips, Select } from "../components/form/controls";
import { Card, Collapse, Page, Tabs, useTab, type TabDef } from "../components/ui/Shell";
import BenchHistoryCard from "../components/server/BenchHistoryCard";
import SpecCard from "../components/server/SpecCard";
import PowerCard from "../components/server/PowerCard";
import ClusterPanel from "../components/server/ClusterPanel";
import ConnectCard from "../components/server/ConnectCard";
import EngineConfigSection from "../components/server/EngineConfigSection";
import GlobalFlagsCard from "../components/server/GlobalFlagsCard";
import ServeStatsCard from "../components/server/ServeStatsCard";
import ServerHeader from "../components/server/ServerHeader";
import GettingStarted, { CHAVE_GUIA } from "../components/server/GettingStarted";
import UseElsewhere from "../components/server/UseElsewhere";

const MAX_LOG_LINES = 500;

/** De quanto em quanto tempo a faixa do topo relê modelo e velocidade. */
const RESUMO_MS = 10_000;

export default function LocalServer() {
  const { t } = useTranslation();
  const [tab, setTab] = useTab("ow.server.tab", "overview");
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
  // O que a faixa do topo mostra: o modelo que o Router tem carregado e a
  // velocidade média do que já foi servido nesta sessão.
  const [loadedModel, setLoadedModel] = useState<string | null>(null);
  const [genTps, setGenTps] = useState<number | null>(null);
  const [served, setServed] = useState(false);
  const [guideHidden, setGuideHidden] = useState(() => {
    try {
      return localStorage.getItem(CHAVE_GUIA) === "1";
    } catch {
      return false;
    }
  });

  // Quem chegou por um botão de outra tela ("Ajustar para esta máquina",
  // "GPU extra na rede") vem buscar uma coisa específica: a aba lembrada
  // cede a vez para a aba que a navegação pediu.
  useEffect(() => {
    const pedida = takePendingServerTab();
    if (pedida) setTab(pedida);
  }, [setTab]);

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

  // O resumo da faixa. Roda mesmo com o servidor parado: "já serviu alguma
  // coisa alguma vez" é o que decide se o guia de três passos ainda aparece,
  // e isso não pode depender de o motor estar de pé agora.
  useEffect(() => {
    let alive = true;
    const ler = () => {
      serveStats(null)
        .then((d) => {
          if (!alive) return;
          setGenTps(d.session.avgGenTps ?? d.allTime.avgGenTps);
          setServed(d.allTime.totalTokens > 0);
        })
        .catch(() => {});
      if (status?.running) {
        routerModels()
          .then((ms) => {
            if (alive) {
              setLoadedModel(ms.find((m) => m.state === "loaded")?.id ?? null);
            }
          })
          .catch(() => {});
      } else {
        setLoadedModel(null);
      }
    };
    ler();
    const id = window.setInterval(ler, RESUMO_MS);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, [status?.running]);

  const toggle = useCallback(async () => {
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
  }, [status?.running]);

  const running = !!status?.running;
  const abas: TabDef[] = [
    { id: "overview", label: t("server.tabs.overview") },
    { id: "performance", label: t("server.tabs.performance") },
    { id: "network", label: t("server.tabs.network") },
    { id: "advanced", label: t("server.tabs.advanced") },
  ];

  return (
    <Page title={t("server.title")} subtitle={t("server.subtitle")}>
      <ServerHeader
        status={status}
        busy={busy}
        onToggle={() => void toggle()}
        model={loadedModel}
        genTps={genTps}
        error={error}
      />

      <Tabs tabs={abas} value={tab} onChange={setTab} />

      {tab === "overview" && (
        <>
          {!guideHidden && !served && (
            <GettingStarted
              running={running}
              baseUrl={status?.baseUrl ?? null}
              served={served}
              onDismiss={() => {
                setGuideHidden(true);
                try {
                  localStorage.setItem(CHAVE_GUIA, "1");
                } catch {
                  // sem armazenamento: o guia volta na próxima visita
                }
              }}
            />
          )}
          <ConnectCard
            status={status}
            apiKey={apiKey}
            onApiKeyChange={setApiKey}
          />
          <UseElsewhere
            baseUrl={status?.baseUrl ?? null}
            apiKey={apiKey}
            model={loadedModel ?? selectedModel}
            loaded={loadedModel != null}
            running={running}
          />
          <ServeStatsCard running={running} />
        </>
      )}

      {tab === "performance" && (
        <>
          <EngineConfigSection
            running={running}
            hasGpu={hasGpu}
            selected={selectedModel}
            onSelect={setSelectedModel}
          />
          <SpecCard model={selectedModel} />
          <BenchHistoryCard model={selectedModel} running={running} />
          <PowerCard />
        </>
      )}

      {tab === "network" && (
        <>
          <ServerConfig running={running} />
          <ClusterPanel />
        </>
      )}

      {tab === "advanced" && (
        <>
          <GlobalFlagsCard running={running} />
          <Logs />
        </>
      )}
    </Page>
  );
}

/**
 * Porta, acesso pela rede e os dois números que dividem a placa.
 *
 * Continua sendo um formulário com botão de salvar — mudar a porta de um
 * servidor no ar por acidente seria pior que um clique a mais.
 */
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
    <Card title={t("server.network.title")} hint={t("server.network.hint")}>
      <div className="mt-4 grid grid-cols-2 gap-4">
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
    </Card>
  );
}

/** O log do motor, em bruto — fechado até alguém precisar dele. */
function Logs() {
  const { t } = useTranslation();
  const [lines, setLines] = useState<string[]>([]);

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

  return (
    <Collapse
      title={t("server.logs")}
      hint={t("server.logsHint")}
      badge={
        lines.length > 0 ? (
          <span className="rounded-full bg-panel2 px-1.5 py-0.5 text-[10px] text-dim">
            {lines.length}
          </span>
        ) : undefined
      }
    >
      <LogLines lines={lines} />
    </Collapse>
  );
}

function LogLines({ lines }: { lines: string[] }) {
  // O rolar automático segue a última linha enquanto o bloco estiver aberto.
  const [el, setEl] = useState<HTMLDivElement | null>(null);
  useEffect(() => {
    el?.scrollTo({ top: el.scrollHeight });
  }, [el, lines]);
  return (
    <div
      ref={setEl}
      className="max-h-64 overflow-y-auto font-mono text-[11px] leading-relaxed text-dim"
    >
      {lines.length ? lines.map((l, i) => <div key={i}>{l}</div>) : <div>—</div>}
    </div>
  );
}
