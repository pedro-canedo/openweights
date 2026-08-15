// Identidade OpenWeights: O + W entrelaçados (barra do W corta o anel),
// como na logo original. viewBox 100×100 para o ícone do app.

/** Monograma O+W em `currentColor`. */
export function OwMark({ className = "h-7" }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 100 100"
      className={className}
      aria-hidden="true"
      preserveAspectRatio="xMidYMid meet"
    >
      <path
        fill="currentColor"
        fillRule="evenodd"
        d="M36 16a34 34 0 1 1 0 68 34 34 0 0 1 0-68zm0 14a20 20 0 1 0 0 40 20 20 0 0 0 0-40zM48 78 78 22 68 16 38 72z"
      />
      <path
        fill="currentColor"
        d="M46 74 76 18 86 24 56 80zM72 78 98 30 88 24 62 72z"
      />
    </svg>
  );
}

/** Marca horizontal: monograma + OpenWeights. Para a sidebar. */
export function OwWordmark({ className = "" }: { className?: string }) {
  return (
    <span
      className={`inline-flex select-none items-center gap-2 font-semibold tracking-tight ${className}`}
    >
      <OwMark className="h-[1.25em] w-auto shrink-0" />
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
      <OwMark className="h-[2.6em] w-auto" />
      <span>OpenWeights</span>
    </span>
  );
}
