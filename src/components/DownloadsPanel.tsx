// Painel global de downloads: botão flutuante no canto inferior direito com
// progresso agregado; expande para listar cada download com barra, velocidade,
// ETA e ações de pausar/retomar/cancelar. Some quando não há downloads.

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { DownloadStatus } from "../lib/types";
import {
  cancelDownload,
  listDownloads,
  onDownloadEvent,
  pauseDownload,
  resumeDownload,
} from "../lib/api";
import { formatBytes, formatEta } from "../lib/format";

const ACTIVE_STATES = new Set(["queued", "running", "paused"]);

function percentOf(d: DownloadStatus): number {
  if (d.totalBytes <= 0) return 0;
  return Math.max(0, Math.min(100, (d.receivedBytes / d.totalBytes) * 100));
}

function IconButton({
  label,
  path,
  danger,
  onClick,
}: {
  label: string;
  path: string;
  danger?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      title={label}
      aria-label={label}
      className={`rounded-md p-1.5 text-dim transition-colors hover:bg-panel2 ${
        danger ? "hover:text-bad" : "hover:text-ink"
      }`}
    >
      <svg
        className="h-3.5 w-3.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
        viewBox="0 0 24 24"
      >
        <path d={path} />
      </svg>
    </button>
  );
}

const ICONS = {
  pause: "M10 5v14M14 5v14",
  resume: "M7 5l12 7-12 7V5z",
  cancel: "M6 6l12 12M18 6L6 18",
  download: "M4 16v2a2 2 0 002 2h12a2 2 0 002-2v-2M7 10l5 5 5-5M12 15V3",
  chevronDown: "M6 9l6 6 6-6",
};

function DownloadRow({ status }: { status: DownloadStatus }) {
  const { t } = useTranslation();
  const pct = percentOf(status);
  const remaining = status.totalBytes - status.receivedBytes;
  const eta =
    status.state === "running" && status.bytesPerSec > 0
      ? formatEta(remaining / status.bytesPerSec)
      : null;

  // Erros nas ações são registrados; o estado real volta pelos eventos.
  const act = (p: Promise<void>) => void p.catch(console.error);

  const barColor =
    status.state === "done"
      ? "bg-ok"
      : status.state === "error"
        ? "bg-bad"
        : status.state === "paused"
          ? "bg-warn"
          : "bg-accent";

  return (
    <div className="border-b border-edge px-4 py-3 last:border-b-0">
      <div className="flex items-center gap-2">
        <div className="min-w-0 flex-1">
          <p className="truncate text-xs font-medium text-ink">
            {status.artifactName}
          </p>
          <p className="truncate text-[11px] text-dim">{status.repoId}</p>
        </div>

        <div className="flex shrink-0 items-center">
          {status.state === "running" && (
            <IconButton
              label={t("downloadsPanel.pause")}
              path={ICONS.pause}
              onClick={() => act(pauseDownload(status.id))}
            />
          )}
          {status.state === "paused" && (
            <IconButton
              label={t("downloadsPanel.resume")}
              path={ICONS.resume}
              onClick={() => act(resumeDownload(status.id))}
            />
          )}
          {(ACTIVE_STATES.has(status.state) || status.state === "error") && (
            <IconButton
              label={t("downloadsPanel.cancel")}
              path={ICONS.cancel}
              danger
              onClick={() => act(cancelDownload(status.id))}
            />
          )}
        </div>
      </div>

      <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-panel2">
        <div
          className={`h-full rounded-full ${barColor} transition-[width] duration-300`}
          style={{ width: `${status.state === "done" ? 100 : pct}%` }}
        />
      </div>

      <div className="mt-1.5 flex items-center gap-2 text-[11px] tabular-nums text-dim">
        {status.state === "done" ? (
          <span className="font-medium text-ok">{t("downloadsPanel.done")}</span>
        ) : status.state === "error" ? (
          <span
            className="truncate font-medium text-bad"
            title={status.error ?? undefined}
          >
            {t("downloadsPanel.error")}
            {status.error ? ` — ${status.error}` : ""}
          </span>
        ) : (
          <>
            <span>
              {formatBytes(status.receivedBytes)} /{" "}
              {formatBytes(status.totalBytes)}
            </span>
            {status.state === "running" && (
              <span className="ml-auto">
                {formatBytes(status.bytesPerSec)}/s
                {eta ? ` · ${eta}` : ""}
              </span>
            )}
          </>
        )}
      </div>
    </div>
  );
}

