// Popover e menu: painéis presos a um botão, que não bloqueiam a tela.
//
// Os sete painéis flutuantes do app (anexos, modelo, propriedades do chat,
// contexto, monitor, downloads, geração) tinham cada um seu Esc — ou não
// tinham —, nenhum dizia ao leitor de tela que estava aberto
// (`aria-expanded`) e nenhum se marcava como camada, então a webview do
// AgenticOw ficava por cima deles.
//
// O posicionamento continua com quem usa (`className`, geralmente
// `absolute … top-full`): cada painel tem seu canto certo, e trocar isso por
// cálculo de posição seria mexer em sete layouts para ganhar nada.

import {
  useId,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
  type RefObject,
} from "react";
import Icon, { type IconName } from "./Icon";
import { useCamada } from "./camada";

/** Estado de um popover e as props do botão que o abre. */
export function usePopover(tipo: "dialog" | "menu" = "dialog") {
  const [open, setOpen] = useState(false);
  const anchorRef = useRef<HTMLButtonElement>(null);
  const id = useId();
  const close = () => setOpen(false);
  return {
    open,
    setOpen,
    close,
    toggle: () => setOpen((v) => !v),
    triggerProps: {
      ref: anchorRef,
      "aria-expanded": open,
      "aria-haspopup": tipo,
      "aria-controls": open ? id : undefined,
      onClick: () => setOpen((v) => !v),
    },
    popoverProps: { id, open, onClose: close, anchorRef },
  };
}

export function Popover({
  id,
  open,
  onClose,
  anchorRef,
  label,
  className = "",
  children,
}: {
  id?: string;
  open: boolean;
  onClose: () => void;
  anchorRef?: RefObject<HTMLElement | null>;
  /** Nome acessível do painel ("Anexar", "Monitor de hardware"…). */
  label: string;
  className?: string;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useCamada(ref, { aberto: open, onFechar: onClose, ancora: anchorRef });
  if (!open) return null;
  return (
    <div
      ref={ref}
      id={id}
      role="dialog"
      aria-label={label}
      tabIndex={-1}
      data-overlay=""
      className={`z-40 rounded-xl border border-edge bg-panel shadow-xl outline-none ${className}`}
    >
      {children}
    </div>
  );
}

// ------------------------------------------------------------------ menu ---

export interface MenuItem {
  id: string;
  label: ReactNode;
  icon?: IconName;
  onSelect: () => void;
  danger?: boolean;
  disabled?: boolean;
}

/**
 * Lista de ações: setas para cima e para baixo, Home e End, Enter escolhe,
 * Esc fecha e devolve o foco ao botão. Escolher fecha o menu.
 */
export function Menu({
  id,
  open,
  onClose,
  anchorRef,
  label,
  items,
  className = "",
}: {
  id?: string;
  open: boolean;
  onClose: () => void;
  anchorRef?: RefObject<HTMLElement | null>;
  label: string;
  items: MenuItem[];
  className?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useCamada(ref, { aberto: open, onFechar: onClose, ancora: anchorRef });
  if (!open) return null;

  function mover(e: KeyboardEvent<HTMLDivElement>) {
    const botoes = [
      ...(ref.current?.querySelectorAll<HTMLButtonElement>(
        '[role="menuitem"]:not([disabled])',
      ) ?? []),
    ];
    if (botoes.length === 0) return;
    const i = botoes.indexOf(document.activeElement as HTMLButtonElement);
    const para = (n: number) => {
      e.preventDefault();
      botoes[(n + botoes.length) % botoes.length].focus();
    };
    if (e.key === "ArrowDown") para(i + 1);
    else if (e.key === "ArrowUp") para(i < 0 ? botoes.length - 1 : i - 1);
    else if (e.key === "Home") para(0);
    else if (e.key === "End") para(botoes.length - 1);
  }

  return (
    <div
      ref={ref}
      id={id}
      role="menu"
      aria-label={label}
      tabIndex={-1}
      data-overlay=""
      onKeyDown={mover}
      className={`z-40 min-w-44 rounded-xl border border-edge bg-panel p-1 shadow-xl outline-none ${className}`}
    >
      {items.map((item, n) => (
        <button
          key={item.id}
          type="button"
          role="menuitem"
          disabled={item.disabled}
          data-autofocus={n === 0 ? "" : undefined}
          onClick={() => {
            onClose();
            item.onSelect();
          }}
          className={`flex w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-sm transition-colors disabled:opacity-50 ${
            item.danger
              ? "text-bad hover:bg-bad/10 focus-visible:bg-bad/10"
              : "text-ink hover:bg-panel2 focus-visible:bg-panel2"
          }`}
        >
          {item.icon && <Icon name={item.icon} className="h-4 w-4 text-dim" />}
          {item.label}
        </button>
      ))}
    </div>
  );
}
