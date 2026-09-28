// Dica que aparece no hover E no foco do teclado.
//
// O app explicava quase tudo por `title` nativo (120 ocorrências): só aparece
// com o mouse parado, depois de um atraso que o sistema escolhe, e nunca pelo
// teclado. Esta dica abre no foco na hora, no hover com um pequeno atraso,
// fecha no Esc, e liga o texto ao controle por `aria-describedby`.

import {
  cloneElement,
  useEffect,
  useId,
  useRef,
  useState,
  type ReactElement,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";

const ATRASO_HOVER_MS = 350;

export function Tooltip({
  content,
  children,
  side = "top",
}: {
  content: ReactNode;
  /** Um único elemento focável (botão, link). */
  children: ReactElement<Record<string, unknown>>;
  side?: "top" | "bottom";
}) {
  const id = useId();
  const caixa = useRef<HTMLSpanElement>(null);
  const [pos, setPos] = useState<{ x: number; y: number } | null>(null);
  const timer = useRef<number | undefined>(undefined);

  const mostrar = (agora: boolean) => {
    window.clearTimeout(timer.current);
    const abrir = () => {
      const r = caixa.current?.getBoundingClientRect();
      if (!r) return;
      setPos({ x: r.left + r.width / 2, y: side === "top" ? r.top : r.bottom });
    };
    if (agora) abrir();
    else timer.current = window.setTimeout(abrir, ATRASO_HOVER_MS);
  };
  const esconder = () => {
    window.clearTimeout(timer.current);
    setPos(null);
  };

  useEffect(() => {
    if (!pos) return;
    const tecla = (e: KeyboardEvent) => {
      if (e.key === "Escape") esconder();
    };
    document.addEventListener("keydown", tecla);
    return () => document.removeEventListener("keydown", tecla);
  }, [pos]);

  useEffect(() => () => window.clearTimeout(timer.current), []);

  return (
    <span
      ref={caixa}
      className="inline-flex"
      onMouseEnter={() => mostrar(false)}
      onMouseLeave={esconder}
      onFocus={() => mostrar(true)}
      onBlur={esconder}
    >
      {cloneElement(children, { "aria-describedby": pos ? id : undefined })}
      {pos &&
        createPortal(
          <span
            id={id}
            role="tooltip"
            style={{ left: pos.x, top: pos.y }}
            className={`pointer-events-none fixed z-[60] max-w-xs -translate-x-1/2 rounded-lg border border-edge bg-panel2 px-2.5 py-1.5 text-[12px] leading-snug text-ink shadow-lg ${
              side === "top" ? "-translate-y-[calc(100%+6px)]" : "translate-y-1.5"
            }`}
          >
            {content}
          </span>,
          document.body,
        )}
    </span>
  );
}
