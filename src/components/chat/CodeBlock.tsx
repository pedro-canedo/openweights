// Bloco de código com destaque de sintaxe via shiki (import dinâmico,
// highlighter em cache de módulo). Enquanto o shiki carrega — ou durante o
// streaming — mostramos um <pre> simples como fallback.

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

// Cache de módulo: o bundle do shiki só é baixado uma vez, sob demanda.
// bundle/web = só linguagens comuns na web (o pacote completo tem ~200
// gramáticas e multiplicava o tamanho do dist por ~10).
let shikiPromise: Promise<typeof import("shiki/bundle/web")> | null = null;
const loadShiki = () => (shikiPromise ??= import("shiki/bundle/web"));

function currentTheme(): string {
  return document.documentElement.dataset.theme === "light"
    ? "github-light"
    : "github-dark-default";
}

export default function CodeBlock({
  code,
  lang,
}: {
  code: string;
  lang: string;
}) {
  const { t } = useTranslation();
  const [html, setHtml] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let cancelled = false;
    // Pequeno debounce: durante o streaming o código muda a cada delta.
    const timer = window.setTimeout(async () => {
      try {
        const shiki = await loadShiki();
        const theme = currentTheme();
        let out: string;
        try {
          out = await shiki.codeToHtml(code, { lang, theme });
        } catch {
          // Linguagem desconhecida — cai para texto puro.
          out = await shiki.codeToHtml(code, { lang: "text", theme });
        }
        if (!cancelled) setHtml(out);
      } catch {
        // shiki indisponível — mantém o fallback <pre>.
      }
    }, 120);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [code, lang]);

  const copy = () => {
    void navigator.clipboard.writeText(code).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    });
  };

  return (
    <div className="group relative my-2 overflow-hidden rounded-lg border border-edge">
      <div className="flex items-center justify-between border-b border-edge bg-panel2/60 px-3 py-1">
        <span className="text-[10px] uppercase tracking-wide text-dim">
          {lang}
        </span>
        <button
          onClick={copy}
          className="rounded px-1.5 py-0.5 text-[10px] text-dim opacity-0 transition-opacity group-hover:opacity-100 hover:bg-panel2 hover:text-ink"
        >
          {copied ? t("server.copied") : t("server.copy")}
        </button>
      </div>
      {html ? (
        <div
          className="text-[13px] leading-relaxed [&_pre]:overflow-x-auto [&_pre]:p-3"
          // HTML gerado localmente pelo shiki a partir do texto do modelo.
          dangerouslySetInnerHTML={{ __html: html }}
        />
      ) : (
        <pre className="overflow-x-auto bg-panel p-3 text-[13px] leading-relaxed">
          <code>{code}</code>
        </pre>
      )}
    </div>
  );
}
