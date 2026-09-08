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
let currentProfileKey = "current";
(window as any).__applied = [];
const scenario = new URLSearchParams(location.search).get("scenario");
const distribution = (n: number) => ({
  median: n,
  min: n * 0.99,
  max: n * 1.01,
});
const arm = (fast: boolean) => ({
  profile: { ctx: 32768, engine: fast ? "moeCache" : "official" },
  genTps: distribution(fast ? 28 : 19),
  promptTps: distribution(fast ? 120 : 90),
  totalMs: distribution(fast ? 900 : 1300),
  gpuFreeBytes: 2 ** 30,
  runtimeIdentity: {
    source: fast ? "moeCache" : "official",
    revision: fast ? "b46f7f7a" : "b10441",
    backend: "cuda-13.3",
    platform: "windows",
  },
});
(window as any).__TAURI_INTERNALS__ = {
  transformCallback(fn: (event: unknown) => void) {
    callbacks.set(++id, fn);
    return id;
  },
  unregisterCallback(n: number) {
    callbacks.delete(n);
  },
  async invoke(cmd: string, args: any) {
    if (cmd === "plugin:event|listen") {
      handler = args.handler;
      return handler;
    }
    if (cmd === "plugin:event|unlisten") return;
    if (cmd === "compare_latest") return saved;
    if (cmd === "hardware_profile")
      return { gpus: [], ramBandwidthBytesS: null };
    if (cmd === "perf_history")
      return {
        gpuName: "NVIDIA GeForce RTX 3090",
        currentProfileKey,
        bestProfileKey: "current",
        usage: [],
        rows: [
          {
            measuredAt: 1788879600000,
            genTps: 27.6,
            promptTps: 1005.8,
            profileKey: "current",
            profileSummary: {
              "gpu-layers": "65",
              "ctx-size": "65536",
              "flash-attn": "on",
              "cache-type-k": "q8_0",
            },
            profile: { ctx: 65536, threads: 8, engine: "official" },
            suspect: false,
            buildNumber: 10441,
            powerLimitW: 260,
            deltaPct: -14.5,
            deltaReason: "powerChanged",
          },
          {
            measuredAt: 1788879500000,
            genTps: 33.1,
            promptTps: 1168.7,
            profileKey: "alternative",
            profileSummary: {
              "gpu-layers": "65",
              "ctx-size": "8192",
              "flash-attn": "on",
            },
            profile: { ctx: 8192, threads: 16, engine: "official" },
            suspect: true,
            buildNumber: 10441,
            powerLimitW: 370,
            deltaPct: null,
            deltaReason: "suspect",
          },
          {
            measuredAt: 1788879400000,
            genTps: 32.2,
            promptTps: 1187.4,
            profileKey: "legacy",
            profileSummary: { "ctx-size": "32768" },
            profile: null,
            suspect: false,
            buildNumber: 10441,
            powerLimitW: 370,
            deltaPct: null,
            deltaReason: "first",
          },
        ],
      };
    if (cmd === "tune_apply") {
      (window as any).__applied.push(args);
      if (scenario === "history-busy") throw new Error("engine-busy:external");
      if (scenario === "history-failure")
        return { ok: false, error: "load-failed", profile: null };
      currentProfileKey = args.profile.ctx === 8192 ? "alternative" : "current";
      return { ok: true, profile: args.profile };
    }
    if (cmd === "tune_bench_cancel") {
      cancelled = true;
      return;
    }
    if (cmd === "optimize_run") {
      cancelled = false;
      callbacks.get(handler)?.({
        payload: { model: args.model, arm: 0, sample: 0, stage: "installing" },
      });
      await new Promise((resolve) =>
        setTimeout(resolve, scenario === "cancel" ? 1500 : 150),
      );
      if (cancelled) throw new Error("comparison-cancelled");
      // Sem vencedor, o braço 0 É a configuração atual: nada foi aplicado,
      // e `applied` tem de continuar falso — foi aqui que a tela oferecia
      // "restaurar" para uma mudança que nunca houve.
      saved =
        scenario === "inconclusive"
          ? {
              model: args.model,
              workload: "fixture",
              arms: [arm(false)],
              winner: 0,
              inconclusive: true,
              applied: false,
              warnings: [],
            }
          : {
              model: args.model,
              workload: "fixture",
              arms: [arm(false), arm(true)],
              winner: 1,
              inconclusive: false,
              applied: false,
              warnings:
                scenario === "unavailable"
                  ? ["optimization-package-unavailable"]
                  : [],
            };
      saved.currentArm = 0;
      // Perfil editado à mão depois da medição: não corresponde a braço
      // nenhum, e é o caso em que aplicar por cima apagaria o ajuste.
      if (scenario === "hand-edited") saved.currentArm = null;
      if (scenario === "fast-generation") {
        saved.arms[1].genTps = distribution(143);
        saved.arms[1].totalMs = distribution(1500);
        saved.winner = 0;
        saved.inconclusive = true;
      }
      return saved;
    }
    if (cmd === "compare_apply") {
      if (scenario === "rollback")
        throw new Error("comparison-restore-failed: fixture");
      (window as any).__applied.push(args);
      saved.currentArm = args.restore ? 0 : (args.armIndex ?? saved.winner);
      saved.applied = saved.currentArm > 0;
      return;
    }
    throw new Error(`Unexpected fixture command: ${cmd}`);
  },
};
const { default: ComparisonCard } =
  await import("../src/components/server/ComparisonCard");
const { default: BenchHistoryCard } =
  await import("../src/components/server/BenchHistoryCard");
createRoot(document.getElementById("root")!).render(
  <main className="mx-auto max-w-5xl p-4 sm:p-8">
    {scenario?.startsWith("history") ? (
      <BenchHistoryCard
        model="Qwopus3-27B-Flash-MTP-Q4_K_M.gguf"
        running={true}
      />
    ) : (
      <ComparisonCard model="Qwen test" />
    )}
  </main>,
);
