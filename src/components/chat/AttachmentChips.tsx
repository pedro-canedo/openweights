// Chips de anexos pendentes acima do composer: nome + tamanho + remover.
// Também concentra o domínio de anexos (extensões aceitas, limites e
// leitura via FileReader) usado pelo Chat e pelo Composer.

import { useTranslation } from "react-i18next";
import { formatBytes } from "../../lib/format";

export interface Attachment {
  id: string;
  name: string;
  size: number;
  kind: "text" | "image";
  /** Conteúdo do arquivo: texto puro ou dataURL (imagens). */
  data: string;
}

/** Extensões de texto/código aceitas como anexo. */
const TEXT_EXTS = new Set([
  "txt", "md", "markdown", "json", "js", "ts", "tsx", "jsx", "py", "rs",
  "toml", "yaml", "yml", "css", "html", "htm", "c", "cpp", "h", "hpp",
  "java", "go", "rb", "php", "sh", "sql", "csv", "log",
]);

const IMAGE_EXTS = new Set(["png", "jpg", "jpeg", "webp", "gif"]);

export const MAX_TEXT_BYTES = 300 * 1024; // 300 KB
export const MAX_IMAGE_BYTES = 8 * 2 ** 20; // 8 MB

export type AttachmentKind = "text" | "image" | "unsupported";

/** Classifica o arquivo pela extensão (imagens coladas vêm sem nome útil). */
export function classifyFile(file: File): AttachmentKind {
  if (file.type.startsWith("image/")) {
    const sub = file.type.slice("image/".length);
    if (IMAGE_EXTS.has(sub)) return "image";
  }
  const ext = file.name.split(".").pop()?.toLowerCase() ?? "";
  if (IMAGE_EXTS.has(ext)) return "image";
  if (TEXT_EXTS.has(ext)) return "text";
  return "unsupported";
}

function readFile(file: File, asDataUrl: boolean): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result ?? ""));
    reader.onerror = () => reject(reader.error);
    if (asDataUrl) reader.readAsDataURL(file);
    else reader.readAsText(file);
  });
}

let attachSeq = 0;

/**
 * Lê um arquivo e devolve um Attachment pronto.
 * Lança erros com as chaves i18n `tooBig` / `unsupported` como marcador —
 * o chamador traduz e exibe.
 */
export async function readAttachment(file: File): Promise<Attachment> {
  const kind = classifyFile(file);
  if (kind === "unsupported") throw new Error("unsupported");
  const max = kind === "image" ? MAX_IMAGE_BYTES : MAX_TEXT_BYTES;
  if (file.size > max) throw new Error("tooBig");
  const data = await readFile(file, kind === "image");
  attachSeq += 1;
  return {
    id: `att-${Date.now()}-${attachSeq}`,
    name: file.name || `imagem-${attachSeq}.png`,
    size: file.size,
    kind,
    data,
  };
}

export default function AttachmentChips({
  attachments,
  onRemove,
}: {
  attachments: Attachment[];
  onRemove: (id: string) => void;
}) {
  const { t } = useTranslation();
  if (attachments.length === 0) return null;

  return (
    <div className="mx-auto mb-2 flex max-w-2xl flex-wrap gap-2">
      {attachments.map((a) => (
        <div
          key={a.id}
          className="flex items-center gap-2 rounded-lg border border-edge bg-panel2 px-2.5 py-1.5 text-xs"
        >
          {a.kind === "image" ? (
            <img
              src={a.data}
              alt={a.name}
              className="h-6 w-6 rounded object-cover"
            />
          ) : (
            <svg
              className="h-4 w-4 shrink-0 text-dim"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.8"
              strokeLinecap="round"
              strokeLinejoin="round"
              viewBox="0 0 24 24"
            >
              <path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8zM14 2v6h6" />
            </svg>
          )}
          <div className="min-w-0">
            <div className="flex items-center gap-1.5">
              <span className="max-w-40 truncate text-ink" title={a.name}>
                {a.name}
              </span>
              <span className="shrink-0 tabular-nums text-dim">
                {formatBytes(a.size)}
              </span>
            </div>
            {a.kind === "image" && (
              <div className="text-[10px] text-warn">{t("chat.imageNote")}</div>
            )}
          </div>
          <button
            onClick={() => onRemove(a.id)}
            className="ml-1 flex h-5 w-5 shrink-0 items-center justify-center rounded text-dim hover:bg-bad/20 hover:text-bad"
            title={t("common.close")}
          >
            <svg
              className="h-3 w-3"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              viewBox="0 0 24 24"
            >
              <path d="M18 6L6 18M6 6l12 12" />
            </svg>
          </button>
        </div>
      ))}
    </div>
  );
}
