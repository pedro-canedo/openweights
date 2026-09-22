// Jev: a camada de decisão barata na frente dos modelos locais.
//
// Três blocos, na ordem em que a decisão acontece: o decisor LOCAL (um
// segundo llama-server, do fork `parallel-decision`, com um modelo pequeno só
// para decidir — instalado daqui, com progresso no painel de downloads), a
// RESERVA no OpenRouter (o Jev da TypeSafe, que precisa da chave da aba ao
// lado), e os portões: onde a decisão se aplica, o limiar, o teste, os
// contadores. O aviso de privacidade fica junto da reserva, porque é ela
// que tira a mensagem da máquina.

import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { listLocalModels } from "../../lib/api";
import { DECISION_UNSUPPORTED, decisionStore, instalarDecisor } from "../../lib/decision";
import { formatBytes } from "../../lib/format";
import {
  jevConfigGet,
  jevConfigSet,
  jevStatus,
  jevTest,
  onJevStatus,
  type JevConfig,
  type JevStatus,
  type JevTestResult,
} from "../../lib/jev";
import { errorMessage } from "../../lib/serverSession";
import type { LocalModel } from "../../lib/types";
import { Card, StatusDot } from "../ui/Shell";

const LIMIARES = [0.5, 0.6, 0.7, 0.8, 0.9];

