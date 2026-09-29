// Erro conhecido → o que aconteceu, por quê, e o que fazer agora.
//
// O app recebe erros de quatro lugares: o `fetch` do Chat ("HTTP 500: {corpo}"),
// o Rust (`EngineError`, `RuntimeError`, `disk-space:…`), os provedores
// (401/402/429 da OpenRouter e do 9router) e o próprio navegador ("Failed to
// fetch"). Mostrar isso cru deixa a pessoa sem saída; `classificarErro` põe os
// que conhecemos em palavras e diz que botões resolvem. O que não conhecemos
// vira "algo deu errado" — com o texto original sempre à mão, recolhido, para
// quem precisa dele (e para quem vai pedir ajuda).

import i18n from "../i18n";
import { formatBytes } from "./format";

/** O que um botão do cartão de erro faz; quem mostra o cartão liga cada um. */
export type AcaoDoErro =
  /** Refaz o que falhou (só quando quem mostra sabe refazer). */
  | "tentar"
  | "servidor"
  | "fontes"
  | "descobrir"
  | "modelos"
  | "novaConversa"
  | "motor";

export type TipoDeErro =
  | "motor-ausente"
  | "sem-servidor"
  | "sem-memoria"
  | "contexto"
  | "modelo-nao-carregou"
  | "travou"
  | "chave"
  | "creditos"
  | "limite"
  | "modelo-inexistente"
  | "disco"
  | "rede"
  | "desconhecido";

export interface ErroClassificado {
  tipo: TipoDeErro;
  titulo: string;
  explicacao: string;
  /** Em ordem de importância; a primeira é o botão principal. */
  acoes: AcaoDoErro[];
  /** O texto original, para copiar e para quem quer entender. */
  detalhe: string;
}

/** Texto de um erro qualquer: `Error`, string crua do `invoke`, ou outra coisa. */
export function textoDoErro(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  try {
    return JSON.stringify(e) ?? String(e);
  } catch {
    return String(e);
  }
}

const REGRAS: {
  tipo: TipoDeErro;
  quando: (texto: string, status: number | null) => boolean;
  acoes: AcaoDoErro[];
  params?: (texto: string) => Record<string, string>;
}[] = [
  {
    tipo: "motor-ausente",
    quando: (t) => /runtime do llama\.cpp ainda não está instalado/i.test(t),
    acoes: ["motor"],
  },
  {
    // Um 500 do llama-server com a memória de vídeo esgotada, ou a mensagem
    // própria do app quando o Router não carregou o modelo.
    tipo: "sem-memoria",
    quando: (t) =>
      /out of memory|cudaMalloc|failed to allocate|memória de vídeo|não consegui carregar o modelo/i.test(t),
    acoes: ["servidor", "tentar"],
  },
  {
    tipo: "contexto",
    quando: (t) =>
      /exceed_context_size|exceeds the available context|context (length|size).{0,40}(exceed|too)|maximum context/i.test(
        t,
      ),
    acoes: ["novaConversa", "servidor"],
  },
  {
    tipo: "travou",
    quando: (t) => /o modelo ficou \d+s sem emitir nada/i.test(t),
    acoes: ["tentar", "servidor"],
  },
  {
    tipo: "modelo-nao-carregou",
    quando: (t, s) => /failed to load model|model .* not found in (the )?router/i.test(t) || (s === 503 && /loading model/i.test(t)),
    acoes: ["tentar", "modelos"],
  },
  {
    tipo: "chave",
    quando: (t, s) => s === 401 || s === 403 || /missing api key|invalid api key|unauthorized/i.test(t),
    acoes: ["fontes", "servidor"],
  },
  {
    tipo: "creditos",
    quando: (t, s) => s === 402 || /insufficient (credits|funds)|payment required/i.test(t),
    acoes: ["fontes"],
  },
  {
    tipo: "limite",
    quando: (t, s) => s === 429 || /rate.?limit|too many requests/i.test(t),
    acoes: ["tentar"],
  },
  {
    tipo: "modelo-inexistente",
    quando: (t, s) => s === 404 && /model/i.test(t),
    acoes: ["descobrir", "fontes"],
  },
  {
    tipo: "disco",
    quando: (t) => /^disk-space:\d+:\d+$/.test(t) || /espaço em disco insuficiente|no space left/i.test(t),
    acoes: ["modelos"],
    params: (t) => {
      const m = /^disk-space:(\d+):(\d+)$/.exec(t);
      return m ? { need: formatBytes(Number(m[1])), free: formatBytes(Number(m[2])) } : { need: "", free: "" };
    },
  },
  {
    // O navegador não conseguiu nem abrir a conexão: o servidor não está de pé.
    tipo: "sem-servidor",
    quando: (t) =>
      /failed to fetch|networkerror|load failed|engine não está rodando|connection refused|fetch failed/i.test(t) ||
      /falha de rede ao falar com o engine/i.test(t),
    acoes: ["servidor", "tentar"],
  },
  {
    tipo: "rede",
    quando: (t) => /falha de rede|timed? ?out|dns|error sending request|connection (reset|closed)/i.test(t),
    acoes: ["tentar"],
  },
];

/** O status de um "HTTP 500: {corpo}" ou "HTTP 500 do engine: …". */
function statusHttp(texto: string): number | null {
  const m = /^(?:[^:]{0,40}:\s*)?HTTP (\d{3})\b/.exec(texto);
  return m ? Number(m[1]) : null;
}

export function classificarErro(e: unknown): ErroClassificado {
  const detalhe = textoDoErro(e).trim();
  // O prefixo "Algo deu errado: " que o Chat põe na bolha não é do erro.
  const texto = detalhe.replace(new RegExp(`^${escapar(i18n.t("common.error"))}:\\s*`), "");
  const status = statusHttp(texto);
  for (const r of REGRAS) {
    if (r.quando(texto, status)) {
      const params = r.params?.(texto) ?? {};
      return {
        tipo: r.tipo,
        titulo: i18n.t(`actionableError.${r.tipo}.title`, params),
        explicacao: i18n.t(`actionableError.${r.tipo}.hint`, params),
        acoes: r.acoes,
        detalhe,
      };
    }
  }
  return {
    tipo: "desconhecido",
    titulo: i18n.t("actionableError.desconhecido.title"),
    explicacao: i18n.t("actionableError.desconhecido.hint"),
    acoes: ["tentar"],
    detalhe,
  };
}

/**
 * Uma linha para onde não cabe o cartão (um download que falhou, uma lista):
 * o título do erro conhecido, ou o texto original quando não o reconhecemos —
 * dizer "algo deu errado" onde havia um motivo específico seria pior.
 */
export function resumoDoErro(e: unknown): string {
  const c = classificarErro(e);
  return c.tipo === "desconhecido" ? c.detalhe : c.titulo;
}

function escapar(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
