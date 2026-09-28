import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import StudioOverview, {
  StudioHistory,
} from "../components/studio/StudioOverview";
import Icon from "../components/ui/Icon";
import { Page } from "../components/ui/Shell";
import { isTauri, listen } from "../lib/tauri";
import { navigate } from "../lib/nav";
import { formatNumber } from "../lib/format";
import {
  finished,
  studioRequest as api,
  studioInvoke as invoke,
  standaloneStudio,
  uploadStudioFile,
  type StudioDataset,
  type StudioJob,
  type StudioModel,
  type StudioProject,
} from "../lib/studio";

/** Estados e etapas que o serviço manda; os que chegarem novos aparecem crus. */
const ROTULADOS = new Set([
  "queued", "preparing", "running", "cancelling", "cancelled", "interrupted",
  "failed", "completed", "downloading", "benchmark", "training", "evaluating",
  "exporting", "ready", "comparing_original", "comparing_trained",
]);
const button =
  "rounded-xl border border-edge px-4 py-2 text-sm disabled:opacity-40";
const primary = `${button} bg-ink text-panel`;
const field = "mt-1 w-full rounded-lg border border-edge bg-panel p-3 text-sm";
const panel = "rounded-2xl border border-edge bg-panel p-6";
const gb = (v: number) => `${formatNumber(v / 1024 ** 3, 1)} GB`;
type Preview = {
  examples: { text?: string; messages?: { role: string; content: string }[] }[];
  manifest: { files?: unknown[] };
};
type Resources = {
  ram_required_bytes: number;
  disk_required_bytes: number;
  vram_estimated_mb: number;
};
type Answers = { original?: { text: string }; trained?: { text: string } };

