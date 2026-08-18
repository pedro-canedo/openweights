// Ícone de arquivo por extensão, no espírito do explorador do VS Code.
//
// Não é um pacote de ícones: são sete formas (código, chaves, estilo, texto,
// imagem, terminal, dados) pintadas com a cor da linguagem. Um conjunto
// completo teria dezenas de SVGs para manter, e o que faz a lista ficar
// legível de relance é sobretudo a COR — a forma só precisa separar código de
// imagem, de dados, de documento.
//
// Extensão desconhecida cai na folha de papel neutra, nunca em nada.

type Forma = "code" | "braces" | "style" | "doc" | "image" | "shell" | "data";

interface Tipo {
  forma: Forma;
  cor: string;
}

// Cores próximas das do tema Seti (VS Code) para quem já tem o reflexo
// treinado: amarelo = JS, azul = TS, laranja = HTML, roxo = CSS.
const TIPOS: Record<string, Tipo> = {
  js: { forma: "code", cor: "#e8c22e" },
  mjs: { forma: "code", cor: "#e8c22e" },
  cjs: { forma: "code", cor: "#e8c22e" },
  jsx: { forma: "code", cor: "#e8c22e" },
  ts: { forma: "code", cor: "#3b9ddd" },
  tsx: { forma: "code", cor: "#3b9ddd" },
  py: { forma: "code", cor: "#4b8bbe" },
  rs: { forma: "code", cor: "#dd8a5b" },
  go: { forma: "code", cor: "#48c7d9" },
  java: { forma: "code", cor: "#cc6b52" },
  rb: { forma: "code", cor: "#d9534f" },
  php: { forma: "code", cor: "#8b93cc" },
  c: { forma: "code", cor: "#7ba7d1" },
  h: { forma: "code", cor: "#7ba7d1" },
  cpp: { forma: "code", cor: "#7ba7d1" },
  hpp: { forma: "code", cor: "#7ba7d1" },
  html: { forma: "code", cor: "#e5734d" },
  htm: { forma: "code", cor: "#e5734d" },
  xml: { forma: "code", cor: "#8fb573" },
  svg: { forma: "image", cor: "#e5b34d" },

  json: { forma: "braces", cor: "#e8c22e" },
  jsonc: { forma: "braces", cor: "#e8c22e" },
  lock: { forma: "braces", cor: "#8a8f98" },

  css: { forma: "style", cor: "#a97fd6" },
  scss: { forma: "style", cor: "#d6689b" },
  sass: { forma: "style", cor: "#d6689b" },
  less: { forma: "style", cor: "#6b8fd6" },

  md: { forma: "doc", cor: "#5aa9e6" },
  markdown: { forma: "doc", cor: "#5aa9e6" },
  txt: { forma: "doc", cor: "#9aa0a6" },
  log: { forma: "doc", cor: "#9aa0a6" },
  pdf: { forma: "doc", cor: "#d9534f" },

  png: { forma: "image", cor: "#7ec699" },
  jpg: { forma: "image", cor: "#7ec699" },
  jpeg: { forma: "image", cor: "#7ec699" },
  gif: { forma: "image", cor: "#7ec699" },
  webp: { forma: "image", cor: "#7ec699" },
  bmp: { forma: "image", cor: "#7ec699" },
  ico: { forma: "image", cor: "#7ec699" },

  sh: { forma: "shell", cor: "#89c07a" },
  bash: { forma: "shell", cor: "#89c07a" },
  zsh: { forma: "shell", cor: "#89c07a" },
  ps1: { forma: "shell", cor: "#5aa9e6" },
  bat: { forma: "shell", cor: "#89c07a" },
  cmd: { forma: "shell", cor: "#89c07a" },

  toml: { forma: "data", cor: "#c98b6b" },
  yaml: { forma: "data", cor: "#d16b7c" },
  yml: { forma: "data", cor: "#d16b7c" },
  ini: { forma: "data", cor: "#9aa0a6" },
  cfg: { forma: "data", cor: "#9aa0a6" },
  env: { forma: "data", cor: "#e8c22e" },
  csv: { forma: "data", cor: "#7ec699" },
  sql: { forma: "data", cor: "#c98b6b" },
  db: { forma: "data", cor: "#c98b6b" },
  sqlite: { forma: "data", cor: "#c98b6b" },
};

