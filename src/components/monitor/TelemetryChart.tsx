// Gráfico de linha em tempo real com uPlot. A instância é criada UMA vez
// por montagem; as atualizações usam apenas setData (nunca recriar por tick).

import { useEffect, useRef } from "react";
import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";
import { series, telemetryStore } from "./telemetryStore";

export type ChartKind = "cpu" | "gpu" | "vram";

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement)
    .getPropertyValue(name)
    .trim();
}

function buildData(kind: ChartKind): uPlot.AlignedData {
  const y =
    kind === "cpu"
      ? series.cpuPercent
      : kind === "gpu"
        ? series.gpuPercent
        : series.vramUsedGb;
  return [series.ts, y] as uPlot.AlignedData;
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

    const accent = cssVar("--lr-accent") || "#7c5cff";
    const dim = cssVar("--lr-dim") || "#8b93a5";
    const edge = cssVar("--lr-edge") || "#232a38";

    const isPercent = kind !== "vram";
    const range: uPlot.Scale.Range = isPercent
      ? ([0, 100] as [number, number])
      : (_u, _min, max): [number, number] => [
          0,
          Math.max(max ?? 0, telemetryStore.getVramTotalGb(), 1),
        ];

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
