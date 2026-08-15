// Caixa de envio de mensagem: Enter envia, Shift+Enter quebra linha.
// Durante a geração o botão vira "Parar" (aborta o streaming).
// Suporta anexos (botão 📎, colar imagem) com chips acima do campo.

import { useRef, type ClipboardEvent } from "react";
import { useTranslation } from "react-i18next";
import AttachmentChips, { type Attachment } from "./AttachmentChips";

export default function Composer({
  draft,
  onDraftChange,
  onSend,
  onStop,
  generating,
  disabled,
  attachments,
  onAttachFiles,
  onRemoveAttachment,
  attachError,
  editing,
  onCancelEdit,
}: {
  draft: string;
  onDraftChange: (text: string) => void;
  onSend: () => void;
  onStop: () => void;
  generating: boolean;
  disabled: boolean;
  attachments: Attachment[];
  onAttachFiles: (files: File[]) => void;
  onRemoveAttachment: (id: string) => void;
  attachError: string | null;
  /** true quando o composer está editando a última mensagem do usuário. */
  editing: boolean;
  onCancelEdit: () => void;
}) {
  const { t } = useTranslation();
  const fileRef = useRef<HTMLInputElement>(null);

  const canSend =
    !disabled && (draft.trim().length > 0 || attachments.length > 0);

  const handlePaste = (e: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = Array.from(e.clipboardData?.files ?? []).filter((f) =>
      f.type.startsWith("image/"),
    );
    if (files.length > 0) {
      e.preventDefault();
      onAttachFiles(files);
    }
  };

  return (
    <div className="border-t border-edge p-4">
      {editing && (
        <div className="mx-auto mb-2 flex max-w-3xl items-center gap-2 text-xs text-warn">
          <span>{t("chat.editResend")}</span>
          <button
            onClick={onCancelEdit}
            className="rounded border border-edge px-1.5 py-0.5 text-[11px] text-dim transition-colors hover:border-bad hover:text-bad"
          >
            {t("common.cancel")}
          </button>
        </div>
      )}

      <AttachmentChips attachments={attachments} onRemove={onRemoveAttachment} />

      {attachError && (
        <div className="mx-auto mb-2 max-w-3xl text-xs text-bad">
          {attachError}
        </div>
      )}

      <div className="mx-auto flex max-w-3xl items-end gap-2">
        <input
          ref={fileRef}
          type="file"
          multiple
          className="hidden"
          onChange={(e) => {
            const files = Array.from(e.target.files ?? []);
            if (files.length > 0) onAttachFiles(files);
            e.target.value = ""; // permite re-anexar o mesmo arquivo
          }}
        />
        <button
          onClick={() => fileRef.current?.click()}
          title={t("chat.attach")}
          disabled={generating}
          className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl border border-edge bg-panel text-dim transition-colors hover:border-accent hover:text-ink disabled:opacity-40"
        >
          <svg
            className="h-4.5 w-4.5"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.8"
            strokeLinecap="round"
            strokeLinejoin="round"
            viewBox="0 0 24 24"
          >
            <path d="M21.44 11.05l-9.19 9.19a6 6 0 01-8.49-8.49l9.19-9.19a4 4 0 015.66 5.66l-9.2 9.19a2 2 0 01-2.83-2.83l8.49-8.48" />
          </svg>
        </button>

        <textarea
          value={draft}
          onChange={(e) => onDraftChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              if (!generating && canSend) onSend();
            }
          }}
          onPaste={handlePaste}
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
            disabled={!canSend}
            className="h-11 rounded-xl bg-accent px-5 text-sm font-medium text-white disabled:opacity-40"
          >
            {t("chat.send")}
          </button>
        )}
      </div>
    </div>
  );
}
