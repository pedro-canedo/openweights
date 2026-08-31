// Flags globais do processo do servidor — as que valem para todos os
// modelos. O destino de cada uma (argumento de CLI × seção `[*]` do INI) e a
// natureza (switch × com valor) são decididos AQUI, no salvamento, com o
// catálogo em mãos; o boot só reproduz. Guardadas no setting
// `server_extra_flags` como JSON de `GlobalFlag`.

import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { getSetting, setSetting } from "../../lib/api";
import {
  flagsCatalog,
  flagsValidate,
  type EnvVar,
  type FlagCatalog,
  type FlagIssue,
  type FlagSpec,
  type GlobalFlag,
} from "../../lib/flags";
import FlagControl from "../form/FlagControl";
import { Chips } from "../form/controls";

const SETTING = "server_extra_flags";
const SETTING_ENV = "server_env_vars";

// O app é dono da chave de API e do INI por modelo. Uma variável dessas
// vinda daqui trocaria o segredo ou atropelaria a configuração por modelo em
// silêncio — a mesma regra que o backend aplica de novo antes do spawn.
function envGerenciada(key: string): boolean {
  const k = key.trim().toUpperCase();
  return k === "LLAMA_API_KEY" || k.startsWith("LLAMA_ARG_");
}

export default function GlobalFlagsCard({ running }: { running: boolean }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [flags, setFlags] = useState<GlobalFlag[]>([]);
  const [catalog, setCatalog] = useState<FlagCatalog | null>(null);
  const [issues, setIssues] = useState<FlagIssue[]>([]);
  const [search, setSearch] = useState("");
  const [dirty, setDirty] = useState(false);
  const [envs, setEnvs] = useState<EnvVar[]>([]);
  const [envKey, setEnvKey] = useState("");
  const [envValue, setEnvValue] = useState("");
  const [envErro, setEnvErro] = useState<string | null>(null);

  useEffect(() => {
    if (!open || catalog) return;
    void flagsCatalog().then(setCatalog).catch(() => {});
    void getSetting(SETTING).then((raw) => {
      if (!raw) return;
      try {
        setFlags(JSON.parse(raw) as GlobalFlag[]);
      } catch {
        setFlags([]);
      }
    });
    void getSetting(SETTING_ENV).then((raw) => {
      if (!raw) return;
      try {
        setEnvs(JSON.parse(raw) as EnvVar[]);
      } catch {
        setEnvs([]);
      }
    });
  }, [open, catalog]);

  const persistEnv = (next: EnvVar[]) => {
    setEnvs(next);
    setDirty(running);
    void setSetting(SETTING_ENV, JSON.stringify(next));
  };

  const setEnv = (key: string, value: string | null) => {
    const nome = key.trim();
    if (!nome) return;
    if (envGerenciada(nome)) {
      setEnvErro(nome);
      return;
    }
    setEnvErro(null);
    const rest = envs.filter((e) => e.key !== nome);
    persistEnv(value == null ? rest : [...rest, { key: nome, value }]);
  };

  const envValor = (key: string): string | null =>
    envs.find((e) => e.key === key)?.value ?? null;

  const persist = (next: GlobalFlag[]) => {
    setFlags(next);
    setDirty(running);
    void setSetting(SETTING, JSON.stringify(next));
    void flagsValidate(
      "global",
      next.map((f) => [f.key, f.value]),
    )
      .then(setIssues)
      .catch(() => {});
  };

  const setFlag = (spec: FlagSpec, value: string | null) => {
    const rest = flags.filter((f) => f.key !== spec.key);
    if (value == null) {
      persist(rest);
      return;
    }
    persist([
      ...rest,
      {
        key: spec.key,
        value,
        // Global puro = argumento de CLI; `both`/router = seção `[*]`, que a
        // seção do modelo pode vencer (é o contrato de "padrão global").
        place: spec.scope === "global" ? "args" : "ini",
        switch: spec.kind.type === "bool",
      },
    ]);
  };

  const value = (key: string): string | null =>
    flags.find((f) => f.key === key)?.value ?? null;

  const matches: FlagSpec[] = useMemo(() => {
    if (!catalog) return [];
    const q = search.trim().toLowerCase();
    const usable = catalog.flags.filter(
      (f) => f.scope === "global" || f.scope === "both",
    );
    if (!q) return usable.filter((f) => value(f.key) != null);
    return usable
      .filter(
        (f) =>
          f.key.includes(q) || f.aliases.some((a) => a.toLowerCase().includes(q)),
      )
      .slice(0, 30);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [catalog, search, flags]);

  return (
    <div className="mt-4 rounded-xl border border-edge bg-panel">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center justify-between px-5 py-3 text-sm"
      >
        {t("server.globalFlags.title")}
        <span className="text-dim">{open ? "▾" : "▸"}</span>
      </button>
      {open && (
        <div className="border-t border-edge px-5 py-4">
          <p className="text-[11px] leading-relaxed text-dim">
            {t("server.globalFlags.hint")}
          </p>
          {flags.length > 0 && (
            <div className="mt-2 flex flex-wrap items-center gap-1">
              {flags.map((f) => (
                <span
                  key={f.key}
                  className="flex items-center gap-1 rounded-full border border-accent bg-accent/10 px-2 py-0.5 font-mono text-[11px] text-ink"
                >
                  {f.key}
                  {!f.switch && ` = ${f.value}`}
                  <button
                    type="button"
                    onClick={() => persist(flags.filter((x) => x.key !== f.key))}
                    className="text-dim hover:text-bad"
                  >
                    ×
                  </button>
                </span>
              ))}
            </div>
          )}
          {issues.length > 0 && (
            <div className="mt-2 rounded-lg border border-warn/40 bg-warn/10 px-3 py-2">
              {issues.map((i) => (
                <p
                  key={`${i.key}-${i.code}`}
                  className="text-[11px] leading-relaxed text-warn"
                >
                  <span className="font-mono">{i.key}</span>:{" "}
                  {t(`flags.issues.${i.code}`, { detail: i.detail })}
                </p>
              ))}
            </div>
          )}
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t("server.engineConfig.searchPlaceholder")}
            className="mt-3 w-full rounded-lg border border-edge bg-panel2 px-3 py-2 text-sm outline-none placeholder:text-dim focus:border-accent"
          />
          <div className="mt-3 flex flex-col gap-3">
            {matches.map((f) => (
              <div key={f.key} className="rounded-lg border border-edge bg-panel2/50 p-3">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-mono text-xs text-ink">--{f.key}</span>
                  {f.curated && (
                    <span className="text-xs text-dim">
                      {t(`flags.catalog.${f.key}.label`, "")}
                    </span>
                  )}
                  {f.default && (
                    <span className="text-[10px] text-dim">
                      {t("server.engineConfig.default", { v: f.default })}
                    </span>
                  )}
                </div>
                {(f.curated || f.helpText) && (
                  <p className="mt-1 text-[11px] leading-relaxed text-dim">
                    {f.curated ? t(`flags.catalog.${f.key}.hint`, "") : f.helpText}
                  </p>
                )}
                <div className="mt-2">
                  <FlagControl
                    spec={f}
                    value={value(f.key)}
                    onChange={(v) => setFlag(f, v)}
                  />
                </div>
              </div>
            ))}
            {search.trim() && matches.length === 0 && (
              <p className="text-[11px] text-dim">{t("server.engineConfig.noMatches")}</p>
            )}
          </div>

          {/* variáveis de ambiente do processo */}
          <div className="mt-5 border-t border-edge pt-4">
            <span className="text-sm font-medium">{t("server.envVars.title")}</span>
            <p className="mt-1 text-[11px] leading-relaxed text-dim">
              {t("server.envVars.hint")}
            </p>
            {envs.length > 0 && (
              <div className="mt-2 flex flex-wrap items-center gap-1">
                {envs.map((e) => (
                  <span
                    key={e.key}
                    className="flex items-center gap-1 rounded-full border border-accent bg-accent/10 px-2 py-0.5 font-mono text-[11px] text-ink"
                  >
                    {e.key}={e.value}
                    <button
                      type="button"
                      onClick={() => persistEnv(envs.filter((x) => x.key !== e.key))}
                      className="text-dim hover:text-bad"
                    >
                      ×
                    </button>
                  </span>
                ))}
              </div>
            )}
            {envErro && (
              <p className="mt-2 text-[11px] leading-relaxed text-warn">
                {t("server.envVars.managed", { key: envErro })}
              </p>
            )}
            <div className="mt-3 flex flex-col gap-3">
              {(catalog?.envVars ?? []).map((spec) => (
                <div key={spec.key} className="rounded-lg border border-edge bg-panel2/50 p-3">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="font-mono text-xs text-ink">{spec.key}</span>
                    {spec.default && (
                      <span className="text-[10px] text-dim">
                        {t("server.engineConfig.default", { v: spec.default })}
                      </span>
                    )}
                  </div>
                  <p className="mt-1 text-[11px] leading-relaxed text-dim">
                    {t(`flags.env.${spec.key}.hint`, "")}
                  </p>
                  <div className="mt-2">
                    {spec.kind.type === "bool" ? (
                      <Chips
                        value={envValor(spec.key) ?? "auto"}
                        onChange={(v) => setEnv(spec.key, v === "auto" ? null : v)}
                        options={[
                          { id: "auto", label: t("chat.engine.autoPlaceholder") },
                          { id: "1", label: t("server.envVars.on") },
                          { id: "0", label: t("server.envVars.off") },
                        ]}
                      />
                    ) : (
                      <input
                        type="number"
                        value={envValor(spec.key) ?? ""}
                        placeholder={spec.default ?? ""}
                        onChange={(e) => setEnv(spec.key, e.target.value || null)}
                        className="w-32 rounded-lg border border-edge bg-panel2 px-2 py-1.5 font-mono text-xs outline-none placeholder:text-dim focus:border-accent"
                      />
                    )}
                  </div>
                </div>
              ))}
              <div className="flex flex-wrap items-end gap-2">
                <input
                  type="text"
                  value={envKey}
                  onChange={(e) => setEnvKey(e.target.value)}
                  placeholder={t("server.envVars.namePlaceholder")}
                  className="w-56 rounded-lg border border-edge bg-panel2 px-2 py-1.5 font-mono text-xs outline-none placeholder:text-dim focus:border-accent"
                />
                <input
                  type="text"
                  value={envValue}
                  onChange={(e) => setEnvValue(e.target.value)}
                  placeholder={t("server.envVars.valuePlaceholder")}
                  className="w-40 rounded-lg border border-edge bg-panel2 px-2 py-1.5 font-mono text-xs outline-none placeholder:text-dim focus:border-accent"
                />
                <button
                  type="button"
                  disabled={!envKey.trim()}
                  onClick={() => {
                    setEnv(envKey, envValue);
                    if (!envGerenciada(envKey)) {
                      setEnvKey("");
                      setEnvValue("");
                    }
                  }}
                  className="rounded-lg border border-edge px-3 py-1.5 text-xs transition-colors hover:border-accent disabled:opacity-40"
                >
                  {t("server.envVars.add")}
                </button>
              </div>
            </div>
          </div>

          {dirty && (
            <p className="mt-3 text-[11px] text-warn">{t("server.applyHint")}</p>
          )}
        </div>
      )}
    </div>
  );
}
