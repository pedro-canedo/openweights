import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ensureRuntime,
  getHardwareProfile,
  getRuntimeStatus,
  onRuntimeEvent,
} from "../lib/api";
import { formatBytes } from "../lib/format";
import type { HardwareProfile, RuntimeEvent } from "../lib/types";
import { OwLockup } from "./OpenWeightsLogo";
import { Button } from "./ui/Button";
import { Dialog } from "./ui/Dialog";

type Step = "hidden" | "welcome" | "installing" | "ready";

/**
 * Primeira execução: apresenta o hardware detectado em linguagem simples e
 * instala o runtime do llama.cpp com progresso. Aparece quando o runtime
 * ainda não está instalado (a menos que o usuário tenha pulado nesta sessão).
 */
export default function Onboarding() {
  const { t } = useTranslation();
  const [step, setStep] = useState<Step>("hidden");
  const [profile, setProfile] = useState<HardwareProfile | null>(null);
  const [progress, setProgress] = useState<RuntimeEvent | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (sessionStorage.getItem("onboarding_skipped")) return;
    Promise.all([getRuntimeStatus(), getHardwareProfile()]).then(
      ([rt, hw]) => {
        setProfile(hw);
        if (!rt.installed) setStep("welcome");
      },
      () => {},
    );
  }, []);

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
      setStep("ready");
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

  return (
    <Dialog
      open
      // Esc e clique fora valem "pular" — menos durante a instalação, que
      // não pode ser largada pela metade.
      onClose={step === "ready" ? () => setStep("hidden") : skip}
      dismissable={step !== "installing"}
      title={t("onboarding.welcome")}
      hideTitle
      className="p-8"
    >
        <div className="flex justify-center">
          <OwLockup className="text-2xl" />
        </div>
        <p className="mt-3 text-center text-sm text-dim">
          {t("onboarding.tagline")}
        </p>

        {profile && (
          <div className="mt-6 rounded-xl border border-edge bg-panel2 p-4 text-[13px]">
            <div className="font-medium">{t("onboarding.yourMachine")}</div>
            <div className="mt-2 flex flex-col gap-1 text-dim">
              <span>
                {profile.cpuName} · {profile.cpuCores} cores ·{" "}
                {formatBytes(profile.ramTotalBytes)} RAM
              </span>
              {gpu ? (
                <>
                  <span>
                    {gpu.name} · {formatBytes(gpu.vramTotalBytes)} VRAM
                  </span>
                  <span className="text-ok">
                    {t("onboarding.gpuSummary", {
                      size: `${maxParams}B`,
                    })}
                  </span>
                </>
              ) : (
                <span>{t("onboarding.cpuSummary")}</span>
              )}
            </div>
          </div>
        )}

        {step === "installing" && (
          <div className="mt-5">
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
                {formatBytes(progress.receivedBytes)} /{" "}
                {formatBytes(progress.totalBytes)}
              </div>
            )}
          </div>
        )}

        {step === "ready" && (
          <div className="mt-5 text-center text-sm text-ok">
            {t("onboarding.runtimeReady")}
          </div>
        )}

        {error && (
          <div role="alert" className="mt-4 select-text text-[12px] text-bad">
            {error}
          </div>
        )}

        <div className="mt-6 flex justify-center gap-3">
          {step === "welcome" && (
            <>
              <Button variant="ghost" onClick={skip}>
                {t("onboarding.skip")}
              </Button>
              <Button variant="primary" onClick={install} data-autofocus="">
                {t("onboarding.start")}
              </Button>
            </>
          )}
          {step === "ready" && (
            <Button
              variant="primary"
              onClick={() => setStep("hidden")}
              data-autofocus=""
            >
              {t("onboarding.start")}
            </Button>
          )}
        </div>
    </Dialog>
  );
}
