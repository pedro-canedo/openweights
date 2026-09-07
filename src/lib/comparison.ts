import { invoke, isTauri, listen } from "./tauri";
import { generationStore } from "./generationStore";
import type { ModelProfile } from "./tuning";

export interface Distribution { median: number; min: number; max: number }
export interface ComparisonArm { profile: ModelProfile; genTps: Distribution; promptTps: Distribution; totalMs: Distribution; gpuFreeBytes: number | null }
export interface Comparison { model: string; workload: string; arms: ComparisonArm[]; inconclusive: boolean; applied: boolean }
export interface ComparisonProgress { model: string; arm: number; sample: number }
export const onComparisonProgress = (fn: (p: ComparisonProgress) => void) => listen<ComparisonProgress>("comparison-progress", fn);
export const latestComparison = (model: string) => isTauri ? invoke<Comparison | null>("compare_latest", { model }) : Promise.resolve(null);

interface Operation { model: string; kind: "measure" | "apply" | null; progress: ComparisonProgress | null; error: string }
let operation: Operation = { model: "", kind: null, progress: null, error: "" };
const listeners = new Set<() => void>();
export const comparisonOperation = {
  snapshot: () => operation,
  subscribe: (fn: () => void) => { listeners.add(fn); return () => { listeners.delete(fn); }; },
};
function publish(value: Partial<Operation>) {
  operation = { ...operation, ...value };
  listeners.forEach(fn => fn());
}

// A trava vive fora da tela: navegar para o chat não libera a placa em teste.
export async function runComparison(model: string): Promise<Comparison> {
  const release = generationStore.acquireBenchmark();
  publish({ model, kind: "measure", progress: null, error: "" });
  let unlisten: (() => void) | undefined;
  try {
    if (!isTauri) throw new Error("comparison-desktop-only");
    unlisten = await onComparisonProgress(progress => {
      if (progress.model === model) publish({ progress });
    });
    return await invoke<Comparison>("compare_run", { model });
  } catch (error) {
    publish({ error: String(error) });
    throw error;
  } finally { unlisten?.(); release(); publish({ kind: null }); }
}
export async function applyComparison(model: string, restore = false): Promise<void> {
  const release = generationStore.acquireBenchmark();
  publish({ model, kind: "apply", progress: null, error: "" });
  try { await invoke("compare_apply", { model, restore }); }
  catch (error) { publish({ error: String(error) }); throw error; }
  finally { release(); publish({ kind: null }); }
}