export default function Studio() {
  const { t } = useTranslation();
  const rotulo = (k: string) => (ROTULADOS.has(k) ? t(`studio.status.${k}`) : k);
  const [installed, setInstalled] = useState<boolean>();
  const [compatible, setCompatible] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [download, setDownload] = useState("");
  const [projects, setProjects] = useState<StudioProject[]>([]);
  const [project, setProject] = useState<StudioProject>();
  const [models, setModels] = useState<StudioModel[]>([]);
  const [datasets, setDatasets] = useState<StudioDataset[]>([]);
  const [history, setHistory] = useState<StudioJob[]>([]);
  const [job, setJob] = useState<StudioJob>();
  const [preview, setPreview] = useState<Preview>();
  const [resources, setResources] = useState<Resources>();
  const [picker, setPicker] = useState(false);
  const [tab, setTab] = useState("catalog");
  const [source, setSource] = useState("hub");
  const [location, setLocation] = useState("");
  const [prompt, setPrompt] = useState("");
  const [pending, setPending] = useState<Record<string, unknown>>();
  const [saved, setSaved] = useState("");
  const files = useRef<HTMLInputElement>(null);
  const folder = useRef<HTMLInputElement>(null);
  const names = useRef<Record<string, string>>({});
  const saves = useRef<Promise<unknown>>(Promise.resolve());
  const lastRequest = useRef<{ key: string; id: string } | undefined>(
    undefined,
  );
  const mounted = useRef(true);
  const active = !!job && !finished(job);
  const dataset = datasets.find((d) => d.id === project?.dataset_id);
  const model = models.find((m) => m.id === project?.model_id);
  const latest = job?.metrics?.at(-1);
  const steps = Math.min(
    latest?.total_steps ?? Infinity,
    job?.progress?.total_steps ?? Infinity,
    Number(job?.config?.max_steps ?? Infinity),
  );
  const percent =
    latest && Number.isFinite(steps)
      ? Math.min(100, (latest.step / steps) * 100)
      : undefined;
  const answers =
    job?.kind === "comparison" && job.status === "completed"
      ? (job.result as unknown as Answers)
      : undefined;

  async function refresh() {
    const caps = await api<{ studio_contract?: number }>("/capabilities");
    setCompatible((caps.studio_contract ?? 1) >= 2);
    if ((caps.studio_contract ?? 1) < 2) return;
    const [ps, ms, ds, runs, sources] = await Promise.all([
      api<StudioProject[]>("/projects"),
      api<StudioModel[]>("/models"),
      api<StudioDataset[]>("/datasets"),
      api<StudioJob[]>("/runs"),
      api<{ id: string; name: string }[]>("/sources"),
    ]);
    if (!mounted.current) return;
    setProjects(ps);
    setModels(ms);
    setDatasets(ds);
    setHistory(runs);
    names.current = Object.fromEntries(sources.map((s) => [s.id, s.name]));
    const running = runs.find((r) => !finished(r));
    if (running) {
      setJob(running);
      setProject(
        ps.find(
          (p) => p.id === running.project_id || p.preparation_id === running.id,
        ),
      );
    }
  }
  useEffect(() => {
    mounted.current = true;
    let unlisten: (() => void) | undefined;
    if (isTauri || standaloneStudio) {
      void invoke<{ installed: boolean }>("studio_status")
        .then(async (s) => {
          if (mounted.current) {
            setInstalled(s.installed);
            if (s.installed) await refresh();
          }
        })
        .catch((e) => setError(String(e)));
      if (isTauri)
        void listen<{ received: number; total: number; stage?: string }>(
          "studio-install",
          (e) => {
            const etapas = ["catalog", "verifying", "extracting", "activating"];
            setDownload(
              e.stage && etapas.includes(e.stage)
                ? t(`studio.install.stage.${e.stage}`)
                : t("studio.install.progress", { received: gb(e.received), total: gb(e.total) }),
            );
          },
        ).then((un) => {
          if (mounted.current) unlisten = un;
          else un();
        });
    } else setInstalled(false);
    return () => {
      mounted.current = false;
      unlisten?.();
    };
  }, []);
  function save(p: StudioProject) {
    setSaved(t("studio.saving"));
    const request = saves.current
      .catch(() => {})
      .then(() => api<StudioProject>("/projects", p));
    saves.current = request;
    return request.then((result) => {
      if (mounted.current) {
        setProjects((ps) => [result, ...ps.filter((v) => v.id !== result.id)]);
        setSaved(t("studio.saved"));
      }
      return result;
    });
  }
  useEffect(() => {
    if (!project || !compatible) return;
    const timer = setTimeout(() => {
      void save(project).catch((e) => {
        setSaved(t("studio.saveFailed"));
        setError(String(e));
      });
    }, 600);
    return () => clearTimeout(timer);
  }, [project, compatible]);
  useEffect(() => {
    setResources(undefined);
    setPreview(undefined);
    setPending(undefined);
  }, [project]);
  useEffect(() => {
    if (!job || finished(job)) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const next = await api<StudioJob>(`/runs/${job.id}`);
        if (cancelled) return;
        setJob(next);
        if (finished(next)) {
          setHistory(await api("/runs"));
          if (
            next.kind === "guided_prepare" &&
            next.status === "completed" &&
            next.result
          ) {
            const ds = next.result;
            setDatasets((list) => [ds, ...list.filter((d) => d.id !== ds.id)]);
            setProject((p) =>
              p?.preparation_id === next.id ? { ...p, dataset_id: ds.id } : p,
            );
          }
          return;
        }
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
      if (!cancelled) timer = setTimeout(poll, 1200);
    };
    timer = setTimeout(poll, 300);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [job?.id, job?.status]);
  async function action(work: () => Promise<void>) {
    setBusy(true);
    setError("");
    try {
      await work();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  function change(update: Partial<StudioProject>) {
    setProject((p) => (p ? { ...p, ...update } : p));
  }
  async function createProject() {
    if (project) await save(project);
    const p: StudioProject = {
      id: crypto.randomUUID().replaceAll("-", ""),
      name: t("studio.defaultName"),
      source_ids: [],
      dataset_id: null,
      preparation_id: null,
      model_id: "qwen3-0.6b",
      recipe_id: "quick",
      preset: "auto",
      context: 1024,
      rank: 16,
      learning_rate: 0.0001,
      max_minutes: 30,
    };
    await save(p);
    setProject(p);
    setJob(undefined);
  }
  async function openProject(p: StudioProject) {
    if (project) await save(project);
    setProject(p);
    const run =
      history.find((r) => r.project_id === p.id) ??
      history.find((r) => r.id === p.preparation_id);
    if (!run) {
      setJob(undefined);
      return;
    }
    const detail = await api<StudioJob>(`/runs/${run.id}`);
    setJob(detail);
    if (
      detail.kind === "guided_prepare" &&
      detail.status === "completed" &&
      detail.result
    )
      setProject({ ...p, dataset_id: detail.result.id });
  }
  async function addFiles(list: File[]) {
    if (!project) return;
    let p = {
      ...project,
      source_ids: [...project.source_ids],
      dataset_id: null,
      preparation_id: null,
    };
    for (const file of list) {
      for (const added of await uploadStudioFile(file)) {
        p.source_ids.push(added.id);
        names.current[added.id] = added.name;
      }
      await save(p);
      setProject({ ...p });
    }
  }
  function runBody() {
    if (!project) throw new Error(t("studio.openProjectFirst"));
    const value = {
      dataset_id: project.dataset_id,
      project_id: project.id,
      model_id: project.model_id,
      recipe_id: project.recipe_id,
      context: project.context,
      rank: project.rank,
      learning_rate: project.learning_rate,
      max_minutes: project.max_minutes,
    };
    const key = JSON.stringify(value);
    if (lastRequest.current?.key !== key)
      lastRequest.current = { key, id: crypto.randomUUID() };
    return { ...value, request_id: lastRequest.current.id };
  }
  async function gpu(args: Record<string, unknown>) {
    try {
      setJob(await invoke<StudioJob>("studio_train", args));
      setPending(undefined);
      lastRequest.current = undefined;
    } catch (e) {
      if (String(e) === "motor_active") setPending(args);
      else throw e;
    }
  }
  return (
    <Page
      wide
      icon="cpu"
      actions={
        installed && compatible ? (
          <button
            className={`${primary} inline-flex items-center gap-2`}
            disabled={active || busy}
            onClick={() => void action(createProject)}
          >
            <Icon name="sparkles" /> {t("studio.newTraining")}
          </button>
        ) : undefined
      }
      title={t("studio.title")}
      subtitle={t("studio.subtitle")}
    >
      {error && (
        <div
          role="alert"
          className="my-5 rounded-xl border border-red-400/40 p-4 text-sm"
        >
          {error}
        </div>
      )}
      {installed === undefined ? (
        <p role="status">{t("studio.checking")}</p>
      ) : !installed ? (
        <section className={`${panel} mt-8`}>
          <h2 className="text-xl font-semibold">{t("studio.install.title")}</h2>
          <p className="my-4 text-dim">{t("studio.install.body")}</p>
          <button
            className={primary}
            disabled={busy || !isTauri}
            onClick={() =>
              void action(async () => {
                await invoke("studio_install");
                setInstalled(true);
                await refresh();
              })
            }
          >
            {busy
              ? download || t("studio.install.connecting")
              : t("studio.install.button")}
          </button>
        </section>
      ) : !compatible ? (
        <section className={panel}>
          <p>{t("studio.incompatible")}</p>
          <button
            className={`${button} mt-4`}
            onClick={() => void action(refresh)}
          >
            {t("studio.checkAgain")}
          </button>
        </section>
      ) : (
        <>
          {!project && !job ? (
            <StudioOverview
              projects={projects}
              models={models}
              runs={history}
              disabled={busy || active}
              onCreate={() => void action(createProject)}
              onProject={(p) => void action(() => openProject(p))}
              onRun={(run) =>
                void action(async () => setJob(await api(`/runs/${run.id}`)))
              }
            />
          ) : (
            <button
              className={`${button} mb-6 inline-flex items-center gap-2 hover:bg-panel2`}
              disabled={active || busy}
              onClick={() =>
                void action(async () => {
                  if (project) await save(project);
                  setProject(undefined);
                  setJob(undefined);
                })
              }
            >
              <Icon name="arrow-right" className="h-4 w-4 rotate-180" />{" "}
              {t("studio.back")}
            </button>
          )}
          {project && (
            <>
              <label className="mb-4 block text-sm">
                {t("studio.projectName")}
                <input
                  className={field}
                  maxLength={100}
                  disabled={active || busy}
                  value={project.name}
                  onChange={(e) => change({ name: e.target.value })}
                />
              </label>
              <p role="status" className="mb-4 text-xs text-dim">
                {saved}
              </p>
              <ol
                className="studio-steps mb-6 grid grid-cols-3 gap-2 text-sm"
                aria-label={t("studio.steps.label")}
              >
                {[t("studio.steps.data"), t("studio.steps.training"), t("studio.steps.result")].map((title, i) => (
                  <li
                    key={title}
                    aria-current={
                      (job?.chat_model || answers ? 3 : dataset ? 2 : 1) ===
                      i + 1
                        ? "step"
                        : undefined
                    }
                  >
                    {i + 1}. {title}
                  </li>
                ))}
              </ol>
              <div className="grid gap-5 lg:grid-cols-2">
                <section className={panel}>
                  <h2 className="font-semibold">{t("studio.data.title")}</h2>
                  <div
                    className="my-4 rounded-xl border-2 border-dashed border-edge p-6 text-center"
                    onDragOver={(e) => e.preventDefault()}
                    onDrop={(e) => {
                      e.preventDefault();
                      if (!busy && !active)
                        void action(() =>
                          addFiles(Array.from(e.dataTransfer.files)),
                        );
                    }}
                  >
                    <p>{t("studio.data.drop")}</p>
                    <p className="my-2 text-xs text-dim">{t("studio.data.limit")}</p>
                    <button
                      className={button}
                      disabled={active || busy}
                      onClick={() => files.current?.click()}
                    >
                      {t("studio.data.chooseFiles")}
                    </button>{" "}
                    <button
                      className={button}
                      disabled={active || busy}
                      onClick={() => folder.current?.click()}
                    >
                      {t("studio.data.chooseFolder")}
                    </button>
                    <input
                      ref={files}
                      type="file"
                      multiple
                      hidden
                      onChange={(e) => {
                        const list = Array.from(e.target.files ?? []);
                        e.target.value = "";
                        void action(() => addFiles(list));
                      }}
                    />
                    <input
                      ref={folder}
                      type="file"
                      multiple
                      hidden
                      {...{ webkitdirectory: "" }}
                      onChange={(e) => {
                        const list = Array.from(e.target.files ?? []);
                        e.target.value = "";
                        void action(() => addFiles(list));
                      }}
                    />
                  </div>
                  <ul className="mb-4 max-h-36 space-y-2 overflow-auto">
                    {project.source_ids.map((id, i) => (
                      <li
                        key={id}
                        className="flex justify-between gap-3 text-sm"
                      >
                        <span className="truncate">
                          {names.current[id] || t("studio.data.file", { n: i + 1 })}
                        </span>
                        <button
                          className="underline"
                          disabled={active || busy}
                          onClick={() =>
                            change({
                              source_ids: project.source_ids.filter(
                                (v) => v !== id,
                              ),
                              dataset_id: null,
                              preparation_id: null,
                            })
                          }
                        >
                          {t("studio.data.remove")}
                        </button>
                      </li>
                    ))}
                  </ul>
                  <label className="block text-sm">
                    {t("studio.data.content")}
                    <select
                      className={field}
                      disabled={active || busy}
                      value={project.preset}
                      onChange={(e) =>
                        change({ preset: e.target.value, dataset_id: null })
                      }
                    >
                      <option value="auto">{t("studio.data.contentAuto")}</option>
                      <option value="documents">{t("studio.data.contentBook")}</option>
                      <option value="conversations">{t("studio.data.contentChats")}</option>
                      <option value="code">{t("studio.data.contentCode")}</option>
                    </select>
                  </label>
                  <button
                    className={`${primary} mt-4`}
                    disabled={
                      active ||
                      busy ||
                      !project.name.trim() ||
                      !project.source_ids.length
                    }
                    onClick={() =>
                      void action(async () => {
                        await save(project);
                        const next = await api<StudioJob>("/prepare", {
                          source_ids: project.source_ids,
                          preset: project.preset,
                          name: project.name,
                        });
                        setJob(next);
                        change({
                          preparation_id: next.id,
                          dataset_id:
                            next.status === "completed"
                              ? (next.result?.id ?? null)
                              : null,
                        });
                        if (next.status === "completed")
                          setDatasets(await api("/datasets"));
                      })
                    }
                  >
                    {t("studio.data.prepare")}
                  </button>
                  {dataset && (
                    <div className="mt-5 text-sm">
                      <p>
                        {t("studio.data.counts", {
                          count: dataset.count,
                          train: dataset.split_counts.train ?? 0,
                          validation: dataset.split_counts.validation ?? 0,
                          test: dataset.split_counts.test ?? 0,
                        })}
                      </p>
                      {dataset.warnings?.map((w) => (
                        <p key={w} className="mt-2 text-dim">
                          {w}
                        </p>
                      ))}
                      {dataset.next_action && (
                        <p role="status" className="mt-3">
                          {dataset.next_action}
                        </p>
                      )}
                      <button
                        className={`${button} mt-3`}
                        disabled={busy}
                        onClick={() =>
                          void action(async () =>
                            setPreview(
                              await api(`/datasets/${dataset.id}/preview`),
                            ),
                          )
                        }
                      >
                        {t("studio.data.checkText")}
                      </button>
                      {preview && (
                        <div className="mt-3 max-h-64 overflow-auto rounded-lg border border-edge p-3">
                          <p className="mb-3 text-xs text-dim">{t("studio.data.previewNote")}</p>
                          {preview.examples.map((ex, i) => (
                            <p key={i} className="mb-4 whitespace-pre-wrap">
                              {ex.text ??
                                ex.messages
                                  ?.map((m) => `${m.role}: ${m.content}`)
                                  .join("\n")}
                            </p>
                          ))}
                          <details>
                            <summary>{t("studio.data.report")}</summary>
                            <pre className="whitespace-pre-wrap text-xs">
                              {JSON.stringify(preview.manifest.files, null, 2)}
                            </pre>
                          </details>
                        </div>
                      )}
                    </div>
                  )}
                </section>
                <section className={panel}>
                  <h2 className="font-semibold">{t("studio.base.title")}</h2>
                  <div className="my-4 rounded-xl border border-edge p-4">
                    <p className="text-lg font-semibold">
                      {model?.name ?? t("studio.base.choose")}
                    </p>
                    <p className="my-2 text-sm text-dim">
                      {model?.downloaded
                        ? t("studio.base.local")
                        : model
                          ? t("studio.base.toDownload", { size: gb(model.download_bytes) })
                          : ""}{" "}
                      · {model?.license}
                    </p>
                    <p className="mb-3 text-xs text-dim">
                      {model?.tested ? t("studio.base.tested") : t("studio.base.candidate")}
                    </p>
                    <button
                      className={button}
                      disabled={active || busy}
                      onClick={() => setPicker(!picker)}
                    >
                      {t("studio.base.change")}
                    </button>
                  </div>
                  {picker && (
                    <div className="mb-4 rounded-xl border border-edge p-4">
                      <div className="mb-3 flex flex-wrap gap-2">
                        {[
                          ["catalog", t("studio.base.tabCatalog")],
                          ["downloaded", t("studio.base.tabDownloaded")],
                          ["import", t("studio.base.tabImport")],
                        ].map(([id, title]) => (
                          <button
                            key={id}
                            className={button}
                            aria-pressed={tab === id}
                            onClick={() => setTab(id)}
                          >
                            {title}
                          </button>
                        ))}
                      </div>
                      {tab === "import" && !standaloneStudio && (
                        <button
                          className={`${button} mb-3`}
                          disabled={busy || active}
                          onClick={() =>
                            void action(async () => {
                              const imported = await invoke<StudioModel | null>(
                                "studio_import_model",
                              );
                              if (imported) {
                                setModels(await api("/models"));
                                change({ model_id: imported.id });
                                setPicker(false);
                              }
                            })
                          }
                        >
                          {t("studio.base.importFolder")}
                        </button>
                      )}
                      {tab !== "import" ? (
                        <ul className="space-y-2">
                          {models
                            .filter((m) => tab !== "downloaded" || m.downloaded)
                            .map((m) => (
                              <li key={m.id}>
                                <button
                                  className={`${button} w-full text-left`}
                                  disabled={active || busy}
                                  onClick={() => {
                                    change({ model_id: m.id });
                                    setPicker(false);
                                  }}
                                >
                                  {m.name}
                                  <span className="block text-xs text-dim">
                                    {gb(m.download_bytes)} ·{" "}
                                    {m.tested ? t("studio.base.testedShort") : t("studio.base.needsValidation")} ·{" "}
                                    {m.license}
                                  </span>
                                </button>
                              </li>
                            ))}
                        </ul>
                      ) : (
                        <div>
                          <p className="mb-3 text-xs text-dim">{t("studio.base.importNote")}</p>
                          <label className="text-sm">
                            {t("studio.base.source")}
                            <select
                              className={field}
                              value={source}
                              onChange={(e) => setSource(e.target.value)}
                            >
                              <option value="hub">Hugging Face</option>
                              <option value="local">{t("studio.base.sourceLocal")}</option>
                            </select>
                          </label>
                          <label className="mt-3 block text-sm">
                            {source === "hub"
                              ? t("studio.base.repoId")
                              : t("studio.base.weightsFolder")}
                            <input
                              className={field}
                              value={location}
                              onChange={(e) => setLocation(e.target.value)}
                              placeholder={
                                source === "hub"
                                  ? "Qwen/Qwen3-1.7B"
                                  : t("studio.base.localPlaceholder")
                              }
                            />
                          </label>
                          <button
                            className={`${button} mt-3`}
                            disabled={active || busy || !location.trim()}
                            onClick={() =>
                              void action(async () => {
                                const imported = await api<StudioModel>(
                                  "/models/import",
                                  { source, location },
                                );
                                setModels(await api("/models"));
                                change({ model_id: imported.id });
                                setPicker(false);
                              })
                            }
                          >
                            {t("studio.base.importCheck")}
                          </button>
                        </div>
                      )}
                    </div>
                  )}
                  <p className="mb-4 text-sm text-dim">
                    {dataset?.kind === "chat" ? t("studio.goal.chat") : t("studio.goal.text")}
                  </p>
                  <label className="block text-sm">
                    {t("studio.recipe.label")}
                    <select
                      className={field}
                      disabled={active || busy}
                      value={project.recipe_id}
                      onChange={(e) =>
                        change({
                          recipe_id: e.target
                            .value as StudioProject["recipe_id"],
                        })
                      }
                    >
                      <option value="quick">{t("studio.recipe.quick")}</option>
                      <option value="recommended">{t("studio.recipe.recommended")}</option>
                    </select>
                  </label>
                  <p className="mt-2 text-xs text-dim">{t("studio.recipe.note")}</p>
                  <details className="my-4 text-sm">
                    <summary>{t("studio.advanced.title")}</summary>
                    <div className="mt-3 grid grid-cols-2 gap-3">
                      <label>
                        {t("studio.advanced.context")}
                        <select
                          className={field}
                          disabled={active || busy}
                          value={project.context}
                          onChange={(e) =>
                            change({ context: Number(e.target.value) })
                          }
                        >
                          {[512, 1024, 2048, 4096].map((n) => (
                            <option key={n} value={n}>
                              {t("studio.advanced.tokens", { n })}
                            </option>
                          ))}
                        </select>
                      </label>
                      <label>
                        {t("studio.advanced.rank")}
                        <select
                          className={field}
                          disabled={active || busy}
                          value={project.rank}
                          onChange={(e) =>
                            change({ rank: Number(e.target.value) })
                          }
                        >
                          {[8, 16, 32].map((n) => (
                            <option key={n}>{n}</option>
                          ))}
                        </select>
                      </label>
                      <label>
                        {t("studio.advanced.learningRate")}
                        <input
                          className={field}
                          type="number"
                          min={0.000001}
                          max={0.001}
                          step={0.00001}
                          disabled={active || busy}
                          value={project.learning_rate}
                          onChange={(e) =>
                            change({ learning_rate: Number(e.target.value) })
                          }
                        />
                      </label>
                      <label>
                        {t("studio.advanced.maxMinutes")}
                        <input
                          className={field}
                          type="number"
                          min={1}
                          max={1440}
                          disabled={active || busy}
                          value={project.max_minutes}
                          onChange={(e) =>
                            change({ max_minutes: Number(e.target.value) })
                          }
                        />
                      </label>
                    </div>
                  </details>
                  <div className="flex flex-wrap gap-2">
                    <button
                      className={button}
                      disabled={active || busy || !dataset?.trainable}
                      onClick={() =>
                        void action(async () =>
                          setResources(await api("/preflight", runBody())),
                        )
                      }
                    >
                      {t("studio.checkResources")}
                    </button>
                    <button
                      className={primary}
                      disabled={active || busy || !dataset?.trainable}
                      onClick={() =>
                        void action(async () => {
                          await save(project);
                          await gpu({
                            body: runBody(),
                            stopEngine: false,
                            resumeId: null,
                          });
                        })
                      }
                    >
                      {t("studio.train")}
                    </button>
                  </div>
                  {resources && (
                    <p className="mt-4 text-sm text-dim">
                      {t("studio.estimate", {
                        vram: formatNumber(resources.vram_estimated_mb / 1024, 1),
                        ram: gb(resources.ram_required_bytes),
                        disk: gb(resources.disk_required_bytes),
                      })}
                    </p>
                  )}
                </section>
              </div>
            </>
          )}
          {pending && (
            <div role="alert" className={`${panel} mt-5`}>
              <p>{t("studio.engineStop.body")}</p>
              <button
                className={`${primary} mt-3`}
                disabled={busy}
                onClick={() =>
                  void action(() => gpu({ ...pending, stopEngine: true }))
                }
              >
                {t("studio.engineStop.button")}
              </button>
            </div>
          )}
          {job && (
            <section className={`${panel} mt-5`} aria-live="polite">
              <p className="mb-2 text-xs text-dim">
                {job.name} · #{job.id.slice(0, 8)}
                {job.training_info?.base?.name
                  ? ` · ${job.training_info.base.name}`
                  : ""}
              </p>
              <h2 className="font-semibold">
                {rotulo(job.status)}
                {active && job.stage ? ` · ${rotulo(job.stage)}` : ""}
              </h2>
              {active && (
                <progress
                  className="my-4 w-full"
                  max={100}
                  value={job.stage === "training" ? percent : undefined}
                  aria-label={t("studio.job.progress")}
                />
              )}
              {job.stage === "downloading" && job.download_progress && (
                <p className="my-2 text-sm text-dim">
                  {t("studio.job.files", {
                    received: job.download_progress.received_files,
                    total: job.download_progress.total_files,
                  })}
                </p>
              )}
              {latest && (
                <p className="my-3 text-sm text-dim">
                  {Number.isFinite(steps)
                    ? t("studio.job.stepOf", { step: latest.step, total: steps })
                    : t("studio.job.step", { step: latest.step })}{" "}
                  ·{" "}
                  {t("studio.job.peak", {
                    vram: latest.vram_gb != null ? formatNumber(latest.vram_gb, 1) : "—",
                    minutes: Math.round((latest.elapsed ?? 0) / 60),
                  })}
                </p>
              )}
              {active &&
                job.stage === "training" &&
                latest &&
                latest.step > 0 &&
                Number.isFinite(steps) && (
                  <p className="text-xs text-dim">
                    {t("studio.job.remaining", {
                      minutes: Math.max(
                        1,
                        Math.ceil(
                          (((latest.elapsed ?? 0) / latest.step) *
                            Math.max(0, steps - latest.step)) /
                            60,
                        ),
                      ),
                    })}
                  </p>
                )}
              {job.chat_model &&
                job.training_info?.evaluation?.held_out_test?.test_loss !==
                  undefined && (
                  <p className="my-3 text-sm">
                    {t("studio.job.testLoss", {
                      loss: formatNumber(job.training_info.evaluation.held_out_test.test_loss, 3),
                    })}
                  </p>
                )}
              {job.error && (
                <p role="alert" className="my-3">
                  {job.error}
                </p>
              )}
              <div className="my-3 flex flex-wrap gap-2">
                {active && (
                  <button
                    className={button}
                    disabled={busy || job.status === "cancelling"}
                    onClick={() =>
                      void action(async () =>
                        setJob(await api(`/runs/${job.id}/cancel`, {})),
                      )
                    }
                  >
                    {t("studio.job.cancel")}
                  </button>
                )}
                {job.kind === "guided_prepare" &&
                  finished(job) &&
                  job.status !== "completed" && (
                    <>
                      {!standaloneStudio && job.error?.includes("OCR") && (
                        <button
                          className={button}
                          disabled={busy}
                          onClick={() =>
                            void action(async () => {
                              await invoke("studio_install", {
                                component: "ocr",
                              });
                            })
                          }
                        >
                          {t("studio.job.installOcr")} {download}
                        </button>
                      )}
                      <button
                        className={button}
                        disabled={busy}
                        onClick={() =>
                          void action(async () =>
                            setJob(
                              await api(`/preparations/${job.id}/resume`, {}),
                            ),
                          )
                        }
                      >
                        {t("studio.job.resumePrep")}
                      </button>
                    </>
                  )}
                {job.kind === "workflow" &&
                  finished(job) &&
                  job.status !== "completed" &&
                  (!!job.checkpoints?.length || job.can_retry_export) && (
                    <button
                      className={button}
                      disabled={busy}
                      onClick={() =>
                        void action(() =>
                          gpu({
                            body: {},
                            resumeId: job.id,
                            stopEngine: false,
                          }),
                        )
                      }
                    >
                      {job.can_retry_export ? t("studio.job.retryExport") : t("studio.job.resumeCheckpoint")}
                    </button>
                  )}
                {job.chat_model &&
                  (standaloneStudio ? (
                    <a
                      className={primary}
                      href={`/api/v1/runs/${job.id}/download`}
                    >
                      {t("studio.job.downloadGguf")}
                    </a>
                  ) : (
                    <button
                      className={primary}
                      onClick={() =>
                        navigate("chat", { chatModel: job.chat_model })
                      }
                    >
                      {t("studio.job.openInChat")}
                    </button>
                  ))}
              </div>
              {job.chat_model && (
                <div className="mt-4">
                  <p className="text-sm text-dim">{t("studio.job.ggufNote")}</p>
                  <label className="mt-4 block text-sm">
                    {t("studio.job.compareLabel")}
                    <textarea
                      className={field}
                      maxLength={8000}
                      value={prompt}
                      onChange={(e) => setPrompt(e.target.value)}
                      placeholder={t("studio.job.comparePlaceholder")}
                    />
                  </label>
                  <button
                    className={`${button} mt-2`}
                    disabled={busy || !prompt.trim()}
                    onClick={() =>
                      void action(() =>
                        gpu({
                          body: {
                            comparison_run: job.id,
                            prompt,
                            request_id: crypto.randomUUID(),
                          },
                          stopEngine: false,
                          resumeId: null,
                        }),
                      )
                    }
                  >
                    {t("studio.job.compareButton")}
                  </button>
                  <p className="mt-2 text-xs text-dim">{t("studio.job.compareNote")}</p>
                </div>
              )}
              {answers && (
                <div className="mt-4 grid gap-4 md:grid-cols-2">
                  {[
                    ["original", t("studio.job.original")],
                    ["trained", t("studio.job.trained")],
                  ].map(([id, title]) => (
                    <div key={id} className="rounded-xl border border-edge p-4">
                      <h3 className="mb-3 font-semibold">{title}</h3>
                      <p className="whitespace-pre-wrap text-sm">
                        {answers[id as "original" | "trained"]?.text}
                      </p>
                    </div>
                  ))}
                </div>
              )}
              <details className="mt-4 text-sm">
                <summary>{t("studio.job.technical")}</summary>
                <pre className="mt-3 max-h-64 overflow-auto whitespace-pre-wrap text-xs">
                  {job.log || t("studio.job.logEmpty")}
                </pre>
              </details>
            </section>
          )}
          {(project || job) && (
            <StudioHistory
              runs={history.filter(
                (r) =>
                  !project ||
                  r.project_id === project.id ||
                  r.id === project.preparation_id,
              )}
              disabled={active || busy}
              onOpen={(run) =>
                void action(async () => setJob(await api(`/runs/${run.id}`)))
              }
            />
          )}
        </>
      )}
    </Page>
  );
}
