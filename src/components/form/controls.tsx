// Controles de formulário compartilhados — promovidos do antigo painel de
// carga do chat para servirem à tela de configuração do servidor.
//
// A convenção que todos seguem: valor `null`/ausente = "auto" (o llama.cpp
// decide). É o que permite ligar de volta o `--fit` quando ninguém opinou.

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

export function chipClass(active: boolean): string {
  return `rounded-lg border px-2 py-1 text-[11px] transition-colors disabled:opacity-40 ${
    active
      ? "border-accent bg-accent/15 text-ink"
      : "border-edge text-dim hover:border-accent hover:text-ink"
  }`;
}

export function Chips<T extends string>({
  value,
  options,
  disabled,
  onChange,
}: {
  value: T;
  options: { id: T; label: string }[];
  disabled?: boolean;
  onChange: (v: T) => void;
}) {
  return (
    <div className="flex flex-wrap gap-1">
      {options.map((o) => (
        <button
          key={o.id}
          type="button"
          disabled={disabled}
          onClick={() => onChange(o.id)}
          className={chipClass(value === o.id)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** Número opcional: vazio = "auto". Commit no blur/Enter, com clamp. */
export function OptionalNum({
  label,
  hint,
  value,
  min,
  max,
  step,
  placeholder,
  disabled,
  onCommit,
}: {
  /** Ausente = campo "nu" (embutido em outro controle, como o NumChips). */
  label?: string;
  hint?: string;
  value: number | null | undefined;
  min: number;
  max: number;
  step?: number;
  placeholder: string;
  disabled?: boolean;
  onCommit: (n: number | null) => void;
}) {
  const [text, setText] = useState(value == null ? "" : String(value));
  useEffect(() => {
    setText(value == null ? "" : String(value));
  }, [value]);

  const commit = () => {
    const trimmed = text.trim();
    if (!trimmed) {
      onCommit(null);
      return;
    }
    const n = Number(trimmed);
    if (!Number.isFinite(n)) {
      setText(value == null ? "" : String(value));
      return;
    }
    const clamped = Math.min(max, Math.max(min, n));
    // Passo fracionário = flag de ponto flutuante (ex.: spec-draft-p-min):
    // arredondar aqui devolveria 1 quando a pessoa digitou 0.75.
    const fracionario = step != null && step > 0 && step < 1;
    onCommit(fracionario ? Math.round(clamped * 100) / 100 : Math.round(clamped));
  };

  return (
    <label className="flex flex-col gap-1">
      {label && <span className="text-xs text-dim">{label}</span>}
      {hint && <span className="text-[11px] leading-relaxed text-dim">{hint}</span>}
      <input
        type="number"
        min={min}
        max={max}
        step={step ?? 1}
        disabled={disabled}
        placeholder={placeholder}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") (e.target as HTMLInputElement).blur();
        }}
        className="w-24 rounded-lg border border-edge bg-panel2 px-2 py-1.5 text-xs tabular-nums outline-none placeholder:text-dim focus:border-accent disabled:opacity-40"
      />
    </label>
  );
}

/**
 * `<select>` com a receita visual de Ajustes. Um valor gravado que não está
 * nas opções (legado, edição à mão) vira uma opção "atual: X" — a tela nunca
 * mente mostrando outra escolha no lugar da que está valendo.
 */
export function Select({
  value,
  options,
  onChange,
  className,
  disabled,
}: {
  value: string;
  options: { value: string; label: string }[];
  onChange: (v: string) => void;
  className?: string;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  const conhecido = options.some((o) => o.value === value);
  return (
    <select
      value={value}
      disabled={disabled}
      onChange={(e) => onChange(e.target.value)}
      className={`rounded-lg border border-edge bg-panel2 px-3 py-1.5 text-sm outline-none disabled:opacity-40 ${className ?? ""}`}
    >
      {!conhecido && (
        <option value={value}>{t("common.currentValue", { v: value })}</option>
      )}
      {options.map((o) => (
        <option key={o.value} value={o.value}>
          {o.label}
        </option>
      ))}
    </select>
  );
}

/**
 * Chips de valores sugeridos + "outro…" que revela um campo numérico livre.
 *
 * Valor fora das sugestões (legado ou digitado) mantém o chip "outro" ativo
 * com o número à vista no campo. Com `allowAuto`, o chip "auto" devolve
 * `null` — o mesmo contrato do OptionalNum (null = o llama.cpp decide).
 */
export function NumChips({
  value,
  suggestions,
  min,
  max,
  step,
  allowAuto,
  disabled,
  placeholder,
  format,
  onCommit,
}: {
  value: number | null | undefined;
  suggestions: number[];
  min: number;
  max: number;
  step?: number;
  allowAuto?: boolean;
  disabled?: boolean;
  /** Placeholder do campo "outro…" (padrão: o rótulo de auto). */
  placeholder?: string;
  /** Como exibir cada sugestão (ex.: 16384 → "16k"). */
  format?: (n: number) => string;
  onCommit: (n: number | null) => void;
}) {
  const { t } = useTranslation();
  const fora = value != null && !suggestions.includes(value);
  // O "outro…" fica aberto depois de clicado, mesmo antes de digitar algo.
  const [outroAberto, setOutroAberto] = useState(fora);
  // Valor trocado por fora (preset aplicado, reset): o "outro…" acompanha —
  // abre se o valor novo está fora dos chips, fecha se caiu num deles.
  // Estado derivado ajustado durante o render, sem efeito, como o React
  // recomenda para reagir a mudança de prop.
  const [lastValue, setLastValue] = useState(value);
  if (value !== lastValue) {
    setLastValue(value);
    setOutroAberto(fora);
  }
  const outroAtivo = outroAberto || fora;

  return (
    <div className="flex flex-wrap items-center gap-1">
      {allowAuto && (
        <button
          type="button"
          disabled={disabled}
          onClick={() => {
            setOutroAberto(false);
            onCommit(null);
          }}
          className={chipClass(value == null && !outroAtivo)}
        >
          {t("server.fields.auto")}
        </button>
      )}
      {suggestions.map((n) => (
        <button
          key={n}
          type="button"
          disabled={disabled}
          onClick={() => {
            setOutroAberto(false);
            onCommit(n);
          }}
          className={`${chipClass(value === n && !outroAtivo)} tabular-nums`}
        >
          {format ? format(n) : n}
        </button>
      ))}
      <button
        type="button"
        disabled={disabled}
        onClick={() => setOutroAberto(true)}
        className={chipClass(outroAtivo)}
      >
        {t("server.fields.other")}
      </button>
      {outroAtivo && (
        <OptionalNum
          value={value}
          min={min}
          max={max}
          step={step}
          placeholder={placeholder ?? (allowAuto ? t("server.fields.auto") : "")}
          disabled={disabled}
          onCommit={onCommit}
        />
      )}
    </div>
  );
}

/** O tri-state das flags booleanas opcionais: auto (llama.cpp) / on / off. */
export type Tri = "auto" | "on" | "off";

export function triFrom(v: boolean | null | undefined): Tri {
  if (v == null) return "auto";
  return v ? "on" : "off";
}

export function triTo(v: Tri): boolean | null {
  if (v === "auto") return null;
  return v === "on";
}
