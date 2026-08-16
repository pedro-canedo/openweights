// Caixa de envio de mensagem: Enter envia, Shift+Enter quebra linha.
// Durante a geração o botão vira "Parar" (aborta o streaming).
// Anexos (botão + / colar imagem) e ditado por voz (microfone).
//
// A barra de baixo tem dois grupos, e a divisão não é estética: à ESQUERDA o
// que muda o pedido (anexo, pasta, modo de trabalho); à DIREITA o que muda a
// resposta (modelo, esforço) e as ações de enviar/ditar. Tudo junto, com o
// mesmo peso, virava uma fileira de sete controles onde nada se achava.

import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ClipboardEvent,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import type { WorkspaceFile } from "../../lib/types";
import AttachMenu from "./AttachMenu";
import AttachmentChips, { type Attachment } from "./AttachmentChips";
import VoiceButton from "./VoiceButton";

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
  workspaceFiles = [],
  startActions,
  leftActions,
  rightActions,
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
  workspaceFiles?: WorkspaceFile[];
  startActions?: ReactNode;
  leftActions?: ReactNode;
  rightActions?: ReactNode;
}) {
  const { t } = useTranslation();
  const fileRef = useRef<HTMLInputElement>(null);
  const taRef = useRef<HTMLTextAreaElement>(null);
  const [mentionIdx, setMentionIdx] = useState(0);

  const mention = useMemo(() => {
    const m = /(?:^|[\s])@([^\s@]*)$/.exec(draft);
    if (!m || workspaceFiles.length === 0) return null;
    const q = m[1].toLowerCase();
    const hits = workspaceFiles
      .filter(
        (f) =>
          f.name.toLowerCase().includes(q) || f.path.toLowerCase().includes(q),
      )
      .slice(0, 8);
    return { query: m[1], start: m.index + m[0].indexOf("@"), hits };
  }, [draft, workspaceFiles]);

  useEffect(() => {
    setMentionIdx(0);
  }, [mention?.query]);

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

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "u") {
        e.preventDefault();
        if (!generating) fileRef.current?.click();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [generating]);

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

      <div className="relative mx-auto max-w-3xl">
        {mention && mention.hits.length > 0 && (
          <div className="absolute right-0 bottom-full left-0 z-20 mb-2 overflow-hidden rounded-xl border border-edge bg-panel py-1 shadow-xl">
            {mention.hits.map((f, i) => (
              <button
                key={f.path}
                type="button"
                onMouseDown={(e) => {
                  e.preventDefault();
                  const after = draft.slice(
                    mention.start + 1 + mention.query.length,
                  );
                  onDraftChange(
                    `${draft.slice(0, mention.start)}@${f.path} ${after}`,
                  );
                }}
                className={`flex w-full px-3 py-1.5 text-left text-xs ${
                  i === mentionIdx ? "bg-panel2 text-ink" : "text-dim"
                }`}
              >
                <span className="truncate">{f.path}</span>
              </button>
            ))}
          </div>
        )}
      <div className="flex flex-col rounded-[28px] border border-edge bg-panel2 px-3.5 py-3 shadow-[0_8px_32px_rgba(0,0,0,0.28)]">
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
        <textarea
          ref={taRef}
          value={draft}
          onChange={(e) => {
            onDraftChange(e.target.value);
            resize(e.target);
          }}
          onKeyDown={(e) => {
            if (mention && mention.hits.length > 0) {
              if (e.key === "ArrowDown") {
                e.preventDefault();
                setMentionIdx((i) => (i + 1) % mention.hits.length);
                return;
              }
              if (e.key === "ArrowUp") {
                e.preventDefault();
                setMentionIdx(
                  (i) => (i - 1 + mention.hits.length) % mention.hits.length,
                );
                return;
              }
              if (e.key === "Enter" || e.key === "Tab") {
                e.preventDefault();
                const hit = mention.hits[mentionIdx] ?? mention.hits[0];
                const after = draft.slice(
                  mention.start + 1 + mention.query.length,
                );
                onDraftChange(
                  `${draft.slice(0, mention.start)}@${hit.path} ${after}`,
                );
                return;
              }
              if (e.key === "Escape") {
                e.preventDefault();
                onDraftChange(draft + " ");
                return;
              }
            }
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              if (!generating && canSend) onSend();
            }
          }}
          onPaste={handlePaste}
          placeholder={t("chat.placeholder")}
          rows={1}
          className="max-h-40 min-h-10 w-full resize-none bg-transparent px-1.5 py-1.5 text-sm outline-none select-text placeholder:text-dim"
        />

        <div className="mt-1.5 flex items-center gap-2">
          <div className="flex min-w-0 items-center gap-1">
            <AttachMenu
              disabled={generating}
              onAttach={() => fileRef.current?.click()}
            />
            {startActions}
            {leftActions}
          </div>
          <div className="ml-auto flex shrink-0 items-center gap-1">
            {rightActions}
            <VoiceButton
              draft={draft}
              onDraftChange={onDraftChange}
              disabled={generating || disabled}
            />
            {generating ? (
              <button
                onClick={onStop}
                title={t("chat.stop")}
                className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-panel text-ink transition-colors hover:bg-bad/20 hover:text-bad"
              >
                <span className="h-2.5 w-2.5 rounded-[2px] bg-current" />
              </button>
            ) : (
              <button
                onClick={onSend}
                disabled={!canSend}
                title={t("chat.send")}
                className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-ink text-bg transition-opacity disabled:opacity-25"
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
      </div>
      </div>
    </div>
  );
}
