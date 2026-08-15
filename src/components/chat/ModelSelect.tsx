// Seletor de modelo do Chat. Em Router mode, trocar de modelo entre
// mensagens é permitido — o llama-server carrega sob demanda.
// Visual: pílula compacta (ícone + nome + seta) sobre um <select> nativo.

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
  const options =
    value && !models.includes(value) ? [value, ...models] : models;
  const empty = options.length === 0;

  return (
    <label className="relative inline-flex max-w-64 cursor-pointer items-center gap-2 rounded-full border border-edge bg-panel2 px-3 py-1.5 text-xs text-ink">
      <svg
        className="h-3.5 w-3.5 shrink-0 text-dim"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
        viewBox="0 0 24 24"
      >
        <path d="M12 3l1.8 5.4L19 10.2l-5.2 1.8L12 17.4l-1.8-5.4L5 10.2l5.2-1.8L12 3z" />
      </svg>
      <span className="min-w-0 truncate">
        {empty ? t("chat.modelSelect") : value}
      </span>
      <svg
        className="h-3 w-3 shrink-0 text-dim"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
        viewBox="0 0 24 24"
      >
        <path d="M6 9l6 6 6-6" />
      </svg>
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        disabled={disabled || empty}
        aria-label={t("chat.modelSelect")}
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
