import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { applyComparison, comparisonOperation, latestComparison, runComparison, type Comparison } from "../../lib/comparison";
import { tuneBenchCancel } from "../../lib/tuning";
import { useProfileRevision } from "../../lib/profileChanges";
import { formatBytes } from "../../lib/format";
import Icon from "../ui/Icon";

const button = "inline-flex items-center justify-center gap-2 rounded-xl border border-edge px-4 py-2.5 text-xs font-medium transition-colors hover:bg-panel2 focus-visible:outline-2 focus-visible:outline-accent disabled:cursor-not-allowed disabled:opacity-40";

export default function ComparisonCard({ model }: { model: string }) {
  const { t } = useTranslation();
  const [result, setResult] = useState<Comparison | null>(null);
  const [selected, setSelected] = useState(0);
  const [error, setError] = useState("");
  const operation = useSyncExternalStore(comparisonOperation.subscribe, comparisonOperation.snapshot);
  const revision = useProfileRevision(model);
  const busy = operation.kind !== null;
  const ownOperation = operation.model === model;
  const measuring = ownOperation && operation.kind === "measure";
  const modelRef = useRef(model);
  modelRef.current = model;

  useEffect(() => { setResult(null); setError(""); setSelected(0); }, [model]);
  useEffect(() => {
    if (!model || busy) return;
    let alive = true;
    void latestComparison(model).then(value => {
      if (!alive) return;
      setResult(value);
      setSelected(value?.currentArm != null && value.currentArm > 0 ? value.currentArm : value?.winner ?? 0);
    }).catch(e => { if (alive) setError(String(e)); });
    return () => { alive = false; };
  }, [model, busy, revision]);

  async function measure() {
    setError("");
    try {
      const value = await runComparison(model);
      if (modelRef.current === model) { setResult(value); setSelected(value.winner ?? 0); }
    } catch (e) { if (modelRef.current === model) setError(String(e)); }
  }
  async function apply(restore: boolean) {
    setError("");
    try { await applyComparison(model, restore, restore ? undefined : selected); }
    catch (e) { if (modelRef.current === model) setError(String(e)); }
  }
  const failure = error || (ownOperation ? operation.error : "");
  const codeOf = (text: string) => text.match(/(?:comparison|optimization)-([a-z-]+)/)?.[1];
  const errorKey = failure.includes("engine-busy") ? "busy" : codeOf(failure) ?? "failed";
  const warningText = (warning: string) => {
    const key = codeOf(warning);
    return key ? t(`comparison.errors.${key}`, { defaultValue: warning }) : warning;
  };
  const visible = result?.model === model && !measuring ? result : null;
  const current = visible?.currentArm !== undefined ? visible.currentArm : (visible?.applied ? visible.winner : 0);
  const fastest = visible?.arms.reduce((best, arm, i, arms) => arm.genTps.median > arms[best].genTps.median ? i : best, 0);
  const chosen = visible?.arms[selected];
  const progress = ownOperation ? operation.progress : null;

  return <section aria-label={t("comparison.title")} className="rounded-2xl border border-edge bg-panel p-4 sm:p-6">
    <div className="flex flex-wrap items-start justify-between gap-4">
      <div className="flex min-w-0 flex-1 items-start gap-3">
        <span className="rounded-xl bg-accent/10 p-2.5 text-accent"><Icon name="sparkles" className="h-5 w-5" /></span>
        <div className="min-w-0"><h3 className="text-sm font-semibold">{t("comparison.title")}</h3>
          <p className="mt-1 break-all text-xs text-dim">{model || t("comparison.selectModel")}</p>
        </div>
      </div>
      <button className={button} disabled={!model || busy} onClick={() => void measure()}><Icon name="tune" />{t("comparison.measure")}</button>
    </div>
    <p className="mt-4 max-w-3xl text-xs leading-relaxed text-dim">{t("comparison.description")}</p>
    {busy && ownOperation && <div role="status" aria-live="polite" className="mt-4 flex flex-wrap items-center justify-between gap-3 rounded-xl bg-panel2 p-3 text-xs">
      <span className="flex items-center gap-2"><span className="h-2 w-2 animate-pulse rounded-full bg-accent" />{operation.kind === "apply" ? t("comparison.applying") : progress?.stage ? t(`comparison.${progress.stage}`) : progress ? t("comparison.progress", { arm: progress.arm + 1, sample: progress.sample }) : t("comparison.preparing")}</span>
      {measuring && <button className={button} onClick={() => void tuneBenchCancel().catch(e => setError(String(e)))}><Icon name="close" />{t("comparison.cancel")}</button>}
    </div>}
    {failure && <div role="alert" className="mt-4 rounded-xl border border-warn/20 bg-warn/5 p-3 text-xs text-warn">{t(`comparison.errors.${errorKey}`, { defaultValue: t("comparison.errors.failed") })}<details className="mt-2"><summary>{t("comparison.details")}</summary><p className="mt-2 break-words">{failure}</p></details></div>}
    {visible && <div className="mt-5 border-t border-edge pt-5">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h4 role="status" className="text-sm font-medium">{t(visible.applied ? "comparison.activeSuccess" : visible.inconclusive ? "comparison.inconclusive" : "comparison.improved")}</h4>
        <span className="text-xs text-dim">{t("comparison.sampleCount")}</span>
      </div>
      <p className="mt-2 text-xs leading-relaxed text-dim">{t("comparison.selectionHelp")}</p>
      <fieldset className="mt-4 grid min-w-0 gap-3 sm:grid-cols-2 xl:grid-cols-3">
        <legend className="sr-only">{t("comparison.configurations")}</legend>
        {visible.arms.map((arm, i) => <label key={i} className={`relative min-w-0 cursor-pointer rounded-xl border p-4 transition-colors focus-within:ring-2 focus-within:ring-accent ${selected === i ? "border-accent/60 bg-accent/5" : "border-edge hover:bg-panel2"}`}>
          <div className="flex items-center gap-2">
            <input type="radio" name={`comparison-${model}`} className="accent-accent" checked={selected === i} disabled={busy} onChange={() => setSelected(i)} aria-label={i === 0 ? t("comparison.reference") : t("comparison.resultNumber", { n: i })} />
            <span className="text-xs font-medium">{i === 0 ? t("comparison.reference") : t("comparison.resultNumber", { n: i })}</span>
            {current === i && <span className="ml-auto flex items-center gap-1 text-[10px] text-ok"><Icon name="check" />{t("tune.history.current")}</span>}
          </div>
          <div className="mt-3 flex flex-wrap gap-x-3 gap-y-1 text-[10px] text-dim">
            <span>{t(arm.profile.engine === "moeCache" ? "comparison.fork" : "comparison.official")}</span>
            {i === fastest && <span className="text-accent">{t("comparison.fastestGeneration")}</span>}
            {!visible.inconclusive && i === visible.winner && <span className="text-ok">{t("comparison.recommended")}</span>}
          </div>
          <div className="mt-4"><span className="text-3xl font-semibold tracking-tight tabular-nums">{arm.genTps.median.toFixed(1)}</span><span className="ml-2 text-xs text-dim">tok/s</span><p className="mt-1 text-[11px] text-dim">{t("comparison.generationLabel")}</p></div>
          <dl className="mt-4 grid grid-cols-2 gap-3 border-t border-edge pt-3 text-xs">
            <div><dt className="text-[10px] text-dim">{t("comparison.readingLabel")}</dt><dd className="mt-1 tabular-nums">{arm.promptTps.median.toFixed(1)} tok/s</dd></div>
            <div><dt className="text-[10px] text-dim">{t("comparison.durationLabel")}</dt><dd className="mt-1 tabular-nums">{(arm.totalMs.median / 1000).toFixed(2)} s</dd></div>
          </dl>
          <p className="mt-3 text-[10px] leading-relaxed text-dim">{t("comparison.profileSummary", { ctx: arm.profile.ctx ?? "—", threads: arm.profile.threads ?? "auto" })}</p>
        </label>)}
      </fieldset>
      {chosen && selected !== current && <p className="mt-3 text-xs text-dim">{t("comparison.selectedRates", { gen: chosen.genTps.median.toFixed(1), prompt: chosen.promptTps.median.toFixed(1) })}</p>}
      {chosen?.profile.engine === "moeCache" && <p className="mt-3 text-xs text-warn">{t("comparison.optionalEngineNote")}</p>}
      <div className="mt-4 flex flex-wrap items-center gap-2">
        <button className={`${button} border-accent/40 bg-accent/10 text-ink hover:bg-accent/20`} disabled={busy || !chosen || selected === current} onClick={() => void apply(false)}><Icon name="check" />{t("comparison.apply")}</button>
        {visible.applied && <button className={button} disabled={busy} onClick={() => void apply(true)}><Icon name="history" />{t("comparison.restore")}</button>}
      </div>
      {!!visible.warnings?.length && <details className="mt-4 rounded-xl bg-warn/5 p-3 text-xs text-warn"><summary className="cursor-pointer">{t("comparison.warning")}</summary><ul className="mt-2 space-y-1 pl-4 list-disc">{visible.warnings.map((w, i) => <li key={i}>{warningText(w)}</li>)}</ul></details>}
      <details className="mt-4 text-xs text-dim"><summary className="cursor-pointer">{t("comparison.allMeasurements")}</summary>
        <p className="mt-3">{t("comparison.median")}</p>
        <div className="mt-3 overflow-x-auto"><table className="w-full text-left text-xs"><thead><tr><th className="p-2">{t("comparison.metric")}</th>{visible.arms.map((_, i) => <th className="p-2" key={i}>{i === 0 ? t("comparison.reference") : t("comparison.resultNumber", { n: i })}</th>)}</tr></thead><tbody>
          {(["genTps", "promptTps", "totalMs"] as const).map(key => <tr key={key} className="border-t border-edge"><td className="p-2">{t(`comparison.${key}`)}</td>{visible.arms.map((arm, i) => <td className="whitespace-nowrap p-2 tabular-nums" key={i}>{arm[key].median.toFixed(1)} ({arm[key].min.toFixed(1)}–{arm[key].max.toFixed(1)})</td>)}</tr>)}
          {(["peakRamBytes", "peakVramBytes"] as const).map(key => <tr key={key} className="border-t border-edge"><td className="p-2">{t(key === "peakRamBytes" ? "comparison.peakRam" : "comparison.peakVram")}</td>{visible.arms.map((arm, i) => <td className="p-2" key={i}>{arm[key] == null ? t("generation.unavailable") : formatBytes(arm[key])}</td>)}</tr>)}
        </tbody></table></div>
        <p className="mt-2">{t("comparison.memoryNote")}</p>
        <details className="mt-3"><summary>{t("comparison.configurations")}</summary><pre className="mt-2 overflow-x-auto text-[10px]">{JSON.stringify(visible.arms.map(a => ({ profile: a.profile, runtimeIdentity: a.runtimeIdentity })), null, 2)}</pre></details>
      </details>
    </div>}
  </section>;
}
