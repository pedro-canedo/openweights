// Os poucos blocos que as telas de configuração compartilham.
//
// Servidor, Ajustes e Fontes respondem à mesma família de perguntas — "o que
// está ligado?", "o que eu preciso preencher?", "onde ficam as coisas que eu
// só mexo uma vez?" — e vinham respondendo cada uma com uma pilha de cards
// de peso idêntico. Sem hierarquia, quem nunca montou um servidor não sabe
// por onde começar, e quem sabe precisa rolar a tela inteira para achar o
// campo que veio buscar.
//
// O que este arquivo estabelece:
//
// - `Page` — o mesmo cabeçalho em todas: título, uma linha do que a tela faz.
// - `Tabs` — a divisão por assunto, com a fita da marca sob a aba ativa.
// - `Card` — o contêiner único; título opcional, uma linha de explicação em
//   linguagem comum e um canto para a ação principal.
// - `Collapse` — o que é técnico começa fechado, sem sumir.
// - `StatusDot` / `Row` — estado e par rótulo-valor, ditos do mesmo jeito.
//
// Nada aqui sabe o que é um servidor ou um provedor: são caixas.

import {
  useEffect,
  useState,
  type ReactNode,
} from "react";

// ---------------------------------------------------------------- página ---

export function Page({
  title,
  subtitle,
  actions,
  children,
  wide,
}: {
  title: string;
  subtitle?: string;
  /** O que fica à direita do título — um botão que vale para a tela toda. */
  actions?: ReactNode;
  children: ReactNode;
  /** Telas com tabela ou lista longa respiram melhor em 5xl. */
  wide?: boolean;
}) {
  return (
    <div className={`mx-auto ${wide ? "max-w-5xl" : "max-w-4xl"} px-8 py-8`}>
      <header className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <h1 className="text-xl font-semibold">{title}</h1>
          {subtitle && (
            <p className="mt-1 max-w-2xl text-sm leading-relaxed text-dim">
              {subtitle}
            </p>
          )}
        </div>
        {actions && <div className="shrink-0">{actions}</div>}
      </header>
      {children}
    </div>
  );
}

// ------------------------------------------------------------------ abas ---

export interface TabDef {
  id: string;
  label: string;
  /** Número ou "!" ao lado do rótulo: o que pede atenção nesta aba. */
  badge?: string | number;
  /** Chama a atenção sem contar nada — usado para "há algo errado aqui". */
  alert?: boolean;
}

/**
 * A divisão por assunto.
 *
 * A aba escolhida sobrevive ao ir e voltar de tela (`memoryKey`): quem estava
 * mexendo em flags não quer recomeçar na visão geral toda vez.
 */
export function Tabs({
  tabs,
  value,
  onChange,
}: {
  tabs: TabDef[];
  value: string;
  onChange: (id: string) => void;
}) {
  return (
    <div
      role="tablist"
      className="mt-6 flex items-center gap-1 overflow-x-auto border-b border-edge"
    >
      {tabs.map((tab) => {
        const ativa = tab.id === value;
        return (
          <button
            key={tab.id}
            role="tab"
            aria-selected={ativa}
            onClick={() => onChange(tab.id)}
            className={`relative shrink-0 px-3.5 py-2.5 text-sm transition-colors ${
              ativa ? "text-ink" : "text-dim hover:text-ink"
            }`}
          >
            <span className="flex items-center gap-1.5">
              {tab.label}
              {tab.badge != null && (
                <span className="rounded-full bg-panel2 px-1.5 py-0.5 text-[10px] text-dim">
                  {tab.badge}
                </span>
              )}
              {tab.alert && <span className="h-1.5 w-1.5 rounded-full bg-warn" />}
            </span>
            {/* A fita da marca marca onde você está. */}
            {ativa && (
              <span className="brand-gradient absolute inset-x-2 -bottom-px h-0.5 rounded-full" />
            )}
          </button>
        );
      })}
    </div>
  );
}

