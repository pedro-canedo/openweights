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
import { copiarTexto, lerTexto } from "./clipboard";
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

export type Layout = 1 | 2 | 4;

export interface EstadoTerminais {
  sessoes: SessaoTerminal[];
  /** Quantos painéis a grade mostra. */
  layout: Layout;
  /**
   * A sessão de cada painel. Sempre 4 posições; só as `layout` primeiras
   * aparecem — reduzir a grade esconde, não fecha, e crescer de novo traz
   * de volta. Uma sessão nunca está em dois painéis (um elemento DOM só).
   */
  paineis: (number | null)[];
  /** O painel com foco: é nele que entra a sessão escolhida na lista. */
  foco: number;
  /** A sessão do painel com foco. */
  ativa: number | null;
  /** A sessão ativa leva o foco ao montar? Não quando se anda pela lista com setas. */
  focar: boolean;
  erro: string | null;
  pronto: boolean;
}

const CHAVE_LAYOUT = "ow.owcli.layout";

function layoutGuardado(): Layout {
  try {
    const n = Number(localStorage.getItem(CHAVE_LAYOUT));
    return n === 2 || n === 4 ? n : 1;
  } catch {
    return 1;
  }
}

let estado: EstadoTerminais = {
  sessoes: [],
  layout: 1,
  paineis: [null, null, null, null],
  foco: 0,
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

/** Troca painéis e foco juntos, mantendo `ativa` coerente com eles. */
function mudarPaineis(paineis: (number | null)[], foco: number, focar: boolean) {
  mudar({ paineis, foco, ativa: paineis[foco] ?? null, focar });
}

/** A sessão está num painel que aparece agora? */
function visivel(id: number): boolean {
  const j = estado.paineis.indexOf(id);
  return j >= 0 && j < estado.layout;
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

  // Ctrl+Shift+C / Ctrl+Shift+V (e Cmd+C / Cmd+V no macOS): o Ctrl+C e o
  // Ctrl+V sozinhos são do programa (interromper, inserção literal no vim).
  const mac = navigator.platform.toLowerCase().includes("mac");
  term.attachCustomKeyEventHandler((ev) => {
    if (ev.type !== "keydown") return true;
    const tecla = ev.key.toLowerCase();
    const copiar = mac ? ev.metaKey && tecla === "c" : ev.ctrlKey && ev.shiftKey && tecla === "c";
    const colar = mac ? ev.metaKey && tecla === "v" : ev.ctrlKey && ev.shiftKey && tecla === "v";
    // `false` só tira a tecla do xterm; sem o preventDefault o WebKitGTK
    // também cola sozinho (Ctrl+Shift+V é colar nativo no GTK) e o texto
    // entrava duas vezes.
    if (copiar && term.hasSelection()) {
      ev.preventDefault();
      void copiarSelecao(id);
      return false;
    }
    if (colar) {
      ev.preventDefault();
      void colarNo(id);
      return false;
    }
    return true;
  });

  // OSC 52: o programa pede para copiar (tmux, vim, ssh). Só escrita — pedido
  // de LEITURA ("?") é ignorado, senão qualquer saída leria a área de
  // transferência de quem usa.
  term.parser.registerOscHandler(52, (dados) => {
    const b64 = dados.slice(dados.indexOf(";") + 1);
    if (!b64 || b64 === "?") return true;
    try {
      const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
      void copiarTexto(new TextDecoder().decode(bytes)).catch(() => {});
    } catch {
      // base64 inválido: nada a copiar.
    }
    return true;
  });

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
      // Webview recarregada: as sessões mais recentes voltam aos painéis.
      const layout = layoutGuardado();
      const paineis: (number | null)[] = [null, null, null, null];
      existentes.slice(-layout).forEach((s, i) => (paineis[i] = s.id));
      const foco = Math.max(0, Math.min(layout, existentes.length) - 1);
      mudar({ sessoes: existentes, layout, pronto: true });
      mudarPaineis(paineis, foco, true);
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
      if (visivel(id) && telaVisivel && document.hasFocus()) {
        void backend.visto(id).catch(() => {});
      } else {
        mudarSessao(id, { atencao: true });
        avisarNoSistema(id, aviso.texto);
      }
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

/** A tela do OwCLI está montada? (ela avisa ao montar e ao desmontar) */
let telaVisivel = false;

export function marcarTelaVisivel(visivel: boolean) {
  telaVisivel = visivel;
}

const ultimoAviso = new Map<number, number>();
/** Um agente que pede aprovação a cada passo não pode virar uma rajada. */
const INTERVALO_ENTRE_AVISOS_MS = 15_000;

/** Aviso do sistema quando a pessoa não está olhando a sessão que chamou. */
function avisarNoSistema(id: number, texto: string) {
  if (document.hasFocus() && telaVisivel && visivel(id)) return;
  const agora = Date.now();
  if (agora - (ultimoAviso.get(id) ?? 0) < INTERVALO_ENTRE_AVISOS_MS) return;
  ultimoAviso.set(id, agora);
  const sessao = estado.sessoes.find((s) => s.id === id);
  const titulo = sessao?.titulo || i18n.t("nav.owcli");
  const corpo = texto || i18n.t("owcli.needsYou");
  void (async () => {
    if (!isTauri) return;
    const n = await import("@tauri-apps/plugin-notification");
    let permitido = await n.isPermissionGranted();
    if (!permitido) permitido = (await n.requestPermission()) === "granted";
    if (permitido) n.sendNotification({ title: titulo, body: corpo });
  })().catch(() => {});
}

export async function copiarSelecao(id: number) {
  const v = vivos.get(id);
  const texto = v?.term.getSelection();
  if (texto) await copiarTexto(texto).catch(() => {});
}

export async function colarNo(id: number) {
  const v = vivos.get(id);
  if (!v) return;
  const texto = await lerTexto().catch(() => "");
  // `paste` aplica o bracketed paste quando o programa pediu: um texto de
  // várias linhas não vira vários comandos executados.
  if (texto) v.term.paste(texto);
}

export function temSelecao(id: number): boolean {
  return vivos.get(id)?.term.hasSelection() ?? false;
}

export function limpar(id: number) {
  vivos.get(id)?.term.clear();
}

export async function novoShell(pasta: string | null = null): Promise<number | null> {
  await iniciar();
  try {
    const id = await backend.abrirShell(pasta, 80, 24);
    const v = criarXterm(id);
    const novas = await backend.listar();
    const paineis = [...estado.paineis];
    paineis[estado.foco] = id;
    mudar({ sessoes: novas, erro: null });
    mudarPaineis(paineis, estado.foco, true);
    await anexar(id, v);
    return id;
  } catch (e) {
    mudar({ erro: String(e) });
    return null;
  }
}

/**
 * Mostra a sessão: se ela já está num painel à vista, o foco vai até lá;
 * senão ela entra no painel com foco (saindo de um painel escondido, se
 * estava num).
 */
export function ativar(id: number, focar = true) {
  const paineis = [...estado.paineis];
  const j = paineis.indexOf(id);
  let foco = estado.foco;
  if (j >= 0 && j < estado.layout) {
    foco = j;
  } else {
    if (j >= 0) paineis[j] = null;
    paineis[foco] = id;
  }
  mudarPaineis(paineis, foco, focar);
  mudarSessao(id, { atencao: false });
  void backend.visto(id).catch(() => {});
}

/** O painel `i` passa a ter o foco (clique dentro dele). */
export function focarPainel(i: number) {
  if (i === estado.foco) return;
  mudarPaineis(estado.paineis, i, false);
  const id = estado.paineis[i];
  if (id != null) {
    mudarSessao(id, { atencao: false });
    void backend.visto(id).catch(() => {});
  }
}

export function definirLayout(layout: Layout) {
  try {
    localStorage.setItem(CHAVE_LAYOUT, String(layout));
  } catch {
    // sem armazenamento: vale até fechar o app
  }
  const foco = estado.foco < layout ? estado.foco : 0;
  mudar({ layout });
  mudarPaineis(estado.paineis, foco, false);
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
  const paineis = estado.paineis.map((p) => (p === id ? null : p));
  // O painel com foco que ficou vazio recebe a sessão mais recente que não
  // está à vista — com um painel só, fechar a aba mostra a anterior.
  if (paineis[estado.foco] == null) {
    const vistas = new Set(paineis.slice(0, estado.layout));
    const proxima = restantes.filter((s) => !vistas.has(s.id)).at(-1)?.id ?? null;
    const j = proxima == null ? -1 : paineis.indexOf(proxima);
    if (j >= 0) paineis[j] = null;
    paineis[estado.foco] = proxima;
  }
  mudar({ sessoes: restantes });
  mudarPaineis(paineis, estado.foco, true);
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
          else if (cmd === "avisar") {
            // Como o agente pedindo aprovação (OSC 9), um pouco depois.
            window.setTimeout(() => aviso?.(id, { kind: "atencao", texto: resto.join(" ") }), 300);
          }
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
