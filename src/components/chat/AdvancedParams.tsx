// Amostragem avançada do chat (modo Avançado): o que o painel de parâmetros
// não mostra por padrão. Campo vazio = não enviar (vale o padrão do servidor).
// Também: usar os padrões do servidor de uma vez, pedir JSON e ver como o
// esforço fixa o teto de tokens.

import { useState } from "react";
import { useTranslation } from "react-i18next";
import { EFFORT_MAX_TOKENS, type ChatParams } from "../../lib/types";

const campo =
  "w-24 rounded-lg border border-edge bg-panel2 px-2 py-1.5 text-xs tabular-nums outline-none focus-visible:border-accent disabled:opacity-40";

/** Um número opcional: vazio vira `null`, e o que não é número é ignorado. */
function NumeroOpcional({
  label,
  hint,
  value,
  step,
  min,
  max,
  disabled,
  inteiro,
  onChange,
}: {
  label: string;
  hint?: string;
  value: number | null | undefined;
  step: number;
  min?: number;
  max?: number;
  disabled?: boolean;
  inteiro?: boolean;
  onChange: (v: number | null) => void;
}) {
  return (
    <label className="flex flex-col gap-1">
      <span className="flex items-center justify-between text-xs">
        <span className="text-dim">{label}</span>
        <input
          type="number"
          inputMode="decimal"
          step={step}
          min={min}
          max={max}
          disabled={disabled}
          value={value ?? ""}
          placeholder="—"
          onChange={(e) => {
            const bruto = e.target.value.trim();
            if (bruto === "") return onChange(null);
            const n = Number(bruto);
            if (Number.isFinite(n)) onChange(inteiro ? Math.round(n) : n);
          }}
          className={campo}
        />
      </span>
      {hint && <span className="text-[10px] leading-snug text-dim">{hint}</span>}
    </label>
  );
}

export default function AdvancedParams({
  params,
  patch,
}: {
  params: ChatParams;
  patch: (p: Partial<ChatParams>) => void;
}) {
  const { t } = useTranslation();
  // O esquema é texto livre enquanto se digita: o erro aparece, o valor fica.
  const [esquemaInvalido, setEsquemaInvalido] = useState(false);
  const tetoDoEsforco = EFFORT_MAX_TOKENS[params.effort];

  return (
    <details className="rounded-lg border border-edge bg-panel2/40 px-2.5 py-2">
      <summary className="cursor-pointer text-xs font-medium text-ink select-none">
        {t("chat.advanced.title")}
      </summary>
      <div className="mt-3 flex flex-col gap-3">
        <label className="flex items-start gap-2 text-xs text-dim">
          <input
            type="checkbox"
            className="mt-0.5 accent-accent"
            checked={!!params.serverDefaults}
            onChange={(e) => patch({ serverDefaults: e.target.checked })}
          />
          <span>
            {t("chat.advanced.serverDefaults")}
            <span className="mt-0.5 block text-[10px] leading-snug">
              {t("chat.advanced.serverDefaultsHint")}
            </span>
          </span>
        </label>

        <NumeroOpcional
          label={t("chat.advanced.minP")}
          value={params.minP}
          step={0.01}
          min={0}
          max={1}
          onChange={(v) => patch({ minP: v })}
        />
        <NumeroOpcional
          label={t("chat.advanced.repeatPenalty")}
          hint={t("chat.advanced.repeatPenaltyHint")}
          value={params.repeatPenalty}
          step={0.05}
          min={0}
          onChange={(v) => patch({ repeatPenalty: v })}
        />
        <NumeroOpcional
          label={t("chat.advanced.presencePenalty")}
          value={params.presencePenalty}
          step={0.1}
          min={-2}
          max={2}
          onChange={(v) => patch({ presencePenalty: v })}
        />
        <NumeroOpcional
          label={t("chat.advanced.frequencyPenalty")}
          value={params.frequencyPenalty}
          step={0.1}
          min={-2}
          max={2}
          onChange={(v) => patch({ frequencyPenalty: v })}
        />
        <NumeroOpcional
          label={t("chat.advanced.seed")}
          hint={t("chat.advanced.seedHint")}
          value={params.seed}
          step={1}
          inteiro
          onChange={(v) => patch({ seed: v })}
        />

        <label className="flex flex-col gap-1">
          <span className="text-xs text-dim">{t("chat.advanced.stop")}</span>
          <textarea
            rows={2}
            spellCheck={false}
            value={(params.stop ?? []).join("\n")}
            placeholder={t("chat.advanced.stopPlaceholder")}
            onChange={(e) => {
              const linhas = e.target.value.split("\n").filter((l) => l !== "");
              patch({ stop: linhas });
            }}
            className="resize-y rounded-lg border border-edge bg-panel2 px-2.5 py-1.5 font-mono text-[11px] outline-none select-text focus-visible:border-accent"
          />
        </label>

        <div className="flex flex-col gap-1.5">
          <label className="flex items-center gap-2 text-xs text-dim">
            <input
              type="checkbox"
              className="accent-accent"
              checked={!!params.jsonMode}
              onChange={(e) => patch({ jsonMode: e.target.checked })}
            />
            {t("chat.advanced.jsonMode")}
          </label>
          <label className="flex flex-col gap-1">
            <span className="text-xs text-dim">{t("chat.advanced.jsonSchema")}</span>
            <textarea
              rows={3}
              spellCheck={false}
              value={params.jsonSchema ?? ""}
              placeholder='{"type":"object","properties":{…}}'
              aria-invalid={esquemaInvalido}
              onChange={(e) => {
                const texto = e.target.value;
                let valido = true;
                if (texto.trim() !== "") {
                  try {
                    JSON.parse(texto);
                  } catch {
                    valido = false;
                  }
                }
                setEsquemaInvalido(!valido);
                patch({ jsonSchema: texto === "" ? null : texto });
              }}
              className="resize-y rounded-lg border border-edge bg-panel2 px-2.5 py-1.5 font-mono text-[11px] outline-none select-text focus-visible:border-accent aria-[invalid=true]:border-bad"
            />
            {esquemaInvalido && (
              <span role="alert" className="text-[10px] text-bad">
                {t("chat.advanced.jsonSchemaInvalid")}
              </span>
            )}
          </label>
        </div>

        <p className="text-[10px] leading-snug text-dim">
          {t("chat.advanced.effortTokens", {
            effort: t(`chat.effort.${params.effort}`, params.effort),
            tokens: tetoDoEsforco == null ? t("chat.noLimit") : tetoDoEsforco,
          })}
        </p>
        <p className="text-[10px] leading-snug text-dim">{t("chat.advanced.localOnly")}</p>
      </div>
    </details>
  );
}
