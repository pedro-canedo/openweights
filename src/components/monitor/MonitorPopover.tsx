// Popover de monitoramento de hardware: gráficos uPlot (CPU%, GPU%, VRAM)
// alimentados pela assinatura única de telemetria (telemetryStore).

import { useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { formatBytes } from "../../lib/format";
import TelemetryChart from "./TelemetryChart";
import { telemetryStore } from "./telemetryStore";

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
  kind: "cpu" | "gpu" | "vram";
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

export default function MonitorPopover() {
  const { t } = useTranslation();
  const tel = useLatestTelemetry();

  const gpu = tel?.gpus[0] ?? null;
  let vramUsed: number | null = null;
  let vramTotal = 0;
  let gpuUtil: number | null = null;
  for (const g of tel?.gpus ?? []) {
    if (g.utilPercent != null) gpuUtil = Math.max(gpuUtil ?? 0, g.utilPercent);
    if (g.vramUsedBytes != null) vramUsed = (vramUsed ?? 0) + g.vramUsedBytes;
    vramTotal += g.vramTotalBytes;
  }

  return (
    <div className="w-[340px] rounded-xl border border-edge bg-panel p-3 shadow-2xl">
      {/* Sem chave i18n dedicada para o título do monitor */}
      <div className="mb-2 px-1 text-xs font-semibold">
        Monitor de hardware
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
          </>
        ) : (
          <div className="rounded-lg border border-edge bg-panel2/50 p-3 text-center text-[11px] text-dim">
            {t("status.noGpu")}
          </div>
        )}
      </div>
    </div>
  );
}
