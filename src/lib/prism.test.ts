import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PrismStatus, RuntimeEvent } from "./types";

// O store conversa com o backend por `./api`; aqui ele é um dublê, e o
// evento `runtime-prism` é entregue à mão.
const backend = vi.hoisted(() => ({
  evento: null as null | ((e: RuntimeEvent) => void),
  status: vi.fn(),
  ensure: vi.fn(),
  cancel: vi.fn(),
  retry: vi.fn(),
}));
vi.mock("./tauri", () => ({ isTauri: true }));
vi.mock("./api", () => ({
  getPrismStatus: backend.status,
  ensurePrism: backend.ensure,
  cancelPrism: backend.cancel,
  retryPrismCuda: backend.retry,
  onPrismEvent: async (h: (e: RuntimeEvent) => void) => {
    backend.evento = h;
    return () => {};
  },
}));

import {
  carregarPrism,
  ehCartaoDoChatAtivo,
  instalarPrism,
  isPrismCancelled,
  isPrismRequired,
  PRISM_CANCELLED,
  prismCardMode,
  prismParaDownload,
  prismStore,
  registrarCartaoDoChat,
  tentarCudaDeNovo,
  variantLabel,
} from "./prism";

describe("isPrismRequired", () => {
  it("recognises the typed error however it arrives", () => {
    expect(isPrismRequired("prism-required")).toBe(true);
    expect(isPrismRequired(new Error("prism-required"))).toBe(true);
    expect(isPrismRequired("Erro: prism-required")).toBe(true);
  });
  it("does not mistake other failures for a missing engine", () => {
    expect(isPrismRequired("HTTP 500: failed to load model")).toBe(false);
    expect(isPrismRequired(new Error("engine-busy:chat"))).toBe(false);
    expect(isPrismRequired(null)).toBe(false);
    expect(isPrismRequired(undefined)).toBe(false);
  });
});

// A máquina do Pedro (Linux, RTX 3090) vinda da 0.24.2: o Vulkan do fork
// instalado, o CUDA 12.8 possível e ainda não baixado.
const CUDA_BYTES = 762_542_805;
function status(over: Partial<PrismStatus>): PrismStatus {
  return {
    tag: "prism-b10709-9a9394a",
    variant: "vulkan",
    installed: true,
    serverExe: "/d/runtimes/prism-b10709-9a9394a/vulkan/llama-server",
    preferred: "cuda128",
    installedVariant: "vulkan",
    downloadBytes: 0,
    diskBytes: null,
    cudaDownloadBytes: CUDA_BYTES,
    cudaFailed: false,
    pq2: false,
    pq2OnCpu: false,
    ...over,
  };
}

describe("prismCardMode", () => {
  it("offers the CUDA version for a PQ2_0 file when only Vulkan is installed", () => {
    const st = status({
      variant: "cuda128",
      installed: false,
      serverExe: null,
      pq2: true,
      downloadBytes: CUDA_BYTES,
      diskBytes: 1_074_957_745,
    });
    expect(prismCardMode(st)).toEqual({
      kind: "install",
      bytes: CUDA_BYTES,
      diskBytes: 1_074_957_745,
      upgradeFrom: "vulkan",
    });
  });

  it("offers the CUDA version too when the installed engine is the CPU build", () => {
    // NVIDIA onde o CUDA e o Vulkan reprovaram; um driver novo invalidou a
    // reprovação do CUDA e o PQ2_0 volta a pedi-lo. O motor ESTÁ instalado.
    const st = status({
      variant: "cuda128",
      installed: false,
      serverExe: null,
      installedVariant: "cpu",
      pq2: true,
      downloadBytes: CUDA_BYTES,
    });
    expect(prismCardMode(st)).toMatchObject({ kind: "install", upgradeFrom: "cpu" });
  });

  it("is the plain install card when nothing is installed", () => {
    const st = status({
      variant: "cuda128",
      installed: false,
      serverExe: null,
      installedVariant: null,
      downloadBytes: CUDA_BYTES,
    });
    expect(prismCardMode(st)).toMatchObject({ kind: "install", upgradeFrom: null });
  });

  it("keeps a PTQ1_0 on the installed Vulkan without asking for anything", () => {
    expect(prismCardMode(status({}))).toEqual({ kind: "hidden" });
  });

  it("warns without blocking when PQ2_0 will run on the CPU", () => {
    const st = status({ pq2: true, pq2OnCpu: true, cudaFailed: true, cudaDownloadBytes: null });
    expect(prismCardMode(st)).toEqual({ kind: "pq2OnCpu", cudaFailed: true });
  });

  it("hides while the backend has not answered", () => {
    expect(prismCardMode(null)).toEqual({ kind: "hidden" });
  });
});

