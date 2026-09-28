// A paleta de comandos: o que o app faz, por nome, com Ctrl+K.
//
// É também o que garante que o modo Simples não tira nada do alcance: toda
// tela e toda aba abre daqui, e os atalhos de teclado são os mesmos comandos.

import type { TFunction } from "i18next";
import { restartServer, startServer, stopServer } from "./api";
import { chatStore } from "./chatStore";
import { definirModo, type Modo } from "./mode";
import { navigate, type NavPayload, type Screen } from "./nav";
import { openUrl } from "./openExternal";
import { aplicarTema, type Tema } from "./tema";

/** O guia do site, no idioma da interface. */
export function enderecoDaAjuda(idioma: string): string {
  return idioma.startsWith("pt")
    ? "https://pedro-canedo.github.io/openweights/pt/guia/"
    : "https://pedro-canedo.github.io/openweights/guide/";
}

export type GrupoDeComando = "screens" | "actions" | "server" | "preferences";

export interface Comando {
  id: string;
  grupo: GrupoDeComando;
  rotulo: string;
  /** Outras palavras que acham o comando (sinônimos, o nome em inglês). */
  palavras?: string;
  /** Como o atalho aparece: "Ctrl+N". */
  atalho?: string;
  executar: () => unknown;
}

/** As telas na ordem da barra lateral: é a ordem do Ctrl+1…9. */
export const TELAS_POR_NUMERO: Screen[] = [
  "chat",
  "discover",
  "models",
  "agenticow",
  "owcli",
  "server",
  "providers",
  "studio",
  "settings",
];

export interface ContextoDosComandos {
  modo: Modo;
  idioma: string;
  tema: Tema;
  mostrarAtalhos: () => void;
  /** Recebe o erro de uma ação que falhou (subir o servidor, por exemplo). */
  aoFalhar: (erro: unknown) => void;
}

export function novaConversa() {
  chatStore.requestNew();
  navigate("chat");
}

export function montarComandos(t: TFunction, ctx: ContextoDosComandos): Comando[] {
  const tela = (s: Screen, i: number): Comando => ({
    id: `screen.${s}`,
    grupo: "screens",
    rotulo: t("palette.goTo", { screen: t(`nav.${s}`) }),
    palavras: s,
    atalho: `Ctrl+${i + 1}`,
    executar: () => navigate(s),
  });
  const aba = (s: Screen, rotulo: string, payload: NavPayload, id: string): Comando => ({
    id: `tab.${id}`,
    grupo: "screens",
    rotulo: t("palette.goToTab", { screen: t(`nav.${s}`), tab: rotulo }),
    executar: () => navigate(s, payload),
  });
  const acao = (fn: () => Promise<unknown>) => () => fn().catch(ctx.aoFalhar);

  return [
    ...TELAS_POR_NUMERO.map(tela),
    ...(["overview", "performance", "network", "advanced"] as const).map((tab) =>
      aba("server", t(`server.tabs.${tab}`), { serverTab: tab }, `server.${tab}`),
    ),
    aba("providers", t("providers.name.openrouter"), { providersTab: "openrouter" }, "providers.openrouter"),
    aba("providers", t("providers.name.9router"), { providersTab: "9router" }, "providers.9router"),
    aba("providers", t("providers.gateway.tab"), { providersTab: "gateway" }, "providers.gateway"),
    aba("providers", t("providers.decisions.tab"), { providersTab: "decisions" }, "providers.decisions"),
    {
      id: "chat.new",
      grupo: "actions",
      rotulo: t("chat.newChat"),
      palavras: "new chat conversa",
      atalho: "Ctrl+N",
      executar: novaConversa,
    },
    {
      id: "terminal.new",
      grupo: "actions",
      rotulo: t("palette.newTerminal"),
      palavras: "terminal shell bash owcli",
      executar: () => {
        navigate("owcli");
        // Sob demanda: o módulo dos terminais carrega o xterm inteiro.
        void import("./terminals").then((m) => m.novoShell());
      },
    },
    {
      id: "owcli.agent",
      grupo: "actions",
      rotulo: t("palette.openAgent"),
      palavras: "agente agent owcli codex código code ia ai",
      executar: () => {
        navigate("owcli");
        // A tela decide: abrir, instalar antes, ou dizer que não há pacote.
        void import("./terminals").then((m) => m.pedirAgente());
      },
    },
    {
      id: "help",
      grupo: "actions",
      rotulo: t("palette.help"),
      palavras: "ajuda help documentação docs guia guide site",
      executar: () => void openUrl(enderecoDaAjuda(ctx.idioma)),
    },
    {
      id: "shortcuts",
      grupo: "actions",
      rotulo: t("palette.shortcuts"),
      palavras: "atalhos teclado keyboard shortcuts",
      atalho: "?",
      executar: ctx.mostrarAtalhos,
    },
    {
      id: "server.start",
      grupo: "server",
      rotulo: t("palette.serverStart"),
      palavras: "motor engine llama iniciar start",
      executar: acao(startServer),
    },
    {
      id: "server.stop",
      grupo: "server",
      rotulo: t("palette.serverStop"),
      palavras: "motor engine llama parar stop",
      executar: acao(stopServer),
    },
    {
      id: "server.restart",
      grupo: "server",
      rotulo: t("palette.serverRestart"),
      palavras: "motor engine llama reiniciar restart",
      executar: acao(() => restartServer()),
    },
    ctx.modo === "simples"
      ? {
          id: "mode.advanced",
          grupo: "preferences",
          rotulo: t("palette.modeAdvanced"),
          palavras: "modo mode avançado advanced",
          executar: () => definirModo("avancado"),
        }
      : {
          id: "mode.simple",
          grupo: "preferences",
          rotulo: t("palette.modeSimple"),
          palavras: "modo mode simples simple",
          executar: () => definirModo("simples"),
        },
    ctx.tema === "dark"
      ? {
          id: "theme.light",
          grupo: "preferences",
          rotulo: t("palette.themeLight"),
          palavras: "tema theme claro light",
          executar: () => aplicarTema("light"),
        }
      : {
          id: "theme.dark",
          grupo: "preferences",
          rotulo: t("palette.themeDark"),
          palavras: "tema theme escuro dark",
          executar: () => aplicarTema("dark"),
        },
  ];
}

/** Minúsculas e sem acento: "conf" acha "Configurações", "servidor" acha "Servidor". */
export function normalizar(texto: string): string {
  return texto
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase();
}

/**
 * Os comandos que batem com a busca: cada palavra digitada tem de aparecer no
 * rótulo ou nas palavras extras. Quem começa pela busca vem antes; o resto
 * mantém a ordem do registro.
 */
export function filtrarComandos(lista: Comando[], busca: string): Comando[] {
  const termos = normalizar(busca).split(/\s+/).filter(Boolean);
  if (termos.length === 0) return lista;
  const achados = lista.filter((c) => {
    const alvo = normalizar(`${c.rotulo} ${c.palavras ?? ""}`);
    return termos.every((t) => alvo.includes(t));
  });
  const inicio = termos[0];
  const comeca = (c: Comando) =>
    normalizar(c.rotulo)
      .split(/\s+/)
      .some((p) => p.startsWith(inicio));
  return [...achados.filter(comeca), ...achados.filter((c) => !comeca(c))];
}
