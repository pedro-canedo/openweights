// Avisos passageiros: "copiado", "modelo pronto → Conversar", "apagado →
// Desfazer".
//
// Sem isto, o fim de um download não avisava nada, apagar não tinha volta, e
// confirmações de cópia só trocavam um ícone que o leitor de tela não
// anunciava. O aviso mora numa região `aria-live` que existe desde o boot —
// região criada junto com o texto não é anunciada — e pausa o relógio
// enquanto o mouse ou o foco estiverem nele, para dar tempo de clicar na ação.

import { useEffect, useRef, useSyncExternalStore, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { IconButton } from "./Button";

export type ToastTone = "info" | "ok" | "warn" | "bad";

export interface ToastOptions {
  message: ReactNode;
  tone?: ToastTone;
  /** Uma ação só ("Desfazer", "Conversar"). Clicar fecha o aviso. */
  action?: { label: string; run: () => void };
  /** Milissegundos na tela; 0 = até alguém fechar. */
  duration?: number;
}

interface Aviso extends ToastOptions {
  id: number;
}

const DURACAO_PADRAO_MS = 5000;
const MAXIMO = 4;

let avisos: Aviso[] = [];
let proximo = 1;
const ouvintes = new Set<() => void>();
const avisar = () => ouvintes.forEach((f) => f());

export function toast(opcoes: ToastOptions): number {
  const id = proximo++;
  avisos = [...avisos, { ...opcoes, id }].slice(-MAXIMO);
  avisar();
  return id;
}

export function dismissToast(id: number) {
  avisos = avisos.filter((a) => a.id !== id);
  avisar();
}

const COR: Record<ToastTone, string> = {
  info: "border-edge",
  ok: "border-ok/50",
  warn: "border-warn/50",
  bad: "border-bad/50",
};

function Item({ aviso }: { aviso: Aviso }) {
  const { t } = useTranslation();
  const pausado = useRef(false);
  const duracao = aviso.duration ?? DURACAO_PADRAO_MS;

  useEffect(() => {
    if (duracao === 0) return;
    let restante = duracao;
    let ultimo = Date.now();
    const relogio = window.setInterval(() => {
      const agora = Date.now();
      if (!pausado.current) restante -= agora - ultimo;
      ultimo = agora;
      if (restante <= 0) dismissToast(aviso.id);
    }, 200);
    return () => window.clearInterval(relogio);
  }, [aviso.id, duracao]);

  return (
    <div
      role={aviso.tone === "bad" ? "alert" : undefined}
      onMouseEnter={() => (pausado.current = true)}
      onMouseLeave={() => (pausado.current = false)}
      onFocus={() => (pausado.current = true)}
      onBlur={() => (pausado.current = false)}
      className={`pointer-events-auto flex max-w-md items-center gap-3 rounded-xl border bg-panel py-2 pl-4 pr-2 text-sm text-ink shadow-xl ${COR[aviso.tone ?? "info"]}`}
    >
      <span className="min-w-0 flex-1 select-text">{aviso.message}</span>
      {aviso.action && (
        <button
          type="button"
          onClick={() => {
            dismissToast(aviso.id);
            aviso.action!.run();
          }}
          className="shrink-0 rounded-lg px-2 py-1 text-sm font-medium text-accent-ink hover:bg-panel2"
        >
          {aviso.action.label}
        </button>
      )}
      <IconButton
        icon="close"
        size="sm"
        label={t("common.close")}
        onClick={() => dismissToast(aviso.id)}
      />
    </div>
  );
}

/** Montado uma vez no App, acima da barra de status. */
export function ToastHost() {
  const lista = useSyncExternalStore(
    (f) => {
      ouvintes.add(f);
      return () => ouvintes.delete(f);
    },
    () => avisos,
  );
  return (
    <div
      aria-live="polite"
      className="pointer-events-none fixed inset-x-0 bottom-12 z-[55] flex flex-col items-center gap-2 px-4"
    >
      {lista.map((a) => (
        <Item key={a.id} aviso={a} />
      ))}
    </div>
  );
}
