// Card "Conectar": tudo que outro aplicativo precisa para usar este servidor
// como um provedor OpenAI — endereço, chave de API, modelo carregado e um
// teste de conexão de verdade.
//
// A chave é escrita EXCLUSIVAMENTE aqui (gerar/regenerar/remover): o card de
// configurações não conhece mais o setting `server_api_key`, senão o save()
// dele sobrescreveria a chave recém-gerada com valor velho.
//
// O teste usa `GET /props`, que É autenticado — nunca `/v1/models`, que o
// llama-server serve público e responderia 200 até com a chave errada.

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  engineBusyReason,
  getServerStatus,
  restartServer,
  serverGenerateApiKey,
  setSetting,
} from "../../lib/api";
import { routerModels } from "../../lib/flags";
import { errorMessage } from "../../lib/serverSession";
import type { ServerStatus } from "../../lib/types";

type TestResult =
  | { kind: "ok"; n: number }
  | { kind: "authStale" }
  | { kind: "down" };

/** `sk-local-abc…xyz` → `sk-local-…wxyz`: dá para reconhecer sem expor. */
function maskKey(key: string): string {
  const prefixo = key.startsWith("sk-local-") ? "sk-local-" : key.slice(0, 3);
  return `${prefixo}…${key.slice(-4)}`;
}