describe("prismParaDownload", () => {
  it("brings the CUDA version along with a PQ2_0 where CUDA is possible", () => {
    expect(prismParaDownload(status({}), "Ternary-Bonsai-2-27B-PQ2_0.gguf")).toEqual({
      bytes: CUDA_BYTES,
      pq2NaCpu: false,
    });
  });

  it("downloads nothing more for a PTQ1_0 when the engine is already here", () => {
    expect(prismParaDownload(status({}), "Ternary-Bonsai-2-27B-PTQ1_0.gguf")).toEqual({
      bytes: null,
      pq2NaCpu: false,
    });
  });

  it("points PQ2_0 to PTQ1_0 where the engine is Vulkan (AMD, old driver, CUDA failed)", () => {
    const amd = status({ preferred: "vulkan", cudaDownloadBytes: null });
    expect(prismParaDownload(amd, "Bonsai-PQ2_0.gguf")).toEqual({ bytes: null, pq2NaCpu: true });
    const nada = status({
      preferred: "vulkan",
      installed: false,
      serverExe: null,
      installedVariant: null,
      downloadBytes: 34_248_149,
      cudaDownloadBytes: null,
    });
    expect(prismParaDownload(nada, "Bonsai-PQ2_0.gguf")).toEqual({
      bytes: 34_248_149,
      pq2NaCpu: true,
    });
  });

  it("does not warn where the engine is CUDA (Windows NVIDIA)", () => {
    const win = status({
      variant: "cuda13",
      preferred: "cuda13",
      installedVariant: "cuda13",
      cudaDownloadBytes: null,
    });
    expect(prismParaDownload(win, "Bonsai-PQ2_0.gguf").pq2NaCpu).toBe(false);
  });
});

describe("variantLabel", () => {
  it("names every variant the backend can send", () => {
    expect(variantLabel("cuda128")).toBe("CUDA 12.8");
    expect(variantLabel("vulkan")).toBe("Vulkan");
    expect(variantLabel("cuda13")).toBe("CUDA 13.3");
    expect(variantLabel("macos-arm64")).toBe("Metal");
  });
});

// Deixa as promessas pendentes (o evento, a releitura) andarem.
const assentar = () => new Promise((r) => setTimeout(r, 0));

describe("prismStore", () => {
  const MODELO = "Ternary-Bonsai-2-27B-PQ2_0.gguf";
  const semCuda = status({
    variant: "cuda128",
    installed: false,
    serverExe: null,
    pq2: true,
    downloadBytes: CUDA_BYTES,
  });
  const comCuda = status({
    variant: "cuda128",
    installedVariant: "cuda128",
    serverExe: "/d/runtimes/prism-b10709-9a9394a/cuda-12.8/llama-server",
    pq2: true,
    cudaDownloadBytes: null,
  });

  beforeEach(() => {
    backend.status.mockReset();
    backend.ensure.mockReset();
    backend.cancel.mockReset();
    backend.retry.mockReset();
  });

  it("re-reads what the screen asked when an install it did not start finishes", async () => {
    backend.status.mockResolvedValue(semCuda);
    await carregarPrism(MODELO);
    await assentar();
    expect(prismStore.get().porModelo[MODELO]?.installed).toBe(false);

    // A instalação que o download do Bonsai disparou em segundo plano.
    backend.evento?.({ kind: "progress", asset: "x", receivedBytes: 1, totalBytes: 2 });
    expect(prismStore.get().installing).toBe(true);
    backend.status.mockResolvedValue(comCuda);
    backend.evento?.({ kind: "ready" });
    await assentar();
    const snap = prismStore.get();
    expect(snap.installing).toBe(false);
    expect(snap.porModelo[MODELO]?.installed).toBe(true);
    expect(snap.state?.installed).toBe(true);
  });

  it("treats a cancelled install as stopped, not as an error", async () => {
    backend.status.mockResolvedValue(semCuda);
    await carregarPrism();
    await assentar();
    backend.evento?.({ kind: "failed", message: PRISM_CANCELLED });
    expect(prismStore.get().error).toBeNull();
    backend.evento?.({ kind: "failed", message: "falha de rede" });
    expect(prismStore.get().error).toBe("falha de rede");

    backend.ensure.mockRejectedValue(PRISM_CANCELLED);
    await expect(instalarPrism()).rejects.toBe(PRISM_CANCELLED);
    expect(prismStore.get().error).toBeNull();
    expect(prismStore.get().installing).toBe(false);
    expect(isPrismCancelled(new Error(PRISM_CANCELLED))).toBe(true);
    expect(isPrismCancelled("prism-required")).toBe(false);
  });

  it("retrying the CUDA version forgets the failure, re-reads, then installs", async () => {
    const ordem: string[] = [];
    backend.retry.mockImplementation(async () => void ordem.push("retry"));
    backend.status.mockImplementation(async () => {
      ordem.push("status");
      return semCuda;
    });
    backend.ensure.mockImplementation(async () => {
      ordem.push("ensure");
      return comCuda;
    });
    await tentarCudaDeNovo();
    expect(ordem[0]).toBe("retry");
    expect(ordem.indexOf("ensure")).toBeGreaterThan(ordem.indexOf("status"));
  });

  it("lets only the most recent chat card announce the engine", () => {
    const a = Symbol("a");
    const b = Symbol("b");
    const tiraA = registrarCartaoDoChat(a);
    const tiraB = registrarCartaoDoChat(b);
    expect(ehCartaoDoChatAtivo(b)).toBe(true);
    expect(ehCartaoDoChatAtivo(a)).toBe(false);
    tiraB();
    expect(ehCartaoDoChatAtivo(a)).toBe(true);
    tiraA();
    expect(ehCartaoDoChatAtivo(a)).toBe(false);
  });
});
