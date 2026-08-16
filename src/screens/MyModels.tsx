// Tela Meus Modelos: grade da biblioteca local + histórico de downloads
// incompletos (retomáveis depois de reiniciar o PC).

import { Fragment, useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { LocalModel } from "../lib/types";
import { deleteModel, listLocalModels } from "../lib/api";
import { formatBytes } from "../lib/format";
import { navigate } from "../lib/nav";
import IncompleteDownloads from "../components/models/IncompleteDownloads";
import TunePanel from "../components/models/TunePanel";

function ModelCard({
  model,
  onDeleted,
  tuning,
  onToggleTune,
}: {
  model: LocalModel;
  onDeleted: () => void;
  /** O painel de ajuste deste modelo está aberto? */
  tuning: boolean;
  onToggleTune: () => void;
}) {
  const { t } = useTranslation();
  const [confirming, setConfirming] = useState(false);
  const [deleting, setDeleting] = useState(false);

  const remove = () => {
    setDeleting(true);
    deleteModel(model.repoId, model.name)
      .catch(console.error)
      .finally(onDeleted);
  };

  return (
    <div className="flex flex-col rounded-xl border border-edge bg-panel p-4">
      <div className="flex min-w-0 items-start gap-2">
        <p className="min-w-0 flex-1 truncate text-sm font-medium text-ink" title={model.name}>
          {model.name}
        </p>
        {model.quantLabel && (
          <span className="shrink-0 rounded-md bg-accent/15 px-1.5 py-0.5 font-mono text-[11px] font-medium text-accent">
            {model.quantLabel}
          </span>
        )}
      </div>
      <p className="mt-1 truncate text-xs text-dim" title={model.repoId}>
        {model.repoId || t("models.loose")}
        {" · "}
        <span className="tabular-nums">{formatBytes(model.totalBytes)}</span>
      </p>

      <div className="mt-auto pt-4">
        {confirming ? (
          <div className="flex flex-col gap-2">
            <span className="text-xs text-bad">
              {t("models.deleteConfirm", { name: model.name })}
            </span>
            <div className="flex gap-2">
              <button
                onClick={remove}
                disabled={deleting}
                className="rounded-lg bg-bad px-3 py-1.5 text-xs font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-40"
              >
                {t("models.delete")}
              </button>
              <button
                onClick={() => setConfirming(false)}
                disabled={deleting}
                className="rounded-lg border border-edge px-3 py-1.5 text-xs font-medium text-dim transition-colors hover:text-ink disabled:opacity-40"
              >
                {t("common.cancel")}
              </button>
            </div>
          </div>
        ) : (
          <div className="flex flex-wrap gap-2">
            <button
              onClick={() => navigate("chat", { chatModel: model.name })}
              className="rounded-lg bg-accent px-3 py-1.5 text-xs font-medium text-white transition-opacity hover:opacity-90"
            >
              {t("models.chatWith")}
            </button>
            <button
              onClick={onToggleTune}
              aria-expanded={tuning}
              className="rounded-lg border border-edge px-3 py-1.5 text-xs font-medium text-dim transition-colors hover:border-accent hover:text-ink"
            >
              {t("tune.open")}
            </button>
            <button
              onClick={() => setConfirming(true)}
              className="rounded-lg border border-edge px-3 py-1.5 text-xs font-medium text-dim transition-colors hover:border-bad/60 hover:text-bad"
            >
              {t("models.delete")}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

export default function MyModels() {
  const { t } = useTranslation();
  const [models, setModels] = useState<LocalModel[] | null>(null);
  // Qual modelo está com o painel de ajuste aberto. Mora aqui, e não no
  // cartão, porque o painel ocupa a LINHA inteira da grade: dentro de uma
  // célula ele nasce espremido e transborda por cima do cartão vizinho.
  const [tuning, setTuning] = useState<string | null>(null);

  const refresh = useCallback(() => {
    listLocalModels()
      .then(setModels)
      .catch((err) => {
        console.error(err);
        setModels([]);
      });
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  return (
    <div className="mx-auto max-w-5xl px-8 py-8">
      <h1 className="text-xl font-semibold">{t("models.title")}</h1>

      <div className="mt-6">
        {models == null ? (
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-3">
            {Array.from({ length: 3 }).map((_, i) => (
              <div
                key={i}
                className="animate-pulse rounded-xl border border-edge bg-panel p-4"
              >
                <div className="h-4 w-3/5 rounded bg-panel2" />
                <div className="mt-3 h-3 w-4/5 rounded bg-panel2" />
                <div className="mt-6 h-7 w-24 rounded-lg bg-panel2" />
              </div>
            ))}
          </div>
        ) : models.length === 0 ? (
          <div className="rounded-xl border border-dashed border-edge p-10 text-center">
            <p className="text-sm text-dim">{t("models.empty")}</p>
            <button
              onClick={() => navigate("discover")}
              className="mt-4 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-white transition-opacity hover:opacity-90"
            >
              {t("nav.discover")}
            </button>
          </div>
        ) : (
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-3">
            {models.map((m) => (
              <Fragment key={m.primaryPath}>
                <ModelCard
                  model={m}
                  onDeleted={refresh}
                  tuning={tuning === m.name}
                  onToggleTune={() =>
                    setTuning((atual) => (atual === m.name ? null : m.name))
                  }
                />
                {/* `col-span-full` empurra o painel para uma linha só dele:
                    é o que dá espaço às quatro propostas lado a lado. */}
                {tuning === m.name && (
                  <div className="col-span-full">
                    <TunePanel model={m.name} onClose={() => setTuning(null)} />
                  </div>
                )}
              </Fragment>
            ))}
          </div>
        )}
      </div>

      <IncompleteDownloads onFinished={refresh} />
    </div>
  );
}