export default function ConnectCard({
  status,
  apiKey,
  onApiKeyChange,
}: {
  status: ServerStatus | null;
  apiKey: string;
  onApiKeyChange: (key: string) => void;
}) {
  const { t } = useTranslation();
  const running = !!status?.running;
  const [loadedModels, setLoadedModels] = useState<string[]>([]);
  const [keyBusy, setKeyBusy] = useState(false);
  const [needsRestart, setNeedsRestart] = useState(false);
  const [restarting, setRestarting] = useState(false);
  const [busyWith, setBusyWith] = useState<string[]>([]);
  const [testing, setTesting] = useState(false);
  const [test, setTest] = useState<TestResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  // "Reinicie para aplicar" não pode viver só em estado local — trocar de aba
  // remonta o card e a pendência sumiria. O backend calcula `keyStale`
  // (processo de pé com chave diferente da gravada); o estado aqui espelha a
  // prop e é re-obtido logo após gerar/remover, sem esperar o próximo evento.
  const [keyStale, setKeyStale] = useState(!!status?.keyStale);
  useEffect(() => {
    setKeyStale(!!status?.keyStale);
  }, [status?.keyStale]);

  /** Re-lê o status para pegar o `keyStale` recalculado no backend. */
  async function refreshKeyStale() {
    try {
      setKeyStale(!!(await getServerStatus()).keyStale);
    } catch {
      // sem resposta, fica valendo o que veio pela prop
    }
  }

  // A porta vem preenchida no status mesmo com o servidor parado — a base
  // URL derivada dela deixa o card útil antes do primeiro start.
  const base = status
    ? (status.baseUrl ?? `http://127.0.0.1:${status.port}`)
    : "http://127.0.0.1:11711";

  // Modelos carregados no Router, com o servidor de pé (mesmo poll da
  // seção de configuração do motor).
  useEffect(() => {
    if (!running) {
      setLoadedModels([]);
      return;
    }
    let alive = true;
    const poll = () =>
      routerModels()
        .then((list) => {
          if (alive)
            setLoadedModels(
              list.filter((m) => m.state === "loaded").map((m) => m.id),
            );
        })
        .catch(() => {});
    poll();
    const timer = setInterval(poll, 5000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [running]);

  // Processo derrubado (pelo botão de cima, ou por qualquer caminho): a
  // pendência morre com ele — o próximo boot já sobe com a chave gravada.
  // A mensagem de "motor ocupado" também: ela era sobre reiniciar um
  // processo que não existe mais.
  useEffect(() => {
    if (!running) {
      setNeedsRestart(false);
      setBusyWith([]);
    }
  }, [running]);

  async function gerar() {
    setKeyBusy(true);
    setError(null);
    setTest(null);
    try {
      const key = await serverGenerateApiKey();
      onApiKeyChange(key);
      // Chave nova só vale no próximo boot do processo. O needsRestart local
      // é o feedback imediato; o keyStale do backend é a fonte durável.
      if (running) setNeedsRestart(true);
      await refreshKeyStale();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setKeyBusy(false);
    }
  }

  async function remover() {
    setKeyBusy(true);
    setError(null);
    setTest(null);
    try {
      await setSetting("server_api_key", "");
      onApiKeyChange("");
      if (running) setNeedsRestart(true);
      await refreshKeyStale();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setKeyBusy(false);
    }
  }

  async function reiniciar() {
    setRestarting(true);
    setBusyWith([]);
    setError(null);
    // O resultado do "Testar conexão" descreve o processo antigo — some junto.
    setTest(null);
    try {
      const s = await restartServer();
      setNeedsRestart(false);
      setKeyStale(!!s.keyStale);
    } catch (e) {
      const quem = engineBusyReason(e);
      if (quem) setBusyWith(quem);
      else setError(errorMessage(e));
    } finally {
      setRestarting(false);
    }
  }

  async function testar() {
    setTesting(true);
    setTest(null);
    try {
      const headers: Record<string, string> = {};
      if (apiKey) headers.Authorization = `Bearer ${apiKey}`;
      const res = await fetch(`${base}/props`, { headers });
      if (res.status === 401) {
        // O processo está com OUTRA chave ativa — a gravada ainda não valeu.
        setTest({ kind: "authStale" });
      } else if (res.ok) {
        // `/v1/models` é público: serve para CONTAR, nunca para validar auth.
        let n = 0;
        try {
          const json = (await (await fetch(`${base}/v1/models`)).json()) as {
            data?: unknown[];
          };
          n = (json.data ?? []).length;
        } catch {
          // contagem é cosmética — o teste em si já passou
        }
        setTest({ kind: "ok", n });
      } else {
        setTest({ kind: "down" });
      }
    } catch {
      setTest({ kind: "down" });
    } finally {
      setTesting(false);
    }
  }

  const envBlock = `OPENAI_BASE_URL=${base}/v1\nOPENAI_API_KEY=${apiKey || "local"}`;

  const label = "text-[12px] text-dim";
  const btn =
    "rounded-lg border border-edge px-2.5 py-1.5 text-xs text-dim transition-colors hover:border-accent hover:text-ink disabled:opacity-40";

  return (
    <div className="mt-4 rounded-xl border border-edge bg-panel p-5">
      <div className="text-sm font-medium">{t("server.connect.title")}</div>

      {/* endereço */}
      <div className="mt-3">
        <div className={label}>{t("server.connect.baseUrl")}</div>
        <div className="mt-1 flex flex-wrap items-center gap-2">
          <CopyChip value={`${base}/v1`} />
        </div>
      </div>

      {/* chave */}
      <div className="mt-4">
        <div className={label}>{t("server.connect.key")}</div>
        {apiKey ? (
          <div className="mt-1 flex flex-wrap items-center gap-2">
            <span className="rounded-lg border border-edge bg-panel2 px-3 py-1.5 font-mono text-[12px]">
              {maskKey(apiKey)}
            </span>
            <span className="text-[11px] text-dim">
              {t("server.connect.keySet")}
            </span>
            <CopyButton text={apiKey} label={t("server.connect.copy")} />
            <button
              type="button"
              disabled={keyBusy}
              onClick={() => void gerar()}
              className={btn}
            >
              {keyBusy ? t("common.loading") : t("server.connect.regenerate")}
            </button>
            <button
              type="button"
              disabled={keyBusy}
              onClick={() => void remover()}
              className={`${btn} hover:border-bad hover:text-bad`}
            >
              {t("server.connect.remove")}
            </button>
          </div>
        ) : (
          <div className="mt-1 flex flex-col gap-2">
            <p className="text-[11px] leading-relaxed text-warn">
              {t("server.connect.noKey")} — {t("server.connect.noKeyWarning")}
              {status?.lan && ` ${t("server.connect.noKeyLanWarning")}`}
            </p>
            <button
              type="button"
              disabled={keyBusy}
              onClick={() => void gerar()}
              className={`${btn} self-start`}
            >
              {keyBusy ? t("common.loading") : t("server.connect.generate")}
            </button>
          </div>
        )}
        {/* Servidor parado pega a chave nova no próximo start — o aviso só
            faz sentido com o processo de pé, rodando com a chave antiga.
            O botão fica visível mesmo com a mensagem de ocupado logo abaixo:
            é por ele que a pessoa tenta de novo quando o motor liberar. */}
        {running && (keyStale || needsRestart) && (
          <div className="mt-2 flex items-center gap-3">
            <span className="text-[11px] text-warn">
              {t("server.connect.restartHint")}
            </span>
            <button
              type="button"
              disabled={restarting}
              onClick={() => void reiniciar()}
              className={btn}
            >
              {restarting
                ? t("common.loading")
                : t("server.connect.restartNow")}
            </button>
          </div>
        )}
        {busyWith.length > 0 && (
          <p className="mt-2 text-[11px] leading-relaxed text-warn">
            {t("server.busyToApply", {
              who: busyWith.map((w) => t(`server.busyWith.${w}`)).join(", "),
            })}
          </p>
        )}
      </div>

      {/* modelo carregado */}
      <div className="mt-4">
        <div className={label}>{t("server.connect.model")}</div>
        {loadedModels.length > 0 ? (
          <div className="mt-1 flex flex-wrap items-center gap-2">
            {loadedModels.map((id) => (
              <CopyChip key={id} value={id} />
            ))}
          </div>
        ) : (
          <p className="mt-1 text-[11px] leading-relaxed text-dim">
            {t("server.connect.noModel")}
          </p>
        )}
      </div>

      {/* variáveis de ambiente */}
      <div className="mt-4">
        <div className={label}>{t("server.connect.envBlock")}</div>
        <div className="relative mt-1">
          <pre className="overflow-x-auto rounded-lg border border-edge bg-panel2 p-3 font-mono text-[11.5px] leading-relaxed text-dim">
            {envBlock}
          </pre>
          <div className="absolute right-2 top-2">
            <CopyButton text={envBlock} label={t("server.connect.copy")} />
          </div>
        </div>
      </div>

      {/* testar conexão */}
      <div className="mt-4 flex flex-wrap items-center gap-3">
        <button
          type="button"
          disabled={testing}
          onClick={() => void testar()}
          className={btn}
        >
          {testing ? t("common.loading") : t("server.connect.test")}
        </button>
        {test?.kind === "ok" && (
          <span className="text-[11px] text-ok">
            ✓ {t("server.connect.testOk", { n: test.n })}
          </span>
        )}
        {test?.kind === "authStale" && (
          <span className="text-[11px] text-warn">
            {t("server.connect.testAuthStale")}
          </span>
        )}
        {test?.kind === "down" && (
          <span className="text-[11px] text-dim">
            {t("server.connect.testDown")}
          </span>
        )}
      </div>

      {error && <p className="mt-2 text-[12px] text-bad">{error}</p>}
    </div>
  );
}

function CopyChip({ value }: { value: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      onClick={() => {
        void navigator.clipboard.writeText(value).then(() => {
          setCopied(true);
          setTimeout(() => setCopied(false), 1200);
        });
      }}
      className="rounded-lg border border-edge bg-panel2 px-3 py-1.5 font-mono text-[12px] text-dim hover:text-ink"
    >
      {value} {copied ? "✓" : "⧉"}
    </button>
  );
}

function CopyButton({ text, label }: { text: string; label: string }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      onClick={() => {
        void navigator.clipboard.writeText(text).then(() => {
          setCopied(true);
          setTimeout(() => setCopied(false), 1200);
        });
      }}
      className="rounded-md border border-edge bg-panel px-2 py-1 text-[11px] text-dim hover:text-ink"
    >
      {copied ? t("server.copied") : label}
    </button>
  );
}
