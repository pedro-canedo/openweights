// Histórico de benchmark por GPU: as medições reais deste modelo NESTA
// máquina (machine_key atual — trocar de placa ou driver começa série nova),
// com a variação entre medições comparáveis e o uso real vindo do chat.
//
// O Δ% só aparece quando a comparação é honesta: mesma build do motor e
// nenhum dos dois lados marcado como suspeito de aquecimento — o resto é
// mostrado como "—" com o motivo no title.

import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import Icon from "../ui/Icon";
import { comparisonOperation } from "../../lib/comparison";
import { useProfileRevision } from "../../lib/profileChanges";
import {
  engineBusyReason,
  getHardwareProfile,
  getModelProfile,
} from "../../lib/api";
import { errorMessage } from "../../lib/serverSession";
import { listen } from "../../lib/tauri";
import {
  emptyProfile,
  perfHistory,
  tuneApply,
  tuneBench,
  type BenchProgress,
  type ModelProfile,
  type PerfHistoryDto,
  type PerfRowDto,
} from "../../lib/tuning";
import type { HardwareProfile } from "../../lib/types";

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
  "load-mode": "lm",
  threads: "threads",
  parallel: "par",
};

/// Ordem de relevância do resumo: o que a pessoa reconhece primeiro.
const SUMMARY_PRIORITY = [
  "gpu-layers",
  "n-gpu-layers",
  // Onde moram os especialistas é a decisão que mais muda tok/s num MoE: se
  // ela não couber nas quatro colunas do resumo, a tabela esconde justamente
  // o que a pessoa acabou de mexer.
  "n-cpu-moe",
  "ctx-size",
  "load-mode",
  "flash-attn",
  "cache-type-k",
  "batch-size",
  "ubatch-size",
  "parallel",
];

function shortVal(key: string, value: string): string {
  if (key === "ctx-size") {
    const n = Number(value);
    if (Number.isFinite(n) && n >= 1024 && n % 1024 === 0)
      return `${n / 1024}k`;
  }
  return value;
}

function summaryPairs(summary: Record<string, string>): [string, string][] {
  const entries = Object.entries(summary);
  const rank = (k: string) => {
    const i = SUMMARY_PRIORITY.indexOf(k);
    return i < 0 ? SUMMARY_PRIORITY.length : i;
  };
  return entries.sort(([a], [b]) => rank(a) - rank(b));
}

