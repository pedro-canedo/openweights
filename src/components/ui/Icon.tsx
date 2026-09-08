// Ícones de linha do app — SVG próprio, sem glifos de texto nem emoticons.
//
// Regra: nenhum ícone é caractere (`✓ × ▾ ▸ ♥ ↓ ↑ ↗ ⚠ ⧉ → • ⭐`).
// Todo pictograma aqui é `stroke="currentColor"`, grid 24, traço 1.8,
// para o peso visual não variar por tela ou por fonte do SO.

// `as const satisfies` e não `: Record<string, string>`: com a anotação larga,
// `keyof typeof PATHS` colapsa em `string`, `<Icon name="chevrom-baixo" />`
// compila, e o erro de digitação vira um SVG vazio — invisível na tela e mudo
// no `tsc`. O `satisfies` confere a forma sem alargar as chaves.
const PATHS = {
  // Colapsáveis (Shell Collapse, SpecCard, PowerCard, BenchHistory, flags).
  "chevron-down": "M6 9l6 6 6-6",
  "chevron-right": "M9 6l6 6-6 6",
  // Fechar / remover (editor, explorador, chips de flag, prévia).
  close: "M6 6l12 12M18 6L6 18",
  // Confirmado / salvo / teste ok.
  check: "M20 6L9 17l-5-5",
  // Copiar valor (endereço, chave, comando).
  copy: "M8 8h12v12H8zM8 8V6a2 2 0 012-2h10a2 2 0 012 2v10a2 2 0 01-2 2h-2",
  // Abrir fora do app (Hugging Face, pasta no SO, harness).
  external: "M14 5h5v5M10 14L19 5M19 13v5a1 1 0 01-1 1H6a1 1 0 01-1-1V6a1 1 0 011-1h5",
  // Download (contagem, botão, taxa de rede).
  download: "M12 4v11m0 0l-4-4m4 4l4-4M4 17v2a1 1 0 001 1h14a1 1 0 001-1v-2",
  // Upload / taxa de envio da rede.
  upload: "M12 15V4m0 0L8 8m4-4l4 4M4 17v2a1 1 0 001 1h14a1 1 0 001-1v-2",
  // Curtidas no Hub.
  heart: "M12 20s-7-4.6-7-10a4 4 0 017-2.6A4 4 0 0119 10c0 5.4-7 10-7 10z",
  // "Ver config completa", "configurar motor".
  "arrow-right": "M4 12h16m0 0l-5-5m5 5l-5 5",
  // Aviso (qualidade divergente, suspeito térmico).
  alert: "M12 4L2.5 20h19L12 4zM12 10v4m0 3.5h.01",
} as const satisfies Record<string, string>;

export type IconName = keyof typeof PATHS;

/**
 * `title` só quando o ícone É a informação.
 *
 * Um pictograma ao lado de texto é decoração e fica `aria-hidden`, senão o
 * leitor de tela ouve tudo duas vezes. Mas quando o ícone carrega sozinho o
 * significado — a seta que diz se a taxa é de descida ou de subida, o coração
 * que diz que o número é de curtidas — esconder é apagar a informação para
 * quem não vê o desenho.
 */
export default function Icon({
  name,
  className = "h-3.5 w-3.5",
  title,
}: {
  name: IconName;
  className?: string;
  title?: string;
}) {
  return (
    <svg
      className={`shrink-0 ${className}`}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      viewBox="0 0 24 24"
      role={title ? "img" : undefined}
      aria-hidden={title ? undefined : "true"}
    >
      {title && <title>{title}</title>}
      <path d={PATHS[name]} />
    </svg>
  );
}

/** Ponto de "modificado" do editor — CSS, não caractere. */
export function DirtyDot({ className = "h-1.5 w-1.5" }: { className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={`inline-block shrink-0 rounded-full bg-current ${className}`}
    />
  );
}
