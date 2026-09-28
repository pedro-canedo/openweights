import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { motivoDoDownload, silenciarDownload } from "../lib/downloads";
import {
  ensureRuntime,
  getHardwareProfile,
  getModelQuants,
  getRuntimeStatus,
  listLocalModels,
  onDownloadEvent,
  onRuntimeEvent,
  startDownload,
} from "../lib/api";
import { formatBytes } from "../lib/format";
import { navigate } from "../lib/nav";
import { CANDIDATOS, escolherSugestoes, type Sugestao } from "../lib/primeiroModelo";
import type { DownloadStatus, HardwareProfile, RuntimeEvent } from "../lib/types";
import VerdictBadge from "./discover/VerdictBadge";
import { OwLockup } from "./OpenWeightsLogo";
import { Button } from "./ui/Button";
import { Dialog } from "./ui/Dialog";
import Icon from "./ui/Icon";

type Step = "hidden" | "welcome" | "installing" | "model";

/**
 * Primeira execução, em três passos: o computador (o hardware em linguagem
 * simples), o motor (o llama.cpp, com progresso) e o primeiro modelo (três
 * sugestões que cabem nesta máquina, com "Baixar e conversar"). Aparece
 * quando o motor ainda não está instalado, a menos que a pessoa tenha pulado
 * nesta sessão.
 */
export default function Onboarding() {
  const { t } = useTranslation();
  const [step, setStep] = useState<Step>("hidden");
  const [profile, setProfile] = useState<HardwareProfile | null>(null);
  const [progress, setProgress] = useState<RuntimeEvent | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (sessionStorage.getItem("onboarding_skipped")) return;
    getRuntimeStatus().then(
      (rt) => {
        if (!rt.installed) setStep("welcome");
      },
      () => {},
    );
    getHardwareProfile().then(setProfile, () => {});
  }, []);

  const fechar = useCallback(() => setStep("hidden"), []);

  if (step === "hidden") return null;

  const gpu = profile?.gpus.find((g) => !g.isIntegrated) ?? profile?.gpus[0];
  const vramGb = gpu ? gpu.vramTotalBytes / 2 ** 30 : 0;
  const maxParams = Math.max(1, Math.floor((vramGb - 2) / 0.6));

  async function install() {
    setStep("installing");
    setError(null);
    const un = await onRuntimeEvent(setProgress);
    try {
      await ensureRuntime();
      // Quem já tem modelo na biblioteca não precisa do terceiro passo.
      const modelos = await listLocalModels().catch(() => []);
      setStep(modelos.length === 0 ? "model" : "hidden");
    } catch (e) {
      setError(String(e));
      setStep("welcome");
    } finally {
      un();
      setProgress(null);
    }
  }

  function skip() {
    sessionStorage.setItem("onboarding_skipped", "1");
    setStep("hidden");
  }

  const pct =
    progress?.kind === "progress" && progress.totalBytes > 0
      ? Math.round((progress.receivedBytes / progress.totalBytes) * 100)
      : null;

  const atual = step === "model" ? 3 : step === "installing" || profile ? 2 : 1;

  return (
    <Dialog
      open
      // Esc e clique fora valem "pular" — menos durante a instalação, que
      // não pode ser largada pela metade.
      onClose={step === "model" ? fechar : skip}
      dismissable={step !== "installing"}
      title={t("onboarding.welcome")}
      hideTitle
      className="p-8"
    >
      <div className="flex justify-center">
        <OwLockup className="text-2xl" />
      </div>
      <p className="mt-3 text-center text-sm text-dim">{t("onboarding.tagline")}</p>

      <Passos atual={atual} />

      {step === "model" ? (
        <PrimeiroModelo aoFechar={fechar} />
      ) : (
        <>
          <div className="mt-6 rounded-xl border border-edge bg-panel2 p-4 text-[13px]">
            <div className="font-medium">{t("onboarding.yourMachine")}</div>
            {profile ? (
              <div className="mt-2 flex flex-col gap-1 text-dim">
                <span>
                  {profile.cpuName} · {profile.cpuCores} cores · {formatBytes(profile.ramTotalBytes)}{" "}
                  RAM
                </span>
                {gpu ? (
                  <>
                    <span>
                      {gpu.name}
                      {gpu.vramTotalBytes > 0 && ` · ${formatBytes(gpu.vramTotalBytes)} VRAM`}
                    </span>
                    {/* Sem a VRAM (o driver não respondeu), nada de prometer
                        tamanho: "até ~1B" seria inventado. */}
                    {gpu.vramTotalBytes > 0 && (
                      <span className="text-ok">
                        {t("onboarding.gpuSummary", { size: `${maxParams}B` })}
                      </span>
                    )}
                  </>
                ) : (
                  <span>{t("onboarding.cpuSummary")}</span>
                )}
              </div>
            ) : (
              <p role="status" className="mt-2 text-dim">
                {t("onboarding.detecting")}
              </p>
            )}
          </div>

          <p className="mt-4 text-[13px] leading-relaxed text-dim">{t("onboarding.engineHint")}</p>

          {step === "installing" && (
            <div className="mt-4">
              <div className="text-[13px] text-dim">
                {progress?.kind === "extracting"
                  ? `${t("onboarding.runtimeStep")} (${progress.asset})`
                  : t("onboarding.runtimeStep")}
              </div>
              <div
                role="progressbar"
                aria-label={t("onboarding.runtimeStep")}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={pct ?? undefined}
                className="mt-2 h-1.5 w-full overflow-hidden rounded-full bg-panel2"
              >
                <div
                  className="h-full rounded-full bg-accent transition-[width]"
                  style={{ width: pct != null ? `${pct}%` : "15%" }}
                />
              </div>
              {progress?.kind === "progress" && (
                <div className="mt-1 text-[11px] text-dim">
                  {formatBytes(progress.receivedBytes)} / {formatBytes(progress.totalBytes)}
                </div>
              )}
            </div>
          )}

          {error && (
            <div role="alert" className="mt-4 select-text text-[12px] text-bad">
              {error}
            </div>
          )}

          {step === "welcome" && (
            <div className="mt-6 flex justify-center gap-3">
              <Button variant="ghost" onClick={skip}>
                {t("onboarding.skip")}
              </Button>
              <Button variant="primary" onClick={install} data-autofocus="">
                {error ? t("common.retry") : t("onboarding.installEngine")}
              </Button>
            </div>
          )}
        </>
      )}
    </Dialog>
  );
}