export default function JevCard() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<JevStatus | null>(null);
  const [cfg, setCfg] = useState<JevConfig | null>(null);
  const [modelos, setModelos] = useState<LocalModel[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [amostra, setAmostra] = useState<JevTestResult | null>(null);
  const instalacao = useSyncExternalStore(decisionStore.subscribe, decisionStore.get);

  const recarregar = useCallback(async () => {
    try {
      const [s, c, m] = await Promise.all([jevStatus(), jevConfigGet(), listLocalModels().catch(() => [])]);
      setStatus(s);
      setCfg(c);
      setModelos(m.filter((x) => !x.requiresPrism));
    } catch (e) {
      setError(errorMessage(e));
    }
  }, []);

  // Ao montar, quando a janela volta ao foco (a chave pode ter sido salva
  // na outra aba) e quando o backend avisa que o decisor mudou de estado.
  useEffect(() => {
    void recarregar();
    const aoFocar = () => void recarregar();
    window.addEventListener("focus", aoFocar);
    let un: (() => void) | undefined;
    void onJevStatus((s) => setStatus(s)).then((f) => {
      un = f;
    });
    return () => {
      window.removeEventListener("focus", aoFocar);
      un?.();
    };
  }, [recarregar]);

  async function salvar(mudanca: Partial<JevConfig>) {
    if (!cfg) return;
    const novo = { ...cfg, ...mudanca };
    setCfg(novo);
    setBusy(true);
    setError(null);
    try {
      setStatus(await jevConfigSet(novo));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  async function instalar() {
    setError(null);
    try {
      await instalarDecisor();
      await recarregar();
    } catch (e) {
      const msg = errorMessage(e);
      setError(msg.includes(DECISION_UNSUPPORTED) ? t("providers.decisions.local.unsupported") : msg);
    }
  }

  async function testar() {
    setBusy(true);
    setError(null);
    setAmostra(null);
    try {
      setAmostra(await jevTest());
      setStatus(await jevStatus());
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  const local = status?.local;
  const semChave = status != null && !status.keyPresent;
  const temDecisor = !!local?.runtimeInstalled && !!local?.modelPresent;
  const podeLigar = temDecisor || !semChave;
  const ligado = !!cfg?.enabled && podeLigar;
  const c = status?.contadores;
  const ultima = c?.ultima?.decisao;
  const instalando = instalacao.installing;
  const reservaPossivel = !!cfg?.remoteFallback && !semChave;

  const fonte = (f: "local" | "jev" | null | undefined) =>
    t(f === "local" ? "providers.decisions.source.local" : "providers.decisions.source.jev");

  return (
    <Card
      title={t("providers.decisions.title")}
      hint={t("providers.decisions.subtitle")}
      tone={ligado ? "ok" : "normal"}
      action={
        <label className="flex items-center gap-2 text-[12px] text-dim">
          <input
            type="checkbox"
            checked={!!cfg?.enabled}
            disabled={busy || !cfg || !podeLigar}
            onChange={(e) => void salvar({ enabled: e.target.checked })}
          />
          {t("providers.jev.enable")}
        </label>
      }
    >
      <div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-2">
        <StatusDot tone={ligado ? "ok" : "off"} />
        <span className="text-sm">
          {!podeLigar
            ? t("providers.decisions.needsOne")
            : ligado
              ? t("providers.jev.on")
              : t("providers.jev.off")}
        </span>
        {status?.proxyRunning && status.proxyBaseUrl && (
          <code className="rounded bg-panel2 px-1.5 py-0.5 text-[11px] text-dim">
            {t("providers.jev.proxy", { url: status.proxyBaseUrl })}
          </code>
        )}
      </div>

      {/* ------------------------------------------------ decisor local --- */}
      <section className="mt-4 rounded-lg border border-edge bg-panel2/40 p-3">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
          <StatusDot
            tone={local?.ready ? "ok" : local?.running ? "warn" : "off"}
            pulse={!!local?.running && !local?.ready}
          />
          <span className="text-sm font-medium">{t("providers.decisions.local.title")}</span>
          <span className="text-[12px] text-dim">
            {local == null
              ? t("common.loading")
              : !local.supported
                ? t("providers.decisions.local.unsupported")
                : instalando
                  ? t("providers.decisions.local.installing")
                  : !temDecisor
                    ? t("providers.decisions.local.notInstalled")
                    : local.ready
                      ? t("providers.decisions.local.ready", { url: local.baseUrl ?? "" })
                      : local.running
                        ? t("providers.decisions.local.loading")
                        : !local.enabled
                          ? t("providers.decisions.local.off")
                          : t("providers.decisions.local.idle")}
          </span>
        </div>
        <p className="mt-1.5 text-[11px] text-dim">{t("providers.decisions.local.what")}</p>

        {local?.supported && !temDecisor && (
          <div className="mt-2 flex flex-wrap items-center gap-2">
            <button
              type="button"
              onClick={() => void instalar()}
              disabled={instalando}
              className="rounded-lg bg-accent px-3 py-1.5 text-[12px] font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-50"
            >
              {instalando
                ? t("providers.decisions.local.installing")
                : t("providers.decisions.local.install", {
                    engine: formatBytes(local.runtimeApproxBytes),
                    model: formatBytes(local.modelBytes),
                  })}
            </button>
            <span className="text-[11px] text-dim">
              {local.runtimeInstalled
                ? t("providers.decisions.local.missingModel", { model: local.model })
                : local.modelPresent
                  ? t("providers.decisions.local.missingEngine")
                  : t("providers.decisions.local.both", { model: local.model })}
            </span>
          </div>
        )}
        {instalacao.error && <p className="mt-2 text-[12px] text-bad">{instalacao.error}</p>}

        {cfg && local?.supported && (
          <div className="mt-3 flex flex-col gap-2 sm:flex-row sm:flex-wrap sm:items-center sm:gap-x-5">
            <label className="flex items-center gap-2 text-[12px] text-dim">
              <input
                type="checkbox"
                checked={cfg.localEnabled ?? local.vramOk}
                disabled={busy || !temDecisor}
                onChange={(e) => void salvar({ localEnabled: e.target.checked })}
              />
              {t("providers.decisions.local.enable")}
              {cfg.localEnabled == null && (
                <span className="text-[11px] opacity-70">
                  {t(local.vramOk ? "providers.decisions.local.autoOn" : "providers.decisions.local.autoOff")}
                </span>
              )}
            </label>
            <label className="flex items-center gap-2 text-[12px] text-dim">
              {t("providers.decisions.local.model")}
              <select
                value={cfg.localModel}
                disabled={busy}
                onChange={(e) => void salvar({ localModel: e.target.value })}
                className="max-w-[16rem] rounded-lg border border-edge bg-panel2 px-2 py-1 text-sm outline-none focus:border-accent"
              >
                {!modelos.some((m) => m.name === cfg.localModel) && (
                  <option value={cfg.localModel}>{cfg.localModel}</option>
                )}
                {modelos.map((m) => (
                  <option key={m.name} value={m.name}>
                    {m.name}
                  </option>
                ))}
              </select>
            </label>
          </div>
        )}
        {local?.supported && !local.vramOk && (
          <p className="mt-2 text-[11px] text-dim">{t("providers.decisions.local.vram")}</p>
        )}
      </section>

      {/* ------------------------------------------------------ reserva --- */}
      <section className="mt-3 rounded-lg border border-edge bg-panel2/40 p-3">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
          <StatusDot tone={reservaPossivel ? "ok" : "off"} />
          <span className="text-sm font-medium">{t("providers.decisions.remote.title")}</span>
          <span className="text-[12px] text-dim">
            {semChave ? t("providers.jev.needsKey") : t("providers.decisions.remote.model", { model: status?.model ?? "" })}
          </span>
        </div>
        {cfg && (
          <label className="mt-2 flex items-center gap-2 text-[12px] text-dim">
            <input
              type="checkbox"
              checked={cfg.remoteFallback}
              disabled={busy}
              onChange={(e) => void salvar({ remoteFallback: e.target.checked })}
            />
            {t("providers.decisions.remote.enable")}
          </label>
        )}
        {reservaPossivel && <p className="mt-2 text-[11px] text-dim">{t("providers.jev.privacy")}</p>}
      </section>

      {/* ------------------------------------------------------ portões --- */}
      {cfg && podeLigar && (
        <div className="mt-3 flex flex-col gap-2 sm:flex-row sm:flex-wrap sm:items-center sm:gap-x-5">
          <label className="flex items-center gap-2 text-[12px] text-dim">
            <input
              type="checkbox"
              checked={cfg.gateChat}
              disabled={busy || !cfg.enabled}
              onChange={(e) => void salvar({ gateChat: e.target.checked })}
            />
            {t("providers.jev.gateChat")}
          </label>
          <label className="flex items-center gap-2 text-[12px] text-dim">
            <input
              type="checkbox"
              checked={cfg.gateHarness}
              disabled={busy || !cfg.enabled}
              onChange={(e) => void salvar({ gateHarness: e.target.checked })}
            />
            {t("providers.jev.gateHarness")}
          </label>
          <label className="flex items-center gap-2 text-[12px] text-dim">
            {t("providers.jev.minConfidence")}
            <select
              value={String(
                LIMIARES.reduce((a, b) =>
                  Math.abs(b - cfg.minConfidence) < Math.abs(a - cfg.minConfidence) ? b : a,
                ),
              )}
              disabled={busy || !cfg.enabled}
              onChange={(e) => void salvar({ minConfidence: Number(e.target.value) })}
              className="rounded-lg border border-edge bg-panel2 px-2 py-1 text-sm outline-none focus:border-accent"
            >
              {LIMIARES.map((l) => (
                <option key={l} value={String(l)}>
                  {Math.round(l * 100)}%
                </option>
              ))}
            </select>
          </label>
          <button
            type="button"
            onClick={() => void testar()}
            disabled={busy || !cfg.enabled}
            className="rounded-lg border border-edge px-3 py-1.5 text-[12px] text-dim transition-colors hover:border-accent hover:text-ink disabled:opacity-50"
          >
            {busy ? t("common.loading") : t("providers.jev.test")}
          </button>
        </div>
      )}

      {amostra && (
        <p className="mt-2 text-[12px]">
          {amostra.origem !== "padrao"
            ? t("providers.decisions.testOk", {
                level: t(`providers.jev.level.${amostra.nivel ?? "nenhum"}`),
                confidence: Math.round((amostra.confianca ?? 0) * 100),
                source: fonte(amostra.fonte),
                ms: amostra.ms,
              })
            : t("providers.jev.testDefault", { reason: amostra.motivo ?? "" })}
        </p>
      )}

      {c && c.chamadas > 0 && (
        <p className="mt-2 text-[11px] text-dim">
          {t("providers.decisions.counters", {
            calls: c.chamadas,
            applied: c.aplicadas,
            failures: c.falhas,
            local: c.locais,
            remote: c.remotas,
            cost: c.custo.toFixed(4),
          })}
          {ultima && (
            <>
              {" · "}
              {ultima.origem !== "padrao"
                ? t("providers.decisions.last", {
                    level: t(`providers.jev.level.${ultima.nivel ?? "nenhum"}`),
                    confidence: Math.round((ultima.confianca ?? 0) * 100),
                    where: t(`providers.jev.where.${c.ultima?.superficie ?? "chat"}`),
                    source: fonte(ultima.fonte),
                  })
                : t("providers.jev.lastDefault", { reason: ultima.motivo ?? "" })}
            </>
          )}
        </p>
      )}

      {error && <p className="mt-2 text-[12px] text-bad">{error}</p>}
    </Card>
  );
}
