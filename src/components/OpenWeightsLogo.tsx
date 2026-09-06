// Identidade OpenWeights.
//
// A marca é um monograma O+W: um anel prata ABERTO — o "open", o modelo que
// dá para enxergar por dentro — que desce pela esquerda e, sem emenda, vira
// uma fita serpenteando como um W, do roxo ao ciano. Do braço direito se
// soltam quatro cubos: os pesos saindo do modelo.
//
// A geometria não mora aqui: vem de brandArt.ts, gerado por
// scripts/gen_icons.py junto com os ícones do app e o favicon. Assim a marca
// da barra lateral e a do instalador não podem divergir — para mudar a
// forma, mexa no script e rode `python3 scripts/gen_icons.py`.
//
// A única liberdade que a UI toma é o anel: em vez do prata fixo do ícone,
// ele usa `currentColor`, para ficar claro no tema escuro e escuro no tema
// claro — como as duas versões da folha de identidade.

import { useId } from "react";

import { BRAND_PAINTS, BRAND_PIECES, type BrandStop } from "./brandArt";

/** O relevo do anel: em 24px ninguém vê, e no tema claro uma sombra prata
 *  clarearia em vez de escurecer. O anel da UI é chapado. */
const SEM_RELEVO = new Set(["prataBaixo"]);

/** O anel seguindo o texto, com o mesmo degradê do ícone. */
const ANEL: BrandStop[] = [
  { at: 0, color: "currentColor" },
  { at: 1, color: "color-mix(in srgb, currentColor 58%, var(--lr-bg))" },
];

/** Monograma; a altura vem do className. */
export function OwMark({ className = "h-7" }: { className?: string }) {
  const uid = useId().replace(/[^a-zA-Z0-9]/g, "");
  const url = (nome: string) => `url(#${uid}-${nome})`;

  return (
    <svg
      viewBox="0 0 100 100"
      className={className}
      aria-hidden="true"
      preserveAspectRatio="xMidYMid meet"
    >
      <defs>
        {Object.entries(BRAND_PAINTS).map(([nome, tinta]) => {
          if (!("stops" in tinta) || SEM_RELEVO.has(nome)) return null;
          const stops = nome === "prata" ? ANEL : tinta.stops;
          return (
            <linearGradient
              key={nome}
              id={`${uid}-${nome}`}
              gradientUnits="userSpaceOnUse"
              x1={tinta.x1}
              y1={tinta.y1}
              x2={tinta.x2}
              y2={tinta.y2}
            >
              {stops.map((s, i) => (
                <stop key={i} offset={s.at} stopColor={s.color} />
              ))}
            </linearGradient>
          );
        })}
      </defs>
      {BRAND_PIECES.filter((p) => !SEM_RELEVO.has(p.paint)).map((p, i) => {
        const tinta = BRAND_PAINTS[p.paint];
        const cor = "solid" in tinta ? tinta.solid : url(p.paint);
        return p.kind === "stroke" ? (
          <path
            key={i}
            d={p.d}
            fill="none"
            stroke={cor}
            strokeWidth={p.w}
            strokeLinecap="butt"
            strokeLinejoin="round"
          />
        ) : (
          <path key={i} d={p.d} fill={cor} />
        );
      })}
    </svg>
  );
}

/** O nome, com "Weights" na fita da marca — ciano à esquerda, roxo à direita,
 *  como no logotipo. */
function OwNome() {
  return (
    <span>
      Open<span className="brand-word">Weights</span>
    </span>
  );
}

/** Marca horizontal: monograma + nome. Usada na barra lateral. */
export function OwWordmark({ className = "" }: { className?: string }) {
  return (
    <span
      className={`inline-flex select-none items-center gap-2 font-semibold tracking-tight ${className}`}
    >
      <OwMark className="h-[1.45em] w-auto shrink-0" />
      <OwNome />
    </span>
  );
}

/** Lockup vertical (marca acima do nome). Usado na tela de boas-vindas. */
export function OwLockup({ className = "" }: { className?: string }) {
  return (
    <span
      className={`inline-flex select-none flex-col items-center gap-3 font-semibold tracking-tight ${className}`}
    >
      <OwMark className="h-[2.6em] w-auto" />
      <OwNome />
    </span>
  );
}
