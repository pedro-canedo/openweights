// Caixa de envio de mensagem: Enter envia, Shift+Enter quebra linha.
// Durante a geração o botão vira "Parar" (aborta o streaming).
// Suporta anexos (botão 📎, colar imagem) com chips acima do campo.

import { useEffect, useRef, type ClipboardEvent } from "react";
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
  const taRef = useRef<HTMLTextAreaElement>(null);

  const canSend =
    !disabled && (draft.trim().length > 0 || attachments.length > 0);

  const resize = (el: HTMLTextAreaElement) => {
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  };

  useEffect(() => {
    const el = taRef.current;
    if (!el) return;
    resize(el);
    if (draft && document.activeElement !== el) {
      el.focus();
      el.setSelectionRange(el.value.length, el.value.length);
    }
  }, [draft]);

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
    <div className="px-6 pt-1 pb-5">
      {editing && (
        <div className="mx-auto mb-2 flex max-w-2xl items-center gap-2 text-xs text-warn">
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
        <div className="mx-auto mb-2 max-w-2xl text-xs text-bad">
          {attachError}
        </div>
      )}

      <div className="mx-auto flex max-w-2xl items-end gap-1 rounded-[28px] border border-edge bg-panel2 px-2.5 py-2 shadow-[0_8px_32px_rgba(0,0,0,0.28)]">
        <input
          ref={fileRef}
          type="file"
          multiple
          className="hidden"
          onChange={(e) => {
            const files = Array.from(e.target.files ?? []);
            if (files.length > 0) onAttachFiles(files);
            e.target.value = "";
          }}
        />
        <button
          onClick={() => fileRef.current?.click()}
          title={t("chat.attach")}
          disabled={generating}
          className="mb-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-dim transition-colors hover:bg-panel hover:text-ink disabled:opacity-40"
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
            <rect x="3" y="5" width="18" height="14" rx="2" />
            <circle cx="8.5" cy="10" r="1.5" />
            <path d="M21 16l-5-5-8 8" />
          </svg>
        </button>

        <textarea
          ref={taRef}
          value={draft}
          onChange={(e) => {
            onDraftChange(e.target.value);
            resize(e.target);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              if (!generating && canSend) onSend();
            }
          }}
          onPaste={handlePaste}
          placeholder={t("chat.placeholder")}
          rows={1}
          className="max-h-40 min-h-10 flex-1 resize-none bg-transparent px-1 py-2 text-sm outline-none select-text placeholder:text-dim"
        />

        {generating ? (
          <button
            onClick={onStop}
            title={t("chat.stop")}
            className="mb-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-panel text-ink transition-colors hover:bg-bad/20 hover:text-bad"
          >
            <span className="h-2.5 w-2.5 rounded-[2px] bg-current" />
          </button>
        ) : (
          <button
            onClick={onSend}
            disabled={!canSend}
            title={t("chat.send")}
            className="mb-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-ink text-bg transition-opacity disabled:opacity-25"
          >
            <svg
              className="h-4 w-4"
              fill="none"
              stroke="currentColor"
              strokeWidth="2.2"
              strokeLinecap="round"
              strokeLinejoin="round"
              viewBox="0 0 24 24"
            >
              <path d="M12 19V5M6 11l6-6 6 6" />
            </svg>
          </button>
        )}
      </div>
    </div>
  );
}
