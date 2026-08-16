// Drawer lateral (overlay à direita) com as quantizações de um repositório,
// ordenadas por bits, cada uma com veredito de compatibilidade e botão de
// download — referência: widget "Hardware compatibility" do Hugging Face.

import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { ModelSummary, QuantView } from "../../lib/types";
import { getModelQuants, startDownload } from "../../lib/api";
import { formatBytes } from "../../lib/format";
import { AuthorAvatar } from "./ModelCard";
import VerdictBadge from "./VerdictBadge";

async function openOnHf(repoId: string) {
  const url = `https://huggingface.co/${repoId}`;
  try {
    await openUrl(url);
  } catch {
    // Modo navegador (sem Tauri): abre numa aba comum.
    window.open(url, "_blank", "noopener");
  }
}

function QuantItem({
  quant,
  started,
  onDownload,
}: {
  quant: QuantView;
  started: boolean;
  onDownload: () => void;
}) {
  const { t } = useTranslation();

  return (
    <div
      className={`rounded-xl border bg-panel2/40 p-3 ${
        quant.recommended ? "border-accent/60" : "border-edge"
      }`}
    >
      <div className="flex items-center gap-2">
        <span className="min-w-0 flex-1 truncate font-mono text-[13px] font-medium text-ink">
          {quant.label}
        </span>
        <span className="shrink-0 text-xs tabular-nums text-dim">
          {formatBytes(quant.sizeBytes)}
        </span>
      </div>

      <div className="mt-2 flex flex-wrap items-center gap-1.5">
        {quant.recommended && (
          <span className="rounded-full bg-accent/15 px-2 py-0.5 text-[11px] font-medium text-accent">
            {t("badge.recommended")}
          </span>
        )}
        <VerdictBadge verdict={quant.verdict} />
      </div>

      {quant.verdict.kind === "partial" && (
        <p className="mt-1.5 text-[11px] text-warn/90">
          {t("badge.partialWarn")}
        </p>
      )}

      <button
        onClick={onDownload}
        disabled={started}
        className={`mt-2.5 w-full rounded-lg px-3 py-1.5 text-xs font-medium transition-colors ${
          started
            ? "cursor-default bg-panel2 text-dim"
            : "bg-accent text-white hover:opacity-90"
        }`}
      >
        {started ? t("discover.downloading") : t("discover.download")}
      </button>
    </div>
  );
}

export default function QuantDrawer({
  model,
  onClose,
}: {
  model: ModelSummary;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [quants, setQuants] = useState<QuantView[] | null>(null);
  const [failed, setFailed] = useState(false);
  const [started, setStarted] = useState<ReadonlySet<string>>(new Set());
  const [shown, setShown] = useState(false);

  // Entrada animada (duas fases: monta fora da tela e desliza para dentro).
  useEffect(() => {
    const id = requestAnimationFrame(() => setShown(true));
    return () => cancelAnimationFrame(id);
  }, []);

  // Fecha com Escape.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  useEffect(() => {
    let cancelled = false;
    setQuants(null);
    setFailed(false);
    getModelQuants(model.id, model.paramsTotal)
      .then((q) => {
        if (!cancelled) setQuants(q);
      })
      .catch(() => {
        if (!cancelled) setFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [model.id, model.paramsTotal]);

  // Ordena por bits (desconhecidos por último) e desempata pelo tamanho.
  const sorted = useMemo(
    () =>
      quants
        ? [...quants].sort(
            (a, b) =>
              (a.bits ?? Number.MAX_SAFE_INTEGER) -
                (b.bits ?? Number.MAX_SAFE_INTEGER) ||
              a.sizeBytes - b.sizeBytes,
          )
        : null,
    [quants],
  );

  const download = (q: QuantView) => {
    // Feedback imediato; reverte se o backend recusar.
    setStarted((prev) => new Set(prev).add(q.artifactName));
    startDownload(model.id, q.artifactName).catch((err) => {
      console.error(err);
      setStarted((prev) => {
        const next = new Set(prev);
        next.delete(q.artifactName);
        return next;
      });
    });
  };

  return (
    <div className="fixed inset-0 z-50">
      <div
        className={`absolute inset-0 bg-black/50 transition-opacity duration-200 ${
          shown ? "opacity-100" : "opacity-0"
        }`}
        onClick={onClose}
      />

      <aside
        className={`absolute top-0 right-0 flex h-full w-full max-w-md flex-col border-l border-edge bg-panel shadow-2xl transition-transform duration-200 ${
          shown ? "translate-x-0" : "translate-x-full"
        }`}
      >
        <header className="flex items-start gap-3 border-b border-edge px-5 py-4">
          <AuthorAvatar author={model.author} size={36} />
          <div className="min-w-0 flex-1">
            <div className="truncate text-sm">
              <span className="text-dim">{model.author}/</span>
              <span className="font-medium text-ink">{model.name}</span>
            </div>
            <h2 className="mt-1 text-base font-semibold">
              {t("discover.quantsTitle")}
            </h2>
            <p className="mt-1 text-xs text-dim">{t("discover.quantsHint")}</p>
          </div>
          <button
            onClick={onClose}
            aria-label={t("common.close")}
            className="rounded-lg p-1.5 text-dim transition-colors hover:bg-panel2 hover:text-ink"
          >
            <svg
              className="h-4 w-4"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              viewBox="0 0 24 24"
            >
              <path d="M6 6l12 12M18 6L6 18" />
            </svg>
          </button>
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
          {model.gated && (
            <div className="mb-4 rounded-xl border border-warn/40 bg-warn/10 p-3">
              <p className="text-xs text-ink">{t("discover.gatedHint")}</p>
              <button
                onClick={() => void openOnHf(model.id)}
                className="mt-2 rounded-lg border border-edge bg-panel px-3 py-1.5 text-xs font-medium text-ink transition-colors hover:border-accent"
              >
                {t("discover.openOnHf")} ↗
              </button>
            </div>
          )}

          {failed ? (
            <div className="rounded-xl border border-dashed border-edge p-8 text-center text-sm text-dim">
              {t("common.error")}
            </div>
          ) : sorted == null ? (
            <div className="flex flex-col gap-3">
              {Array.from({ length: 4 }).map((_, i) => (
                <div
                  key={i}
                  className="animate-pulse rounded-xl border border-edge p-3"
                >
                  <div className="h-4 w-1/3 rounded bg-panel2" />
                  <div className="mt-3 h-3 w-2/3 rounded bg-panel2" />
                  <div className="mt-3 h-7 rounded-lg bg-panel2" />
                </div>
              ))}
            </div>
          ) : (
            <div className="flex flex-col gap-3">
              {sorted.map((q) => (
                <QuantItem
                  key={q.artifactName}
                  quant={q}
                  started={started.has(q.artifactName)}
                  onDownload={() => download(q)}
                />
              ))}
            </div>
          )}
        </div>
      </aside>
    </div>
  );
}
