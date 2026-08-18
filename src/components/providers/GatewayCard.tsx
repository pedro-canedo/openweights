// Ponto de entrada único (Traefik local).
//
// Recurso opcional e desligado por padrão: nada no chat depende dele. O que
// ele entrega é uma URL só, estável, que roteia por prefixo para o motor
// local e para o 9router — útil para apontar uma ferramenta externa para o
// OpenWeights sem decorar duas portas. Não é túnel: não expõe nada na
// internet.

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  gatewayConfigSet,
  gatewayInstall,
  gatewayStart,
  gatewayStatus,
  gatewayStop,
  gatewayUninstall,
  type GatewayStatus,
} from "../../lib/providers";
import { errorMessage } from "../../lib/serverSession";

const botao =
  "rounded-lg border border-edge px-3 py-2 text-sm text-dim hover:text-ink disabled:opacity-50";

export default function GatewayCard() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<GatewayStatus | null>(null);
  const [porta, setPorta] = useState("11700");
  const [lan, setLan] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const recarregar = useCallback(async () => {
    try {
      const s = await gatewayStatus();
      setStatus(s);
      setPorta(String(s.port));
      setLan(s.exposeLan);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, []);

  useEffect(() => {
    void recarregar();
  }, [recarregar]);

  async function acao(f: () => Promise<GatewayStatus>) {
    setBusy(true);
    setError(null);
    try {
      setStatus(await f());
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  async function salvar() {
    const n = Number(porta);
    if (!Number.isInteger(n) || n < 1024 || n > 65535) {
      setError(t("providers.gateway.badPort"));
      return;
    }
    await acao(async () => {
      await gatewayConfigSet(n, lan);
      return gatewayStatus();
    });
  }

  return (
    <div className="rounded-xl border border-edge bg-panel px-5 py-4">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="text-sm">{t("providers.gateway.title")}</div>
          <div className="mt-1 text-[12px] text-dim">
            {t("providers.gateway.subtitle")}
          </div>
        </div>
        <div className="flex shrink-0 gap-2">
          {!status?.installed && (
            <button
              onClick={() => void acao(gatewayInstall)}
              disabled={busy}
              className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-white disabled:opacity-50"
            >
              {busy ? t("common.loading") : t("providers.gateway.install")}
            </button>
          )}
          {status?.installed && !status.running && (
            <button
              onClick={() => void acao(gatewayStart)}
              disabled={busy}
              className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-white disabled:opacity-50"
            >
              {t("providers.gateway.start")}
            </button>
          )}
          {status?.running && (
            <button
              onClick={() => void acao(gatewayStop)}
              disabled={busy}
              className={botao}
            >
              {t("providers.gateway.stop")}
            </button>
          )}
        </div>
      </div>

      <p className="mt-2 text-[11px] text-dim">{t("providers.gateway.notTunnel")}</p>

      {status?.installed && (
        <>
          <div className="mt-3 flex flex-col gap-2 sm:flex-row sm:items-center">
            <label className="flex items-center gap-2 text-[12px] text-dim">
              {t("providers.gateway.port")}
              <input
                value={porta}
                onChange={(e) => setPorta(e.target.value)}
                inputMode="numeric"
                className="w-24 rounded-lg border border-edge bg-panel2 px-2 py-1.5 text-sm outline-none focus:border-accent"
              />
            </label>
            <label className="flex items-center gap-2 text-[12px] text-dim">
              <input
                type="checkbox"
                checked={lan}
                onChange={(e) => setLan(e.target.checked)}
              />
              {t("providers.gateway.exposeLan")}
            </label>
            <button onClick={() => void salvar()} disabled={busy} className={botao}>
              {t("common.save")}
            </button>
          </div>
          {lan && (
            <p className="mt-2 rounded-lg border border-warn/40 bg-warn/10 px-3 py-2 text-[12px] text-warn">
              {t("providers.gateway.lanWarning")}
            </p>
          )}
        </>
      )}

      {status?.running && status.baseUrl && (
        <div className="mt-3 rounded-lg border border-edge bg-panel2 px-3 py-2">
          <div className="text-[12px] text-dim">
            {t("providers.gateway.routesTitle")}
          </div>
          {status.routes.length === 0 ? (
            <p className="mt-1 text-[12px] text-dim">
              {t("providers.gateway.noRoutes")}
            </p>
          ) : (
            <ul className="mt-1 space-y-0.5">
              {status.routes.map((r) => (
                <li key={r} className="font-mono text-[11px]">
                  {status.baseUrl}
                  {r}/v1/chat/completions
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      {error && (
        <p className="mt-2 rounded-lg border border-bad/40 bg-bad/10 px-3 py-2 text-[12px] text-bad">
          {error}
        </p>
      )}

      {status?.installed && !status.running && (
        <button
          onClick={() => void acao(gatewayUninstall)}
          disabled={busy}
          className="mt-3 rounded-lg border border-edge px-3 py-2 text-sm text-dim hover:border-bad/60 hover:text-bad disabled:opacity-50"
        >
          {t("providers.gateway.uninstall")}
        </button>
      )}
    </div>
  );
}
