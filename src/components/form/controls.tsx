// Controles de formulário compartilhados — promovidos do antigo painel de
// carga do chat para servirem à tela de configuração do servidor.
//
// A convenção que todos seguem: valor `null`/ausente = "auto" (o llama.cpp
// decide). É o que permite ligar de volta o `--fit` quando ninguém opinou.

import { useEffect, useState } from "react";

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
  label: string;
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
      <span className="text-xs text-dim">{label}</span>
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
