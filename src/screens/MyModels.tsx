// Tela Meus Modelos: grade da biblioteca local + histórico de downloads
// incompletos (retomáveis depois de reiniciar o PC).

import { Fragment, useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { LocalModel } from "../lib/types";
import { deleteModel, listLocalModels } from "../lib/api";
import { formatBytes } from "../lib/format";
import { navigate } from "../lib/nav";
import { autorDoRepo } from "../lib/authorAvatars";
import AuthorAvatar from "../components/discover/AuthorAvatar";
import IncompleteDownloads from "../components/models/IncompleteDownloads";
import TunePanel from "../components/models/TunePanel";
import { Page } from "../components/ui/Shell";
import Icon from "../components/ui/Icon";

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

  // O modelo solto (arrastado para a pasta) não tem repositório, logo não tem
  // autor: ali o quadrado da foto seria um "?" ocupando lugar à toa.
  const autor = autorDoRepo(model.repoId);

  return (
    <article className="model-library-card flex min-w-0 flex-col rounded-2xl border border-edge bg-panel p-6">
      <div className="flex min-w-0 items-start gap-3">
        {autor && (
          <AuthorAvatar author={autor} size={44} className="rounded-xl" />
        )}
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-start gap-2">
            <p
              className="min-w-0 flex-1 break-words text-base font-semibold leading-relaxed text-ink"
              title={model.name}
            >
              {model.name.replace(/\.gguf$/i, "")}
            </p>
            {model.quantLabel && (
              <span className="shrink-0 rounded-md bg-accent/15 px-1.5 py-0.5 font-mono text-[11px] font-medium text-accent">
                {model.quantLabel}
              </span>
            )}
          </div>
          <p className="mt-1 truncate text-xs text-dim" title={model.repoId}>
            {model.repoId || t("models.loose")}
          </p>
        </div>
      </div>

      <div className="my-5 flex items-center gap-2 border-y border-edge/70 py-4 text-xs text-dim"><Icon name="disk" /><span className="font-medium tabular-nums text-ink">{formatBytes(model.totalBytes)}</span><span className="ml-auto flex items-center gap-1.5"><Icon name="check" className="h-3.5 w-3.5 text-ok" />{t("interface.localFile")}</span></div>

      <div className="mt-auto">
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
              className="flex items-center gap-2 rounded-lg bg-accent px-4 py-2.5 text-sm font-medium text-white transition-opacity hover:opacity-90"
            >
              {t("models.chatWith")}
              <Icon name="arrow-right" />
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
              className="ml-auto rounded-lg px-2 py-1.5 text-xs font-medium text-dim transition-colors hover:bg-bad/10 hover:text-bad"
            >
              {t("models.delete")}
            </button>
          </div>
        )}
      </div>
    </article>
  );
}

export default function MyModels() {
  const { t } = useTranslation();
  const [models, setModels] = useState<LocalModel[] | null>(null);
  const [query, setQuery] = useState("");
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

  const visibleModels = models?.filter(m => `${m.name} ${m.repoId} ${m.quantLabel ?? ""}`.toLocaleLowerCase().includes(query.toLocaleLowerCase().trim()));

  return (
    <Page icon="layers" title={t("models.title")} subtitle={t("interface.modelsSubtitle")}
      actions={<button onClick={() => navigate("discover")} className="flex items-center gap-2 rounded-xl bg-accent px-4 py-2.5 text-sm font-medium text-white hover:opacity-90"><Icon name="download" />{t("interface.addModels")}</button>}>

      <div className="library-toolbar">
        <div className="flex flex-wrap items-center gap-4 text-xs text-dim"><span>{t("interface.modelCount", { count: models?.length ?? 0 })}</span><span className="flex items-center gap-2"><Icon name="disk" />{models ? formatBytes(models.reduce((sum, m) => sum + m.totalBytes, 0)) : "—"}</span></div>
        <label className="flex w-full items-center gap-2 rounded-xl border border-edge bg-panel px-3 py-2.5 sm:w-80"><Icon name="search" className="h-4 w-4 text-dim" /><input aria-label={t("interface.searchModels")} placeholder={t("interface.searchModels")} value={query} onChange={e => setQuery(e.target.value)} className="min-w-0 flex-1 bg-transparent text-sm outline-none" /></label>
      </div>

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
          <div className="grid grid-cols-1 gap-5 lg:grid-cols-2">
            {visibleModels?.length === 0 && <p role="status" className="col-span-full rounded-xl border border-dashed border-edge p-10 text-center text-sm text-dim">{t("interface.noModels")}</p>}
            {visibleModels?.map((m) => (
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
    </Page>
  );
}