/** Os três passos, com o de agora marcado. */
function Passos({ atual }: { atual: 1 | 2 | 3 }) {
  const { t } = useTranslation();
  const rotulos = [t("onboarding.stepMachine"), t("onboarding.stepEngine"), t("onboarding.stepModel")];
  return (
    <ol className="mt-5 flex flex-wrap items-center justify-center gap-y-1 text-[12px]">
      {rotulos.map((rotulo, i) => {
        const n = i + 1;
        const feito = n < atual;
        const agora = n === atual;
        return (
          <li
            key={rotulo}
            aria-current={agora ? "step" : undefined}
            className={`flex items-center ${agora ? "text-ink" : "text-dim"}`}
          >
            <span
              className={`mr-1.5 flex h-5 w-5 items-center justify-center rounded-full border text-[10px] ${
                feito
                  ? "border-ok text-ok"
                  : agora
                    ? "border-accent-ink text-accent-ink"
                    : "border-edge"
              }`}
            >
              {feito ? <Icon name="check" className="h-3 w-3" /> : n}
            </span>
            {rotulo}
            {n < rotulos.length && <span aria-hidden className="mx-2.5 h-px w-5 bg-edge" />}
          </li>
        );
      })}
    </ol>
  );
}

/**
 * O terceiro passo: três modelos que cabem nesta máquina, cada um na
 * quantização que o advisor recomenda. Ao terminar o download, o Chat abre
 * com ele.
 */
