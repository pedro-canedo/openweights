// Ponte com o backend Tauri, com fallback simulado quando aberto
// num navegador comum (npm run dev sem Tauri) para desenvolver a UI.

import type { HardwareProfile, Telemetry } from "./types";

export const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export async function invoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (isTauri) {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<T>(cmd, args);
  }
  return mockInvoke(cmd, args) as Promise<T>;
}

export async function listen<T>(
  event: string,
  handler: (payload: T) => void,
): Promise<() => void> {
  if (isTauri) {
    const { listen } = await import("@tauri-apps/api/event");
    const un = await listen<T>(event, (e) => handler(e.payload));
    return un;
  }
  return mockListen(event, handler as (payload: unknown) => void);
}

// ---------------------------------------------------------------- mocks ---

const mockProfile: HardwareProfile = {
  os: "browser",
  arch: "x64",
  cpuName: "CPU simulada (modo navegador)",
  cpuCores: 8,
  avx2: true,
  avx512: false,
  ramTotalBytes: 32 * 2 ** 30,
  gpus: [
    {
      name: "GPU simulada 16 GB",
      vendor: "nvidia",
      vramTotalBytes: 16 * 2 ** 30,
      isIntegrated: false,
      driverVersion: "580.00",
      cudaCompute: [12, 0],
    },
  ],
};

// Estado simulado do DeepSeek Harness gerenciado: começa não instalado para
// que o CTA do Chat exercite o caminho completo (instalar → subir → abrir).
let dshMock = {
  nodeInstalled: false,
  installed: false,
  running: false,
  port: 0,
  panelUrl: null as string | null,
  version: "",
};

async function mockInvoke(cmd: string, _args?: Record<string, unknown>) {
  switch (cmd) {
    case "hardware_profile":
      return mockProfile;
    case "app_version":
      return "0.1.0-dev";
    case "dsh_status":
      return { ...dshMock };
    case "dsh_install":
      dshMock = {
        ...dshMock,
        nodeInstalled: true,
        installed: true,
        version: "0.1.1-rc.2",
      };
      return { ...dshMock };
    case "dsh_start":
      dshMock = {
        nodeInstalled: true,
        installed: true,
        running: true,
        port: 3080,
        panelUrl: "http://127.0.0.1:3080/",
        version: "0.1.1-rc.2",
      };
      return { ...dshMock };
    case "dsh_open_panel":
      // Navegador: não há janela Tauri para abrir — no-op coerente.
      return undefined;
    case "dsh_stop":
      dshMock = { ...dshMock, running: false, port: 0, panelUrl: null };
      return { ...dshMock };
    case "provider_endpoint":
      // Endpoint local simulado, sem chave — defesa em profundidade para
      // qualquer chamador que chegue aqui fora do Tauri.
      return {
        provider: "local",
        baseUrl: "http://127.0.0.1:11711",
        apiKey: null,
        headers: [],
      };
    default:
      throw new Error(`comando não simulado no navegador: ${cmd}`);
  }
}

function mockListen(event: string, handler: (payload: unknown) => void) {
  if (event !== "telemetry") return () => {};
  let t = 0;
  const timer = setInterval(() => {
    t += 1;
    const wave = (base: number, amp: number, speed: number) =>
      base + amp * (0.5 + 0.5 * Math.sin(t * speed));
    const telemetry: Telemetry = {
      cpuPercent: wave(18, 30, 0.23),
      ramUsedBytes: wave(11, 4, 0.11) * 2 ** 30,
      ramTotalBytes: mockProfile.ramTotalBytes,
      gpus: [
        {
          utilPercent: wave(10, 60, 0.31),
          vramUsedBytes: wave(4, 6, 0.17) * 2 ** 30,
          vramTotalBytes: 16 * 2 ** 30,
        },
      ],
      tsMs: Date.now(),
    };
    handler(telemetry);
  }, 1000);
  return () => clearInterval(timer);
}
