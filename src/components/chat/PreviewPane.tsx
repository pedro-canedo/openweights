// Prévia de arquivos visuais (HTML, imagens, SVG) dentro do app.
//
// O agente gera páginas (jogos, landing pages) na pasta do projeto e, sem
// isto, a pessoa precisava sair do app para VER o resultado. O arquivo é
// carregado pelo protocolo asset do Tauri (`convertFileSrc`): zero cópia de
// string pelo IPC e os recursos relativos (style.css, script.js ao lado do
// index.html) resolvem sozinhos — como abrir o arquivo no navegador.

import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { convertFileSrc } from "@tauri-apps/api/core";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import FileIcon from "./FileIcon";

const HTML_EXTS = ["html", "htm"];
const IMAGE_EXTS = ["png", "jpg", "jpeg", "gif", "webp", "svg", "ico", "bmp"];
// Formatos que também são texto — para eles o toggle "código | prévia" faz sentido.
const TEXT_PREVIEW_EXTS = [...HTML_EXTS, "svg"];

export type PreviewKind = "html" | "image";

function extOf(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot >= 0 ? name.slice(dot + 1).toLowerCase() : "";
}

/** Que tipo de prévia este arquivo tem — ou nenhuma (`null`). */
export function previewKind(name: string): PreviewKind | null {
  const ext = extOf(name);
  if (HTML_EXTS.includes(ext)) return "html";
  if (IMAGE_EXTS.includes(ext)) return "image";
  return null;
}

/** O arquivo visualizável também é texto (HTML/SVG)? Aí o toggle código|prévia existe. */
export function canShowCode(name: string): boolean {
  return TEXT_PREVIEW_EXTS.includes(extOf(name));
}

function ReloadIcon({ className = "h-3.5 w-3.5" }: { className?: string }) {
  return (
    <svg
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      viewBox="0 0 24 24"
    >
      <path d="M20 12a8 8 0 11-2.34-5.66" />
      <path d="M20 4v4.3h-4.3" />
    </svg>
  );
}

function BrowserIcon({ className = "h-3.5 w-3.5" }: { className?: string }) {
  return (
    <svg
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      viewBox="0 0 24 24"
    >
      <circle cx="12" cy="12" r="9" />
      <path d="M3 12h18" />
      <path d="M12 3a14.5 14.5 0 010 18a14.5 14.5 0 010-18z" />
    </svg>
  );
}

export default function PreviewPane({
  path,
  name,
  refreshKey = 0,
  onClose,
  onShowCode,
}: {
  /** Caminho ABSOLUTO do arquivo (o protocolo asset não conhece a pasta da sessão). */
  path: string;
  name: string;
  /** Muda quando a lista de arquivos atualiza (agente gravou algo) — recarrega a prévia. */
  refreshKey?: number;
  onClose: () => void;
  /** Presente quando dá para alternar para o código (HTML/SVG) — vira o toggle no cabeçalho. */
  onShowCode?: () => void;
}) {
  const { t } = useTranslation();
  // Contador local do botão de recarregar; combinado com o refreshKey externo
  // ele troca a key do iframe/img (remonta) e o `?v=` (fura o cache HTTP).
  const [tick, setTick] = useState(0);
  const stamp = `${refreshKey}-${tick}`;
  const kind = previewKind(name) ?? "html";

  const src = useMemo(
    () => `${convertFileSrc(path)}?v=${stamp}`,
    [path, stamp],
  );

  const openExternal = async () => {
    try {
      // Abre com o aplicativo padrão do sistema (para .html, o navegador).
      await openPath(path);
    } catch {
      // `open_path` exige a permissão opener:allow-open-path, que a capability
      // atual não concede. Revelar no gerenciador de arquivos está no conjunto
      // padrão do plugin — é o plano B que sempre funciona.
      await revealItemInDir(path).catch(() => {});
    }
  };

  return (
    <div className="flex h-full w-full flex-col overflow-hidden rounded-xl border border-edge bg-panel">
      <div className="flex items-center gap-2 border-b border-edge px-3 py-2">
        <FileIcon name={name} className="h-4 w-4" />
        <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-ink">
          {name}
        </span>
        {onShowCode && (
          <div className="flex shrink-0 overflow-hidden rounded-md border border-edge text-[11px]">
            <button
              type="button"
              onClick={onShowCode}
              className="px-2 py-0.5 text-dim hover:text-ink"
            >
              {t("workspace.preview.code")}
            </button>
            <span className="bg-panel2 px-2 py-0.5 text-ink">
              {t("workspace.preview.preview")}
            </span>
          </div>
        )}
        <button
          type="button"
          onClick={() => setTick((n) => n + 1)}
          title={t("workspace.preview.reload")}
          className="flex h-6 w-6 shrink-0 items-center justify-center rounded text-dim hover:bg-panel2 hover:text-ink"
        >
          <ReloadIcon />
        </button>
        <button
          type="button"
          onClick={() => void openExternal()}
          title={t("workspace.preview.openBrowser")}
          className="flex h-6 w-6 shrink-0 items-center justify-center rounded text-dim hover:bg-panel2 hover:text-ink"
        >
          <BrowserIcon />
        </button>
        <button
          type="button"
          onClick={onClose}
          title={t("common.close")}
          className="flex h-6 w-6 shrink-0 items-center justify-center rounded text-dim hover:bg-panel2 hover:text-ink"
        >
          ×
        </button>
      </div>

      {kind === "html" ? (
        // SEM `allow-same-origin`: o documento roda com origem opaca — o jogo
        // funciona, o teclado funciona quando o iframe tem foco, e o conteúdo
        // gerado não conversa com o app. A prévia executa código gerado pelo
        // modelo: é o mesmo nível de confiança de abrir o index.html no
        // navegador, que é exatamente o fluxo que estamos substituindo.
        // A `key` remonta o iframe no recarregar; fundo branco para páginas
        // sem estilo parecerem com o navegador, não com o painel escuro.
        <iframe
          key={stamp}
          src={src}
          title={name}
          sandbox="allow-scripts allow-pointer-lock"
          className="min-h-0 w-full flex-1 border-0 bg-white"
        />
      ) : (
        <div
          className="flex min-h-0 flex-1 items-center justify-center overflow-auto p-4"
          // Quadriculado sutil em CSS puro: dois gradientes deslocados fazem o
          // padrão de transparência clássico dos editores de imagem.
          style={{
            backgroundImage:
              "linear-gradient(45deg, rgba(128,128,128,0.14) 25%, transparent 25%, transparent 75%, rgba(128,128,128,0.14) 75%), linear-gradient(45deg, rgba(128,128,128,0.14) 25%, transparent 25%, transparent 75%, rgba(128,128,128,0.14) 75%)",
            backgroundSize: "16px 16px",
            backgroundPosition: "0 0, 8px 8px",
          }}
        >
          <img
            key={stamp}
            src={src}
            alt={name}
            className="max-h-full max-w-full object-contain"
          />
        </div>
      )}
    </div>
  );
}