function PrimeiroModelo({ aoFechar }: { aoFechar: () => void }) {
  const { t } = useTranslation();
  const [sugestoes, setSugestoes] = useState<Sugestao[] | null>(null);
  const [baixando, setBaixando] = useState<{
    sugestao: Sugestao;
    id: string;
    status: DownloadStatus | null;
  } | null>(null);
  const [erro, setErro] = useState<string | null>(null);

  useEffect(() => {
    let vivo = true;
    void Promise.all(
      CANDIDATOS.map((candidato) =>
        getModelQuants(candidato.repoId, null).then(
          (v) => ({ candidato, quants: v.quants }),
          () => ({ candidato, quants: null }),
        ),
      ),
    ).then((avaliados) => {
      if (vivo) setSugestoes(escolherSugestoes(avaliados));
    });
    return () => {
      vivo = false;
    };
  }, []);

  const idBaixando = baixando?.id;
  useEffect(() => {
    if (!idBaixando) return;
    let vivo = true;
    let desligar: (() => void) | null = null;
    void onDownloadEvent((e) => {
      if (e.kind !== "update" || e.status.id !== idBaixando) return;
      setBaixando((b) => b && { ...b, status: e.status });
      if (e.status.state === "done") {
        aoFechar();
        navigate("chat", { chatModel: e.status.localName ?? e.status.artifactName });
      } else if (e.status.state === "error") {
        setErro(e.status.error ?? t("onboarding.downloadFailed"));
      }
    }).then((un) => {
      if (vivo) desligar = un;
      else un();
    });
    return () => {
      vivo = false;
      desligar?.();
    };
  }, [idBaixando, aoFechar, t]);

  async function baixar(sugestao: Sugestao) {
    setErro(null);
    try {
      // O primeiro uso cuida do fim sozinho (abre o Chat): nada de aviso.
      silenciarDownload(sugestao.candidato.repoId, sugestao.quant.artifactName);
      const id = await startDownload(sugestao.candidato.repoId, sugestao.quant.artifactName);
      setBaixando({ sugestao, id, status: null });
    } catch (e) {
      setErro(motivoDoDownload(e));
    }
  }

  const irParaDescobrir = () => {
    aoFechar();
    navigate("discover");
  };

  const status = baixando?.status;
  const pct =
    status && status.totalBytes > 0
      ? Math.round((status.receivedBytes / status.totalBytes) * 100)
      : null;

  return (
    <div className="mt-6">
      <p className="text-center text-[12px] text-ok">{t("onboarding.runtimeReady")}</p>
      <h2 className="mt-3 text-base font-semibold text-ink">{t("onboarding.modelTitle")}</h2>
      <p className="mt-1 text-[13px] leading-relaxed text-dim">{t("onboarding.modelHint")}</p>

      {baixando ? (
        <div className="mt-4 rounded-xl border border-edge bg-panel2 p-4">
          <div className="text-[13px] text-ink">
            {t("onboarding.downloading", { name: baixando.sugestao.candidato.nome })}
          </div>
          <div
            role="progressbar"
            aria-label={t("onboarding.downloading", { name: baixando.sugestao.candidato.nome })}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={pct ?? undefined}
            className="mt-2 h-1.5 w-full overflow-hidden rounded-full bg-panel"
          >
            <div
              className="h-full rounded-full bg-accent transition-[width]"
              style={{ width: pct != null ? `${pct}%` : "8%" }}
            />
          </div>
          {status && (
            <div className="mt-1 text-[11px] text-dim">
              {formatBytes(status.receivedBytes)} / {formatBytes(status.totalBytes)}
            </div>
          )}
          <p className="mt-3 text-[12px] leading-relaxed text-dim">
            {t("onboarding.downloadBackground")}
          </p>
        </div>
      ) : sugestoes == null ? (
        <p role="status" className="mt-4 text-[13px] text-dim">
          {t("onboarding.modelLoading")}
        </p>
      ) : sugestoes.length === 0 ? (
        <p className="mt-4 text-[13px] text-dim">{t("onboarding.modelNone")}</p>
      ) : (
        <ul className="mt-4 space-y-2">
          {sugestoes.map((s) => (
            <li
              key={s.candidato.repoId}
              className="flex flex-wrap items-center gap-3 rounded-xl border border-edge bg-panel2 px-4 py-3"
            >
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-2 text-[13px] font-medium text-ink">
                  {s.candidato.nome}
                  {s.candidato.padrao && (
                    <span className="rounded-md bg-accent/15 px-1.5 py-0.5 text-[10.5px] font-normal text-accent-ink">
                      {t("onboarding.startHere")}
                    </span>
                  )}
                </div>
                <div className="mt-1 flex flex-wrap items-center gap-2 text-[11px] text-dim">
                  <span>
                    {s.quant.label} · {formatBytes(s.quant.totalBytes)}
                  </span>
                  <VerdictBadge verdict={s.quant.verdict} />
                </div>
              </div>
              <Button
                variant="primary"
                size="sm"
                aria-label={t("onboarding.downloadAndChatOf", { name: s.candidato.nome })}
                onClick={() => void baixar(s)}
              >
                {t("onboarding.downloadAndChat")}
              </Button>
            </li>
          ))}
        </ul>
      )}

      {erro && (
        <div role="alert" className="mt-3 select-text text-[12px] text-bad">
          {erro}
        </div>
      )}

      <div className="mt-6 flex flex-wrap justify-center gap-3">
        {baixando ? (
          <Button variant="primary" onClick={aoFechar} data-autofocus="">
            {t("onboarding.continueBackground")}
          </Button>
        ) : (
          <>
            <Button variant="ghost" onClick={aoFechar}>
              {t("onboarding.notNow")}
            </Button>
            <Button onClick={irParaDescobrir}>{t("onboarding.pickInDiscover")}</Button>
          </>
        )}
      </div>
    </div>
  );
}
