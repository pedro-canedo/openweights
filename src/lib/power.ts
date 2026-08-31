// Energia da GPU: leitura pelo NVML, escrita com permissão do sistema.

import { invoke, isTauri } from "./tauri";

export interface GpuPower {
  /** Índice no NVML — o mesmo que o `nvidia-smi -i` espera. */
  index: number;
  name: string;
  /** Limite em vigor agora, em watts. */
  limitW: number;
  /** O que a placa traz de fábrica. */
  defaultW: number | null;
  minW: number | null;
  maxW: number | null;
  /** Consumo instantâneo. */
  usageW: number | null;
}

export function gpuPowerStatus(): Promise<GpuPower[]> {
  if (!isTauri) {
    // Sem Tauri, um exemplo com a forma real — inclusive a placa do vídeo.
    return Promise.resolve([
      {
        index: 0,
        name: "NVIDIA GeForce RTX 3090",
        limitW: 370,
        defaultW: 370,
        minW: 100,
        maxW: 390,
        usageW: 243,
      },
    ]);
  }
  return invoke<GpuPower[]>("gpu_power_status");
}

/**
 * Aplica um limite. O driver só aceita de um processo elevado, então o
 * sistema pede confirmação — e o comando executado volta aqui para a tela
 * poder mostrá-lo: quem autoriza tem direito de saber o que autorizou.
 */
export function gpuPowerSet(index: number, watts: number): Promise<string> {
  if (!isTauri) return Promise.resolve(`nvidia-smi -i ${index} -pl ${watts}`);
  return invoke<string>("gpu_power_set", { index, watts });
}
