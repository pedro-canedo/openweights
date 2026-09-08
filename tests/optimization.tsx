// Deterministic IPC fixture. Real inference is covered by the Rust GPU test.
import React from "react";
import { createRoot } from "react-dom/client";
import "../src/styles.css";
import "../src/i18n";

const callbacks = new Map<number, (event: unknown) => void>();
let id = 0;
let handler = 0;
let cancelled = false;
let saved: any = null;
const scenario = new URLSearchParams(location.search).get("scenario");
const distribution = (n: number) => ({ median: n, min: n * .99, max: n * 1.01 });
const arm = (fast: boolean) => ({ profile: { ctx: 32768, engine: fast ? "moeCache" : "official" }, genTps: distribution(fast ? 28 : 19), promptTps: distribution(fast ? 120 : 90), totalMs: distribution(fast ? 900 : 1300), gpuFreeBytes: 2 ** 30, runtimeIdentity: { source: fast ? "moeCache" : "official", revision: fast ? "b46f7f7a" : "b10441", backend: "cuda-13.3", platform: "windows" } });
(window as any).__TAURI_INTERNALS__ = {
  transformCallback(fn: (event: unknown) => void) { callbacks.set(++id, fn); return id; },
  unregisterCallback(n: number) { callbacks.delete(n); },
  async invoke(cmd: string, args: any) {
    if (cmd === "plugin:event|listen") { handler = args.handler; return handler; }
    if (cmd === "plugin:event|unlisten") return;
    if (cmd === "compare_latest") return saved;
    if (cmd === "tune_bench_cancel") { cancelled = true; return; }
    if (cmd === "optimize_run") {
      cancelled = false;
      callbacks.get(handler)?.({ payload: { model: args.model, arm: 0, sample: 0, stage: "installing" } });
      await new Promise(resolve => setTimeout(resolve, scenario === "cancel" ? 1500 : 150));
      if (cancelled) throw new Error("comparison-cancelled");
      // Sem vencedor, o braço 0 É a configuração atual: nada foi aplicado,
      // e `applied` tem de continuar falso — foi aqui que a tela oferecia
      // "restaurar" para uma mudança que nunca houve.
      saved = scenario === "inconclusive"
        ? { model: args.model, workload: "fixture", arms: [arm(false)], winner: 0, inconclusive: true, applied: false, warnings: [] }
        : { model: args.model, workload: "fixture", arms: [arm(false), arm(true)], winner: 1, inconclusive: false, applied: false, warnings: scenario === "unavailable" ? ["optimization-package-unavailable"] : [] };
      return saved;
    }
    if (cmd === "compare_apply") {
      if (scenario === "rollback") throw new Error("comparison-restore-failed: fixture");
      saved.applied = !args.restore;
      return;
    }
    throw new Error(`Unexpected fixture command: ${cmd}`);
  },
};
const { default: ComparisonCard } = await import("../src/components/server/ComparisonCard");
createRoot(document.getElementById("root")!).render(<main className="mx-auto max-w-3xl p-8"><ComparisonCard model="Qwen test" /></main>);
