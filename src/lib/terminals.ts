// Os terminais da tela do OwCLI: sessões no backend (lr_pty), xterm.js aqui.
//
// A tela é desmontada ao trocar de tela, e um terminal não pode morrer por
// isso: cada sessão tem UMA instância do xterm, criada aqui, com um elemento
// próprio que a tela pendura no painel ao montar e solta ao desmontar. A
// saída continua chegando e sendo desenhada com a tela fechada.
//
// Endereço por byte: o backend numera a saída; `fim` é até onde este xterm
// desenhou. Uma mensagem que repete o que já foi desenhado é cortada, e o
// controle de fluxo confirma o que o xterm terminou de processar — é isso que
// segura um `cat` de 50 MB sem estourar a memória da webview.
//
// Webview recarregada: as sessões continuam vivas no backend. `iniciar()`
// lista as que existem, cria um xterm novo para cada uma e pede o anel
// inteiro — a tela volta como estava (o que coube em 1 MiB).
//
// No navegador (npm run dev, Playwright) não há backend: um shell simulado
// ecoa o que se digita e responde a `echo`.

import { Terminal, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import "@xterm/xterm/css/xterm.css";
import i18n from "../i18n";
import { openUrl } from "./openExternal";
import { invoke, isTauri, listen } from "./tauri";

// ------------------------------------------------------------ tipos ---

export type TipoSessao =
  | { kind: "shell" }
  | { kind: "owCli" }
  | { kind: "harness"; id: string };

/** Espelho do `lr_pty::Resumo`. */
export interface SessaoTerminal {
  id: number;
  tipo: TipoSessao;
  titulo: string;
  pasta: string;
  atencao: boolean;
  viva: boolean;
  codigoSaida: number | null;
  pid: number | null;
  criadaEmMs: number;
  fim: number;
}

type Aviso =
  | { kind: "titulo"; titulo: string }
  | { kind: "pasta"; pasta: string }
  | { kind: "atencao"; texto: string }
  | { kind: "saiu"; codigo: number | null };

interface Backend {
  listar(): Promise<SessaoTerminal[]>;
  abrirShell(pasta: string | null, colunas: number, linhas: number): Promise<number>;
  anexar(id: number, desde: number | null, onBloco: (b: ArrayBuffer) => void): Promise<boolean>;
  escrever(id: number, dados: string): Promise<void>;
  redimensionar(id: number, colunas: number, linhas: number): Promise<void>;
  confirmar(id: number, ate: number): Promise<void>;
  fechar(id: number): Promise<void>;
  visto(id: number): Promise<void>;
  aoAvisar(f: (id: number, aviso: Aviso) => void): Promise<() => void>;
}

// ------------------------------------------------------------ estado ---

export interface EstadoTerminais {
  sessoes: SessaoTerminal[];
  ativa: number | null;
  /** A sessão ativa leva o foco ao montar? Não quando se anda pela lista com setas. */
  focar: boolean;
  erro: string | null;
  pronto: boolean;
}

let estado: EstadoTerminais = {
  sessoes: [],
  ativa: null,
  focar: true,
  erro: null,
  pronto: false,
};
const ouvintes = new Set<() => void>();

function mudar(parcial: Partial<EstadoTerminais>) {
  estado = { ...estado, ...parcial };
  ouvintes.forEach((f) => f());
}

function mudarSessao(id: number, parcial: Partial<SessaoTerminal>) {
  mudar({
    sessoes: estado.sessoes.map((s) => (s.id === id ? { ...s, ...parcial } : s)),
  });
}

export const terminaisStore = {
  subscribe(f: () => void) {
    ouvintes.add(f);
    return () => ouvintes.delete(f);
  },
  get: () => estado,
};

// ------------------------------------------------------------- xterm ---

interface Vivo {
  term: Terminal;
  fit: FitAddon;
  el: HTMLDivElement;
  aberto: boolean;
  /** Até onde este xterm desenhou (endereço do próximo byte). */
  fim: number;
  confirmado: number;
  timerConfirmacao: number | undefined;
}

const vivos = new Map<number, Vivo>();

/** Confirma no máximo a cada 32 KiB ou 40 ms: sem isso, cada bloco viraria uma ida ao backend. */
const LOTE_DE_CONFIRMACAO = 32 * 1024;

function cor(nome: string, reserva: string) {
  const v = getComputedStyle(document.documentElement).getPropertyValue(nome).trim();
  return v || reserva;
}

function temaDoApp(): ITheme {
  const claro = document.documentElement.dataset.theme === "light";
  return {
    background: cor("--lr-bg", claro ? "#f5f6f9" : "#0a0d14"),
    foreground: cor("--lr-ink", claro ? "#171b26" : "#e6e9f0"),
    cursor: cor("--lr-accent-ink", "#9d87ff"),
    cursorAccent: cor("--lr-bg", "#0a0d14"),
    selectionBackground: claro ? "#6a44ee40" : "#9d87ff55",
  };
}

// O tema do app muda com um atributo no <html>; os terminais acompanham.
if (typeof document !== "undefined") {
  new MutationObserver(() => {
    const tema = temaDoApp();
    vivos.forEach((v) => (v.term.options.theme = tema));
  }).observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
}

/**
 * A primeira fonte monoespaçada que existe DE VERDADE nesta máquina.
 *
 * Não dá para confiar no nome: no Linux o fontconfig responde a qualquer
 * família (`fc-match "JetBrains Mono"` devolve a DejaVu Sans quando ela não
 * está instalada), e o WebKitGTK aceita o substituto — o terminal saía numa
 * fonte proporcional, com o `_` sumindo e a linha digitada embaralhada,
 * porque o xterm mede uma largura de célula e desenha outra. Uma fonte
 * monoespaçada tem "iiiiiiiiii" e "WWWWWWWWWW" da mesma largura.
 */
function fonteMonoespacada(): string {
  const candidatas = [
    '"JetBrains Mono"',
    '"Cascadia Mono"',
    '"SF Mono"',
    "Menlo",
    "Consolas",
    '"Adwaita Mono"',
    '"Noto Sans Mono"',
    '"DejaVu Sans Mono"',
    '"Liberation Mono"',
  ];
  const ctx = document.createElement("canvas").getContext("2d");
  if (!ctx) return "monospace";
  const mono = (familia: string) => {
    ctx.font = `16px ${familia}`;
    return (
      Math.abs(ctx.measureText("iiiiiiiiii").width - ctx.measureText("WWWWWWWWWW").width) < 0.5
    );
  };
  const escolhida = candidatas.find(mono);
  return escolhida ? `${escolhida}, monospace` : "monospace";
}

let fonte: string | null = null;

function criarXterm(id: number): Vivo {
  fonte ??= fonteMonoespacada();
  const term = new Terminal({
    allowProposedApi: true,
    cursorBlink: true,
    fontFamily: fonte,
    fontSize: 13,
    lineHeight: 1.15,
    scrollback: 5000,
    theme: temaDoApp(),
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  term.loadAddon(new WebLinksAddon((_e, url) => void openUrl(url)));
  term.loadAddon(new Unicode11Addon());
  term.unicode.activeVersion = "11";

  const el = document.createElement("div");
  el.className = "h-full w-full";
  el.dataset.terminal = String(id);

  term.onData((dados) => void backend.escrever(id, dados).catch(() => {}));
  term.onBinary((dados) => void backend.escrever(id, dados).catch(() => {}));
  term.onResize(({ cols, rows }) => void backend.redimensionar(id, cols, rows).catch(() => {}));

  const vivo: Vivo = {
    term,
    fit,
    el,
    aberto: false,
    fim: 0,
    confirmado: 0,
    timerConfirmacao: undefined,
  };
  vivos.set(id, vivo);
  return vivo;
}

function confirmarDepois(id: number, v: Vivo) {
  if (v.fim - v.confirmado >= LOTE_DE_CONFIRMACAO) {
    v.confirmado = v.fim;
    window.clearTimeout(v.timerConfirmacao);
    v.timerConfirmacao = undefined;
    void backend.confirmar(id, v.fim).catch(() => {});
    return;
  }
  if (v.timerConfirmacao === undefined) {
    v.timerConfirmacao = window.setTimeout(() => {
      v.timerConfirmacao = undefined;
      if (v.fim > v.confirmado) {
        v.confirmado = v.fim;
        void backend.confirmar(id, v.fim).catch(() => {});
      }
    }, 40);
  }
}

/** Uma mensagem do canal: `[offset u64 LE][truncado u8][dados]`. */
function receber(id: number, v: Vivo, msg: ArrayBuffer) {
  const visao = new DataView(msg);
  const offset = Number(visao.getBigUint64(0, true));
  const truncado = visao.getUint8(8) === 1;
  const dados = new Uint8Array(msg, 9);
  if (truncado) {
    v.term.write(`\x1b[2m[${i18n.t("owcli.truncated")}]\x1b[0m\r\n`);
  }
  const fimDoBloco = offset + dados.length;
  if (fimDoBloco <= v.fim) return;
  const pedaco = offset < v.fim ? dados.subarray(v.fim - offset) : dados;
  v.fim = fimDoBloco;
  v.term.write(pedaco, () => confirmarDepois(id, v));
}

async function anexar(id: number, v: Vivo) {
  const ok = await backend.anexar(id, v.fim === 0 ? null : v.fim, (msg) => receber(id, v, msg));
  if (!ok) mudarSessao(id, { viva: false });
}

// ------------------------------------------------------------- ações ---

let iniciado: Promise<void> | null = null;

/** Liga o store ao backend uma vez: avisos e sessões que já existiam. */
export function iniciar(): Promise<void> {
  iniciado ??= (async () => {
    await backend.aoAvisar(aplicarAviso);
    try {
      const existentes = await backend.listar();
      for (const s of existentes) {
        const v = criarXterm(s.id);
        await anexar(s.id, v);
      }
      mudar({
        sessoes: existentes,
        ativa: existentes.at(-1)?.id ?? null,
        pronto: true,
      });
    } catch (e) {
      mudar({ erro: String(e), pronto: true });
    }
  })();
  return iniciado;
}

function aplicarAviso(id: number, aviso: Aviso) {
  switch (aviso.kind) {
    case "titulo":
      mudarSessao(id, { titulo: aviso.titulo });
      break;
    case "pasta":
      mudarSessao(id, { pasta: aviso.pasta });
      break;
    case "atencao":
      // Na sessão que a pessoa está olhando, o pedido já foi visto.
      if (id === estado.ativa && document.hasFocus()) void backend.visto(id).catch(() => {});
      else mudarSessao(id, { atencao: true });
      break;
    case "saiu": {
      mudarSessao(id, { viva: false, codigoSaida: aviso.codigo });
      const v = vivos.get(id);
      v?.term.write(
        `\r\n\x1b[2m[${i18n.t("owcli.exited", { code: aviso.codigo ?? "?" })}]\x1b[0m\r\n`,
      );
      break;
    }
  }
}

export async function novoShell(pasta: string | null = null): Promise<number | null> {
  await iniciar();
  try {
    const id = await backend.abrirShell(pasta, 80, 24);
    const v = criarXterm(id);
    const novas = await backend.listar();
    mudar({
      sessoes: novas,
      ativa: id,
      focar: true,
      erro: null,
    });
    await anexar(id, v);
    return id;
  } catch (e) {
    mudar({ erro: String(e) });
    return null;
  }
}

export function ativar(id: number, focar = true) {
  mudar({ ativa: id, focar });
  mudarSessao(id, { atencao: false });
  void backend.visto(id).catch(() => {});
}

export async function fechar(id: number) {
  await backend.fechar(id).catch(() => {});
  const v = vivos.get(id);
  if (v) {
    window.clearTimeout(v.timerConfirmacao);
    v.term.dispose();
    v.el.remove();
    vivos.delete(id);
  }
  const restantes = estado.sessoes.filter((s) => s.id !== id);
  mudar({
    sessoes: restantes,
    ativa: estado.ativa === id ? (restantes.at(-1)?.id ?? null) : estado.ativa,
  });
}

/**
 * Pendura o terminal da sessão no painel. O xterm só é aberto na primeira
 * vez (ele mede a fonte no DOM); depois o elemento só muda de lugar.
 */
export function montar(id: number, painel: HTMLElement, focar = true) {
  const v = vivos.get(id);
  if (!v) return;
  if (v.el.parentElement !== painel) painel.appendChild(v.el);
  if (!v.aberto) {
    v.term.open(v.el);
    v.aberto = true;
  }
  ajustar(id);
  if (focar) v.term.focus();
}

/** Solta o elemento do painel; a sessão e o xterm continuam vivos. */
export function desmontar(id: number) {
  vivos.get(id)?.el.remove();
}

/** Recalcula colunas e linhas pelo tamanho do painel (e avisa o backend). */
export function ajustar(id: number) {
  const v = vivos.get(id);
  // Solto do DOM, o fit mediria zero e encolheria o terminal para 1×1.
  if (!v || !v.aberto || !v.el.isConnected || v.el.clientWidth === 0) return;
  try {
    v.fit.fit();
  } catch {
    // Fonte ainda não medida: o próximo ajuste resolve.
  }
}

// ----------------------------------------------------------- backends ---

const backendTauri: Backend = {
  listar: () => invoke<SessaoTerminal[]>("terminal_listar"),
  abrirShell: (pasta, colunas, linhas) =>
    invoke<number>("terminal_abrir_shell", { pedido: { pasta, colunas, linhas } }),
  async anexar(id, desde, onBloco) {
    const { Channel } = await import("@tauri-apps/api/core");
    const canal = new Channel<ArrayBuffer>();
    canal.onmessage = onBloco;
    return invoke<boolean>("terminal_anexar", { id, desde, canal });
  },
  escrever: (id, dados) => invoke("terminal_escrever", { id, dados }),
  redimensionar: (id, colunas, linhas) => invoke("terminal_redimensionar", { id, colunas, linhas }),
  confirmar: (id, ate) => invoke("terminal_confirmar", { id, ate }),
  fechar: (id) => invoke("terminal_fechar", { id }),
  visto: (id) => invoke("terminal_visto", { id }),
  aoAvisar: (f) =>
    listen<{ id: number; aviso: Aviso }>("terminal", ({ id, aviso }) => f(id, aviso)),
};

/** Shell de mentira para o navegador: ecoa, apaga e responde a `echo`. */
function backendSimulado(): Backend {
  interface Simulada {
    resumo: SessaoTerminal;
    saida: Uint8Array[];
    fim: number;
    linha: string;
    destino: ((b: ArrayBuffer) => void) | null;
  }
  const sessoes = new Map<number, Simulada>();
  let proximo = 1;
  let aviso: ((id: number, a: Aviso) => void) | null = null;
  const cod = new TextEncoder();

  function emitir(s: Simulada, texto: string) {
    const dados = cod.encode(texto);
    const msg = new Uint8Array(9 + dados.length);
    new DataView(msg.buffer).setBigUint64(0, BigInt(s.fim), true);
    msg.set(dados, 9);
    s.saida.push(dados);
    s.fim += dados.length;
    s.resumo.fim = s.fim;
    s.destino?.(msg.buffer);
  }
  const prompt = (s: Simulada) => emitir(s, "\x1b[32mvoce@navegador\x1b[0m:~$ ");

  return {
    listar: async () => [...sessoes.values()].map((s) => ({ ...s.resumo })),
    async abrirShell() {
      const id = proximo++;
      const s: Simulada = {
        resumo: {
          id,
          tipo: { kind: "shell" },
          titulo: "bash",
          pasta: "~",
          atencao: false,
          viva: true,
          codigoSaida: null,
          pid: null,
          criadaEmMs: Date.now(),
          fim: 0,
        },
        saida: [],
        fim: 0,
        linha: "",
        destino: null,
      };
      sessoes.set(id, s);
      prompt(s);
      return id;
    },
    async anexar(id, desde, onBloco) {
      const s = sessoes.get(id);
      if (!s) return false;
      s.destino = onBloco;
      const tudo = new Uint8Array(s.saida.reduce((n, p) => n + p.length, 0));
      let pos = 0;
      for (const p of s.saida) {
        tudo.set(p, pos);
        pos += p.length;
      }
      const de = Math.min(desde ?? 0, tudo.length);
      const msg = new Uint8Array(9 + tudo.length - de);
      new DataView(msg.buffer).setBigUint64(0, BigInt(de), true);
      msg.set(tudo.subarray(de), 9);
      onBloco(msg.buffer);
      return true;
    },
    async escrever(id, dados) {
      const s = sessoes.get(id);
      if (!s) return;
      for (const c of dados) {
        if (c === "\r") {
          emitir(s, "\r\n");
          const [cmd, ...resto] = s.linha.trim().split(/\s+/);
          if (cmd === "echo") emitir(s, `${resto.join(" ")}\r\n`);
          else if (cmd === "exit") {
            s.resumo.viva = false;
            aviso?.(id, { kind: "saiu", codigo: 0 });
            return;
          } else if (cmd) emitir(s, `${cmd}: comando não simulado no navegador\r\n`);
          s.linha = "";
          prompt(s);
        } else if (c === "\x7f") {
          if (s.linha) {
            s.linha = s.linha.slice(0, -1);
            emitir(s, "\b \b");
          }
        } else if (c >= " ") {
          s.linha += c;
          emitir(s, c);
        }
      }
    },
    async redimensionar() {},
    async confirmar() {},
    async fechar(id) {
      sessoes.delete(id);
    },
    async visto(id) {
      const s = sessoes.get(id);
      if (s) s.resumo.atencao = false;
    },
    async aoAvisar(f) {
      aviso = f;
      return () => {
        aviso = null;
      };
    },
  };
}

const backend: Backend = isTauri ? backendTauri : backendSimulado();
