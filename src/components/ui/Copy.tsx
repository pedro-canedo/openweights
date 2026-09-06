// Copiar: o gesto mais repetido destas telas.
//
// Endereço, chave, comando, bloco de código — em todos os casos a pessoa vai
// levar aquilo para outro programa. Três variações do mesmo gesto, para não
// haver três implementações com três textos de confirmação diferentes.

import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

/** Copia e diz que copiou, por um segundo. */
function useCopy() {
  const [copied, setCopied] = useState(false);
  const copy = (texto: string) => {
    void navigator.clipboard.writeText(texto).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    });
  };
  return { copied, copy };
}

/** O valor à mostra, em fonte monoespaçada, clicável para copiar. */
export function CopyValue({
  value,
  title,
  className = "",
}: {
  value: string;
  title?: string;
  className?: string;
}) {
  const { t } = useTranslation();
  const { copied, copy } = useCopy();
  return (
    <button
      type="button"
      onClick={() => copy(value)}
      title={title ?? t("server.copy")}
      className={`group inline-flex max-w-full items-center gap-2 rounded-lg border border-edge bg-panel2 px-3 py-1.5 font-mono text-[12px] text-dim transition-colors hover:border-accent hover:text-ink ${className}`}
    >
      <span className="truncate">{value}</span>
      <span className={copied ? "text-ok" : "text-dim"}>
        {copied ? "✓" : "⧉"}
      </span>
    </button>
  );
}

/** Só o botão, para quando o valor já está escrito ao lado. */
export function CopyButton({
  value,
  label,
  className = "",
}: {
  value: string;
  label?: ReactNode;
  className?: string;
}) {
  const { t } = useTranslation();
  const { copied, copy } = useCopy();
  return (
    <button
      type="button"
      onClick={() => copy(value)}
      className={`rounded-lg border border-edge px-2.5 py-1 text-[11px] text-dim transition-colors hover:border-accent hover:text-ink ${className}`}
    >
      {copied ? t("server.copied") : (label ?? t("server.copy"))}
    </button>
  );
}

/** Bloco de código com o botão no canto — o padrão dos exemplos de uso. */
export function CodeBlock({ code }: { code: string }) {
  return (
    <div className="relative">
      <pre className="select-text overflow-x-auto rounded-lg border border-edge bg-panel2 p-3 font-mono text-[11.5px] leading-relaxed text-dim">
        {code}
      </pre>
      <CopyButton
        value={code}
        className="absolute right-2 top-2 bg-panel"
      />
    </div>
  );
}
