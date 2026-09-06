// O motor de IA: o card que responde "isto aqui está funcionando?".
//
// O app não usa o llama.cpp do sistema — ele instala o seu, numa build
// homologada, dentro da própria pasta de dados. O card antigo dizia
// "b10441 · cuda13 · instalado" e oferecia um botão "Instalar / atualizar".
// Três problemas nisso, e todos apareciam justamente na hora em que algo
// dava errado:
//
// - "instalado" era a presença de um arquivo. Um pacote CUDA sem as DLLs do
//   cudart ao lado passa nessa checagem e falha na primeira carga de modelo.
// - a tag mostrada era sempre a que o app espera, nunca a que está no disco:
//   quem tinha uma build velha lia "b10441" e concluía que estava em dia.
// - "Instalar / atualizar" não dizia se havia o que atualizar. Clicar era o
//   único jeito de descobrir — e clicar baixa 1,8 GB.
//
// Agora a verificação executa `llama-server --version` de verdade, compara o
// que respondeu com o que esta versão do app espera, e diz em uma frase o
// que fazer. O que sobrou de builds antigas aparece com o tamanho e um botão
// para devolver o espaço.

import { useEffect, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { ensureRuntime, onRuntimeEvent, runtimePrune } from "../../lib/api";
import { engineStore, verificarMotor } from "../../lib/engine";
import { formatBytes, formatDuration } from "../../lib/format";
import type { EngineVerdict, RuntimeEvent } from "../../lib/types";
import { Card, StatusDot, type Tone } from "../ui/Shell";

/** A bolinha de cada veredito. */
const TOM: Record<EngineVerdict, Tone> = {
  ready: "ok",
  notInstalled: "off",
  updateAvailable: "warn",
  variantChanged: "warn",
  broken: "bad",
};

/** A borda do card. Só o que pede providência se tinge — um card verde
 *  permanente vira decoração e deixa de significar "está tudo bem". */
const BORDA: Record<EngineVerdict, "normal" | "warn" | "bad"> = {
  ready: "normal",
  notInstalled: "normal",
  updateAvailable: "warn",
  variantChanged: "warn",
  broken: "bad",
};

export default function EngineCard() {
  const { t } = useTranslation();
  // O resultado é do aplicativo, não desta tela: a barra lateral lê o mesmo
  // store para marcar o item de Ajustes quando há providência a tomar.
  const { check, checking } = useSyncExternalStore(
    engineStore.subscribe,
    engineStore.get,
  );
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<RuntimeEvent | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pruneArmed, setPruneArmed] = useState(false);
  const [pruned, setPruned] = useState<string | null>(null);

  const verificar = (force = true) => {
    setError(null);
    return verificarMotor(force);
  };

  // Ao abrir a tela: se ninguém verificou ainda nesta sessão, verifica agora.
  // Perguntar "quer verificar?" para quem veio ver se está tudo bem é
  // devolver a pergunta.
  useEffect(() => {
    void verificarMotor();
  }, []);

  async function instalar() {
    setBusy(true);
    setError(null);
    const un = await onRuntimeEvent(setProgress);
    try {
      await ensureRuntime();
      await verificarMotor(true);
    } catch (e) {
      setError(String(e));
    } finally {
      un();
      setBusy(false);
      setProgress(null);
    }
  }

  async function limpar() {
    if (!pruneArmed) {
      setPruneArmed(true);
      setTimeout(() => setPruneArmed(false), 4000);
      return;
    }
    setPruneArmed(false);
    setBusy(true);
    try {
      const r = await runtimePrune();
      setPruned(
        r.failed.length > 0
          ? t("settings.engine.prunedPartial", {
              size: formatBytes(r.freedBytes),
              n: r.failed.length,
            })
          : t("settings.engine.pruned", { size: formatBytes(r.freedBytes) }),
      );
      await verificarMotor(true);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  const verdict = check?.verdict;
  const precisaBaixar =
    verdict === "notInstalled" ||
    verdict === "updateAvailable" ||
    verdict === "variantChanged" ||
    verdict === "broken";

  return (
    <Card
      title={t("settings.runtime")}
      hint={t("settings.engine.hint")}
      tone={verdict ? BORDA[verdict] : "normal"}
      action={
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => void verificar()}
            disabled={checking || busy}
            className="rounded-lg border border-edge px-3 py-2 text-[12px] text-dim transition-colors hover:border-accent hover:text-ink disabled:opacity-50"
          >
            {checking ? t("settings.engine.checking") : t("settings.engine.check")}
          </button>
          <button
            type="button"
            onClick={() => void instalar()}
            disabled={busy || checking}
            className={`rounded-lg px-4 py-2 text-sm font-medium disabled:opacity-50 ${
              precisaBaixar
                ? "bg-accent text-white"
                : "border border-edge text-dim hover:text-ink"
            }`}
          >
            {busy
              ? t("common.loading")
              : precisaBaixar
                ? t("settings.engine.install")
                : t("settings.engine.reinstall")}
          </button>
        </div>
      }
    >
      {/* A frase do estado — a única linha que quem não é técnico precisa
          ler. */}
      <div className="mt-4 flex items-start gap-2.5">
        <span className="mt-1">
          <StatusDot
            tone={checking ? "busy" : verdict ? TOM[verdict] : "off"}
            pulse={checking || busy}
          />
        </span>
        <div className="min-w-0">
          <p className="text-sm">
            {checking
              ? t("settings.engine.checkingLong")
              : verdict
                ? t(`settings.engine.verdict.${verdict}`)
                : t("common.loading")}
          </p>
          {check && !checking && (
            <p className="mt-0.5 text-[12px] leading-relaxed text-dim">
              {t(`settings.engine.advice.${check.verdict}`)}
            </p>
          )}
        </div>
      </div>

      {/* Os fatos por trás da frase, para quem quiser conferir. */}
      {check && (
        <dl className="mt-4 grid grid-cols-2 gap-x-8 gap-y-1.5 border-t border-edge pt-3 text-[12px]">
          <Fato
            termo={t("settings.engine.expected")}
            valor={`${check.expectedTag} · ${check.expectedVariant}`}
          />
          <Fato
            termo={t("settings.engine.onDisk")}
            valor={
              check.active
                ? `${check.active.tag} · ${check.active.variantDir}`
                : check.others.length > 0
                  ? `${check.others[0].tag} · ${check.others[0].variantDir}`
                  : t("settings.engine.none")
            }
          />
          <Fato
            termo={t("settings.engine.reported")}
            valor={
              check.reportedBuild != null
                ? `b${check.reportedBuild}${
                    check.probeMs != null
                      ? ` · ${formatDuration(check.probeMs)}`
                      : ""
                  }`
                : "—"
            }
            title={check.detail ?? undefined}
          />
          <Fato
            termo={t("settings.engine.size")}
            valor={
              check.active ? formatBytes(check.active.sizeBytes) : "—"
            }
          />
          {check.active && (
            <Fato
              termo={t("settings.engine.folder")}
              valor={check.active.dir}
              span
              mono
            />
          )}
        </dl>
      )}

      {/* O detalhe técnico do que falhou fica à vista quando falhou: é o que
          se cola numa busca ou num relato de erro. */}
      {check?.verdict === "broken" && check.detail && (
        <pre className="mt-3 overflow-x-auto rounded-lg border border-bad/30 bg-panel2 p-3 font-mono text-[11px] leading-relaxed text-dim">
          {check.detail}
        </pre>
      )}

      {progress?.kind === "progress" && progress.totalBytes > 0 && (
        <div className="mt-4">
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-panel2">
            <div
              className="brand-gradient h-full rounded-full transition-[width]"
              style={{
                width: `${(progress.receivedBytes / progress.totalBytes) * 100}%`,
              }}
            />
          </div>
          <div className="mt-1 text-[11px] text-dim">
            {formatBytes(progress.receivedBytes)} /{" "}
            {formatBytes(progress.totalBytes)} — {progress.asset}
          </div>
        </div>
      )}
      {progress?.kind === "extracting" && (
        <p className="mt-3 text-[11px] text-dim">
          {t("settings.engine.extracting", { asset: progress.asset })}
        </p>
      )}

      {/* Builds antigas: o app não as usa, e elas ocupam gigabytes. */}
      {check && check.others.length > 0 && (
        <div className="mt-4 flex flex-wrap items-center gap-3 border-t border-edge pt-3">
          <span className="text-[12px] text-dim">
            {t("settings.engine.leftovers", {
              n: check.others.length,
              size: formatBytes(check.reclaimableBytes),
            })}
          </span>
          <button
            type="button"
            onClick={() => void limpar()}
            disabled={busy}
            className={`rounded-lg border px-2.5 py-1 text-[11px] transition-colors disabled:opacity-50 ${
              pruneArmed
                ? "border-bad text-bad"
                : "border-edge text-dim hover:border-accent hover:text-ink"
            }`}
          >
            {pruneArmed
              ? t("settings.engine.cleanConfirm")
              : t("settings.engine.clean")}
          </button>
          <span className="w-full text-[11px] text-dim">
            {check.others
              .map((o) => `${o.tag} · ${o.variantDir} (${formatBytes(o.sizeBytes)})`)
              .join(" · ")}
          </span>
        </div>
      )}
      {pruned && <p className="mt-2 text-[11px] text-ok">{pruned}</p>}

      {/* A release lá fora é informação, não cobrança: a nossa build é
          promovida à mão depois de teste. */}
      {check?.upstreamNewer && check.upstreamTag && (
        <p className="mt-3 text-[11px] leading-relaxed text-dim">
          {t("settings.engine.upstream", { tag: check.upstreamTag })}
        </p>
      )}

      {error && <p className="mt-3 text-[12px] text-bad">{error}</p>}
    </Card>
  );
}

function Fato({
  termo,
  valor,
  title,
  span,
  mono,
}: {
  termo: string;
  valor: string;
  title?: string;
  span?: boolean;
  mono?: boolean;
}) {
  return (
    <div className={span ? "col-span-2 min-w-0" : "min-w-0"}>
      <dt className="text-dim">{termo}</dt>
      <dd
        className={`truncate ${mono ? "font-mono text-[11px]" : ""}`}
        title={title ?? valor}
      >
        {valor}
      </dd>
    </div>
  );
}
