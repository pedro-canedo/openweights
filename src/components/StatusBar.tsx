import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { genStats } from "../lib/llama";
import { formatBytes } from "../lib/format";
import MonitorPopover from "./monitor/MonitorPopover";
import { telemetryStore } from "./monitor/telemetryStore";
import { ClusterChip } from "./server/ClusterPanel";

function Meter({ percent }: { percent: number }) {
  const p = Math.max(0, Math.min(100, percent));
  const color = p > 90 ? "bg-bad" : p > 70 ? "bg-warn" : "bg-accent";
  return (
    <div className="h-1.5 w-16 overflow-hidden rounded-full bg-panel2">
      <div
        className={`h-full rounded-full ${color} transition-[width] duration-700`}
        style={{ width: `${p}%` }}
      />
    </div>
  );
}

function Stat({
  label,
  percent,
  detail,
}: {
  label: string;
  percent: number;
  detail: string;
}) {
  return (
    <div className="flex items-center gap-2">
      <span className="w-9 text-[11px] font-medium text-dim">{label}</span>
      <Meter percent={percent} />
      <span className="min-w-16 text-[11px] tabular-nums text-dim">
        {detail}
      </span>
    </div>
  );
}

export default function StatusBar() {
  const { t } = useTranslation();
  // Assinatura única de telemetria, compartilhada com o popover de monitor.
  const tel = useSyncExternalStore(telemetryStore.subscribe, telemetryStore.get);
  const gen = useSyncExternalStore(genStats.subscribe, genStats.get);
  const [monitorOpen, setMonitorOpen] = useState(false);
  const monitorRef = useRef<HTMLDivElement>(null);

  // Fecha o popover ao clicar fora dele.
  useEffect(() => {
    if (!monitorOpen) return;
    const onDown = (e: MouseEvent) => {
      if (!monitorRef.current?.contains(e.target as Node)) {
        setMonitorOpen(false);
      }
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [monitorOpen]);

  return (
    <footer className="flex h-9 shrink-0 items-center gap-6 border-t border-edge bg-panel px-4">
      {tel ? (
        <>
          <Stat
            label={t("status.cpu")}
            percent={tel.cpuPercent}
            detail={`${tel.cpuPercent.toFixed(0)}%${
              tel.cpuTempC != null ? ` ${tel.cpuTempC.toFixed(0)}°C` : ""
            }`}
          />
          <Stat
            label={t("status.ram")}
            percent={(tel.ramUsedBytes / tel.ramTotalBytes) * 100}
            detail={`${formatBytes(tel.ramUsedBytes)} / ${formatBytes(tel.ramTotalBytes)}`}
          />
          {tel.gpus.map((g, i) => (
            <div key={i} className="flex items-center gap-6">
              {g.utilPercent != null && (
                <Stat
                  label={t("status.gpu")}
                  percent={g.utilPercent}
                  detail={`${g.utilPercent.toFixed(0)}%${
                    i === 0 && tel.gpuTempC != null
                      ? ` ${tel.gpuTempC.toFixed(0)}°C`
                      : ""
                  }`}
                />
              )}
              {g.vramUsedBytes != null && (
                <Stat
                  label={t("status.vram")}
                  percent={(g.vramUsedBytes / g.vramTotalBytes) * 100}
                  detail={`${formatBytes(g.vramUsedBytes)} / ${formatBytes(g.vramTotalBytes)}`}
                />
              )}
            </div>
          ))}
          {tel.diskUsedPct != null && (
            <Stat
              label={t("status.disk")}
              percent={tel.diskUsedPct}
              detail={`${tel.diskUsedPct.toFixed(0)}%${
                tel.diskFreeBytes != null
                  ? ` · ${formatBytes(tel.diskFreeBytes)} ${t("status.free")}`
                  : ""
              }`}
            />
          )}
          {tel.netRxBytesPerSec != null && tel.netTxBytesPerSec != null && (
            <div className="flex items-center gap-2">
              <span className="w-9 text-[11px] font-medium text-dim">
                {t("status.net")}
              </span>
              <span className="min-w-28 text-[11px] tabular-nums text-dim">
                ↓ {formatBytes(tel.netRxBytesPerSec)}/s · ↑{" "}
                {formatBytes(tel.netTxBytesPerSec)}/s
              </span>
            </div>
          )}
        </>
      ) : (
        <span className="text-[11px] text-dim">…</span>
      )}

      <div className="ml-auto flex items-center gap-3">
        <ClusterChip />
        {gen.generating && (
          <span className="flex items-center gap-1.5 text-[11px] tabular-nums text-accent">
            <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-accent" />
            {gen.tokensPerSec != null
              ? `${gen.tokensPerSec.toFixed(1)} ${t("status.tokensPerSec")}`
              : t("chat.generating")}
          </span>
        )}

        <div ref={monitorRef} className="relative">
          <button
            onClick={() => setMonitorOpen((v) => !v)}
            title="Monitor de hardware"
            className={`flex h-6 w-6 items-center justify-center rounded-md transition-colors ${
              monitorOpen
                ? "bg-panel2 text-ink"
                : "text-dim hover:bg-panel2 hover:text-ink"
            }`}
          >
            <svg
              className="h-3.5 w-3.5"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.8"
              strokeLinecap="round"
              strokeLinejoin="round"
              viewBox="0 0 24 24"
            >
              <path d="M3 20h18M4 20V9m5 11V4m5 16v-8m5 8V7" />
            </svg>
          </button>
          {monitorOpen && (
            <div className="absolute right-0 bottom-full z-50 mb-2">
              <MonitorPopover />
            </div>
          )}
        </div>
      </div>
    </footer>
  );
}
