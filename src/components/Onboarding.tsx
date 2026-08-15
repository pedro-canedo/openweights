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

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
      <div className="w-[480px] rounded-2xl border border-edge bg-panel p-8 shadow-2xl">
        <div className="text-center text-3xl">🦙</div>
        <h2 className="mt-3 text-center text-lg font-semibold">
          {t("onboarding.welcome")}
        </h2>
        <p className="mt-1 text-center text-sm text-dim">
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
            <div className="mt-2 h-1.5 w-full overflow-hidden rounded-full bg-panel2">
              <div
                className="h-full rounded-full bg-accent transition-[width]"
                style={{
                  width:
                    progress?.kind === "progress" && progress.totalBytes > 0
                      ? `${(progress.receivedBytes / progress.totalBytes) * 100}%`
                      : "15%",
                }}
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

        {error && <div className="mt-4 text-[12px] text-bad">{error}</div>}

        <div className="mt-6 flex justify-center gap-3">
          {step === "welcome" && (
            <>
              <button
                onClick={skip}
                className="rounded-lg px-4 py-2 text-sm text-dim hover:text-ink"
              >
                {t("onboarding.skip")}
              </button>
              <button
                onClick={install}
                className="rounded-lg bg-accent px-6 py-2 text-sm font-medium text-white"
              >
                {t("onboarding.start")}
              </button>
            </>
          )}
          {step === "ready" && (
            <button
              onClick={() => setStep("hidden")}
              className="rounded-lg bg-accent px-6 py-2 text-sm font-medium text-white"
            >
              {t("onboarding.start")}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
