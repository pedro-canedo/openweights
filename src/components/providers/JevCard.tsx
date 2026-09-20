// Jev (TypeSafe): a camada de decisão barata na frente dos modelos locais.
//
// Vive na aba do OpenRouter porque É por ele que o Jev é alcançado — a chave
// é a mesma, e sem ela o interruptor fica desabilitado com o motivo. O que o
// cartão precisa deixar claro, além de ligar e desligar: que a mensagem
// atual (e um trecho curto da conversa) sai da máquina quando isto está
// ligado. O app promete conversa local; um recurso que quebra essa promessa
// tem de dizer isso onde se liga.

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  jevConfigGet,
  jevConfigSet,
  jevStatus,
  jevTest,
  type JevConfig,
  type JevLastDecision,
  type JevStatus,
} from "../../lib/jev";
import { errorMessage } from "../../lib/serverSession";
import { Card, StatusDot } from "../ui/Shell";

const LIMIARES = [0.5, 0.6, 0.7, 0.8, 0.9];

export default function JevCard() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<JevStatus | null>(null);
  const [cfg, setCfg] = useState<JevConfig | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [amostra, setAmostra] = useState<JevLastDecision["decisao"] | null>(null);

  const recarregar = useCallback(async () => {
    try {
      const [s, c] = await Promise.all([jevStatus(), jevConfigGet()]);
      setStatus(s);
      setCfg(c);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, []);

  // Ao montar e sempre que a janela volta ao foco: a chave pode ter sido
  // salva no cartão de cima, e o proxy sobe ou desce junto com o motor.
  useEffect(() => {
    void recarregar();
    const aoFocar = () => void recarregar();
    window.addEventListener("focus", aoFocar);
    return () => window.removeEventListener("focus", aoFocar);
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

  const semChave = status != null && !status.keyPresent;
  const ligado = !!cfg?.enabled && !semChave;
  const c = status?.contadores;
  const ultima = c?.ultima?.decisao;

  return (
    <Card
      title={t("providers.jev.title")}
      hint={t("providers.jev.subtitle")}
      tone={ligado ? "ok" : "normal"}
      action={
        <label className="flex items-center gap-2 text-[12px] text-dim">
          <input
            type="checkbox"
            checked={!!cfg?.enabled}
            disabled={busy || !cfg || semChave}
            onChange={(e) => void salvar({ enabled: e.target.checked })}
          />
          {t("providers.jev.enable")}
        </label>
      }
    >
      <div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-2">
        <StatusDot tone={ligado ? "ok" : "off"} />
        <span className="text-sm">
          {semChave
            ? t("providers.jev.needsKey")
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

      <p className="mt-2 text-[11px] text-dim">{t("providers.jev.privacy")}</p>

      {cfg && !semChave && (
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
          {amostra.origem === "jev"
            ? t("providers.jev.testOk", {
                level: t(`providers.jev.level.${amostra.nivel ?? "nenhum"}`),
                confidence: Math.round((amostra.confianca ?? 0) * 100),
              })
            : t("providers.jev.testDefault", { reason: amostra.motivo ?? "" })}
        </p>
      )}

      {c && c.chamadas > 0 && (
        <p className="mt-2 text-[11px] text-dim">
          {t("providers.jev.counters", {
            calls: c.chamadas,
            applied: c.aplicadas,
            failures: c.falhas,
            cost: c.custo.toFixed(4),
          })}
          {ultima && (
            <>
              {" · "}
              {ultima.origem === "jev"
                ? t("providers.jev.last", {
                    level: t(`providers.jev.level.${ultima.nivel ?? "nenhum"}`),
                    confidence: Math.round((ultima.confianca ?? 0) * 100),
                    where: t(`providers.jev.where.${c.ultima?.superficie ?? "chat"}`),
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
