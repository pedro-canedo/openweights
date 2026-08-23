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
  type FlagCatalog,
  type FlagIssue,
  type FlagSpec,
  type GlobalFlag,
} from "../../lib/flags";
import FlagControl from "../form/FlagControl";

const SETTING = "server_extra_flags";

export default function GlobalFlagsCard({ running }: { running: boolean }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [flags, setFlags] = useState<GlobalFlag[]>([]);
  const [catalog, setCatalog] = useState<FlagCatalog | null>(null);
  const [issues, setIssues] = useState<FlagIssue[]>([]);
  const [search, setSearch] = useState("");
  const [dirty, setDirty] = useState(false);

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
  }, [open, catalog]);

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
          {dirty && (
            <p className="mt-3 text-[11px] text-warn">{t("server.applyHint")}</p>
          )}
        </div>
      )}
    </div>
  );
}
