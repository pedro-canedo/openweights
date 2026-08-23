// O controle de UMA flag do catálogo, escolhido pelo tipo declarado.
//
// A flag curada tem rótulo/dica em `flags.catalog.<chave>.*`; a dinâmica cai
// no texto original do `--help` (em inglês) — aparecer cru é melhor que não
// aparecer. Badges de requisito nunca bloqueiam: metadado ausente é "não
// sei", não "não pode".

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { FlagRequirement, FlagSpec } from "../../lib/flags";
import type { ModelCaps } from "../../lib/flags";
import { Chips, chipClass } from "./controls";

/** O requisito vale um aviso? `null` = não sabemos afirmar nada. */
function requirementState(
  req: FlagRequirement,
  caps: ModelCaps | null,
  hasGpu: boolean,
): boolean | null {
  switch (req) {
    case "gpu":
      return hasGpu;
    case "moeModel":
      return caps?.moe ?? null;
    case "mtpModel":
      return caps?.mtpHead ?? null;
    case "mmprojPresent":
      return caps ? caps.hasMmproj : null;
    default:
      return null;
  }
}

export function RequirementBadges({
  spec,
  caps,
  hasGpu,
}: {
  spec: FlagSpec;
  caps: ModelCaps | null;
  hasGpu: boolean;
}) {
  const { t } = useTranslation();
  if (!spec.requires.length) return null;
  return (
    <span className="flex flex-wrap gap-1">
      {spec.requires.map((r) => {
        const ok = requirementState(r, caps, hasGpu);
        const tone =
          ok === true
            ? "border-ok/40 bg-ok/10 text-ok"
            : ok === false
              ? "border-warn/40 bg-warn/10 text-warn"
              : "border-edge text-dim";
        return (
          <span
            key={r}
            className={`rounded-full border px-2 py-0.5 text-[10px] ${tone}`}
          >
            {t(`flags.requirements.${r}`)}
          </span>
        );
      })}
    </span>
  );
}

/**
 * Valor de uma flag como texto de INI (`"1"`, `"on"`, `"q8_0"`…). `null` =
 * não configurada. O chamador decide onde guardar (extras, setting global).
 */
export default function FlagControl({
  spec,
  value,
  disabled,
  onChange,
}: {
  spec: FlagSpec;
  value: string | null;
  disabled?: boolean;
  onChange: (v: string | null) => void;
}) {
  const { t } = useTranslation();
  const k = spec.kind;

  if (k.type === "bool") {
    const on = value != null && value !== "false" && value !== "0";
    return (
      <div className="flex gap-1">
        <button
          type="button"
          disabled={disabled}
          onClick={() => onChange(null)}
          className={chipClass(value == null)}
        >
          {t("flags.control.auto")}
        </button>
        <button
          type="button"
          disabled={disabled}
          onClick={() => onChange("true")}
          className={chipClass(on)}
        >
          {t("flags.control.on")}
        </button>
      </div>
    );
  }

  if (k.type === "tri") {
    const cur = value === "on" ? "on" : value === "off" ? "off" : "auto";
    return (
      <Chips
        value={cur}
        disabled={disabled}
        onChange={(v) => onChange(v === "auto" ? null : v)}
        options={[
          { id: "auto", label: t("flags.control.auto") },
          { id: "on", label: t("flags.control.on") },
          { id: "off", label: t("flags.control.off") },
        ]}
      />
    );
  }

  if (k.type === "enum") {
    // Poucas opções cabem em chips; muitas (spec-type tem 11) viram select.
    if (k.options.length <= 5) {
      const cur = value ?? "__auto";
      return (
        <div className="flex flex-wrap gap-1">
          <button
            type="button"
            disabled={disabled}
            onClick={() => onChange(null)}
            className={chipClass(value == null)}
          >
            {t("flags.control.auto")}
          </button>
          {k.options.map((o) => (
            <button
              key={o}
              type="button"
              disabled={disabled}
              onClick={() => onChange(o)}
              className={chipClass(cur === o)}
            >
              {o}
            </button>
          ))}
        </div>
      );
    }
    return (
      <select
        value={value ?? ""}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value || null)}
        className="rounded-lg border border-edge bg-panel2 px-2 py-1.5 text-xs outline-none focus:border-accent"
      >
        <option value="">{t("flags.control.auto")}</option>
        {k.options.map((o) => (
          <option key={o} value={o}>
            {o}
          </option>
        ))}
      </select>
    );
  }

  if (k.type === "int" || k.type === "float") {
    return (
      <NumText
        value={value}
        disabled={disabled}
        min={k.min}
        max={k.max}
        step={k.step}
        float={k.type === "float"}
        placeholder={spec.default ?? t("flags.control.auto")}
        onCommit={onChange}
      />
    );
  }

  // text / path / list: entrada livre — o llama.cpp valida no boot e o erro
  // aparece nos logs do servidor, que ficam nesta mesma tela.
  return (
    <TextCommit
      value={value}
      disabled={disabled}
      placeholder={
        k.type === "path" ? t("flags.control.pathPlaceholder") : t("flags.control.auto")
      }
      onCommit={onChange}
    />
  );
}

function NumText({
  value,
  min,
  max,
  step,
  float,
  placeholder,
  disabled,
  onCommit,
}: {
  value: string | null;
  min: number;
  max: number;
  step: number;
  float: boolean;
  placeholder: string;
  disabled?: boolean;
  onCommit: (v: string | null) => void;
}) {
  const [text, setText] = useState(value ?? "");
  useEffect(() => setText(value ?? ""), [value]);
  const commit = () => {
    const trimmed = text.trim();
    if (!trimmed) {
      onCommit(null);
      return;
    }
    const n = Number(trimmed);
    if (!Number.isFinite(n)) {
      setText(value ?? "");
      return;
    }
    const clamped = Math.min(max, Math.max(min, n));
    onCommit(String(float ? Math.round(clamped * 1000) / 1000 : Math.round(clamped)));
  };
  return (
    <input
      type="number"
      min={min}
      max={max}
      step={step}
      disabled={disabled}
      placeholder={placeholder}
      value={text}
      onChange={(e) => setText(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") (e.target as HTMLInputElement).blur();
      }}
      className="w-28 rounded-lg border border-edge bg-panel2 px-2 py-1.5 text-xs tabular-nums outline-none placeholder:text-dim focus:border-accent disabled:opacity-40"
    />
  );
}

function TextCommit({
  value,
  placeholder,
  disabled,
  onCommit,
}: {
  value: string | null;
  placeholder: string;
  disabled?: boolean;
  onCommit: (v: string | null) => void;
}) {
  const [text, setText] = useState(value ?? "");
  useEffect(() => setText(value ?? ""), [value]);
  return (
    <input
      type="text"
      disabled={disabled}
      placeholder={placeholder}
      value={text}
      onChange={(e) => setText(e.target.value)}
      onBlur={() => onCommit(text.trim() || null)}
      onKeyDown={(e) => {
        if (e.key === "Enter") (e.target as HTMLInputElement).blur();
      }}
      className="w-full min-w-40 rounded-lg border border-edge bg-panel2 px-2 py-1.5 font-mono text-xs outline-none placeholder:text-dim focus:border-accent disabled:opacity-40"
    />
  );
}
