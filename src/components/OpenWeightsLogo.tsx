// Identidade OpenWeights.
//
// A marca é um "W" de duas metades separadas por uma fenda no vértice
// central: a primeira sólida, a segunda mais clara. As duas intensidades
// evocam pesos de valores diferentes; a fenda é o "open" — o modelo aberto,
// que dá para enxergar por dentro.
//
// Traço reto com ponta cortada em reta e junção arredondada embaixo: continua
// legível em 16px na barra de tarefas. A mesma geometria vive em
// scripts/gen_icons.py, que gera os ícones do app.

/** Primeira metade do W (desce e sobe até o vértice central). */
const STROKE_A = "M19 23 L35 77 L48 47";
/** Segunda metade, saindo da fenda. */
const STROKE_B = "M57 47 L69 77 L85 23";

/** Monograma em `currentColor`; a altura vem do className. */
export function OwMark({ className = "h-7" }: { className?: string }) {
  return (
    <svg
      viewBox="10 14 84 72"
      className={className}
      aria-hidden="true"
      preserveAspectRatio="xMidYMid meet"
    >
      <path
        fill="none"
        stroke="currentColor"
        strokeWidth="14"
        strokeLinecap="butt"
        strokeLinejoin="round"
        d={STROKE_A}
      />
      <path
        fill="none"
        stroke="currentColor"
        strokeWidth="14"
        strokeLinecap="butt"
        strokeLinejoin="round"
        opacity="0.5"
        d={STROKE_B}
      />
    </svg>
  );
}

/** Marca horizontal: monograma + nome. Usada na barra lateral. */
export function OwWordmark({ className = "" }: { className?: string }) {
  return (
    <span
      className={`inline-flex select-none items-center gap-2 font-semibold tracking-tight ${className}`}
    >
      <OwMark className="h-[1.05em] w-auto shrink-0" />
      <span>
        Open<span className="text-dim">Weights</span>
      </span>
    </span>
  );
}

/** Lockup vertical (marca acima do nome). Usado na tela de boas-vindas. */
export function OwLockup({ className = "" }: { className?: string }) {
  return (
    <span
      className={`inline-flex select-none flex-col items-center gap-3 font-semibold tracking-tight ${className}`}
    >
      <OwMark className="h-[2.2em] w-auto" />
      <span>
        Open<span className="text-dim">Weights</span>
      </span>
    </span>
  );
}
