// Card de resultado da busca na tela Descobrir: avatar do autor (HF),
// nome, downloads, curtidas, parâmetros, arquitetura, contexto e badge
// de licença (gated).

import { useState } from "react";
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

const AVATAR_TONES = [
  "bg-sky-500/20 text-sky-300",
  "bg-violet-500/20 text-violet-300",
  "bg-emerald-500/20 text-emerald-300",
  "bg-amber-500/20 text-amber-300",
  "bg-rose-500/20 text-rose-300",
  "bg-cyan-500/20 text-cyan-300",
  "bg-orange-500/20 text-orange-300",
  "bg-fuchsia-500/20 text-fuchsia-300",
];

function avatarTone(name: string): string {
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) >>> 0;
  return AVATAR_TONES[h % AVATAR_TONES.length];
}

function initialsOf(author: string): string {
  const cleaned = author.replace(/[-_.]+/g, " ").trim();
  const parts = cleaned.split(/\s+/).filter(Boolean);
  if (parts.length >= 2) {
    return (parts[0][0] + parts[1][0]).toUpperCase();
  }
  return (cleaned.slice(0, 2) || "?").toUpperCase();
}

/** Foto do autor/org no Hub (`/{author}.png`); iniciais se falhar. */
export function AuthorAvatar({
  author,
  size = 40,
}: {
  author: string;
  size?: number;
}) {
  const [failed, setFailed] = useState(!author);
  const src = author
    ? `https://huggingface.co/${encodeURIComponent(author)}.png`
    : "";

  return (
    <span
      className={`relative inline-flex shrink-0 items-center justify-center overflow-hidden rounded-xl ${avatarTone(author || "?")}`}
      style={{ width: size, height: size }}
      aria-hidden
    >
      <span className="text-[11px] font-semibold tracking-wide">
        {initialsOf(author || "?")}
      </span>
      {!failed && (
        <img
          src={src}
          alt=""
          width={size}
          height={size}
          loading="lazy"
          decoding="async"
          referrerPolicy="no-referrer"
          className="absolute inset-0 h-full w-full object-cover"
          onError={() => setFailed(true)}
        />
      )}
    </span>
  );
}

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
      className="w-full rounded-xl border border-edge bg-panel p-4 text-left transition-colors hover:border-accent/60"
    >
      <div className="flex items-start gap-3.5">
        <AuthorAvatar author={model.author} />
        <div className="min-w-0 flex-1">
          <div className="flex items-start gap-3">
            <div className="min-w-0 flex-1 truncate text-sm">
              <span className="text-dim">{model.author}/</span>
              <span className="font-medium text-ink">{model.name}</span>
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

          <div className="mt-2.5 flex flex-wrap items-center gap-x-4 gap-y-1.5">
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
        </div>
      </div>
    </button>
  );
}
