import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DownloadEvent, DownloadStatus } from "./types";

let emitir: (e: DownloadEvent) => void = () => {};
let listaInicial: DownloadStatus[] = [];
vi.mock("./api", () => ({
  onDownloadEvent: async (f: (e: DownloadEvent) => void) => {
    emitir = f;
    return () => {};
  },
  listDownloads: async () => listaInicial,
}));

const {
  _reiniciarDownloads,
  aoConcluirDownload,
  downloadsStore,
  iniciarDownloads,
  silenciarDownload,
} = await import("./downloads");

function status(artifactName: string, state: DownloadStatus["state"]): DownloadStatus {
  return {
    id: `a/b::${artifactName}`,
    repoId: "a/b",
    artifactName,
    receivedBytes: 0,
    totalBytes: 10,
    bytesPerSec: 0,
    state,
    error: null,
    localName: artifactName,
  };
}

describe("downloads", () => {
  beforeEach(() => {
    _reiniciarDownloads();
    listaInicial = [];
  });

  it("só anuncia o fim de um download que esta sessão viu correr", async () => {
    await iniciarDownloads();
    const prontos: string[] = [];
    aoConcluirDownload((s) => prontos.push(s.artifactName));

    // Já estava no disco: vai direto a "done", sem aviso.
    emitir({ kind: "update", status: status("no-disco.gguf", "done") });
    // Correu nesta sessão: avisa uma vez.
    emitir({ kind: "update", status: status("novo.gguf", "running") });
    emitir({ kind: "update", status: status("novo.gguf", "done") });
    emitir({ kind: "update", status: status("novo.gguf", "done") });
    expect(prontos).toEqual(["novo.gguf"]);
    expect(downloadsStore.get().get("a/b::novo.gguf")?.state).toBe("done");
  });

  it("um download que já corria quando o app abriu também avisa", async () => {
    listaInicial = [status("retomado.gguf", "running")];
    await iniciarDownloads();
    const prontos: string[] = [];
    aoConcluirDownload((s) => prontos.push(s.artifactName));
    emitir({ kind: "update", status: status("retomado.gguf", "done") });
    expect(prontos).toEqual(["retomado.gguf"]);
  });

  it("quem silencia cuida do fim sozinho", async () => {
    await iniciarDownloads();
    const prontos: string[] = [];
    aoConcluirDownload((s) => prontos.push(s.artifactName));
    silenciarDownload("a/b", "decisor.gguf");
    emitir({ kind: "update", status: status("decisor.gguf", "running") });
    emitir({ kind: "update", status: status("decisor.gguf", "done") });
    expect(prontos).toEqual([]);
  });

  it("removido sai do mapa", async () => {
    await iniciarDownloads();
    emitir({ kind: "update", status: status("x.gguf", "paused") });
    emitir({ kind: "removed", id: "a/b::x.gguf" });
    expect(downloadsStore.get().has("a/b::x.gguf")).toBe(false);
  });
});
