// Gráfico de linha em tempo real com uPlot. A instância é criada UMA vez
// por montagem; as atualizações usam apenas setData (nunca recriar por tick).

import { useEffect, useRef } from "react";
import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";
import { series, telemetryStore } from "./telemetryStore";

export type ChartKind = "cpu" | "gpu" | "vram" | "power" | "temp" | "tps";

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement)
    .getPropertyValue(name)
    .trim();
}

const SERIE: Record<ChartKind, keyof typeof series> = {
  cpu: "cpuPercent",
  gpu: "gpuPercent",
  vram: "vramUsedGb",
  power: "powerW",
  temp: "gpuTempC",
  tps: "tokensPerSec",
};

function buildData(kind: ChartKind): uPlot.AlignedData {
  return [series.ts, series[SERIE[kind]]] as uPlot.AlignedData;
}

/** O teto do eixo: 100 nas porcentagens e na temperatura; nos outros, o maior
 * entre o visto e a régua (VRAM total, limite de energia). */
function teto(kind: ChartKind, max: number | null): number {
  if (kind === "cpu" || kind === "gpu" || kind === "temp") return 100;
  const regua =
    kind === "vram"
      ? telemetryStore.getVramTotalGb()
      : kind === "power"
        ? telemetryStore.getPowerLimitW()
        : 0;
  return Math.max(max ?? 0, regua, 1);
}

export default function TelemetryChart({
  kind,
  width = 296,
  height = 72,
}: {
  kind: ChartKind;
  width?: number;
  height?: number;
}) {
  const hostRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const accent = cssVar("--lr-accent") || "#7b5cff";
    const dim = cssVar("--lr-dim") || "#8b93a5";
    const edge = cssVar("--lr-edge") || "#232a38";

    const isPercent = kind === "cpu" || kind === "gpu" || kind === "temp";
    const range: uPlot.Scale.Range = (_u, _min, max): [number, number] => [0, teto(kind, max)];

    const u = new uPlot(
      {
        width,
        height,
        padding: [6, 8, 0, 0],
        legend: { show: false },
        cursor: { show: false },
        scales: {
          x: { time: false },
          y: { range },
        },
        axes: [
          { show: false },
          {
            scale: "y",
            stroke: dim,
            size: 34,
            gap: 4,
            font: '10px "Segoe UI", system-ui, sans-serif',
            grid: { stroke: edge, width: 1 },
            ticks: { show: false },
            splits: isPercent ? [0, 50, 100] : undefined,
          },
        ],
        series: [
          {},
          {
            scale: "y",
            stroke: accent,
            width: 1.5,
            fill: `${accent}22`,
            spanGaps: true,
            points: { show: false },
          },
        ],
      },
      buildData(kind),
      host,
    );

    const unsubscribe = telemetryStore.subscribe(() => {
      u.setData(buildData(kind));
    });

    return () => {
      unsubscribe();
      u.destroy();
    };
  }, [kind, width, height]);

  return <div ref={hostRef} />;
}
