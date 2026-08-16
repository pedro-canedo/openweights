// Histórico de downloads incompletos na tela Meus Modelos: sobrevive a
// reinício do app/PC. Cards compactos com busca e filtro por estado.

import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { DownloadState, DownloadStatus } from "../../lib/types";
import {
  cancelDownload,
  listDownloads,
  onDownloadEvent,
  pauseDownload,
  resumeDownload,
} from "../../lib/api";
import { downloadPercent, formatBytes, formatEta } from "../../lib/format";

const INCOMPLETE: ReadonlySet<DownloadState> = new Set([
  "queued",
  "running",
  "paused",
  "error",
]);

type Filter = "all" | "active" | "paused" | "error";

function matchesFilter(d: DownloadStatus, filter: Filter): boolean {
  if (filter === "all") return true;
  if (filter === "active") return d.state === "queued" || d.state === "running";
  return d.state === filter;
}

function barColor(state: DownloadState): string {
  if (state === "error") return "bg-bad";
  if (state === "paused") return "bg-warn";
  if (state === "queued") return "bg-dim";
  return "bg-accent";
}

function MiniCard({
  status,
  onGone,
}: {
  status: DownloadStatus;
  onGone: () => void;
}) {
  const { t } = useTranslation();
  const pct = downloadPercent(status.receivedBytes, status.totalBytes);
  const remaining = status.totalBytes - status.receivedBytes;
  const eta =
    status.state === "running" && status.bytesPerSec > 0
      ? formatEta(remaining / status.bytesPerSec)
      : null;

  const act = (p: Promise<void>) => void p.catch(console.error);

  const stateLabel =
    status.state === "running"
      ? t("downloadsPanel.title")
      : status.state === "queued"
        ? t("models.queued")
        : status.state === "paused"
          ? t("models.paused")
          : t("downloadsPanel.error");

  return (
    <div className="flex flex-col rounded-xl border border-edge bg-panel p-3">
      <p className="truncate text-[13px] font-medium text-ink" title={status.artifactName}>
        {status.artifactName}
      </p>
      <p className="mt-0.5 truncate text-[11px] text-dim" title={status.repoId}>
        {status.repoId}
      </p>

      <div className="mt-2.5 h-1.5 overflow-hidden rounded-full bg-panel2">
        <div
          className={`h-full rounded-full ${barColor(status.state)} transition-[width] duration-300`}
          style={{ width: `${pct}%` }}
        />
      </div>

      <div className="mt-1.5 flex items-center gap-2 text-[11px] tabular-nums text-dim">
        {status.state === "error" ? (
          <span className="truncate font-medium text-bad" title={status.error ?? undefined}>
            {status.error || t("downloadsPanel.error")}
          </span>
        ) : (
          <>
            <span>
              {formatBytes(status.receivedBytes)}
              {status.totalBytes > 0 ? ` / ${formatBytes(status.totalBytes)}` : ""}
            </span>
            <span className="ml-auto">
              {status.state === "running"
                ? `${formatBytes(status.bytesPerSec)}/s${eta ? ` · ${eta}` : ""}`
                : stateLabel}
            </span>
          </>
        )}
      </div>

      <div className="mt-3 flex items-center gap-1.5">
        {status.state === "running" ? (
          <button
            type="button"
            onClick={() => act(pauseDownload(status.id))}
            className="rounded-lg border border-edge px-2.5 py-1 text-[11px] font-medium text-dim transition-colors hover:text-ink"
          >
            {t("downloadsPanel.pause")}
          </button>
        ) : (
          <button
            type="button"
            onClick={() => act(resumeDownload(status.id))}
            className="rounded-lg bg-accent px-2.5 py-1 text-[11px] font-medium text-white transition-opacity hover:opacity-90"
          >
            {t("models.resume")}
          </button>
        )}
        <button
          type="button"
          onClick={() => act(cancelDownload(status.id).then(onGone))}
          className="rounded-lg border border-edge px-2.5 py-1 text-[11px] font-medium text-dim transition-colors hover:border-bad/60 hover:text-bad"
        >
          {t("models.discard")}
        </button>
      </div>
    </div>
  );
}

export default function IncompleteDownloads({
  onFinished,
}: {
  onFinished: () => void;
}) {
  const { t } = useTranslation();
  const [items, setItems] = useState<DownloadStatus[] | null>(null);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");

  useEffect(() => {
    let un: (() => void) | undefined;
    let cancelled = false;

    listDownloads()
      .then((list) => {
        if (!cancelled) setItems(list);
      })
      .catch((err) => {
        console.error(err);
        if (!cancelled) setItems([]);
      });

    onDownloadEvent((e) => {
      if (e.kind === "update" && e.status.state === "done") onFinished();
      setItems((prev) => {
        const cur = prev ?? [];
        if (e.kind === "removed") return cur.filter((d) => d.id !== e.id);
        return cur.some((d) => d.id === e.status.id)
          ? cur.map((d) => (d.id === e.status.id ? e.status : d))
          : [...cur, e.status];
      });
    }).then((f) => {
      if (cancelled) f();
      else un = f;
    });

    return () => {
      cancelled = true;
      un?.();
    };
  }, [onFinished]);

  const incomplete = useMemo(
    () => (items ?? []).filter((d) => INCOMPLETE.has(d.state)),
    [items],
  );

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    return incomplete.filter((d) => {
      if (!matchesFilter(d, filter)) return false;
      if (!q) return true;
      return (
        d.artifactName.toLowerCase().includes(q) ||
        d.repoId.toLowerCase().includes(q)
      );
    });
  }, [incomplete, filter, query]);

  if (items == null || incomplete.length === 0) return null;

  const chips: { id: Filter; label: string }[] = [
    { id: "all", label: t("models.filterAll") },
    { id: "active", label: t("models.filterActive") },
    { id: "paused", label: t("models.filterPaused") },
    { id: "error", label: t("models.filterError") },
  ];

  return (
    <section className="mt-10">
      <div className="flex items-end justify-between gap-3 border-t border-edge pt-8">
        <div>
          <h2 className="text-sm font-semibold text-ink">
            {t("models.incompleteTitle")}
            <span className="ml-2 rounded-full bg-panel2 px-2 py-0.5 text-[11px] font-medium text-dim">
              {incomplete.length}
            </span>
          </h2>
          <p className="mt-1 text-xs text-dim">{t("models.incompleteHint")}</p>
        </div>
      </div>

      <div className="mt-4 flex flex-col gap-3 sm:flex-row sm:items-center">
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t("models.incompleteSearch")}
          className="min-w-0 flex-1 rounded-xl border border-edge bg-panel px-3 py-2 text-sm outline-none placeholder:text-dim focus:border-accent"
        />
        <div className="flex flex-wrap gap-1.5">
          {chips.map((c) => (
            <button
              key={c.id}
              type="button"
              onClick={() => setFilter(c.id)}
              className={`rounded-full px-2.5 py-1 text-[11px] font-medium transition-colors ${
                filter === c.id
                  ? "bg-accent/15 text-accent"
                  : "bg-panel2 text-dim hover:text-ink"
              }`}
            >
              {c.label}
            </button>
          ))}
        </div>
      </div>

      {visible.length === 0 ? (
        <p className="mt-4 text-sm text-dim">{t("models.incompleteNone")}</p>
      ) : (
        <div className="mt-4 grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {visible.map((d) => (
            <MiniCard key={d.id} status={d} onGone={onFinished} />
          ))}
        </div>
      )}
    </section>
  );
}
