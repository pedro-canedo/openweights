// Assinatura ÚNICA do evento "telemetry", compartilhada entre os meters da
// StatusBar e os gráficos do popover de monitor. Mantém o último snapshot
// (compatível com useSyncExternalStore) e os últimos 15 minutos em séries
// alinhadas prontas para o uPlot.

import { onTelemetry } from "../../lib/api";
import type { Telemetry } from "../../lib/types";

/** Quanto o histórico cobre, em segundos: o bastante para ver uma geração
 * longa inteira, ou um ajuste de energia antes e depois. */
export const JANELA_S = 15 * 60;

/** Séries alinhadas (mesmo comprimento) para uPlot; tempo em segundos. */
export const series = {
  ts: [] as number[],
  cpuPercent: [] as number[],
  gpuPercent: [] as (number | null)[],
  vramUsedGb: [] as (number | null)[],
  /** Consumo somado das GPUs, em W. */
  powerW: [] as (number | null)[],
  gpuTempC: [] as (number | null)[],
  /** Tokens por segundo do servidor (a barra de status informa a cada leitura). */
  tokensPerSec: [] as (number | null)[],
};

let latest: Telemetry | null = null;
let vramTotalGb = 0;
let powerLimitW = 0;
let tokensAgora: number | null = null;
let started = false;
const listeners = new Set<() => void>();

function push(t: Telemetry): void {
  latest = t;

  // Multi-GPU: utilização = máximo entre as GPUs; VRAM = soma.
  let util: number | null = null;
  let vramUsed: number | null = null;
  let vramTotal = 0;
  let power: number | null = null;
  let limite = 0;
  for (const g of t.gpus) {
    if (g.utilPercent != null) util = Math.max(util ?? 0, g.utilPercent);
    if (g.vramUsedBytes != null) vramUsed = (vramUsed ?? 0) + g.vramUsedBytes;
    vramTotal += g.vramTotalBytes;
    if (g.powerW != null) power = (power ?? 0) + g.powerW;
    limite += g.powerLimitW ?? 0;
  }
  vramTotalGb = vramTotal / 2 ** 30;
  powerLimitW = limite;

  const agora = t.tsMs / 1000;
  series.ts.push(agora);
  series.cpuPercent.push(t.cpuPercent);
  series.gpuPercent.push(util);
  series.vramUsedGb.push(vramUsed == null ? null : vramUsed / 2 ** 30);
  series.powerW.push(power);
  series.gpuTempC.push(t.gpuTempC ?? null);
  series.tokensPerSec.push(tokensAgora);
  // A janela é por tempo, não por contagem: a frequência do evento varia.
  let velhas = 0;
  while (velhas < series.ts.length && series.ts[velhas] < agora - JANELA_S) velhas++;
  if (velhas > 0) {
    for (const s of Object.values(series)) s.splice(0, velhas);
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
  /** Limite de energia somado das GPUs (W) — o teto do gráfico de energia. */
  getPowerLimitW(): number {
    return powerLimitW;
  },
  /** A barra de status informa os tokens/s a cada leitura do servidor. */
  registrarTokens(tps: number | null): void {
    tokensAgora = tps;
  },
};