/** GB/s com uma casa, para caber ao lado do tok/s. */
function gbs(bytesPerSecond: number): string {
  return (bytesPerSecond / 1_000_000_000).toFixed(0);
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
  const [open, setOpen] = useState(true);
  const revision = useProfileRevision(model);
  const operation = useSyncExternalStore(comparisonOperation.subscribe, comparisonOperation.snapshot);
  const [success, setSuccess] = useState(false);
  const epoch = useRef(0);
  useEffect(() => { epoch.current++; setSuccess(false); setAplicando(null); }, [model]);
  const [data, setData] = useState<PerfHistoryDto | null>(null);
  const [loading, setLoading] = useState(false);
  const [medindo, setMedindo] = useState<BenchProgress | null>(null);
  const [aplicando, setAplicando] = useState<number | null>(null);
  const [busyWith, setBusyWith] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [hw, setHw] = useState<HardwareProfile | null>(null);
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
    setData(null);
    setError(null);
    perfHistory(model)
      .then((d) => alive && setData(d))
      .catch((e) => alive && setError(errorMessage(e)))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, [open, model, revision, operation.kind]);

  // O teto de banda é do hardware, não do modelo: carrega uma vez.
  useEffect(() => {
    if (!open || hw) return;
    let alive = true;
    void getHardwareProfile()
      .then((p) => alive && setHw(p))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [open, hw]);

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
    const current = epoch.current;
    medindoLocal.current = true;
    setMedindo({ model, step: 0, total: 1 });
    setBusyWith([]);
    setError(null);
    try {
      // Mede a configuração VIGENTE do modelo — é ela que entra na série.
      const perfil = (await getModelProfile(model)) ?? emptyProfile();
      const r = await tuneBench(model, [perfil], force);
      if (epoch.current !== current) return;
      // Voltar sem nenhum resultado é falha (modelo não carregou, teste
      // abortou) — fingir sucesso esconderia o problema da pessoa.
      if (r.results.length === 0) setError(t("tune.history.benchFailed"));
      const updated = await perfHistory(model);
      if (epoch.current === current) setData(updated);
    } catch (e) {
      if (epoch.current !== current) return;
      const quem = engineBusyReason(e);
      if (quem) setBusyWith(quem);
      else setError(errorMessage(e));
    } finally {
      medindoLocal.current = false;
      if (epoch.current === current) setMedindo(null);
    }
  }

  const rows = data?.rows ?? [];
  const usage = data?.usage ?? [];

  // Alguma das medições tirou pesos da placa? É a condição para o teto de
  // banda do sistema virar o número que manda.
  const temOffload = rows.some((r) => {
    const n = Number(r.profileSummary?.["n-cpu-moe"] ?? 0);
    return Number.isFinite(n) && n > 0;
  });

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
          {/* Watts diferentes: o número vale, mas a causa é outra. Sem este
              aviso, a tela creditaria à configuração um ganho que veio do
              limite de energia. */}
          {r.deltaReason === "powerChanged" && (
            <span
              className="ml-1 text-dim"
              title={t("tune.history.powerChanged")}
            >
              <Icon name="power" className="inline h-3 w-3" />
            </span>
          )}
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

  /** Volta a uma configuração já medida, sem refazer os ajustes à mão. */
  async function aplicar(profile: ModelProfile, indice: number) {
    const current = epoch.current;
    setSuccess(false);
    setAplicando(indice);
    setError(null);
    try {
      const r = await tuneApply(model, profile);
      if (!r.ok) throw new Error(r.error ?? "tune-apply-failed");
      const updated = await perfHistory(model);
      if (epoch.current === current) { setData(updated); setSuccess(true); }
    } catch (e) {
      if (epoch.current === current) setError(engineBusyReason(e) ? t("comparison.errors.busy") : errorMessage(e) || String(e));
    } finally {
      if (epoch.current === current) setAplicando(null);
    }
  }

  const badge = (cls: string, texto: React.ReactNode, title?: string) => (
    <span
      className={`inline-flex items-center gap-1 rounded-full border px-1.5 py-0.5 text-[10px] ${cls}`}
      title={title}
    >
      {texto}
    </span>
  );

  return (
    <div className="rounded-2xl border border-edge bg-panel">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center justify-between px-5 py-3 text-sm"
      >
        <span className="flex items-center gap-3 font-semibold"><span className="rounded-xl bg-panel2 p-2.5 text-dim"><Icon name="history" className="h-5 w-5" /></span>{t("tune.history.title")}</span>
        <span className="text-dim">
          <Icon name={open ? "chevron-down" : "chevron-right"} />
        </span>
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
            {/* De qual modelo é a série. A tabela sempre foi por modelo, mas
                não dizia qual — e um número de geração só significa alguma
                coisa junto do modelo que o produziu. */}
            <span
              className="max-w-64 truncate rounded-full border border-edge px-2 py-0.5 text-[11px] text-dim"
              title={model}
            >
              {model}
            </span>
            <button
              type="button"
              disabled={!model || medindo != null || aplicando != null || operation.kind != null}
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
          {error && <p role="alert" className="mt-3 rounded-xl bg-bad/5 p-3 text-xs text-bad">{error}</p>}
          {success && <p role="status" className="mt-3 flex items-center gap-2 text-xs text-ok"><Icon name="check" />{t("tune.history.applied")}</p>}

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
              <p className="mt-4 text-xs leading-relaxed text-dim">{t("tune.history.applyHelp")}</p>
              <div className="mt-4 space-y-2">
                {rows.map((r, i) => {
                  const cfg = configLabel(r.profileSummary, r.profileKey);
                  const current = !!data?.currentProfileKey && r.profileKey === data.currentProfileKey;
                  const disabled = aplicando != null || medindo != null || operation.kind != null;
                  return <article key={`${r.measuredAt}-${i}`} className={`rounded-xl border p-3 sm:p-4 ${current ? "border-accent/30 bg-accent/5" : "border-edge"}`}>
                    <div className="flex flex-wrap items-center gap-3">
                      <div className="min-w-0 flex-1 basis-56">
                        <div className="flex flex-wrap items-center gap-2 text-[10px] text-dim">
                          <time dateTime={new Date(r.measuredAt).toISOString()}>{new Date(r.measuredAt).toLocaleString(i18n.language, { dateStyle: "short", timeStyle: "short" })}</time>
                          {current && badge("border-accent/30 text-accent", t("tune.history.current"))}
                          {r.suspect && badge("border-warn/20 text-warn", <><Icon name="alert" className="h-3 w-3" />{t("tune.history.suspect")}</>, t("tune.benchSuspect"))}
                        </div>
                        <p className="mt-2 break-words font-mono text-[11px] leading-relaxed" title={cfg.title}>{cfg.text}</p>
                        <div className="mt-2 flex items-center gap-3 text-[10px] text-dim"><span>{t("tune.history.build", { n: r.buildNumber })}</span>{r.powerLimitW != null && <span className="inline-flex items-center gap-1" title={t("tune.history.powerLimit")}><Icon name="power" className="h-3 w-3" />{r.powerLimitW} W</span>}</div>
                      </div>
                      <dl className="flex flex-wrap gap-5 text-xs tabular-nums">
                        <div><dt className="text-[10px] text-dim">{t("comparison.generationLabel")}</dt><dd className="mt-1 text-lg font-semibold">{r.genTps.toFixed(1)} <span className="text-[10px] font-normal text-dim">tok/s</span></dd></div>
                        <div><dt className="text-[10px] text-dim">{t("comparison.readingLabel")}</dt><dd className="mt-1 text-lg">{r.promptTps != null && r.promptTps > 0 ? r.promptTps.toFixed(1) : "—"}</dd></div>
                        <div><dt className="text-[10px] text-dim">{t("tune.history.delta")}</dt><dd className="mt-2">{deltaCell(r)}</dd></div>
                      </dl>
                      {r.profile ? <button type="button" disabled={disabled} onClick={() => void aplicar(r.profile!, i)} title={cfg.title} className="inline-flex items-center gap-2 rounded-xl border border-edge px-3 py-2 text-xs transition-colors hover:border-accent hover:bg-accent/10 focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-40"><Icon name="arrow-right" />{aplicando === i ? t("comparison.applying") : t("tune.history.useThis")}</button> : <span className="max-w-32 text-[10px] text-dim" title={t("tune.history.legacyHelp")}>{t("tune.history.legacy")}</span>}
                    </div>
                    <details className="mt-3 text-[11px] text-dim"><summary className="cursor-pointer">{t("tune.history.config")}</summary><dl className="mt-2 grid grid-cols-1 gap-1 sm:grid-cols-2">{summaryPairs(r.profileSummary ?? {}).map(([key, value]) => <div key={key} className="flex min-w-0 gap-2"><dt className="shrink-0">{key}</dt><dd className="break-all font-mono text-ink">{value}</dd></div>)}</dl></details>
                  </article>;
                })}
              </div>
              <p className="mt-2 text-[11px] leading-relaxed text-dim">
                {t("tune.history.gpuSeries")}
              </p>
              {/* O teto que nenhuma flag move: com parte dos pesos fora da
                  placa, tokens por segundo deixa de ser um número de GPU e
                  vira um número de banda de memória. */}
              {temOffload && hw?.ramBandwidthBytesS != null && (
                <p className="mt-1 text-[11px] leading-relaxed text-dim">
                  {t("tune.history.bandwidth", {
                    ram: gbs(hw.ramBandwidthBytesS),
                    gpu:
                      hw.gpus.find((g) => g.bandwidthBytesS != null)
                        ?.bandwidthBytesS != null
                        ? gbs(
                            hw.gpus.find((g) => g.bandwidthBytesS != null)!
                              .bandwidthBytesS!,
                          )
                        : "?",
                  })}
                </p>
              )}
            </>
          )}

          {usage.length > 0 && (
            <div className="mt-4 border-t border-edge pt-3">
              <div className="text-[12px] font-medium">
                {t("tune.history.usage")}
              </div>
              <div className="mt-2 flex flex-col gap-1.5">
                {usage.map((u) => {
                  const cfg = configLabel(
                    summaryFor(u.profileKey),
                    u.profileKey,
                  );
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
