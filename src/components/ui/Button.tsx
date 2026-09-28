// Botões, campo de texto e selo — os três controles que cada tela reescrevia.
//
// A auditoria contou 34 botões primários em ~26 variações de classe (px-2.5 a
// px-6, três raios, dois jeitos de desabilitar), 46 secundários com três
// estilos de hover e 35 selos em 28 variações. Não era escolha de design, era
// cópia com deriva. Daqui em diante, tela nova usa isto; tela velha migra
// quando for tocada.
//
// - O botão cheio usa `accent-fill`, não o roxo da marca: branco sobre o roxo
//   da marca fica em 4,4:1, abaixo do AA.
// - `IconButton` exige `label`: é o nome que o leitor de tela lê e a dica que
//   aparece no hover. Botão só de ícone sem nome não compila.
// - Alvo mínimo de 24px (WCAG 2.5.8).

import { useId, type ComponentProps, type ReactNode } from "react";
import Icon, { type IconName } from "./Icon";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
export type ButtonSize = "sm" | "md";

const BASE =
  "inline-flex shrink-0 items-center justify-center gap-1.5 rounded-lg font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50";

const VARIANTE: Record<ButtonVariant, string> = {
  primary: "bg-accent-fill text-white hover:brightness-110 disabled:hover:brightness-100",
  secondary:
    "border border-edge bg-panel text-ink hover:border-edge-strong hover:bg-panel2",
  ghost: "text-dim hover:bg-panel2 hover:text-ink",
  danger: "border border-bad/40 text-bad hover:bg-bad/10",
};

const TAMANHO: Record<ButtonSize, string> = {
  sm: "min-h-7 px-2.5 text-xs",
  md: "min-h-9 px-4 text-sm",
};

/** As classes do botão, para quando o elemento precisa ser outro (um `<a>`). */
export function buttonClass(
  variant: ButtonVariant = "secondary",
  size: ButtonSize = "md",
  extra = "",
) {
  return `${BASE} ${VARIANTE[variant]} ${TAMANHO[size]} ${extra}`.trim();
}

export function Button({
  variant = "secondary",
  size = "md",
  icon,
  busy = false,
  className = "",
  children,
  disabled,
  type = "button",
  ...resto
}: ComponentProps<"button"> & {
  variant?: ButtonVariant;
  size?: ButtonSize;
  icon?: IconName;
  /** Trabalhando: desabilita e anuncia, sem trocar o texto (a largura não pula). */
  busy?: boolean;
}) {
  return (
    <button
      type={type}
      disabled={disabled || busy}
      aria-busy={busy || undefined}
      className={buttonClass(variant, size, className)}
      {...resto}
    >
      {icon && (
        <Icon
          name={icon}
          className={`${size === "sm" ? "h-3.5 w-3.5" : "h-4 w-4"} ${busy ? "animate-pulse" : ""}`}
        />
      )}
      {children}
    </button>
  );
}

export function IconButton({
  icon,
  label,
  size = "md",
  tone = "normal",
  className = "",
  type = "button",
  ...resto
}: Omit<ComponentProps<"button">, "children" | "aria-label"> & {
  icon: IconName;
  /** Nome acessível e dica de hover — obrigatório. */
  label: string;
  size?: ButtonSize;
  tone?: "normal" | "danger";
}) {
  const caixa = size === "sm" ? "h-6 w-6" : "h-7 w-7";
  const cor =
    tone === "danger"
      ? "text-dim hover:bg-bad/10 hover:text-bad"
      : "text-dim hover:bg-panel2 hover:text-ink";
  return (
    <button
      type={type}
      aria-label={label}
      title={label}
      className={`inline-grid shrink-0 place-items-center rounded-lg transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${caixa} ${cor} ${className}`}
      {...resto}
    >
      <Icon name={icon} className={size === "sm" ? "h-3.5 w-3.5" : "h-4 w-4"} />
    </button>
  );
}

// ----------------------------------------------------------------- campo ---

/** Classes de campo, para `<textarea>` e `<select>` que não usam `Input`. */
export const inputClass =
  "w-full rounded-lg border border-edge-strong bg-panel2 px-3 py-1.5 text-sm text-ink placeholder:text-dim outline-none transition-colors focus:border-accent-ink disabled:opacity-50";

/**
 * Campo com rótulo de verdade (`<label htmlFor>`): clicar no rótulo foca o
 * campo e o leitor de tela diz o nome. `hideLabel` mantém o nome para quem
 * não vê a tela e tira da vista — é o caso das buscas, cujo contexto já diz.
 */
export function Input({
  label,
  hideLabel = false,
  hint,
  error,
  className = "",
  id,
  ...resto
}: ComponentProps<"input"> & {
  label: string;
  hideLabel?: boolean;
  hint?: ReactNode;
  error?: ReactNode;
}) {
  const gerado = useId();
  const campo = id ?? gerado;
  const descricao = error ? `${campo}-e` : hint ? `${campo}-h` : undefined;
  return (
    <div className={className}>
      <label
        htmlFor={campo}
        className={hideLabel ? "sr-only" : "mb-1 block text-[12px] text-dim"}
      >
        {label}
      </label>
      <input
        id={campo}
        aria-invalid={error ? true : undefined}
        aria-describedby={descricao}
        className={`${inputClass} ${error ? "border-bad" : ""}`}
        {...resto}
      />
      {error ? (
        <p id={`${campo}-e`} className="mt-1 text-[12px] text-bad">
          {error}
        </p>
      ) : hint ? (
        <p id={`${campo}-h`} className="mt-1 text-[12px] text-dim">
          {hint}
        </p>
      ) : null}
    </div>
  );
}

// ------------------------------------------------------------------ selo ---

export type BadgeTone = "neutral" | "accent" | "ok" | "warn" | "bad";

const SELO: Record<BadgeTone, string> = {
  neutral: "bg-panel2 text-dim",
  accent: "bg-accent/15 text-accent-ink",
  ok: "bg-ok/15 text-ok",
  warn: "bg-warn/15 text-warn",
  bad: "bg-bad/15 text-bad",
};

export function Badge({
  tone = "neutral",
  icon,
  children,
  className = "",
  title,
}: {
  tone?: BadgeTone;
  icon?: IconName;
  children: ReactNode;
  className?: string;
  title?: string;
}) {
  return (
    <span
      title={title}
      className={`inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[11px] font-medium ${SELO[tone]} ${className}`}
    >
      {icon && <Icon name={icon} className="h-3 w-3" />}
      {children}
    </span>
  );
}
