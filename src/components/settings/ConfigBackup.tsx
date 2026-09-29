// Exportar e importar a configuração: o que a pessoa ajustou, num JSON para
// levar a outra máquina ou guardar de reserva. Chaves e senhas só vão se ela
// marcar; importar mescla e pede confirmação, dizendo o que vai mudar.

import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../ui/Button";
import { confirmar } from "../ui/Dialog";
import { toast } from "../ui/Toast";
import { Card } from "../ui/Shell";
import { salvarExportacao, mostrarExportacao } from "../../lib/exportar";
import {
  exportarConfiguracao,
  importarConfiguracao,
  lerArquivo,
  type ConteudoDoArquivo,
} from "../../lib/configuracao";

function motivo(t: (k: string, o?: Record<string, unknown>) => string, e: unknown): string {
  const texto = e instanceof Error ? e.message : String(e);
  if (texto.startsWith("invalid-json")) return t("settings.backup.errInvalid");
  if (texto === "not-openweights") return t("settings.backup.errNotOw");
  if (texto.startsWith("format:")) return t("settings.backup.errFormat", { n: texto.slice(7) });
  return texto;
}

export default function ConfigBackupCard() {
  const { t } = useTranslation();
  const [segredosNaSaida, setSegredosNaSaida] = useState(false);
  const [texto, setTexto] = useState("");
  const [segredosNaEntrada, setSegredosNaEntrada] = useState(false);
  const [erro, setErro] = useState<string | null>(null);
  const [ocupado, setOcupado] = useState(false);

  const copiar = async () => {
    try {
      await navigator.clipboard.writeText(await exportarConfiguracao(segredosNaSaida));
      toast({ tone: "ok", message: t("settings.backup.copied"), duration: 3000 });
    } catch (e) {
      setErro(String(e));
    }
  };
  const salvar = async () => {
    try {
      const data = new Date().toISOString().slice(0, 10);
      const caminho = await salvarExportacao(
        `openweights-configuracao-${data}.json`,
        await exportarConfiguracao(segredosNaSaida),
      );
      toast({
        tone: "ok",
        message: t("settings.backup.saved", { path: caminho }),
        action: { label: t("logs.showFolder"), run: () => void mostrarExportacao(caminho) },
        duration: 8000,
      });
    } catch (e) {
      setErro(String(e));
    }
  };

  const descrever = (c: ConteudoDoArquivo) =>
    t("settings.backup.contents", {
      profiles: c.profiles,
      presets: c.presets + c.enginePresets,
      settings: c.settings,
    });

  const aplicar = async () => {
    setErro(null);
    let conteudo: ConteudoDoArquivo;
    try {
      conteudo = lerArquivo(texto);
    } catch (e) {
      setErro(motivo(t, e));
      return;
    }
    const sim = await confirmar({
      title: t("settings.backup.confirmTitle"),
      message: (
        <>
          {descrever(conteudo)} {t("settings.backup.confirmMerge")}
          {segredosNaEntrada && conteudo.secrets > 0 && (
            <> {t("settings.backup.confirmSecrets", { n: conteudo.secrets })}</>
          )}
        </>
      ),
      confirmLabel: t("settings.backup.apply"),
    });
    if (!sim) return;
    setOcupado(true);
    try {
      const r = await importarConfiguracao(texto, segredosNaEntrada);
      toast({
        tone: "ok",
        message: t("settings.backup.done", {
          profiles: r.profiles,
          presets: r.presets + r.enginePresets,
          settings: r.settings,
        }),
        duration: 8000,
      });
      if (r.ignored > 0) {
        toast({ tone: "warn", message: t("settings.backup.ignored", { n: r.ignored }), duration: 0 });
      }
      setTexto("");
    } catch (e) {
      setErro(motivo(t, e));
    } finally {
      setOcupado(false);
    }
  };

  const caixa = "rounded border-edge accent-[var(--color-accent)]";
  return (
    <Card title={t("settings.backup.title")} hint={t("settings.backup.hint")}>
      <div className="mt-4 flex flex-col gap-5">
        <div className="flex flex-col gap-2">
          <p className="text-sm font-medium">{t("settings.backup.export")}</p>
          <label className="flex items-start gap-2 text-[12px] text-dim">
            <input
              type="checkbox"
              className={`${caixa} mt-0.5`}
              checked={segredosNaSaida}
              onChange={(e) => setSegredosNaSaida(e.target.checked)}
            />
            <span>
              {t("settings.backup.includeSecrets")}
              {segredosNaSaida && (
                <span role="note" className="mt-1 block text-warn">
                  {t("settings.backup.secretsWarning")}
                </span>
              )}
            </span>
          </label>
          <div className="flex flex-wrap gap-2">
            <Button size="sm" onClick={() => void copiar()}>
              {t("settings.backup.copy")}
            </Button>
            <Button size="sm" onClick={() => void salvar()}>
              {t("settings.backup.save")}
            </Button>
          </div>
        </div>

        <div className="flex flex-col gap-2 border-t border-edge pt-4">
          <p className="text-sm font-medium">{t("settings.backup.import")}</p>
          <textarea
            value={texto}
            onChange={(e) => {
              setTexto(e.target.value);
              setErro(null);
            }}
            aria-label={t("settings.backup.pasteLabel")}
            placeholder={t("settings.backup.pasteLabel")}
            rows={4}
            spellCheck={false}
            className="w-full resize-y rounded-lg border border-edge bg-panel2 p-2 font-mono text-[11px] outline-none focus-visible:border-accent"
          />
          <label className="flex items-start gap-2 text-[12px] text-dim">
            <input
              type="checkbox"
              className={`${caixa} mt-0.5`}
              checked={segredosNaEntrada}
              onChange={(e) => setSegredosNaEntrada(e.target.checked)}
            />
            <span>{t("settings.backup.importSecrets")}</span>
          </label>
          {erro && (
            <p role="alert" className="text-[12px] text-bad select-text">
              {erro}
            </p>
          )}
          <div>
            <Button
              size="sm"
              variant="primary"
              busy={ocupado}
              disabled={texto.trim() === ""}
              onClick={() => void aplicar()}
            >
              {t("settings.backup.check")}
            </Button>
          </div>
          <p className="text-[11px] text-dim">{t("settings.backup.restartNote")}</p>
        </div>
      </div>
    </Card>
  );
}
