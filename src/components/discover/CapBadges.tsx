// Os três selos de capacidade de um modelo: visão, ferramentas, raciocínio.
//
// Cada um tem uma fonte verificável do lado do backend (`ModelCaps`) — a
// etiqueta do Hub para visão, o chat template para os outros dois. Nenhum
// deles sai do nome do modelo, então o selo pode ser lido como promessa.
//
// Na lista aparecem só os ícones (o espaço é de uma linha); no detalhe, com
// rótulo. É o mesmo componente para não haver dois vocabulários visuais para
// a mesma informação.

import { useTranslation } from "react-i18next";
import type { ModelCaps } from "../../lib/types";

const ICONES = {
  vision:
    "M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7zm10 3a3 3 0 100-6 3 3 0 000 6z",
  tools:
    "M14.7 6.3a4 4 0 01-5.4 5.4l-5 5a1.5 1.5 0 002.1 2.1l5-5a4 4 0 015.4-5.4l-2.6 2.6 2.1 2.1 2.6-2.6a4 4 0 01-4.2-4.2z",
  reasoning: "M9 18h6M10 21h4M12 3a6 6 0 00-3.5 10.9V16h7v-2.1A6 6 0 0012 3z",
};

type Cap = keyof typeof ICONES;

/** As capacidades presentes, na ordem em que a tela sempre as mostra. */
function presentes(caps: ModelCaps): Cap[] {
  const todas: Cap[] = ["vision", "tools", "reasoning"];
  return todas.filter((c) => caps[c]);
}

export default function CapBadges({
  caps,
  withLabel = false,
}: {
  caps: ModelCaps;
  /** Detalhe: ícone + palavra. Lista: só o ícone, com `title`. */
  withLabel?: boolean;
}) {
  const { t } = useTranslation();
  const itens = presentes(caps);
  if (itens.length === 0) return null;

  return (
    <>
      {itens.map((c) => (
        <span
          key={c}
          title={t(`discover.caps.${c}`)}
          className={`inline-flex items-center gap-1 rounded-full border border-edge bg-panel2/60 text-[11px] text-dim ${
            withLabel ? "px-2 py-0.5" : "p-1"
          }`}
        >
          <svg
            className="h-3.5 w-3.5 shrink-0"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.7"
            strokeLinecap="round"
            strokeLinejoin="round"
            viewBox="0 0 24 24"
          >
            <path d={ICONES[c]} />
          </svg>
          {withLabel && t(`discover.caps.${c}`)}
        </span>
      ))}
    </>
  );
}
