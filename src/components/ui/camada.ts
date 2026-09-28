// O que todo painel sobreposto faz igual: diálogo, popover, menu.
//
// Cada painel do app resolvia isto sozinho, e cada um esquecia uma parte: o
// Onboarding não fechava no Esc e deixava o Tab passear pelo fundo, o monitor
// fechava no clique fora mas não no Esc, e nenhum devolvia o foco a quem o
// abriu — quem navega pelo teclado caía no começo da página.
//
// Regras deste arquivo:
// - Só a camada de cima reage. Um diálogo de confirmação aberto a partir de um
//   popover fecha sozinho no Esc; o popover embaixo continua aberto.
// - Ao fechar, o foco volta ao elemento que estava focado antes de abrir.
// - `prenderFoco` (diálogo modal): Tab e Shift+Tab giram dentro do painel.
// - Todo painel leva `data-overlay`. É assim que a tela do AgenticOw sabe que
//   precisa esconder a webview nativa, que ficaria por cima de qualquer HTML.

import { useEffect, useRef, type RefObject } from "react";

const FOCAVEIS = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled]):not([type='hidden'])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "summary",
  "[tabindex]:not([tabindex='-1'])",
].join(",");

/** Os elementos que o Tab alcança dentro de `raiz`, na ordem do documento. */
export function focaveis(raiz: HTMLElement): HTMLElement[] {
  return [...raiz.querySelectorAll<HTMLElement>(FOCAVEIS)].filter(
    (el) => el.getClientRects().length > 0 && !el.closest("[inert]"),
  );
}

/** Pilha das camadas abertas: só a última responde a Esc e a clique fora. */
const pilha: symbol[] = [];

export function camadasAbertas(): number {
  return pilha.length;
}

export function useCamada(
  ref: RefObject<HTMLElement | null>,
  {
    aberto,
    onFechar,
    prenderFoco = false,
    fecharFora = true,
    ancora,
  }: {
    aberto: boolean;
    onFechar: () => void;
    /** Diálogo modal: o Tab não sai do painel. */
    prenderFoco?: boolean;
    /** Clique fora fecha (popover e menu). O diálogo trata o fundo sozinho. */
    fecharFora?: boolean;
    /** O botão que abriu: clicar nele não conta como "fora" (ele alterna). */
    ancora?: RefObject<HTMLElement | null>;
  },
) {
  const fechar = useRef(onFechar);
  fechar.current = onFechar;

  useEffect(() => {
    if (!aberto) return;
    const eu = Symbol("camada");
    pilha.push(eu);
    const anterior = document.activeElement as HTMLElement | null;
    const raiz = ref.current;

    // Foco inicial: quem pediu (`data-autofocus`), senão o primeiro controle
    // do diálogo, senão o próprio painel — nunca o fundo.
    const inicial =
      raiz?.querySelector<HTMLElement>("[data-autofocus]") ??
      (prenderFoco && raiz ? focaveis(raiz)[0] : null) ??
      raiz;
    inicial?.focus({ preventScroll: true });

    const noTopo = () => pilha[pilha.length - 1] === eu;

    function tecla(e: KeyboardEvent) {
      if (!noTopo()) return;
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        fechar.current();
        return;
      }
      if (prenderFoco && e.key === "Tab" && raiz) {
        // Um editor de código usa o Tab para indentar (`data-keeps-tab`):
        // ali o Tab é dele, e o Shift+Tab continua saindo para os botões.
        const ativo = document.activeElement as HTMLElement | null;
        if (!e.shiftKey && ativo?.closest("[data-keeps-tab]")) return;
        const lista = focaveis(raiz);
        if (lista.length === 0) {
          e.preventDefault();
          raiz.focus();
          return;
        }
        const primeiro = lista[0];
        const ultimo = lista[lista.length - 1];
        const atual = document.activeElement;
        if (e.shiftKey && (atual === primeiro || atual === raiz)) {
          e.preventDefault();
          ultimo.focus();
        } else if (!e.shiftKey && atual === ultimo) {
          e.preventDefault();
          primeiro.focus();
        } else if (!raiz.contains(atual)) {
          e.preventDefault();
          primeiro.focus();
        }
      }
    }

    function clique(e: MouseEvent) {
      if (!fecharFora || !noTopo() || !raiz) return;
      const alvo = e.target as Node;
      if (raiz.contains(alvo) || ancora?.current?.contains(alvo)) return;
      // Um clique dentro de outra camada (ex.: a dica aberta de dentro deste
      // painel, que mora num portal) não fecha esta.
      if ((alvo as Element).closest?.("[data-overlay]")) return;
      fechar.current();
    }

    document.addEventListener("keydown", tecla, true);
    document.addEventListener("mousedown", clique, true);
    return () => {
      document.removeEventListener("keydown", tecla, true);
      document.removeEventListener("mousedown", clique, true);
      const i = pilha.indexOf(eu);
      if (i >= 0) pilha.splice(i, 1);
      // Devolve o foco só se ele ainda estiver dentro do painel que fechou
      // (ou perdido no <body>): quem clicou em outro campo fica onde clicou.
      const agora = document.activeElement;
      const perdido = !agora || agora === document.body || raiz?.contains(agora);
      if (perdido && anterior && document.contains(anterior)) {
        anterior.focus({ preventScroll: true });
      }
    };
    // `ref`, `ancora` e as opções são estáveis durante a vida da camada.
  }, [aberto]);
}

/**
 * Para os painéis que embrulham botão e conteúdo no mesmo elemento (o
 * `ref` devolvido): fecha no Esc e no clique fora do embrulho, entra na pilha
 * de camadas e devolve o foco ao botão. O painel em si ainda precisa de
 * `data-overlay` para a webview do AgenticOw se esconder.
 */
export function useDismiss<T extends HTMLElement = HTMLDivElement>(
  open: boolean,
  onClose: () => void,
) {
  const ref = useRef<T>(null);
  useCamada(ref, { aberto: open, onFechar: onClose });
  return ref;
}
