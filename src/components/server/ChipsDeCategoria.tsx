// Os chips de categoria do catálogo de flags: um clique lista todas as flags
// daquela família (memória, contexto, especulação…), sem precisar saber o nome.

import { useTranslation } from "react-i18next";

export default function ChipsDeCategoria({
  categorias,
  ativa,
  aoEscolher,
}: {
  categorias: [string, number][];
  ativa: string | null;
  aoEscolher: (categoria: string | null) => void;
}) {
  const { t } = useTranslation();
  if (categorias.length < 2) return null;
  return (
    <div role="group" aria-label={t("flags.categoriesLabel")} className="flex flex-wrap gap-1.5">
      {categorias.map(([id, n]) => (
        <button
          key={id}
          type="button"
          aria-pressed={ativa === id}
          onClick={() => aoEscolher(ativa === id ? null : id)}
          className={`rounded-full border px-2.5 py-1 text-[11px] transition-colors ${
            ativa === id
              ? "border-accent-ink bg-accent/15 text-ink"
              : "border-edge text-dim hover:border-edge-strong hover:text-ink"
          }`}
        >
          {t(`flags.categories.${id}`, id)} <span className="tabular-nums opacity-70">{n}</span>
        </button>
      ))}
    </div>
  );
}
