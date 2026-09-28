// Dois painéis com um divisor que se arrasta — ou se move pelo teclado.
//
// Existe para a grade de terminais do OwCLI e para os painéis de largura fixa
// do app (a lista do Descobrir em 380px, os parâmetros do Chat em 288px), que
// numa tela grande sobram e numa pequena apertam. O tamanho escolhido fica
// guardado por `storageKey`.
//
// O divisor é um `role="separator"` focável: setas movem 16px (Shift, 64px),
// Home e End levam aos limites. Arrastar sem teclado alternativo deixaria de
// fora quem não usa mouse.

import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent,
  type ReactNode,
} from "react";

const PASSO = 16;
const PASSO_LONGO = 64;

function lerGuardado(chave: string | undefined, padrao: number) {
  if (!chave) return padrao;
  try {
    const v = Number(localStorage.getItem(chave));
    return Number.isFinite(v) && v > 0 ? v : padrao;
  } catch {
    return padrao;
  }
}

export function Split({
  direction = "horizontal",
  defaultSize,
  min = 160,
  minSecond = 160,
  storageKey,
  label,
  first,
  second,
  className = "",
}: {
  /** `horizontal`: lado a lado. `vertical`: um em cima do outro. */
  direction?: "horizontal" | "vertical";
  /** Tamanho inicial do primeiro painel, em px. */
  defaultSize: number;
  min?: number;
  /** O mínimo que sobra para o segundo painel. */
  minSecond?: number;
  storageKey?: string;
  /** Nome do divisor para o leitor de tela ("Largura da lista de modelos"). */
  label: string;
  first: ReactNode;
  second: ReactNode;
  className?: string;
}) {
  const caixa = useRef<HTMLDivElement>(null);
  const [tamanho, setTamanho] = useState(() => lerGuardado(storageKey, defaultSize));
  const [limite, setLimite] = useState(Number.POSITIVE_INFINITY);
  const lado = direction === "horizontal";

  // O máximo depende do espaço de verdade, que muda com a janela.
  useEffect(() => {
    const el = caixa.current;
    if (!el) return;
    const medir = () => {
      const total = lado ? el.clientWidth : el.clientHeight;
      if (total > 0) setLimite(Math.max(min, total - minSecond));
    };
    medir();
    const obs = new ResizeObserver(medir);
    obs.observe(el);
    return () => obs.disconnect();
  }, [lado, min, minSecond]);

  const aplicar = useCallback(
    (v: number) => {
      const certo = Math.round(Math.min(Math.max(v, min), limite));
      setTamanho(certo);
      if (storageKey) {
        try {
          localStorage.setItem(storageKey, String(certo));
        } catch {
          // sem armazenamento: vale só nesta visita
        }
      }
    },
    [min, limite, storageKey],
  );

  const efetivo = Math.min(Math.max(tamanho, min), limite);

  function arrastar(e: PointerEvent<HTMLDivElement>) {
    const el = caixa.current;
    if (!el || e.button !== 0) return;
    e.preventDefault();
    const alvo = e.currentTarget;
    alvo.setPointerCapture(e.pointerId);
    const origem = el.getBoundingClientRect();
    const mover = (ev: globalThis.PointerEvent) =>
      aplicar(lado ? ev.clientX - origem.left : ev.clientY - origem.top);
    const soltar = () => {
      alvo.removeEventListener("pointermove", mover);
      alvo.removeEventListener("pointerup", soltar);
      alvo.removeEventListener("pointercancel", soltar);
    };
    alvo.addEventListener("pointermove", mover);
    alvo.addEventListener("pointerup", soltar);
    alvo.addEventListener("pointercancel", soltar);
  }

  function tecla(e: KeyboardEvent<HTMLDivElement>) {
    const passo = e.shiftKey ? PASSO_LONGO : PASSO;
    const menos = lado ? "ArrowLeft" : "ArrowUp";
    const mais = lado ? "ArrowRight" : "ArrowDown";
    if (e.key === menos) aplicar(efetivo - passo);
    else if (e.key === mais) aplicar(efetivo + passo);
    else if (e.key === "Home") aplicar(min);
    else if (e.key === "End") aplicar(limite);
    else return;
    e.preventDefault();
  }

  return (
    <div
      ref={caixa}
      className={`flex min-h-0 min-w-0 ${lado ? "flex-row" : "flex-col"} ${className}`}
    >
      <div
        className="min-h-0 min-w-0 shrink-0 overflow-hidden"
        style={lado ? { width: efetivo } : { height: efetivo }}
      >
        {first}
      </div>
      <div
        role="separator"
        aria-orientation={lado ? "vertical" : "horizontal"}
        aria-label={label}
        aria-valuenow={Math.round(efetivo)}
        aria-valuemin={min}
        aria-valuemax={Number.isFinite(limite) ? Math.round(limite) : undefined}
        tabIndex={0}
        onPointerDown={arrastar}
        onKeyDown={tecla}
        className={`group relative shrink-0 touch-none ${
          lado ? "w-1.5 cursor-col-resize" : "h-1.5 cursor-row-resize"
        }`}
      >
        <span
          className={`absolute bg-edge transition-colors group-hover:bg-accent group-focus-visible:bg-accent ${
            lado ? "inset-y-0 left-1/2 w-px -translate-x-1/2" : "inset-x-0 top-1/2 h-px -translate-y-1/2"
          }`}
        />
      </div>
      <div className="min-h-0 min-w-0 flex-1 overflow-hidden">{second}</div>
    </div>
  );
}
