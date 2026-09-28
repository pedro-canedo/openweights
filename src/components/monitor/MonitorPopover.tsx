// Popover de monitoramento de hardware: gráficos uPlot dos últimos 15
// minutos (CPU, GPU, VRAM, energia, temperatura, tokens/s), alimentados pela
// assinatura única de telemetria (telemetryStore). Pode ficar fixado.

import { useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { formatBytes, formatNumber } from "../../lib/format";
import { IconButton } from "../ui/Button";
import TelemetryChart, { type ChartKind } from "./TelemetryChart";
import { series, telemetryStore } from "./telemetryStore";

function useLatestTelemetry() {
  return useSyncExternalStore(telemetryStore.subscribe, telemetryStore.get);
}

function ChartCard({
  label,
  value,
  kind,
}: {
  label: string;
  value: string;
  kind: ChartKind;
}) {
  return (
    <div className="rounded-lg border border-edge bg-panel2/50 p-2">
      <div className="mb-1 flex items-baseline justify-between px-1">
        <span className="text-[11px] font-medium text-dim">{label}</span>
        <span className="text-[11px] tabular-nums">{value}</span>
      </div>
      <TelemetryChart kind={kind} />
    </div>
  );
}

export default function MonitorPopover({
  fixado,
  aoFixar,
  aoFechar,
}: {
  fixado: boolean;
  aoFixar: () => void;
  aoFechar: () => void;
}) {
  const { t } = useTranslation();
  const tel = useLatestTelemetry();

  const gpu = tel?.gpus[0] ?? null;
  let vramUsed: number | null = null;
  let vramTotal = 0;
  let gpuUtil: number | null = null;
  let power: number | null = null;
  let limite = 0;
  for (const g of tel?.gpus ?? []) {
    if (g.utilPercent != null) gpuUtil = Math.max(gpuUtil ?? 0, g.utilPercent);
    if (g.vramUsedBytes != null) vramUsed = (vramUsed ?? 0) + g.vramUsedBytes;
    vramTotal += g.vramTotalBytes;
    if (g.powerW != null) power = (power ?? 0) + g.powerW;
    limite += g.powerLimitW ?? 0;
  }
  // Os gráficos que dependem do driver só aparecem quando ele informa algo.
  const temEnergia = series.powerW.some((v) => v != null);
  const temTemperatura = series.gpuTempC.some((v) => v != null);
  const temTokens = series.tokensPerSec.some((v) => v != null);
  const tps = series.tokensPerSec.at(-1) ?? null;

  return (
    <div className="max-h-[75vh] w-[340px] overflow-y-auto rounded-xl border border-edge bg-panel p-3 shadow-2xl">
      <div className="mb-2 flex items-center gap-1 px-1">
        <span className="text-xs font-semibold">{t("status.monitor")}</span>
        <span className="ml-1 text-[10.5px] text-dim">{t("status.monitorWindow")}</span>
        <span className="ml-auto flex items-center gap-0.5">
          <IconButton
            icon="pin"
            size="sm"
            label={t(fixado ? "status.monitorUnpin" : "status.monitorPin")}
            aria-pressed={fixado}
            onClick={aoFixar}
            className={fixado ? "bg-panel2 text-ink" : ""}
          />
          <IconButton icon="close" size="sm" label={t("status.monitorClose")} onClick={aoFechar} />
        </span>
      </div>
      <div className="flex flex-col gap-2">
        <ChartCard
          label={t("status.cpu")}
          value={tel ? `${tel.cpuPercent.toFixed(0)}%` : "—"}
          kind="cpu"
        />
        {gpu ? (
          <>
            <ChartCard
              label={t("status.gpu")}
              value={gpuUtil != null ? `${gpuUtil.toFixed(0)}%` : "—"}
              kind="gpu"
            />
            <ChartCard
              label={t("status.vram")}
              value={
                vramUsed != null
                  ? `${formatBytes(vramUsed)} / ${formatBytes(vramTotal)}`
                  : formatBytes(vramTotal)
              }
              kind="vram"
            />
            {temEnergia && (
              <ChartCard
                label={t("status.gpuPower")}
                value={
                  power != null
                    ? limite > 0
                      ? `${power} / ${limite} W`
                      : `${power} W`
                    : "—"
                }
                kind="power"
              />
            )}
            {temTemperatura && (
              <ChartCard
                label={t("status.gpuTemperature")}
                value={tel?.gpuTempC != null ? `${tel.gpuTempC.toFixed(0)} °C` : "—"}
                kind="temp"
              />
            )}
          </>
        ) : (
          <div className="rounded-lg border border-edge bg-panel2/50 p-3 text-center text-[11px] text-dim">
            {t("status.noGpu")}
          </div>
        )}
        {temTokens && (
          <ChartCard
            label={t("status.tokensPerSecLabel")}
            value={tps != null ? formatNumber(tps, 1) : "—"}
            kind="tps"
          />
        )}
      </div>
    </div>
  );
}
