// Card de resultado da busca na tela Descobrir: nome, downloads, curtidas,
// parâmetros, arquitetura, contexto e badge de licença (gated).

import { useTranslation } from "react-i18next";
import type { ModelSummary } from "../../lib/types";
import { formatCount, formatParams } from "../../lib/format";

function Stat({ icon, text }: { icon: string; text: string }) {
  return (
    <span className="inline-flex items-center gap-1 text-xs text-dim">
      <svg
        className="h-3.5 w-3.5 shrink-0"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
        viewBox="0 0 24 24"
      >
        <path d={icon} />
      </svg>
      {text}
    </span>
  );
}

const ICONS = {
  download: "M4 16v2a2 2 0 002 2h12a2 2 0 002-2v-2M7 10l5 5 5-5M12 15V3",
  heart:
    "M4.318 6.318a4.5 4.5 0 000 6.364L12 20.364l7.682-7.682a4.5 4.5 0 00-6.364-6.364L12 7.636l-1.318-1.318a4.5 4.5 0 00-6.364 0z",
  params: "M4 6h16M4 12h16M4 18h7",
  context: "M8 7V3m8 4V3M5 11h14M5 21h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z",
};

export default function ModelCard({
  model,
  onClick,
}: {
  model: ModelSummary;
  onClick: () => void;
}) {
  const { t, i18n } = useTranslation();

  const date = model.updatedAt
    ? new Date(model.updatedAt).toLocaleDateString(i18n.language, {
        day: "2-digit",
        month: "short",
        year: "numeric",
      })
    : null;

  return (
    <button
      onClick={onClick}
      className="w-full rounded-xl border border-edge bg-panel p-5 text-left transition-colors hover:border-accent/60"
    >
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm">
            <span className="text-dim">{model.author}/</span>
            <span className="font-medium text-ink">{model.name}</span>
          </div>
        </div>
        {model.gated && (
          <span className="shrink-0 rounded-full bg-warn/15 px-2 py-0.5 text-[11px] font-medium text-warn">
            {t("discover.gated")}
          </span>
        )}
        {date && (
          <span className="shrink-0 text-[11px] text-dim">{date}</span>
        )}
      </div>

      <div className="mt-3 flex flex-wrap items-center gap-x-4 gap-y-1.5">
        <Stat
          icon={ICONS.download}
          text={`${formatCount(model.downloads)} ${t("discover.downloads")}`}
        />
        <Stat
          icon={ICONS.heart}
          text={`${formatCount(model.likes)} ${t("discover.likes")}`}
        />
        {model.paramsTotal != null && (
          <Stat
            icon={ICONS.params}
            text={`${formatParams(model.paramsTotal)} ${t("discover.params")}`}
          />
        )}
        {model.architecture && (
          <span className="rounded-md bg-panel2 px-1.5 py-0.5 text-[11px] text-dim">
            {model.architecture}
          </span>
        )}
        {model.contextLength != null && (
          <Stat
            icon={ICONS.context}
            text={`${t("discover.context")} ${formatCount(model.contextLength)} tokens`}
          />
        )}
      </div>
    </button>
  );
}
