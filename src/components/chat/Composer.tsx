// Caixa de envio de mensagem: Enter envia, Shift+Enter quebra linha.
// Durante a geração o botão vira "Parar" (aborta o streaming).

import { useTranslation } from "react-i18next";

export default function Composer({
  draft,
  onDraftChange,
  onSend,
  onStop,
  generating,
  disabled,
}: {
  draft: string;
  onDraftChange: (text: string) => void;
  onSend: () => void;
  onStop: () => void;
  generating: boolean;
  disabled: boolean;
}) {
  const { t } = useTranslation();

  return (
    <div className="border-t border-edge p-4">
      <div className="mx-auto flex max-w-3xl items-end gap-2">
        <textarea
          value={draft}
          onChange={(e) => onDraftChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              if (!generating && !disabled) onSend();
            }
          }}
          placeholder={t("chat.placeholder")}
          rows={1}
          className="max-h-40 min-h-11 flex-1 resize-none rounded-xl border border-edge bg-panel px-4 py-3 text-sm outline-none select-text placeholder:text-dim focus:border-accent"
        />
        {generating ? (
          <button
            onClick={onStop}
            className="h-11 rounded-xl border border-edge bg-panel2 px-5 text-sm font-medium transition-colors hover:border-bad hover:text-bad"
          >
            {t("chat.stop")}
          </button>
        ) : (
          <button
            onClick={onSend}
            disabled={!draft.trim() || disabled}
            className="h-11 rounded-xl bg-accent px-5 text-sm font-medium text-white disabled:opacity-40"
          >
            {t("chat.send")}
          </button>
        )}
      </div>
    </div>
  );
}
