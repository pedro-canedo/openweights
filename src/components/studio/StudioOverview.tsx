import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import Icon from "../ui/Icon";
import {
  finished,
  standaloneStudio,
  type StudioJob,
  type StudioModel,
  type StudioProject,
} from "../../lib/studio";
import { navigate } from "../../lib/nav";

const control =
  "inline-flex items-center justify-center gap-2 rounded-xl border border-edge px-4 py-2.5 text-sm font-medium hover:bg-panel2 disabled:opacity-40 disabled:cursor-not-allowed";
/** O tipo da execução, por extenso; `null` quando o serviço manda um novo. */
const kind = (t: TFunction, k: string): string | null =>
  ["guided_prepare", "workflow", "comparison"].includes(k) ? t(`studio.kind.${k}`) : null;
/** Os estados que a lista distingue; o resto está em andamento. */
const STATUSES = ["completed", "failed", "cancelled", "interrupted", "queued", "cancelling"];
const normalize = (text: string) =>
  text
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLocaleLowerCase();
function date(lang: string, value?: string) {
  if (!value || Number.isNaN(Date.parse(value))) return "";
  return new Date(value).toLocaleString(lang, {
    day: "2-digit",
    month: "short",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function StudioHistory({
  runs,
  disabled,
  onOpen,
}: {
  runs: StudioJob[];
  disabled: boolean;
  onOpen: (run: StudioJob) => void;
}) {
  const { t, i18n } = useTranslation();
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState("all");
  const visible = runs.filter(
    (run) =>
      normalize(
        `${run.name} ${kind(t, run.kind) ?? ""} ${run.training_info?.base?.name ?? ""} ${run.id}`,
      ).includes(normalize(query.trim())) &&
      (filter === "all" ||
        (filter === "models"
          ? run.status === "completed" && !!run.chat_model
          : filter === "attention"
            ? ["failed", "interrupted", "cancelled"].includes(run.status)
            : !finished(run))),
  );
  return (
    <section className="mt-8" aria-label={t("studio.history.title")}>
      <div className="mb-4 flex items-center justify-between gap-3">
        <div>
          <h2>{t("studio.history.title")}</h2>
          <p className="mt-1 text-sm text-dim">{t("studio.history.hint")}</p>
        </div>
        <span className="text-xs text-dim">{t("studio.history.count", { count: runs.length })}</span>
      </div>
      {runs.length > 0 ? (
        <>
          <div className="mb-4 flex flex-wrap gap-3">
            <label className="flex min-w-0 flex-1 items-center gap-2 rounded-xl border border-edge bg-panel px-3">
              <Icon name="search" />
              <input
                type="search"
                aria-label={t("studio.history.search")}
                placeholder={t("studio.history.searchPlaceholder")}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                className="min-w-0 w-full bg-transparent py-3 text-sm outline-none"
              />
            </label>
            <select
              aria-label={t("studio.history.filter")}
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              className="rounded-xl border border-edge bg-panel px-3 py-3 text-sm"
            >
              <option value="all">{t("studio.history.filterAll")}</option>
              <option value="models">{t("studio.history.filterModels")}</option>
              <option value="active">{t("studio.history.filterActive")}</option>
              <option value="attention">{t("studio.history.filterAttention")}</option>
            </select>
          </div>
          <div className="overflow-hidden rounded-2xl border border-edge bg-panel">
            {visible.map((run) => (
              <article
                key={run.id}
                className="flex flex-wrap items-center gap-4 border-b border-edge p-5 last:border-b-0"
              >
                <div className="hidden rounded-xl bg-panel2 p-3 text-dim sm:block">
                  <Icon
                    name={
                      run.kind === "workflow"
                        ? "cpu"
                        : run.kind === "comparison"
                          ? "layers"
                          : "history"
                    }
                    className="h-5 w-5"
                  />
                </div>
                <div className="min-w-0 flex-1 basis-48">
                  <div className="mb-1 text-xs text-dim">
                    {kind(t, run.kind) ?? t("studio.kind.run")}
                  </div>
                  <h3 className="break-words text-sm font-semibold">
                    {run.name}
                  </h3>
                  {run.training_info?.base?.name && (
                    <p className="mt-1 text-xs text-dim">
                      {run.training_info.base.name}
                    </p>
                  )}
                  <p className="mt-1 text-xs text-dim">
                    <time dateTime={run.created_at}>
                      {date(i18n.language, run.created_at)}
                    </time>
                    {run.created_at ? " · " : ""}
                    <span title={run.id}>#{run.id.slice(0, 8)}</span>
                  </p>
                </div>
                <span
                  className={`inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs ${run.status === "completed" ? "studio-status-success" : finished(run) ? "studio-status-attention" : "bg-accent/10 text-accent-ink"}`}
                >
                  <Icon
                    name={
                      run.status === "completed"
                        ? "check"
                        : finished(run)
                          ? "alert"
                          : "clock"
                    }
                  />
                  {run.chat_model && run.status === "completed"
                    ? t("studio.history.modelReady")
                    : STATUSES.includes(run.status)
                      ? t(`studio.status.${run.status}`)
                      : t("studio.status.running")}
                </span>
                <div className="flex flex-wrap gap-2">
                  <button
                    className={control}
                    disabled={disabled}
                    onClick={() => onOpen(run)}
                    aria-label={t("studio.history.detailsOf", {
                      name: run.name,
                      kind: kind(t, run.kind) ?? t("studio.kind.run"),
                      id: run.id.slice(0, 8),
                    })}
                  >
                    {t("studio.history.details")}
                    <Icon name="arrow-right" />
                  </button>
                  {run.chat_model &&
                    run.status === "completed" &&
                    (standaloneStudio ? (
                      <a
                        className={control}
                        href={`/api/v1/runs/${run.id}/download`}
                      >
                        {t("studio.job.downloadGguf")}
                        <Icon name="download" />
                      </a>
                    ) : (
                      <button
                        className={`${control} bg-ink text-panel hover:bg-ink/90`}
                        disabled={disabled}
                        onClick={() =>
                          navigate("chat", { chatModel: run.chat_model })
                        }
                      >
                        {t("studio.job.openInChat")}
                      </button>
                    ))}
                </div>
              </article>
            ))}
            {!visible.length && (
              <div className="p-8 text-center">
                <p className="text-sm text-dim">{t("studio.history.noMatch")}</p>
                <button
                  className={`${control} mt-4`}
                  onClick={() => {
                    setQuery("");
                    setFilter("all");
                  }}
                >
                  {t("studio.history.clear")}
                </button>
              </div>
            )}
          </div>
        </>
      ) : (
        <div className="rounded-2xl border border-dashed border-edge p-8 text-center text-sm text-dim">
          {t("studio.history.empty")}
        </div>
      )}
    </section>
  );
}

export default function StudioOverview({
  projects,
  models,
  runs,
  disabled,
  onCreate,
  onProject,
  onRun,
}: {
  projects: StudioProject[];
  models: StudioModel[];
  runs: StudioJob[];
  disabled: boolean;
  onCreate: () => void;
  onProject: (project: StudioProject) => void;
  onRun: (run: StudioJob) => void;
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const modelNames = new Map(models.map((model) => [model.id, model.name]));
  const visible = projects.filter((project) =>
    normalize(
      `${project.name} ${modelNames.get(project.model_id) ?? project.model_id}`,
    ).includes(normalize(query.trim())),
  );
  return (
    <>
      <div className="studio-intro rounded-2xl border border-edge bg-panel p-6 sm:p-8">
        <div className="max-w-xl">
          <span className="text-xs font-semibold uppercase tracking-widest text-accent-ink">
            {t("studio.overview.eyebrow")}
          </span>
          <h2 className="mt-3 text-xl font-semibold tracking-tight sm:text-2xl">
            {t("studio.overview.title")}
          </h2>
          <p className="mt-3 text-sm leading-relaxed text-dim">{t("studio.overview.body")}</p>
        </div>
        <ol
          className="mt-6 grid gap-3 sm:grid-cols-3"
          aria-label={t("studio.overview.howItWorks")}
        >
          {(["1", "2", "3"] as const).map((n) => [
            `0${n}`,
            t(`studio.overview.step${n}Title`),
            t(`studio.overview.step${n}Body`),
          ]).map(([number, title, description]) => (
            <li
              key={number}
              className="flex gap-3 rounded-xl border border-edge bg-bg/50 p-4"
            >
              <span className="text-xs font-semibold text-accent-ink">
                {number}
              </span>
              <div>
                <p className="text-sm font-medium">{title}</p>
                <p className="mt-1 text-xs leading-relaxed text-dim">
                  {description}
                </p>
              </div>
            </li>
          ))}
        </ol>
      </div>
      <section className="mt-8" aria-label={t("studio.overview.projects")}>
        <div className="mb-4 flex items-center justify-between gap-3">
          <div>
            <h2>
              {t("studio.overview.projects")}{" "}
              <span className="ml-2 text-xs font-normal text-dim">
                {projects.length}
              </span>
            </h2>
            <p className="mt-1 text-sm text-dim">{t("studio.overview.projectsHint")}</p>
          </div>
          {projects.length > 0 && (
            <input
              type="search"
              aria-label={t("studio.overview.search")}
              placeholder={t("studio.overview.searchPlaceholder")}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              className="w-full rounded-xl border border-edge bg-panel px-3 py-2.5 text-sm sm:w-64"
            />
          )}
        </div>
        {projects.length ? (
          <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
            {visible.map((project) => (
              <button
                key={project.id}
                disabled={disabled}
                onClick={() => onProject(project)}
                className="source-card flex min-w-0 flex-col rounded-2xl border border-edge bg-panel p-5 text-left disabled:opacity-40"
              >
                <div className="mb-4 flex w-full items-center justify-between gap-2">
                  <span className="source-icon">
                    <Icon name="layers" className="h-5 w-5" />
                  </span>
                  <span className="rounded-full bg-panel2 px-2.5 py-1 text-xs text-dim">
                    {project.dataset_id
                      ? t("studio.overview.dataPrepared")
                      : project.source_ids.length
                        ? t("studio.overview.dataAdded")
                        : t("studio.overview.draft")}
                  </span>
                </div>
                <h3 className="w-full break-words font-semibold">
                  {project.name}
                </h3>
                <p className="mt-1 w-full truncate text-sm text-dim">
                  {modelNames.get(project.model_id) ?? project.model_id}
                </p>
                <p className="mt-3 text-xs text-dim">
                  {t("studio.overview.files", { count: project.source_ids.length })} ·{" "}
                  {project.recipe_id === "quick"
                    ? t("studio.overview.quick")
                    : t("studio.overview.recommended")}
                </p>
                <span className="mt-5 flex w-full items-center justify-between border-t border-edge pt-4 text-sm font-medium">
                  {project.dataset_id
                    ? t("studio.overview.continue")
                    : project.source_ids.length
                      ? t("studio.overview.prepareData")
                      : t("studio.overview.addFiles")}
                  <Icon name="arrow-right" />
                </span>
              </button>
            ))}
            {!visible.length && (
              <p className="col-span-full py-6 text-sm text-dim">{t("studio.overview.noProject")}</p>
            )}
          </div>
        ) : (
          <div className="flex flex-wrap items-center gap-4 rounded-2xl border border-dashed border-edge p-6">
            <span className="source-icon">
              <Icon name="layers" className="h-5 w-5" />
            </span>
            <div className="min-w-0 flex-1 basis-52">
              <h3 className="text-sm font-medium">{t("studio.overview.firstTitle")}</h3>
              <p className="mt-1 text-sm text-dim">{t("studio.overview.firstHint")}</p>
            </div>
            <button disabled={disabled} className={control} onClick={onCreate}>
              {t("studio.overview.create")}
              <Icon name="arrow-right" />
            </button>
          </div>
        )}
      </section>
      <StudioHistory runs={runs} disabled={disabled} onOpen={onRun} />
    </>
  );
}
