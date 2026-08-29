// Histórico de benchmark por GPU: as medições reais deste modelo NESTA
// máquina (machine_key atual — trocar de placa ou driver começa série nova),
// com a variação entre medições comparáveis e o uso real vindo do chat.
//
// O Δ% só aparece quando a comparação é honesta: mesma build do motor e
// nenhum dos dois lados marcado como suspeito de aquecimento — o resto é
// mostrado como "—" com o motivo no title.

import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { engineBusyReason, getModelProfile } from "../../lib/api";
import { errorMessage } from "../../lib/serverSession";
import { listen } from "../../lib/tauri";
import {
  emptyProfile,
  perfHistory,
  tuneBench,
  type BenchProgress,
  type PerfHistoryDto,
  type PerfRowDto,
} from "../../lib/tuning";

/// Abreviações dos pares do INI que cabem numa célula de tabela.
const SHORT_NAMES: Record<string, string> = {
  "gpu-layers": "ngl",
  "n-gpu-layers": "ngl",
  "ctx-size": "ctx",
  "flash-attn": "fa",
  "cache-type-k": "kvK",
  "cache-type-v": "kvV",
  "batch-size": "batch",
  "ubatch-size": "ubatch",
  "n-cpu-moe": "ncmoe",
  threads: "threads",
  parallel: "par",
};

/// Ordem de relevância do resumo: o que a pessoa reconhece primeiro.
const SUMMARY_PRIORITY = [
  "gpu-layers",
  "n-gpu-layers",
  "ctx-size",
  "flash-attn",
  "cache-type-k",
  "batch-size",
  "ubatch-size",
  "parallel",
];

function shortVal(key: string, value: string): string {
  if (key === "ctx-size") {
    const n = Number(value);
    if (Number.isFinite(n) && n >= 1024 && n % 1024 === 0) return `${n / 1024}k`;
  }
  return value;
}

function summaryPairs(
  summary: Record<string, string>,
): [string, string][] {
  const entries = Object.entries(summary);
  const rank = (k: string) => {
    const i = SUMMARY_PRIORITY.indexOf(k);
    return i < 0 ? SUMMARY_PRIORITY.length : i;
  };
  return entries.sort(([a], [b]) => rank(a) - rank(b));
}

function shortKey(profileKey: string): string {
  return profileKey.length > 8 ? `${profileKey.slice(0, 8)}…` : profileKey;
}

/** Rótulo curto de uma configuração; `title` recebe todos os pares. */
function configLabel(
  summary: Record<string, string> | null,
  profileKey: string,
): { text: string; title: string } {
  if (summary) {
    const pares = summaryPairs(summary);
    const text = pares
      .slice(0, 4)
      .map(([k, v]) => `${SHORT_NAMES[k] ?? k}=${shortVal(k, v)}`)
      .join(" · ");
    const title = pares.map(([k, v]) => `${k}=${v}`).join(" · ");
    return { text: text || "—", title };
  }
  // Linha antiga sem profile_json: só o hash encurtado — nunca inventar.
  return { text: profileKey ? shortKey(profileKey) : "—", title: profileKey };
}

