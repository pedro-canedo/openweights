// Diálogo modal e confirmação.
//
// Os modais do app eram `div fixed inset-0` feitos à mão, sem `role`: o leitor
// de tela não sabia que havia um diálogo, o Tab passeava pelo fundo, e a tela
// do AgenticOw — que esconde a própria webview nativa quando acha um
// `[role="dialog"]` — nunca achava nada, e a webview cobria o modal.
//
// `confirmar()` substitui o `window.confirm`: devolve uma promessa, então a
// troca é de uma linha (`if (!(await confirmar({...}))) return;`), e o
// diálogo tem a cara do app, diz o que vai acontecer e não trava a janela.

import {
  useId,
  useRef,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { Button } from "./Button";
import { useCamada } from "./camada";

const LARGURA = {
  sm: "max-w-sm",
  md: "max-w-lg",
  lg: "max-w-2xl",
  xl: "max-w-5xl",
} as const;

export function Dialog({
  open,
  onClose,
  title,
  hideTitle = false,
  description,
  children,
  footer,
  size = "md",
  dismissable = true,
  bare = false,
  className = "",
}: {
  open: boolean;
  onClose: () => void;
  /** Sempre há um nome — `hideTitle` só o tira da vista. */
  title: ReactNode;
  hideTitle?: boolean;
  description?: ReactNode;
  children?: ReactNode;
  /** Os botões, alinhados à direita. */
  footer?: ReactNode;
  size?: keyof typeof LARGURA;
  /** `false` para o que não pode ser interrompido (instalação em curso). */
  dismissable?: boolean;
  /**
   * Layout próprio (editor, prévia): o painel só leva o papel de diálogo e o
   * `className` de quem usa; o título fica só para o leitor de tela.
   */
  bare?: boolean;
  /** Troca o acolchoamento padrão; com `bare`, é o estilo inteiro do painel. */
  className?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const id = useId();
  useCamada(ref, {
    aberto: open,
    onFechar: () => {
      if (dismissable) onClose();
    },
    prenderFoco: true,
    fecharFora: false,
  });
  if (!open) return null;
  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
      onMouseDown={(e) => {
        if (dismissable && e.target === e.currentTarget) onClose();
      }}
    >
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby={`${id}-t`}
        aria-describedby={description ? `${id}-d` : undefined}
        tabIndex={-1}
        data-overlay=""
        className={
          bare
            ? `outline-none ${className}`
            : `flex max-h-[calc(100vh-2rem)] w-full ${LARGURA[size]} flex-col overflow-y-auto rounded-2xl border border-edge bg-panel shadow-2xl outline-none ${className || "p-6"}`
        }
      >
        <h2
          id={`${id}-t`}
          className={hideTitle || bare ? "sr-only" : "text-base font-semibold text-ink"}
        >
          {title}
        </h2>
        {description && (
          <p id={`${id}-d`} className="mt-1.5 text-sm leading-relaxed text-dim">
            {description}
          </p>
        )}
        {children}
        {footer && (
          <div className="mt-6 flex flex-wrap justify-end gap-2">{footer}</div>
        )}
      </div>
    </div>,
    document.body,
  );
}

// ------------------------------------------------------------ confirmação ---

export interface ConfirmOptions {
  title: string;
  /** O que vai acontecer, em linguagem comum (com tamanho, quando houver). */
  message?: ReactNode;
  confirmLabel: string;
  cancelLabel?: string;
  /** `danger` para o que apaga ou não tem volta. */
  tone?: "danger" | "primary";
}

interface Pedido extends ConfirmOptions {
  responder: (sim: boolean) => void;
}

let pedido: Pedido | null = null;
const ouvintes = new Set<() => void>();
const avisar = () => ouvintes.forEach((f) => f());

/** Pergunta e espera a resposta. Uma pergunta nova recusa a anterior. */
export function confirmar(opcoes: ConfirmOptions): Promise<boolean> {
  pedido?.responder(false);
  return new Promise((resolve) => {
    pedido = {
      ...opcoes,
      responder: (sim) => {
        pedido = null;
        avisar();
        resolve(sim);
      },
    };
    avisar();
  });
}

/** Montado uma vez no App: é onde os `confirmar()` aparecem. */
export function ConfirmHost() {
  const { t } = useTranslation();
  const atual = useSyncExternalStore(
    (f) => {
      ouvintes.add(f);
      return () => ouvintes.delete(f);
    },
    () => pedido,
  );
  return (
    <Dialog
      open={atual !== null}
      onClose={() => atual?.responder(false)}
      title={atual?.title ?? ""}
      description={atual?.message}
      size="sm"
      footer={
        atual && (
          <>
            {/* No que apaga, o foco nasce no Cancelar: um Enter distraído
                não pode ser o que destrói. */}
            <Button
              onClick={() => atual.responder(false)}
              data-autofocus={atual.tone === "danger" ? "" : undefined}
            >
              {atual.cancelLabel ?? t("common.cancel")}
            </Button>
            <Button
              variant={atual.tone === "danger" ? "danger" : "primary"}
              onClick={() => atual.responder(true)}
              data-autofocus={atual.tone === "danger" ? undefined : ""}
            >
              {atual.confirmLabel}
            </Button>
          </>
        )
      }
    />
  );
}
