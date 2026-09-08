// A barra de status em larguras diferentes, com telemetria fixa.
// Existe porque a reclamação era de layout: rótulos escritos por extenso
// quebravam a barra em duas linhas em telas menores.
import React from "react";
import { createRoot } from "react-dom/client";
import "../src/styles.css";
import "../src/i18n";

(window as any).__TAURI_INTERNALS__ = {
  transformCallback: (fn: unknown) => 1,
  unregisterCallback() {},
  async invoke(cmd: string) {
    if (cmd === "server_live") return null;
    return null;
  },
};

const telemetria = {
  cpuPercent: 15,
  ramUsedBytes: 26 * 2 ** 30,
  ramTotalBytes: 64 * 2 ** 30,
  gpuTempC: 47,
  diskUsedPct: 66,
  diskFreeBytes: 317 * 2 ** 30,
  netRxBytesPerSec: 29 * 2 ** 20,
  netTxBytesPerSec: 161 * 2 ** 10,
  gpus: [
    {
      utilPercent: 26,
      powerW: 46,
      powerLimitW: 370,
      vramUsedBytes: 2.6 * 2 ** 30,
      vramTotalBytes: 24 * 2 ** 30,
    },
  ],
};

const { telemetryStore } =
  await import("../src/components/monitor/telemetryStore");
(telemetryStore as any).set?.(telemetria);
// O store guarda o último valor; sem `set` público, injeta direto.
(telemetryStore as any).get = () => telemetria;

const { default: StatusBar } = await import("../src/components/StatusBar");

const LARGURAS = [1920, 1440, 1100, 900];
createRoot(document.getElementById("root")!).render(
  <div className="bg-[#0b0b0f] p-4">
    {LARGURAS.map((w) => (
      <div key={w} className="mb-6">
        <p className="mb-1 text-[11px] text-white/40">{w}px</p>
        <div
          data-width={w}
          style={{ width: w }}
          className="border border-white/10"
        >
          <StatusBar />
        </div>
      </div>
    ))}
  </div>,
);
