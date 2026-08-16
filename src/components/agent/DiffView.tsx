// Diff unificado (o `preview` de fs_write/fs_edit chega pronto do Rust).
// Linhas +/- em verde/vermelho suaves dos tokens do projeto, cabeçalho de
// hunk discreto e rolagem horizontal própria (o chat nunca rola de lado).

import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

type LineKind = "add" | "del" | "hunk" | "file" | "ctx";

function classify(line: string): LineKind {
  if (line.startsWith("+++") || line.startsWith("---")) return "file";
  if (line.startsWith("@@")) return "hunk";
  if (line.startsWith("+")) return "add";
  if (line.startsWith("-")) return "del";
  return "ctx";
}

const LINE_CLASS: Record<LineKind, string> = {
  add: "bg-ok/10 text-ok",
  del: "bg-bad/10 text-bad",
  hunk: "text-accent/80",
  file: "text-dim",
  ctx: "text-dim",
};

export default function DiffView({
  unified,
  maxLines = 22,
  className = "",
}: {
  unified: string;
  /** Linhas exibidas antes do "ver tudo". */
  maxLines?: number;
  className?: string;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);

  const lines = useMemo(
    () => unified.replace(/\n$/, "").split("\n"),
    [unified],
  );
  const stats = useMemo(() => {
    let added = 0;
    let removed = 0;
    for (const line of lines) {
      const kind = classify(line);
      if (kind === "add") added++;
      if (kind === "del") removed++;
    }
    return { added, removed };
  }, [lines]);

  const visible = open ? lines : lines.slice(0, maxLines);
  const hidden = lines.length - visible.length;

  return (
    <div className={`overflow-hidden rounded-lg border border-edge ${className}`}>
      <div className="flex items-center gap-2 border-b border-edge bg-panel px-2.5 py-1 text-[11px] tabular-nums">
        <span className="text-ok">+{stats.added}</span>
        <span className="text-bad">−{stats.removed}</span>
      </div>
      <div className="overflow-x-auto">
        <pre className="min-w-max py-1 font-mono text-[11px] leading-[1.45] select-text">
          {visible.map((line, i) => {
            const kind = classify(line);
            return (
              <div key={i} className={`px-2.5 ${LINE_CLASS[kind]}`}>
                {line === "" ? " " : line}
              </div>
            );
          })}
        </pre>
      </div>
      {(hidden > 0 || open) && (
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="w-full border-t border-edge px-2.5 py-1 text-left text-[11px] text-dim transition-colors hover:text-ink"
        >
          {open ? t("agent.tool.showLess") : t("agent.tool.showMore")}
          {!open && hidden > 0 ? ` (+${hidden})` : ""}
        </button>
      )}
    </div>
  );
}
