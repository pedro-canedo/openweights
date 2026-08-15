// Assinatura ÚNICA do evento "telemetry", compartilhada entre os meters da
// StatusBar e os gráficos do popover de monitor. Mantém o último snapshot
// (compatível com useSyncExternalStore) e um histórico de 120 amostras em
// séries alinhadas prontas para o uPlot.

import { onTelemetry } from "../../lib/api";
import type { Telemetry } from "../../lib/types";

export const HISTORY_LEN = 120;

/** Séries alinhadas (mesmo comprimento) para uPlot; tempo em segundos. */
export const series = {
  ts: [] as number[],
  cpuPercent: [] as number[],
  gpuPercent: [] as (number | null)[],
  vramUsedGb: [] as (number | null)[],
};

let latest: Telemetry | null = null;
let vramTotalGb = 0;
let started = false;
const listeners = new Set<() => void>();

function push(t: Telemetry): void {
  latest = t;

  // Multi-GPU: utilização = máximo entre as GPUs; VRAM = soma.
  let util: number | null = null;
  let vramUsed: number | null = null;
  let vramTotal = 0;
  for (const g of t.gpus) {
    if (g.utilPercent != null) util = Math.max(util ?? 0, g.utilPercent);
    if (g.vramUsedBytes != null) vramUsed = (vramUsed ?? 0) + g.vramUsedBytes;
    vramTotal += g.vramTotalBytes;
  }
  vramTotalGb = vramTotal / 2 ** 30;

  series.ts.push(t.tsMs / 1000);
  series.cpuPercent.push(t.cpuPercent);
  series.gpuPercent.push(util);
  series.vramUsedGb.push(vramUsed == null ? null : vramUsed / 2 ** 30);
  if (series.ts.length > HISTORY_LEN) {
    series.ts.shift();
    series.cpuPercent.shift();
    series.gpuPercent.shift();
    series.vramUsedGb.shift();
  }
}

function start(): void {
  if (started) return;
  started = true;
  // A assinatura vive pela vida do app (a StatusBar está sempre montada).
  void onTelemetry((t) => {
    push(t);
    for (const fn of listeners) fn();
  });
}

export const telemetryStore = {
  subscribe(fn: () => void): () => void {
    start();
    listeners.add(fn);
    return () => {
      listeners.delete(fn);
    };
  },
  /** Último snapshot recebido (referência estável entre eventos). */
  get(): Telemetry | null {
    return latest;
  },
  /** VRAM total agregada (GiB) — usada como teto do gráfico de VRAM. */
  getVramTotalGb(): number {
    return vramTotalGb;
  },
};
