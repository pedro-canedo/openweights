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
  tune: "M4 7h16M4 17h16M8 4v6m8 4v6",
  history: "M3 11a9 9 0 119 10M3 4v7h7M12 7v5l3 2",
  clock: "M21 12a9 9 0 11-18 0 9 9 0 0118 0M12 7v5l3 2",
  sparkles: "M12 3l2.5 6.5L21 12l-6.5 2.5L12 21l-2.5-6.5L3 12l6.5-2.5L12 3z",
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
  external:
    "M14 5h5v5M10 14L19 5M19 13v5a1 1 0 01-1 1H6a1 1 0 01-1-1V6a1 1 0 011-1h5",
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
  // Barra de status. Em 14px o que distingue um pictograma do vizinho é a
  // silhueta, não o detalhe: chip quadrado, módulo de memória deitado, placa
  // com ventoinha, raio, cilindros empilhados. O nome de cada medidor está no
  // `title`, que é o que responde quando a silhueta não basta.
  // Quadrado com pinos nos quatro lados. Sem retângulo interno: em 14px o
  // segundo contorno vira mancha e come a diferença para a memória.
  cpu: "M7 7h10v10H7zM10 3v4m4-4v4m-4 10v4m4-4v4M3 10h4m-4 4h4m10-4h4m-4 4h4",
  // Módulo deitado, com entalhe embaixo: a silhueta é larga e baixa, o
  // oposto do quadrado da CPU.
  memory: "M2 8h20v8H2zM6 11v2m4-2v2m4-2v2m4-2v2M10 16v2m4-2v2",
  // Placa com ventoinha: o círculo grande é o que a separa do módulo de
  // memória quando as duas têm 14 pixels de largura.
  gpu: "M2 6h20v11H2zM9 11.5a3.5 3.5 0 107 0 3.5 3.5 0 10-7 0M5 9v6M6 20v-3m12 3v-3",
  power: "M13 3L5 13h6l-1 8 8-10h-6z",
  disk: "M4 7c0-1.7 3.6-3 8-3s8 1.3 8 3-3.6 3-8 3-8-1.3-8-3zM4 7v10c0 1.7 3.6 3 8 3s8-1.3 8-3V7M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3",
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
export function DirtyDot({
  className = "h-1.5 w-1.5",
}: {
  className?: string;
}) {
  return (
    <span
      aria-hidden="true"
      className={`inline-block shrink-0 rounded-full bg-current ${className}`}
    />
  );
}