/** Lembra a aba escolhida entre visitas — e volta ao padrão se não der. */
export function useTab(memoryKey: string, padrao: string) {
  const [tab, setTab] = useState(() => {
    try {
      return localStorage.getItem(memoryKey) ?? padrao;
    } catch {
      return padrao;
    }
  });
  useEffect(() => {
    try {
      localStorage.setItem(memoryKey, tab);
    } catch {
      // sem armazenamento: a escolha vale só nesta visita
    }
  }, [memoryKey, tab]);
  return [tab, setTab] as const;
}

// ------------------------------------------------------------------ card ---

export function Card({
  title,
  hint,
  action,
  children,
  tone = "normal",
  className = "",
}: {
  title?: ReactNode;
  /** Uma linha em linguagem comum: para que serve o que está aqui dentro. */
  hint?: ReactNode;
  /** O canto superior direito: a ação principal deste card. */
  action?: ReactNode;
  children?: ReactNode;
  /** `warn` e `bad` tingem a borda — um card que pede providência. */
  tone?: "normal" | "warn" | "bad" | "ok";
  className?: string;
}) {
  const borda =
    tone === "warn"
      ? "border-warn/40"
      : tone === "bad"
        ? "border-bad/40"
        : tone === "ok"
          ? "border-ok/30"
          : "border-edge";
  return (
    <section
      className={`mt-4 rounded-xl border ${borda} bg-panel p-5 ${className}`}
    >
      {(title || action) && (
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            {title && <h2 className="text-sm font-medium text-ink">{title}</h2>}
            {hint && (
              <p className="mt-0.5 max-w-2xl text-[12px] leading-relaxed text-dim">
                {hint}
              </p>
            )}
          </div>
          {action && <div className="shrink-0">{action}</div>}
        </div>
      )}
      {/* Sem cabeçalho nenhum, a explicação abre o card sozinha — e não pode
          sair duas vezes quando existe `action` sem `title`. */}
      {!title && !action && hint && (
        <p className="max-w-2xl text-[12px] leading-relaxed text-dim">{hint}</p>
      )}
      {children}
    </section>
  );
}

/** Uma seção que começa fechada: o que é técnico não some, só espera. */
export function Collapse({
  title,
  hint,
  badge,
  defaultOpen = false,
  children,
}: {
  title: string;
  hint?: string;
  badge?: ReactNode;
  defaultOpen?: boolean;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <section className="mt-4 rounded-xl border border-edge bg-panel">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className="flex w-full items-center gap-3 px-5 py-3.5 text-left"
      >
        <span className="min-w-0">
          <span className="flex items-center gap-2 text-sm font-medium">
            {title}
            {badge}
          </span>
          {hint && (
            <span className="mt-0.5 block text-[12px] leading-relaxed text-dim">
              {hint}
            </span>
          )}
        </span>
        <span className="ml-auto shrink-0 text-dim">{open ? "▾" : "▸"}</span>
      </button>
      {open && <div className="border-t border-edge px-5 py-4">{children}</div>}
    </section>
  );
}

// ---------------------------------------------------------------- estado ---

export type Tone = "ok" | "warn" | "bad" | "off" | "busy";

const COR: Record<Tone, string> = {
  ok: "bg-ok",
  warn: "bg-warn",
  bad: "bg-bad",
  off: "bg-dim",
  busy: "bg-accent",
};

/** A bolinha de estado — o mesmo vocabulário de cor em todas as telas. */
export function StatusDot({ tone, pulse }: { tone: Tone; pulse?: boolean }) {
  return (
    <span className="relative flex h-2.5 w-2.5 shrink-0">
      {pulse && (
        <span
          className={`absolute inline-flex h-full w-full animate-ping rounded-full opacity-60 ${COR[tone]}`}
        />
      )}
      <span className={`relative inline-flex h-2.5 w-2.5 rounded-full ${COR[tone]}`} />
    </span>
  );
}

/** Linha rótulo → valor, para os pares que se leem em sequência. */
export function Row({
  label,
  children,
  hint,
}: {
  label: string;
  children: ReactNode;
  hint?: string;
}) {
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 py-1.5">
      <span className="w-40 shrink-0 text-[12px] text-dim" title={hint}>
        {label}
      </span>
      <span className="min-w-0 flex-1 text-sm">{children}</span>
    </div>
  );
}
