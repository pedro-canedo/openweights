// Identidade visual do Rift: o "I" do wordmark é a FENDA (rift) — um traço
// vertical irregular. Mesmo desenho dos ícones do app (scripts/gen_icons.py).

/** Só a fenda, em `currentColor`. Altura controlada via className. */
export function RiftMark({ className = "h-6" }: { className?: string }) {
  return (
    <svg
      viewBox="38 10 24 84"
      className={className}
      aria-hidden="true"
      preserveAspectRatio="xMidYMid meet"
    >
      <path
        d="M49.4 14 L44 30 L51.4 44 L43.7 60 L50.9 74 L47.4 86
           L48.6 86 L54.1 74 L48.3 60 L55.6 44 L47 30 L50.6 14 Z"
        fill="currentColor"
      />
    </svg>
  );
}

/** Wordmark R|FT com a fenda no lugar do I. Escala com o font-size. */
export function RiftWordmark({ className = "" }: { className?: string }) {
  return (
    <span
      className={`inline-flex select-none items-center font-semibold uppercase tracking-[0.28em] ${className}`}
    >
      R
      <RiftMark className="mx-[0.02em] h-[1.05em] w-auto shrink-0" />
      FT
    </span>
  );
}
