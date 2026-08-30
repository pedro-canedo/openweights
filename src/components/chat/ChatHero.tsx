// Estado vazio do Chat: hero central com atalhos, no estilo de
// uma tela de entrada (saudação + cards + brilho).

import { useTranslation } from "react-i18next";
import { HarnessHeroCard } from "./HarnessCta";

const CARDS = [
  {
    key: "write" as const,
    promptKey: "chat.hero.writePrompt",
    titleKey: "chat.hero.write",
    art: "write" as const,
  },
  {
    key: "code" as const,
    promptKey: "chat.hero.codePrompt",
    titleKey: "chat.hero.code",
    art: "code" as const,
  },
  {
    key: "explain" as const,
    promptKey: "chat.hero.explainPrompt",
    titleKey: "chat.hero.explain",
    art: "explain" as const,
  },
];

function CardArt({ kind }: { kind: "write" | "code" | "explain" }) {
  if (kind === "write") {
    return (
      <div className="pointer-events-none absolute inset-0 overflow-hidden">
        <div className="absolute -top-6 -right-4 h-28 w-28 rotate-12 rounded-2xl bg-white/15 blur-[1px]" />
        <div className="absolute top-8 right-6 h-16 w-20 -rotate-6 rounded-xl bg-white/20" />
        <div className="absolute right-10 bottom-5 h-2 w-10 rounded-full bg-white/40" />
        <div className="absolute right-10 bottom-9 h-1.5 w-7 rounded-full bg-white/25" />
      </div>
    );
  }
  if (kind === "code") {
    return (
      <div className="pointer-events-none absolute inset-0 overflow-hidden">
        <div className="absolute top-5 right-5 flex h-20 w-24 flex-col justify-center gap-1.5 rounded-xl bg-black/25 px-3 font-mono text-[10px] leading-none text-white/80">
          <span>{"{ }"}</span>
          <span className="opacity-70">fn main()</span>
          <span className="opacity-50">ok</span>
        </div>
        <div className="absolute -right-3 -bottom-4 h-16 w-16 rounded-full bg-cyan-200/30 blur-md" />
      </div>
    );
  }
  return (
    <div className="pointer-events-none absolute inset-0 overflow-hidden">
      <div className="absolute top-4 right-4 h-24 w-28 rounded-2xl bg-gradient-to-br from-amber-200/40 to-rose-300/20" />
      <div className="absolute right-8 bottom-4 h-10 w-10 rounded-full bg-white/25 blur-[2px]" />
      <div className="absolute top-8 right-16 h-3 w-3 rounded-full bg-white/70" />
    </div>
  );
}

export default function ChatHero({
  onPick,
}: {
  onPick: (prompt: string) => void;
}) {
  const { t } = useTranslation();

  return (
    <div className="relative flex min-h-0 flex-1 flex-col items-center justify-center overflow-hidden px-6">
      <div className="chat-hero-glow pointer-events-none absolute inset-0" />

      <div className="relative z-10 flex w-full max-w-2xl flex-col items-center">
        <div className="mb-5 flex h-10 w-10 items-center justify-center text-ink">
          <svg viewBox="0 0 24 24" className="h-7 w-7 drop-shadow-[0_0_12px_rgba(255,255,255,0.35)]">
            <path
              fill="currentColor"
              d="M12 1.4l1.55 7.2 7.2 1.55-7.2 1.55L12 18.9l-1.55-7.2-7.2-1.55 7.2-1.55L12 1.4z"
            />
          </svg>
        </div>

        <p className="text-[13px] text-dim">{t("chat.hero.welcome")}</p>
        <h1 className="mt-1 text-center text-[42px] leading-tight font-semibold tracking-tight text-ink">
          {t("chat.hero.headline")}
        </h1>

        <div className="mt-8 grid w-full grid-cols-3 gap-3">
          {CARDS.map((card) => (
            <button
              key={card.key}
              type="button"
              onClick={() => onPick(t(card.promptKey))}
              className={`group relative h-[118px] overflow-hidden rounded-2xl p-3.5 text-left shadow-lg transition-transform duration-200 hover:-translate-y-0.5 ${
                card.key === "write"
                  ? "bg-gradient-to-br from-violet-600 via-indigo-500 to-sky-500"
                  : card.key === "code"
                    ? "bg-gradient-to-br from-sky-500 via-cyan-400 to-teal-400"
                    : "bg-gradient-to-br from-orange-500 via-rose-400 to-pink-500"
              }`}
            >
              <CardArt kind={card.art} />
              <span className="relative z-10 block max-w-[70%] text-[13px] leading-snug font-medium text-white drop-shadow-sm">
                {t(card.titleKey)}
              </span>
            </button>
          ))}
        </div>

        {/* O modo agente virou o DeepSeek Harness — o convite mora aqui. */}
        <HarnessHeroCard />
      </div>
    </div>
  );
}
