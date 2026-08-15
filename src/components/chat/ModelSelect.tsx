// Seletor de modelo do Chat. Em Router mode, trocar de modelo entre
// mensagens é permitido — o llama-server carrega sob demanda.
// Nome curto na barra; o arquivo completo aparece no tooltip.

import { useTranslation } from "react-i18next";

function shortModel(name: string): string {
  return name
    .replace(/\.gguf$/i, "")
    .replace(/-UD-.*$/i, "")
    .replace(/-Q\d.*$/i, "");
}

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
  const options =
    value && !models.includes(value) ? [value, ...models] : models;
  const empty = options.length === 0;
  const title = empty
    ? t("chat.modelSelect")
    : `${t("chat.modelSelect")}: ${value}`;

  return (
    <label
      title={title}
      className="relative flex max-w-36 shrink-0 cursor-pointer items-center rounded-full px-2 py-1 text-xs text-ink transition-colors hover:bg-panel"
    >
      <span className="truncate">
        {empty ? t("chat.modelSelect") : shortModel(value)}
      </span>
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        disabled={disabled || empty}
        aria-label={title}
        className="absolute inset-0 cursor-pointer opacity-0 disabled:cursor-not-allowed"
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
