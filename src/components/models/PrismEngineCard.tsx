// O cartão "este modelo precisa do motor da PrismML": o que é, quanto pesa e
// um botão que instala. Aparece no chat (no lugar da bolha de erro), na
// biblioteca (num modelo Bonsai sem o motor) e some sozinho quando o motor
// chega. O progresso vem do `prismStore`, então começar a instalação num
// lugar e olhar em outro mostra a mesma barra.

import { useEffect, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { formatBytes } from "../../lib/format";
import { PRISM_APPROX_BYTES, carregarPrism, instalarPrism, prismStore } from "../../lib/prism";

export default function PrismEngineCard({
  onInstalled,
  compact = false,
}: {
  /** Chamado quando a instalação termina bem — o chat reenvia a mensagem. */
  onInstalled?: () => void;
  compact?: boolean;
}) {
  const { t } = useTranslation();
  const snap = useSyncExternalStore(prismStore.subscribe, prismStore.get);

  useEffect(() => {
    void carregarPrism();
  }, []);

  if (snap.state?.installed && !snap.installing) return null;

  const instalar = () => {
    void instalarPrism()
      .then(() => onInstalled?.())
      .catch(() => {
        /* o erro já está no store */
      });
  };

  const pct =
    snap.progress?.kind === "progress" && snap.progress.totalBytes > 0
      ? Math.min(100, (snap.progress.receivedBytes / snap.progress.totalBytes) * 100)
      : null;

  return (
    <div
      className={`rounded-xl border border-warn/40 bg-warn/5 ${compact ? "px-3 py-2" : "px-4 py-3"}`}
      role="status"
    >
      <p className="text-sm text-ink">{t("models.prismMissing")}</p>
      {!compact && <p className="mt-1 text-[12px] text-dim">{t("models.prismWhat")}</p>}
      <div className="mt-2 flex flex-wrap items-center gap-3">
        {!snap.installing ? (
          <button
            type="button"
            onClick={instalar}
            className="rounded-lg bg-accent px-3 py-1.5 text-xs font-medium text-white transition-opacity hover:opacity-90"
          >
            {t("models.prismInstall", { size: formatBytes(PRISM_APPROX_BYTES) })}
          </button>
        ) : (
          <div className="flex min-w-0 flex-1 items-center gap-2 text-[12px] text-dim">
            <span className="h-1.5 w-32 overflow-hidden rounded-full bg-panel2">
              <span
                className={`block h-full rounded-full bg-accent transition-[width] duration-300 ${pct == null ? "w-1/3 animate-pulse" : ""}`}
                style={pct == null ? undefined : { width: `${pct}%` }}
              />
            </span>
            <span className="truncate">
              {snap.progress?.kind === "extracting"
                ? t("settings.engine.extracting", { asset: snap.progress.asset })
                : t("models.prismInstalling")}
              {pct != null ? ` · ${pct.toFixed(0)}%` : ""}
            </span>
          </div>
        )}
      </div>
      {snap.error && <p className="mt-2 text-[12px] text-bad">{snap.error}</p>}
    </div>
  );
}
