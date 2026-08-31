// Uma linha da lista de descoberta.
//
// O card antigo empilhava seis estatísticas por modelo e ocupava a largura
// inteira da tela: cabiam sete resultados de uma vez, e comparar dois exigia
// rolar. Aqui cada modelo é uma linha — nome, autor, o que ele sabe fazer e
// quando mudou — e os números todos vivem no painel de detalhe, que fala de
// UM modelo por vez. A lista serve para escolher; o detalhe, para decidir.

import { useTranslation } from "react-i18next";
import type { ModelSummary } from "../../lib/types";
import { formatAgo, formatCount, formatParams } from "../../lib/format";
import AuthorAvatar from "./AuthorAvatar";
import CapBadges from "./CapBadges";

export default function ModelListItem({
  model,
  selected,
  onSelect,
}: {
  model: ModelSummary;
  selected: boolean;
  onSelect: () => void;
}) {
  const { t, i18n } = useTranslation();
  const quando = model.updatedAt
    ? formatAgo(i18n.language, new Date(model.updatedAt).getTime())
    : null;

  return (
    <button
      onClick={onSelect}
      aria-current={selected}
      className={`flex w-full items-center gap-3 rounded-xl border px-3 py-2.5 text-left transition-colors ${
        selected
          ? "border-accent bg-accent/10"
          : "border-transparent hover:border-edge hover:bg-panel2/60"
      }`}
    >
      <AuthorAvatar author={model.author} size={34} className="rounded-lg" />

      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-2">
          <span className="min-w-0 flex-1 truncate text-[13px] font-medium text-ink">
            {model.name}
          </span>
          {model.gated && (
            <span
              title={t("discover.gated")}
              className="shrink-0 rounded-full bg-warn/15 px-1.5 py-0.5 text-[10px] font-medium text-warn"
            >
              {t("discover.gatedShort")}
            </span>
          )}
        </span>

        <span className="mt-1 flex items-center gap-2 text-[11px] text-dim">
          <span className="truncate">{model.author}</span>
          {model.paramsTotal != null && (
            <>
              <span aria-hidden>·</span>
              <span className="shrink-0 tabular-nums">
                {formatParams(model.paramsTotal)}
              </span>
            </>
          )}
          <span aria-hidden>·</span>
          <span className="shrink-0 tabular-nums">
            {formatCount(model.downloads)}
          </span>
        </span>
      </span>

      <span className="flex shrink-0 flex-col items-end gap-1">
        {quando && <span className="text-[10px] text-dim">{quando}</span>}
        <span className="flex items-center gap-1">
          <CapBadges caps={model.caps} />
        </span>
      </span>
    </button>
  );
}
