// O que o llama-server roda (ou vai rodar): o comando, o ambiente e o INI dos
// modelos, à mostra e não escondidos. Diz se o que está no ar é o que está
// configurado (`configStale`), copia como comando de shell (a chave de API
// mascarada) e mostra o arquivo INI na pasta.

import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Badge, Button } from "../ui/Button";
import { toast } from "../ui/Toast";
import { revealWorkspace } from "../../lib/api";
import { comandoShell } from "../../lib/comandoShell";
import type { EnginePreview } from "../../lib/flags";

export default function CommandPreview({
  preview,
  running,
  configStale,
}: {
  preview: EnginePreview | null;
  running: boolean;
  /** Há configuração nova que o processo de pé ainda não usa. */
  configStale: boolean;
}) {
  const { t } = useTranslation();
  const [aberto, setAberto] = useState(true);
  if (!preview) return null;

  const shell = comandoShell("llama-server", preview.args, preview.env);
  const copiar = (texto: string, msg: string) =>
    void navigator.clipboard
      .writeText(texto)
      .then(() => toast({ tone: "ok", message: msg, duration: 2500 }));

  // A pasta e o arquivo do INI, sem depender do separador do sistema.
  const mostrarIni = () => {
    const i = Math.max(preview.iniPath.lastIndexOf("/"), preview.iniPath.lastIndexOf("\\"));
    void revealWorkspace(preview.iniPath.slice(0, i), preview.iniPath.slice(i + 1)).catch((e) =>
      toast({ tone: "bad", message: t("server.preview.revealFailed", { error: String(e) }), duration: 6000 }),
    );
  };

  return (
    <section aria-label={t("server.engineConfig.preview")} className="mt-4 flex flex-col gap-2 border-t border-edge pt-4">
      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          aria-expanded={aberto}
          onClick={() => setAberto((v) => !v)}
          className="text-sm font-medium"
        >
          {t("server.engineConfig.preview")}
        </button>
        {running ? (
          configStale ? (
            <Badge tone="warn">{t("server.preview.pending")}</Badge>
          ) : (
            <Badge tone="ok">{t("server.preview.running")}</Badge>
          )
        ) : (
          <Badge tone="neutral">{t("server.preview.next")}</Badge>
        )}
      </div>
      {running && configStale && (
        <p className="text-[11px] leading-relaxed text-warn">{t("server.preview.pendingHint")}</p>
      )}
      {aberto && (
        <>
          <pre className="select-text overflow-x-auto rounded-lg border border-edge bg-panel2 p-3 font-mono text-[11.5px] leading-relaxed text-dim">
            {[
              `# llama-server ${preview.args.join(" ")}`,
              preview.env.length > 0
                ? `\n# ${t("server.engineConfig.envPreview")}\n${preview.env.join("\n")}`
                : "",
              `\n# ${preview.iniPath}\n${preview.ini}`,
            ].join("\n")}
          </pre>
          <div className="flex flex-wrap gap-2">
            <Button size="sm" onClick={() => copiar(shell, t("server.preview.copiedShell"))}>
              {t("server.preview.copyShell")}
            </Button>
            <Button
              size="sm"
              onClick={() => copiar(preview.ini, t("server.preview.copiedIni"))}
            >
              {t("server.preview.copyIni")}
            </Button>
            <Button size="sm" variant="ghost" onClick={mostrarIni}>
              {t("server.preview.revealIni")}
            </Button>
          </div>
          <p className="text-[11px] text-dim">{t("server.preview.maskedNote")}</p>
        </>
      )}
    </section>
  );
}
