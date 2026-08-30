import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ensureRuntime,
  getHardwareProfile,
  getRuntimeStatus,
  getSetting,
  onRuntimeEvent,
  setSetting,
} from "../lib/api";
import { invoke, isTauri } from "../lib/tauri";
import { formatBytes } from "../lib/format";
import { navigate } from "../lib/nav";
import type { HardwareProfile, RuntimeEvent, RuntimeState } from "../lib/types";

const row =
  "flex items-center justify-between gap-4 rounded-xl border border-edge bg-panel px-5 py-4";

export default function Settings() {
  const { t, i18n } = useTranslation();
  const [profile, setProfile] = useState<HardwareProfile | null>(null);
  const [paths, setPaths] = useState<{ modelsDir: string } | null>(null);
  const [theme, setTheme] = useState(
    () => localStorage.getItem("theme") ?? "dark",
  );

  useEffect(() => {
    getHardwareProfile().then(setProfile).catch(() => setProfile(null));
    if (isTauri) {
      invoke<{ dataDir: string; modelsDir: string }>("app_paths")
        .then(setPaths)
        .catch(() => {});
    }
  }, []);

  function applyTheme(next: string) {
    setTheme(next);
    localStorage.setItem("theme", next);
    if (next === "light") document.documentElement.dataset.theme = "light";
    else delete document.documentElement.dataset.theme;
  }

  function applyLanguage(lng: string) {
    localStorage.setItem("language", lng);
    i18n.changeLanguage(lng);
  }

  return (
    <div className="mx-auto max-w-4xl px-8 py-8">
      <h1 className="text-xl font-semibold">{t("settings.title")}</h1>

      <div className="mt-6 flex flex-col gap-3">
        <div className={row}>
          <span className="text-sm">{t("settings.theme")}</span>
          <select
            value={theme}
            onChange={(e) => applyTheme(e.target.value)}
            className="rounded-lg border border-edge bg-panel2 px-3 py-1.5 text-sm outline-none"
          >
            <option value="dark">{t("settings.dark")}</option>
            <option value="light">{t("settings.light")}</option>
          </select>
        </div>

        <div className={row}>
          <span className="text-sm">{t("settings.language")}</span>
          <select
            value={i18n.language}
            onChange={(e) => applyLanguage(e.target.value)}
            className="rounded-lg border border-edge bg-panel2 px-3 py-1.5 text-sm outline-none"
          >
            <option value="pt-BR">Português (Brasil)</option>
            <option value="en">English</option>
          </select>
        </div>

        <HfTokenRow />
        <RuntimeRow />
        <HardwareCard profile={profile} modelsDir={paths?.modelsDir} />
      </div>
    </div>
  );
}

function HfTokenRow() {
  const { t } = useTranslation();
  const [token, setToken] = useState("");
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    getSetting("hf_token").then((v) => setToken(v ?? ""));
  }, []);

  async function save() {
    await setSetting("hf_token", token.trim());
    setSaved(true);
    setTimeout(() => setSaved(false), 1500);
  }

  return (
    <div className="rounded-xl border border-edge bg-panel px-5 py-4">
      <div className="text-sm">{t("settings.hfToken")}</div>
      <div className="mt-1 text-[12px] text-dim">{t("settings.hfTokenHint")}</div>
      <div className="mt-3 flex gap-2">
        <input
          type="password"
          value={token}
          onChange={(e) => setToken(e.target.value)}
          placeholder="hf_..."
          className="flex-1 rounded-lg border border-edge bg-panel2 px-3 py-2 text-sm outline-none placeholder:text-dim focus:border-accent"
        />
        <button
          onClick={save}
          className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-white"
        >
          {saved ? "✓" : t("common.save")}
        </button>
      </div>
    </div>
  );
}

function RuntimeRow() {
  const { t } = useTranslation();
  const [runtime, setRuntime] = useState<RuntimeState | null>(null);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<RuntimeEvent | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getRuntimeStatus().then(setRuntime).catch(() => {});
  }, []);

  async function install() {
    setBusy(true);
    setError(null);
    const un = await onRuntimeEvent(setProgress);
    try {
      setRuntime(await ensureRuntime());
    } catch (e) {
      setError(String(e));
    } finally {
      un();
      setBusy(false);
      setProgress(null);
    }
  }

  return (
    <div className="rounded-xl border border-edge bg-panel px-5 py-4">
      <div className="flex items-center justify-between gap-4">
        <div>
          <div className="text-sm">{t("settings.runtime")}</div>
          <div className="mt-1 text-[12px] text-dim">
            {runtime
              ? `${runtime.tag} · ${runtime.variant} · ${
                  runtime.installed
                    ? t("settings.runtimeInstalled")
                    : t("settings.runtimeMissing")
                }`
              : t("common.loading")}
          </div>
        </div>
        <button
          onClick={install}
          disabled={busy}
          className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-white disabled:opacity-50"
        >
          {busy ? t("common.loading") : t("settings.runtimeInstall")}
        </button>
      </div>
      {progress?.kind === "progress" && progress.totalBytes > 0 && (
        <div className="mt-3">
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-panel2">
            <div
              className="h-full rounded-full bg-accent transition-[width]"
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
      {error && <div className="mt-2 text-[12px] text-bad">{error}</div>}
    </div>
  );
}

function HardwareCard({
  profile,
  modelsDir,
}: {
  profile: HardwareProfile | null;
  modelsDir?: string;
}) {
  const { t } = useTranslation();
  return (
    <div className="rounded-xl border border-edge bg-panel px-5 py-4">
      <div className="text-sm font-medium">{t("settings.hardware")}</div>
      {profile ? (
        <div className="mt-3 grid grid-cols-2 gap-x-8 gap-y-1.5 text-[13px] text-dim">
          <span>
            CPU: {profile.cpuName} ({profile.cpuCores} cores
            {profile.avx512 ? ", AVX-512" : profile.avx2 ? ", AVX2" : ""})
          </span>
          <span>RAM: {formatBytes(profile.ramTotalBytes)}</span>
          {profile.gpus.map((g, i) => (
            <span key={i} className="col-span-2">
              GPU: {g.name} — {formatBytes(g.vramTotalBytes)} VRAM
              {g.isIntegrated ? " (integrada)" : ""}
              {g.driverVersion ? ` · driver ${g.driverVersion}` : ""}
            </span>
          ))}
          {profile.gpus.length === 0 && (
            <span className="col-span-2">{t("status.noGpu")}</span>
          )}
          {modelsDir && (
            <span className="col-span-2">
              {t("settings.modelsDir")}: {modelsDir}
            </span>
          )}
          <button
            type="button"
            onClick={() => navigate("server")}
            className="col-span-2 mt-1 text-left text-accent hover:underline"
          >
            {t("settings.clusterLink")}
          </button>
        </div>
      ) : (
        <div className="mt-3 text-[13px] text-dim">{t("common.loading")}</div>
      )}
    </div>
  );
}
