// Identidade OpenWeights.
//
// A marca é um monograma O+W: um anel ABERTO embaixo — o "open", o modelo que
// dá para enxergar por dentro — cuja perna esquerda desce e vira o primeiro
// traço de um "W". O braço direito do W sobe alto, saindo do anel. Cortando
// os dois na diagonal, um raio: o peso atravessando a máquina.
//
// A mesma geometria vive em scripts/gen_icons.py, que gera os ícones do app,
// o favicon e os arquivos de marca. Quem mexer aqui precisa mexer lá.
//
// O prata e o raio são da marca, não do tema: o anel e o W usam
// `currentColor` para acompanhar o texto onde estiverem, e só o raio carrega
// a cor fixa — é ele que identifica a marca em qualquer fundo.

/** Anel aberto: começa na perna esquerda, sobe e desce até a perna direita. */
const RING = "M35.3 51.4 A16.7 16.7 0 1 1 60.3 50.7";
/** O W, começando onde a perna esquerda do anel termina. */
const W = "M29.5 53 L42 79 L51 55 L60 79 L80 40";
/** O raio, uma agulha das pontas finas ao meio grosso. */
const BOLT = "M12 76 L51.1 58.9 L89 39 L49.9 56.1 Z";
const STROKE = 7.6;

/** Monograma; a altura vem do className. */
export function OwMark({ className = "h-7" }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 100 100"
      className={className}
      aria-hidden="true"
      preserveAspectRatio="xMidYMid meet"
    >
      <g
        fill="none"
        stroke="currentColor"
        strokeWidth={STROKE}
        strokeLinecap="butt"
        strokeLinejoin="round"
      >
        <path d={RING} />
        <path d={W} />
      </g>
      {/* O raio vem por cima e corta as duas letras, como no ícone. */}
      <path d={BOLT} fill="#6d6dff" />
    </svg>
  );
}

/** Marca horizontal: monograma + nome. Usada na barra lateral. */
export function OwWordmark({ className = "" }: { className?: string }) {
  return (
    <span
      className={`inline-flex select-none items-center gap-2 font-semibold tracking-tight ${className}`}
    >
      <OwMark className="h-[1.35em] w-auto shrink-0" />
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
      <OwMark className="h-[2.6em] w-auto" />
      <span>
        Open<span className="text-dim">Weights</span>
      </span>
    </span>
  );
}
