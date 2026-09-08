import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { applyComparison, comparisonOperation, latestComparison, runComparison, type Comparison, type Distribution } from "../../lib/comparison";
import { tuneBenchCancel } from "../../lib/tuning";
import { formatBytes } from "../../lib/format";

export default function ComparisonCard({ model }: { model: string }) {
  const { t } = useTranslation();
  const [result, setResult] = useState<Comparison | null>(null);
  const operation = useSyncExternalStore(comparisonOperation.subscribe, comparisonOperation.snapshot);
  const busy = operation.kind !== null;
  // Medindo, o resultado da rodada ANTERIOR não pode ficar na tela: ele
  // aparecia embaixo de "medição 3 de 3" com um veredito já formado, como se
  // a bateria em curso tivesse terminado e concluído aquilo.
  const medindo = operation.kind === "measure" && operation.model === model;
  const progress = operation.progress;
  const [error, setError] = useState("");
  const epoch = useRef(0);
  useEffect(() => {
    const current = ++epoch.current;
    setResult(null); setError("");
    if (model) void latestComparison(model).then(r => { if (epoch.current === current) setResult(r); }).catch(e => { if (epoch.current === current) setError(String(e)); });
    return () => { epoch.current++; };
  }, [model, busy]);
  async function measure() {
    const current = epoch.current;
    setError("");
    try { const value = await runComparison(model); if (epoch.current === current) setResult(value); }
    catch (e) { if (epoch.current === current) setError(String(e)); }
  }
  async function apply(restore: boolean) {
    const current = epoch.current;
    setError("");
    try { await applyComparison(model, restore); const value = await latestComparison(model); if (epoch.current === current) setResult(value); }
    catch (e) { if (epoch.current === current) setError(String(e)); }
  }
  const distribution = (v: Distribution) => `${v.median.toFixed(1)} (${v.min.toFixed(1)}–${v.max.toFixed(1)})`;
  const button = "rounded-lg border border-edge px-3 py-2 text-sm disabled:opacity-40 hover:bg-panel2";
  const failure = error || (operation.model === model ? operation.error : "");
  const best = result?.arms[result.winner ?? 1] ?? result?.arms[0];
  const codeOf = (text: string) => text.match(/(?:comparison|optimization)-([a-z-]+)/)?.[1];
  const errorKey = failure.includes("engine-busy") ? "busy" : codeOf(failure) ?? "failed";
  // Um aviso é o que decide se vale aplicar. Mostrá-lo como código cru
  // ("optimization-package-unavailable") transfere a tradução para o leitor.
  const warningText = (warning: string) => {
    const key = codeOf(warning);
    return key ? t(`comparison.errors.${key}`, { defaultValue: warning }) : warning;
  };
  return <section className="rounded-xl border border-edge bg-panel p-5">
    <h3 className="text-sm font-medium">{t("comparison.title")}</h3>
    <p className="mt-1 text-xs text-dim">{t("comparison.description")}</p>
    <div className="mt-3 flex flex-wrap gap-2">
      <button className={button} disabled={!model || busy} onClick={() => void measure()}>{t("comparison.measure")}</button>
      {operation.kind === "measure" && <button className={button} onClick={() => void tuneBenchCancel().catch(e => setError(String(e)))}>{t("comparison.cancel")}</button>}
    </div>
    {busy && <p role="status" aria-live="polite" className="mt-2 text-xs text-dim">{progress?.stage ? t(`comparison.${progress.stage}`) : progress ? t("comparison.progress", { arm: progress.arm + 1, sample: progress.sample }) : t("comparison.preparing")}</p>}
    {failure && <div role="alert" className="mt-2 text-sm text-warn">{t(`comparison.errors.${errorKey}`, { defaultValue: t("comparison.errors.failed") })}<details className="text-xs"><summary>{t("comparison.details")}</summary>{failure}</details></div>}
    {result && !medindo && <>
      <p role="status" className="mt-4 text-sm font-medium">{t(result.inconclusive ? "comparison.inconclusive" : "comparison.improved")}</p>
      {best && <p className="mt-1 text-sm text-dim">{t("comparison.readyRates", { prompt: best.promptTps.median.toFixed(1), gen: best.genTps.median.toFixed(1) })}</p>}
      {!!result.warnings?.length && <div className="mt-2 text-xs text-warn">
        <p>{t("comparison.warning")}</p>
        <ul className="mt-1 list-disc pl-4">{result.warnings.map((w, i) => <li key={i}>{warningText(w)}</li>)}</ul>
      </div>}
      <details className="mt-3"><summary className="cursor-pointer text-xs text-dim">{t("comparison.allMeasurements")}</summary>
      <p className="mt-4 text-xs text-dim">{t("comparison.median")}</p>
      <div className="overflow-x-auto"><table className="mt-2 w-full text-left text-xs">
        <thead><tr><th>{t("comparison.metric")}</th>{result.arms.map((a, i) => <th key={i}>{i === 0 ? t("comparison.reference") : t("comparison.resultNumber", { n: i })}<br />{t(a.profile.engine === "moeCache" ? "comparison.fork" : "comparison.official")}</th>)}</tr></thead>
        <tbody>
          {(["genTps", "promptTps", "totalMs"] as const).map(key => <tr key={key}><td className="py-2">{t(`comparison.${key}`)}</td>{result.arms.map((a, i) => <td key={i}>{distribution(a[key])}</td>)}</tr>)}
          <tr><td>{t("comparison.memory")}</td>{result.arms.map((a, i) => <td key={i}>{a.gpuFreeBytes == null ? t("generation.unavailable") : formatBytes(a.gpuFreeBytes)}</td>)}</tr>
          {(["peakRamBytes", "peakVramBytes"] as const).map(key => <tr key={key}><td>{t(key === "peakRamBytes" ? "comparison.peakRam" : "comparison.peakVram")}</td>{result.arms.map((a, i) => <td key={i}>{a[key] == null ? t("generation.unavailable") : formatBytes(a[key])}</td>)}</tr>)}
        </tbody>
      </table></div>
      <p className="mt-2 text-xs text-dim">{t("comparison.memoryNote")}</p>
      <details className="mt-2 text-xs text-dim"><summary>{t("comparison.configurations")}</summary><pre className="overflow-x-auto">{JSON.stringify(result.arms.map(a => ({ profile: a.profile, runtimeIdentity: a.runtimeIdentity })), null, 2)}</pre></details>
      {!!result.warnings?.length && <details className="mt-2 text-xs text-dim"><summary>{t("comparison.details")}</summary><pre className="overflow-x-auto">{result.warnings.join("\n")}</pre></details>}
      </details>
      <div className="mt-3 flex gap-2">
        {result.applied ? <button className={button} disabled={busy} onClick={() => void apply(true)}>{t("comparison.restore")}</button> : <>
          <button className={button} disabled={busy || result.inconclusive} onClick={() => void apply(false)}>{t("comparison.apply")}</button>
          <button className={button} disabled={busy} onClick={() => setResult(null)}>{t("comparison.keep")}</button>
        </>}
      </div>
    </>}
  </section>;
}
