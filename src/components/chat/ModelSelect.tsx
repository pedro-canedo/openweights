// Seletor de modelo do Chat. Em Router mode, trocar de modelo entre
// mensagens é permitido — o llama-server carrega sob demanda.

import { useTranslation } from "react-i18next";

export default function ModelSelect({
  models,
  value,
  onChange,
  disabled = false,
}: {
  models: string[];
  value: string;
  onChange: (model: string) => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  // Garante que o valor atual sempre exista como opção (ex.: pré-seleção
  // vinda de "Conversar" em Meus Modelos antes de o servidor listar).
  const options =
    value && !models.includes(value) ? [value, ...models] : models;

  return (
    <label className="flex min-w-0 items-center gap-2 text-sm">
      <span className="shrink-0 text-dim">{t("chat.modelSelect")}</span>
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        disabled={disabled || options.length === 0}
        className="min-w-0 max-w-72 truncate rounded-lg border border-edge bg-panel px-3 py-1.5 text-sm outline-none focus:border-accent disabled:opacity-50"
      >
        {options.map((m) => (
          <option key={m} value={m}>
            {m}
          </option>
        ))}
      </select>
    </label>
  );
}