export default function DownloadsPanel() {
  const { t } = useTranslation();
  const [items, setItems] = useState<DownloadStatus[]>([]);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    let un: (() => void) | undefined;
    let cancelled = false;

    listDownloads()
      .then((list) => {
        if (!cancelled) setItems(list);
      })
      .catch(console.error);

    onDownloadEvent((e) => {
      setItems((prev) => {
        if (e.kind === "removed") return prev.filter((d) => d.id !== e.id);
        return prev.some((d) => d.id === e.status.id)
          ? prev.map((d) => (d.id === e.status.id ? e.status : d))
          : [...prev, e.status];
      });
    }).then((f) => {
      if (cancelled) f();
      else un = f;
    });

    return () => {
      cancelled = true;
      un?.();
    };
  }, []);

  if (items.length === 0) return null;

  const active = items.filter((d) => ACTIVE_STATES.has(d.state));
  const totalBytes = active.reduce((s, d) => s + d.totalBytes, 0);
  const receivedBytes = active.reduce((s, d) => s + d.receivedBytes, 0);
  const aggregate = totalBytes > 0 ? (receivedBytes / totalBytes) * 100 : 0;

  return (
    <div className="fixed right-4 bottom-12 z-40 flex flex-col items-end">
      {open && (
        <div className="mb-2 flex max-h-[60vh] w-96 max-w-[calc(100vw-2rem)] flex-col overflow-hidden rounded-xl border border-edge bg-panel shadow-2xl">
          <header className="flex items-center gap-2 border-b border-edge px-4 py-2.5">
            <span className="text-sm font-semibold">
              {t("downloadsPanel.title")}
            </span>
            <span className="rounded-full bg-panel2 px-2 py-0.5 text-[11px] text-dim">
              {items.length}
            </span>
            <button
              onClick={() => setOpen(false)}
              aria-label={t("common.close")}
              className="ml-auto rounded-md p-1 text-dim transition-colors hover:bg-panel2 hover:text-ink"
            >
              <svg
                className="h-4 w-4"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
                viewBox="0 0 24 24"
              >
                <path d={ICONS.chevronDown} />
              </svg>
            </button>
          </header>
          <div className="min-h-0 flex-1 overflow-y-auto">
            {items.map((d) => (
              <DownloadRow key={d.id} status={d} />
            ))}
          </div>
        </div>
      )}

      <button
        onClick={() => setOpen((v) => !v)}
        title={t("downloadsPanel.title")}
        className="flex items-center gap-2.5 rounded-full border border-edge bg-panel py-2 pr-4 pl-3 shadow-lg transition-colors hover:border-accent"
      >
        <svg
          className={`h-4 w-4 ${active.length > 0 ? "text-accent" : "text-dim"}`}
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          viewBox="0 0 24 24"
        >
          <path d={ICONS.download} />
        </svg>
        {active.length > 0 ? (
          <>
            <span className="h-1.5 w-20 overflow-hidden rounded-full bg-panel2">
              <span
                className="block h-full rounded-full bg-accent transition-[width] duration-300"
                style={{ width: `${aggregate}%` }}
              />
            </span>
            <span className="text-xs font-medium tabular-nums text-ink">
              {aggregate.toFixed(0)}%
            </span>
          </>
        ) : (
          <span className="text-xs font-medium text-ink">{items.length}</span>
        )}
      </button>
    </div>
  );
}
