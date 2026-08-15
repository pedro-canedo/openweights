// Tela Meus Modelos: biblioteca local com tamanho, quantização e origem,
// atalho para conversar no Chat e exclusão com confirmação inline.

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { LocalModel } from "../lib/types";
import { deleteModel, listLocalModels } from "../lib/api";
import { formatBytes } from "../lib/format";
import { navigate } from "../lib/nav";

function ModelRow({
  model,
  onDeleted,
}: {
  model: LocalModel;
  onDeleted: () => void;
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
    <div className="rounded-xl border border-edge bg-panel p-5">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-2">
            <span className="truncate text-sm font-medium text-ink">
              {model.name}
            </span>
            {model.quantLabel && (
              <span className="shrink-0 rounded-md bg-accent/15 px-1.5 py-0.5 font-mono text-[11px] font-medium text-accent">
                {model.quantLabel}
              </span>
            )}
          </div>
          <p className="mt-1 truncate text-xs text-dim">
            {model.repoId || t("models.loose")}
            {" · "}
            <span className="tabular-nums">
              {formatBytes(model.totalBytes)}
            </span>
          </p>
        </div>

        {confirming ? (
          <div className="flex shrink-0 items-center gap-2">
            <span className="text-xs text-bad">
              {t("models.deleteConfirm", { name: model.name })}
            </span>
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
        ) : (
          <div className="flex shrink-0 items-center gap-2">
            <button
              onClick={() => navigate("chat", { chatModel: model.name })}
              className="rounded-lg bg-accent px-4 py-1.5 text-xs font-medium text-white transition-opacity hover:opacity-90"
            >
              {t("models.chatWith")}
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
    <div className="mx-auto max-w-4xl px-8 py-8">
      <h1 className="text-xl font-semibold">{t("models.title")}</h1>

      <div className="mt-6">
        {models == null ? (
          <div className="flex flex-col gap-3">
            {Array.from({ length: 2 }).map((_, i) => (
              <div
                key={i}
                className="animate-pulse rounded-xl border border-edge bg-panel p-5"
              >
                <div className="h-4 w-2/5 rounded bg-panel2" />
                <div className="mt-3 h-3 w-3/5 rounded bg-panel2" />
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
          <div className="flex flex-col gap-3">
            {models.map((m) => (
              <ModelRow key={m.primaryPath} model={m} onDeleted={refresh} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
