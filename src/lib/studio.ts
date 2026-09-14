import { invoke, isTauri } from "./tauri";
export const standaloneStudio = document.documentElement.hasAttribute(
  "data-studio-standalone",
);

async function localRequest<T>(path: string, body?: unknown): Promise<T> {
  const response = await fetch(
    path,
    body === undefined
      ? undefined
      : {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(body),
        },
  );
  const result = await response.json();
  if (!response.ok)
    throw new Error(
      result.message ||
        result.detail ||
        "Não foi possível concluir a operação.",
    );
  return result;
}

export async function studioInvoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (!standaloneStudio) return invoke<T>(cmd, args);
  if (cmd === "studio_status") {
    await localRequest("/api/v1/capabilities");
    return { installed: true, running: true } as T;
  }
  if (cmd === "studio_train") {
    const body = args?.body as { comparison_run?: string } | undefined;
    const path = args?.resumeId
      ? `/api/v1/runs/${args.resumeId}/resume`
      : body?.comparison_run
        ? `/api/v1/runs/${body.comparison_run}/compare`
        : "/api/v1/runs";
    return localRequest(path, args?.body ?? {});
  }
  throw new Error("Esta operação está disponível no aplicativo desktop.");
}

export interface StudioDataset {
  id: string;
  name: string;
  kind: string;
  count: number;
  split_counts: Record<string, number>;
  trainable?: boolean;
  next_action?: string;
  warnings?: string[];
}

export interface StudioJob {
  created_at?: string;
  id: string;
  name: string;
  kind: string;
  status: string;
  stage?: string;
  error?: string;
  result?: StudioDataset;
  project_id?: string;
  config?: Record<string, unknown>;
  progress?: { total_steps?: number; training_started_at?: string };
  download_progress?: { received_files: number; total_files: number };
  training_info?: {
    base?: { name?: string; repo?: string };
    evaluation?: { held_out_test?: { test_loss?: number } };
  };
  chat_model?: string;
  checkpoints?: string[];
  can_retry_export?: boolean;
  metrics?: {
    step: number;
    total_steps?: number;
    loss?: number;
    elapsed?: number;
    vram_gb?: number;
  }[];
  log?: string;
}

export interface StudioModel {
  id: string;
  name: string;
  repo: string;
  source: string;
  revision: string;
  downloaded: boolean;
  tested: boolean;
  license: string;
  parameters_b: number;
  download_bytes: number;
}
export interface StudioProject {
  id: string;
  name: string;
  source_ids: string[];
  dataset_id: string | null;
  preparation_id: string | null;
  model_id: string;
  recipe_id: "quick" | "recommended";
  preset: string;
  context: number;
  rank: number;
  learning_rate: number;
  max_minutes: number;
  dataset_versions?: string[];
}
export const finished = (job: StudioJob) =>
  ["completed", "failed", "cancelled", "interrupted"].includes(job.status);

export async function studioRequest<T>(
  path: string,
  body?: unknown,
): Promise<T> {
  if (standaloneStudio) return localRequest<T>(`/api/v1${path}`, body);
  if (!isTauri)
    throw new Error(
      "Abra o OpenWeights desktop para conectar o Studio à GPU local.",
    );
  return invoke<T>("studio_request", {
    path: `/api/v1${path}`,
    method: body === undefined ? "GET" : "POST",
    body: body ?? null,
  });
}

export async function uploadStudioFile(
  file: File,
): Promise<{ id: string; name: string }[]> {
  if (file.size > 32 * 1024 * 1024)
    throw new Error(`${file.name}: escolha um arquivo de até 32 MB.`);
  if (standaloneStudio) {
    const form = new FormData();
    form.append("files", file, file.webkitRelativePath || file.name);
    const response = await fetch("/api/studio/sources", {
      method: "POST",
      body: form,
    });
    const result = await response.json();
    if (!response.ok)
      throw new Error(result.detail || "Não foi possível importar o arquivo.");
    return result;
  }
  return invoke("studio_upload", {
    name: file.webkitRelativePath || file.name,
    bytes: Array.from(new Uint8Array(await file.arrayBuffer())),
  });
}