const NEUTRO: Tipo = { forma: "doc", cor: "#8a8f98" };

/**
 * Nomes sem extensão que ainda assim têm identidade própria — o explorador
 * mostra `.gitignore` e `Dockerfile` o tempo todo, e cair no papel neutro
 * neles é o caso mais visível de "o ícone não diz nada".
 */
const POR_NOME: Record<string, Tipo> = {
  dockerfile: { forma: "data", cor: "#4b8bbe" },
  makefile: { forma: "shell", cor: "#c98b6b" },
  ".gitignore": { forma: "data", cor: "#e5734d" },
  ".env": { forma: "data", cor: "#e8c22e" },
  license: { forma: "doc", cor: "#e8c22e" },
  readme: { forma: "doc", cor: "#5aa9e6" },
};

export function tipoDoArquivo(name: string): Tipo {
  const nome = name.toLowerCase();
  const porNome = POR_NOME[nome] ?? POR_NOME[nome.replace(/\.[^.]+$/, "")];
  if (porNome) return porNome;
  const dot = nome.lastIndexOf(".");
  const ext = dot > 0 ? nome.slice(dot + 1) : "";
  return TIPOS[ext] ?? NEUTRO;
}

function Glifo({ forma }: { forma: Forma }) {
  switch (forma) {
    case "code":
      return <path d="M9 8l-4 4 4 4M15 8l4 4-4 4" />;
    case "braces":
      return (
        <path d="M9 6c-1.5 0-2 .8-2 2v2c0 1-.6 2-1.6 2 1 0 1.6 1 1.6 2v2c0 1.2.5 2 2 2M15 6c1.5 0 2 .8 2 2v2c0 1 .6 2 1.6 2-1 0-1.6 1-1.6 2v2c0 1.2-.5 2-2 2" />
      );
    case "style":
      return (
        <>
          <path d="M5 4h14l-1.3 14.4L12 20l-5.7-1.6z" />
          <path d="M8.5 8.5h7l-.4 4.2-3.1.9-3.1-.9" />
        </>
      );
    case "image":
      return (
        <>
          <rect x="4" y="5" width="16" height="14" rx="2" />
          <circle cx="9" cy="10" r="1.4" />
          <path d="M5.5 17l4-4.2 3 3 2.5-2.2 3.5 3.4" />
        </>
      );
    case "shell":
      return (
        <>
          <rect x="3" y="5" width="18" height="14" rx="2" />
          <path d="M7 10l2.5 2L7 14M12.5 15h4" />
        </>
      );
    case "data":
      return (
        <>
          <ellipse cx="12" cy="6.5" rx="7" ry="2.6" />
          <path d="M5 6.5v11c0 1.4 3.1 2.6 7 2.6s7-1.2 7-2.6v-11" />
          <path d="M5 12c0 1.4 3.1 2.6 7 2.6s7-1.2 7-2.6" />
        </>
      );
    default:
      return (
        <>
          <path d="M6 3.5h7L18 8v12.5H6z" />
          <path d="M13 3.5V8h5" />
        </>
      );
  }
}

/** Ícone do arquivo. A cor vem da extensão; o traço acompanha o tema. */
export default function FileIcon({
  name,
  className = "h-3.5 w-3.5",
}: {
  name: string;
  className?: string;
}) {
  const { forma, cor } = tipoDoArquivo(name);
  return (
    <svg
      className={`shrink-0 ${className}`}
      style={{ color: cor }}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      <Glifo forma={forma} />
    </svg>
  );
}

/** Pasta aberta ou fechada, para a árvore ter o mesmo peso visual. */
export function FolderIcon({
  open = false,
  className = "h-3.5 w-3.5",
}: {
  open?: boolean;
  className?: string;
}) {
  return (
    <svg
      className={`shrink-0 ${className}`}
      style={{ color: "#7d8590" }}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      {open ? (
        <path d="M3 8a2 2 0 012-2h4l2 2h8a2 2 0 012 2H6.6a2 2 0 00-1.9 1.4L3 19z" />
      ) : (
        <path d="M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2z" />
      )}
    </svg>
  );
}
