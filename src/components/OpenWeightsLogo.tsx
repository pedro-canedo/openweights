// Identidade OpenWeights: C aberto à direita + visto (branco / cinza).

const MARK_C =
  "M56.1 30.9A27 27 0 1 0 58.1 69.1L68.5 82.2 79.8 52.4";
const MARK_V = "M79.8 52.4 91.2 27.6";

/** Monograma C+visto em `currentColor`. */
export function OwMark({ className = "h-7" }: { className?: string }) {
  return (
    <svg
      viewBox="12 14 80 78"
      className={className}
      aria-hidden="true"
      preserveAspectRatio="xMidYMid meet"
    >
      <path
        fill="none"
        stroke="currentColor"
        strokeWidth="13"
        strokeLinecap="butt"
        strokeLinejoin="miter"
        strokeMiterlimit={2.4}
        d={MARK_C}
      />
      <path
        fill="none"
        stroke="currentColor"
        strokeWidth="13"
        strokeLinecap="butt"
        opacity="0.4"
        d={MARK_V}
      />
    </svg>
  );
}

/** Marca horizontal: monograma + OpenWeights. Para a sidebar. */
export function OwWordmark({ className = "" }: { className?: string }) {
  return (
    <span
      className={`inline-flex select-none items-center gap-2.5 font-semibold tracking-tight ${className}`}
    >
      <OwMark className="h-[1.15em] w-auto shrink-0" />
      <span>OpenWeights</span>
    </span>
  );
}

/** Lockup vertical (marca acima do nome). */
export function OwLockup({ className = "" }: { className?: string }) {
  return (
    <span
      className={`inline-flex select-none flex-col items-center gap-2.5 font-semibold tracking-tight ${className}`}
    >
      <OwMark className="h-[2.4em] w-auto" />
      <span>OpenWeights</span>
    </span>
  );
}
