// "Abrir em um harness": cartões de agentes de código externos que consomem
// a API local — no espírito da lista "Applications" do Ollama. O comando é
// montado no backend (com segredo mascarado na prévia); aqui só se mostra,
// copia e dispara.

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  harnessLaunch,
  harnessList,
  type HarnessStatus,
} from "../../lib/flags";
import { navigate } from "../../lib/nav";

export default function HarnessLauncher({
  model,
  loaded,
  running,
}: {
  model: string;
  loaded: boolean;
  running: boolean;
}) {
  const { t } = useTranslation();
  const [list, setList] = useState<HarnessStatus[]>([]);
  const [copied, setCopied] = useState<string | null>(null);
  const [launching, setLaunching] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!model) return;
    let cancelled = false;
    harnessList(model)
      .then((l) => {
        if (!cancelled) setList(l);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [model, running]);

  if (!list.length) return null;

  const open = async (id: string) => {
    setLaunching(id);
    setError(null);
    try {
      if (id === "dsh") {
        // O dsh é gerenciado pelo app e tem tela própria: instalar, subir,
        // parar e usar acontecem lá, com progresso e log à vista. Aqui o
        // cartão só leva até ela — instalar por trás deste botão escondia
        // minutos de trabalho numa tela sobre outra coisa. O preview de
        // comando continua aí para quem prefere o terminal.
        navigate("harness");
      } else {
        await harnessLaunch(id, model);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setLaunching(null);
    }
  };

  return (
    <div className="mt-5 border-t border-edge pt-4">
      <div className="text-sm font-medium">{t("server.harness.title")}</div>
      <p className="mt-0.5 text-[11px] leading-relaxed text-dim">
        {t("server.harness.subtitle")}
      </p>
      {!loaded && (
        <p className="mt-1 text-[11px] leading-relaxed text-dim">
          {t("server.harness.needLoaded")}
        </p>
      )}
      <div className="mt-3 flex flex-col gap-2">
        {list.map((h) => (
          <div
            key={h.id}
            className="rounded-lg border border-edge bg-panel2/50 p-3"
          >
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-sm font-medium">{h.name}</span>
              <span
                className={`rounded-full border px-2 py-0.5 text-[10px] ${
                  h.installed
                    ? "border-ok/40 bg-ok/10 text-ok"
                    : h.launchable
                      ? "border-edge text-dim"
                      : "border-warn/40 bg-warn/10 text-warn"
                }`}
              >
                {h.installed
                  ? t("server.harness.installed")
                  : h.launchable
                    ? t("server.harness.viaNpx")
                    : t("server.harness.notInstalled")}
              </span>
              <a
                href={h.docsUrl}
                target="_blank"
                rel="noreferrer"
                className="text-[11px] text-dim underline decoration-edge hover:text-ink"
              >
                {t("server.harness.docs")}
              </a>
              <button
                type="button"
                disabled={
                  (h.id !== "dsh" && (!loaded || !h.launchable)) ||
                  launching != null
                }
                onClick={() => void open(h.id)}
                className="ml-auto rounded-lg bg-accent px-3 py-1.5 text-xs font-medium text-white disabled:opacity-50"
              >
                {launching === h.id
                  ? t("common.loading")
                  : t("server.harness.open")}
              </button>
            </div>
            <div className="relative mt-2">
              <pre className="select-text overflow-x-auto rounded-lg border border-edge bg-panel2 p-2 font-mono text-[11px] leading-relaxed text-dim">
                {h.installed || h.launchable ? h.commandPreview : h.installCmd}
              </pre>
              <button
                type="button"
                onClick={() => {
                  const text =
                    h.installed || h.launchable ? h.commandPreview : h.installCmd;
                  void navigator.clipboard.writeText(text).then(() => {
                    setCopied(h.id);
                    setTimeout(() => setCopied(null), 1200);
                  });
                }}
                className="absolute right-1.5 top-1.5 rounded-md border border-edge bg-panel px-2 py-0.5 text-[10px] text-dim hover:text-ink"
              >
                {copied === h.id ? t("server.copied") : t("server.copy")}
              </button>
            </div>
            {!h.installed && h.launchable && (
              <p className="mt-1 text-[10px] leading-relaxed text-dim">
                {t("server.harness.installHint", { cmd: h.installCmd })}
              </p>
            )}
          </div>
        ))}
      </div>
      {error && <p className="mt-2 text-[12px] text-bad">{error}</p>}
    </div>
  );
}