export default function BenchHistoryCard({
  model,
  running,
}: {
  model: string;
  running: boolean;
}) {
  const { t, i18n } = useTranslation();
  const [open, setOpen] = useState(false);
  const [data, setData] = useState<PerfHistoryDto | null>(null);
  const [loading, setLoading] = useState(false);
  const [medindo, setMedindo] = useState<BenchProgress | null>(null);
  const [busyWith, setBusyWith] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  // Só a medição iniciada AQUI mexe no botão: um bench disparado em outra
  // tela (TunePanel) emite os mesmos eventos `tune-bench`, e reagir a eles
  // deixaria "Medir agora" preso num progresso que nunca termina por aqui.
  const medindoLocal = useRef(false);

  // Histórico carregado ao abrir (e recarregado ao trocar de modelo).
  useEffect(() => {
    if (!open || !model) {
      setData(null);
      return;
    }
    let alive = true;
    setLoading(true);
    setError(null);
    perfHistory(model)
      .then((d) => alive && setData(d))
      .catch((e) => alive && setError(errorMessage(e)))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, [open, model]);

  // O progresso vem por evento: a medição leva minutos, e um spinner mudo
  // durante minutos é indistinguível de travamento.
  useEffect(() => {
    // Trocar de modelo zera qualquer progresso exibido — ele era do outro.
    setMedindo(null);
    if (!model) return;
    const parar = listen<BenchProgress>("tune-bench", (p) => {
      if (medindoLocal.current && p.model === model) setMedindo(p);
    });
    return () => {
      void parar.then((f) => f());
    };
  }, [model]);

  async function medir(force = false) {
    if (!model) return;
    medindoLocal.current = true;
    setMedindo({ model, step: 0, total: 1 });
    setBusyWith([]);
    setError(null);
    try {
      // Mede a configuração VIGENTE do modelo — é ela que entra na série.
      const perfil = (await getModelProfile(model)) ?? emptyProfile();
      const r = await tuneBench(model, [perfil], force);
      // Voltar sem nenhum resultado é falha (modelo não carregou, teste
      // abortou) — fingir sucesso esconderia o problema da pessoa.
      if (r.results.length === 0) setError(t("tune.history.benchFailed"));
      setData(await perfHistory(model));
    } catch (e) {
      const quem = engineBusyReason(e);
      if (quem) setBusyWith(quem);
      else setError(errorMessage(e));
    } finally {
      medindoLocal.current = false;
      setMedindo(null);
    }
  }

  const rows = data?.rows ?? [];
  const usage = data?.usage ?? [];

  /// Resumo conhecido de uma chave de perfil, para rotular o uso real.
  const summaryFor = (profileKey: string): Record<string, string> | null =>
    rows.find((r) => r.profileKey === profileKey && r.profileSummary)
      ?.profileSummary ?? null;

  const deltaCell = (r: PerfRowDto) => {
    if (r.deltaPct != null) {
      const cls =
        r.deltaPct > 0 ? "text-ok" : r.deltaPct < 0 ? "text-bad" : "text-dim";
      return (
        <span className={`tabular-nums ${cls}`}>
          {r.deltaPct > 0 ? "+" : ""}
          {r.deltaPct.toFixed(1)}%
        </span>
      );
    }
    const motivo =
      r.deltaReason === "buildChange"
        ? t("tune.history.engineUpdated")
        : t("tune.history.noBaseline");
    return (
      <span className="text-dim" title={motivo}>
        —
      </span>
    );
  };

  const badge = (cls: string, texto: string, title?: string) => (
    <span
      className={`rounded-full border px-1.5 py-0.5 text-[10px] ${cls}`}
      title={title}
    >
      {texto}
    </span>
  );

  return (
    <div className="mt-4 rounded-xl border border-edge bg-panel">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center justify-between px-5 py-3 text-sm"
      >
        {t("tune.history.title")}
        <span className="text-dim">{open ? "▾" : "▸"}</span>
      </button>
      {open && (
        <div className="border-t border-edge px-5 py-4">
          <div className="flex flex-wrap items-center gap-3">
            {/* Antes de carregar não dá para afirmar "CPU": o chip espera o
                histórico chegar para dizer qual série está em jogo. */}
            {data != null && (
              <span className="rounded-full border border-edge px-2 py-0.5 text-[11px] text-dim">
                {data.gpuName ?? "CPU"}
              </span>
            )}
            <button
              type="button"
              disabled={!model || medindo != null}
              onClick={() => void medir()}
              className="rounded-lg border border-edge px-2.5 py-1.5 text-xs text-dim transition-colors hover:border-accent hover:text-ink disabled:opacity-40"
            >
              {medindo
                ? medindo.total > 0 && medindo.step > 0
                  ? t("tune.benchProgress", {
                      step: medindo.step,
                      total: medindo.total,
                    })
                  : t("tune.benchStarting")
                : t("tune.history.measureNow")}
            </button>
            <span
              className={`text-[11px] leading-relaxed ${running ? "text-warn" : "text-dim"}`}
            >
              {t("tune.history.benchWarning")}
            </span>
          </div>

          {busyWith.length > 0 && (
            <p className="mt-2 text-[11px] leading-relaxed text-warn">
              {t("server.busyToApply", {
                who: busyWith.map((w) => t(`server.busyWith.${w}`)).join(", "),
              })}
            </p>
          )}
          {error && <p className="mt-2 text-[12px] text-bad">{error}</p>}

          {loading && (
            <p className="mt-3 text-[11px] text-dim">{t("common.loading")}</p>
          )}

          {!loading && rows.length === 0 && (
            <div className="mt-3">
              <p className="text-[11px] leading-relaxed text-dim">
                {t("tune.history.empty")}
              </p>
              <p className="mt-1 text-[11px] leading-relaxed text-dim">
                {t("tune.history.gpuSeries")}
              </p>
            </div>
          )}

          {rows.length > 0 && (
            <>
              <div className="mt-3 overflow-x-auto">
                <table className="w-full text-left text-[12px]">
                  <thead>
                    <tr className="text-[11px] text-dim">
                      <th className="py-1.5 pr-3 font-normal">
                        {t("tune.history.date")}
                      </th>
                      <th className="py-1.5 pr-3 font-normal">
                        {t("tune.history.config")}
                      </th>
                      <th className="py-1.5 pr-3 font-normal">
                        {t("tune.history.tps")}
                      </th>
                      <th className="py-1.5 pr-3 font-normal">
                        {t("tune.history.promptTps")}
                      </th>
                      <th className="py-1.5 pr-3 font-normal">
                        {t("tune.history.delta")}
                      </th>
                      <th className="py-1.5 font-normal" />
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map((r, i) => {
                      const cfg = configLabel(r.profileSummary, r.profileKey);
                      return (
                        <tr key={`${r.measuredAt}-${i}`} className="border-t border-edge">
                          <td className="whitespace-nowrap py-1.5 pr-3 tabular-nums text-dim">
                            {/* measuredAt já vem em ms (scheduler::now_ms());
                                a data segue o idioma do app, não o do SO. */}
                            {new Date(r.measuredAt).toLocaleDateString(
                              i18n.language,
                            )}
                          </td>
                          <td
                            className="whitespace-nowrap py-1.5 pr-3 font-mono text-[11px]"
                            title={cfg.title}
                          >
                            {cfg.text}
                          </td>
                          <td className="whitespace-nowrap py-1.5 pr-3 tabular-nums">
                            {r.genTps.toFixed(1)}
                          </td>
                          <td className="whitespace-nowrap py-1.5 pr-3 tabular-nums">
                            {/* Linha antiga pode carregar 0.0 gravado —
                                exibir "0.0" mentiria; null e <= 0 viram —. */}
                            {r.promptTps != null && r.promptTps > 0
                              ? r.promptTps.toFixed(1)
                              : "—"}
                          </td>
                          <td className="whitespace-nowrap py-1.5 pr-3">
                            {deltaCell(r)}
                          </td>
                          <td className="py-1.5">
                            <div className="flex flex-wrap items-center gap-1">
                              {data?.bestProfileKey != null &&
                                r.profileKey === data.bestProfileKey &&
                                badge(
                                  "border-warn/40 bg-warn/10 text-warn",
                                  "⭐",
                                  t("tune.history.best"),
                                )}
                              {data != null &&
                                data.currentProfileKey !== "" &&
                                r.profileKey === data.currentProfileKey &&
                                badge(
                                  "border-accent bg-accent/10 text-ink",
                                  t("tune.history.current"),
                                )}
                              {r.suspect &&
                                badge(
                                  "border-warn/40 bg-warn/10 text-warn",
                                  t("tune.history.suspect"),
                                  t("tune.benchSuspect"),
                                )}
                              {badge(
                                "border-edge text-dim",
                                t("tune.history.build", { n: r.buildNumber }),
                              )}
                            </div>
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
              <p className="mt-2 text-[11px] leading-relaxed text-dim">
                {t("tune.history.gpuSeries")}
              </p>
            </>
          )}

          {usage.length > 0 && (
            <div className="mt-4 border-t border-edge pt-3">
              <div className="text-[12px] font-medium">
                {t("tune.history.usage")}
              </div>
              <div className="mt-2 flex flex-col gap-1.5">
                {usage.map((u) => {
                  const cfg = configLabel(summaryFor(u.profileKey), u.profileKey);
                  return (
                    <div
                      key={u.profileKey}
                      className="flex flex-wrap items-center gap-2 text-[12px]"
                    >
                      <span className="font-mono text-[11px]" title={cfg.title}>
                        {cfg.text}
                      </span>
                      <span className="tabular-nums">
                        {u.avgTps.toFixed(1)} tok/s
                      </span>
                      <span className="text-[11px] text-dim">
                        {t("tune.history.usageSamples", { count: u.samples })}
                      </span>
                    </div>
                  );
                })}
              </div>
              <p className="mt-2 text-[11px] leading-relaxed text-dim">
                {t("tune.history.usageNote")}
              </p>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
