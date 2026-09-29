// O `owcli` em qualquer terminal do sistema (opt-in). Fica em Configurações,
// no modo Avançado: quem só usa o app não precisa disto, e nada muda no
// terminal de ninguém sem o clique. No modo Simples o cartão só aparece se o
// comando já está ligado, para dar como desligá-lo.

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { motivoDoTerminal, owcliNoTerminal, type OwcliNoTerminal } from "../../lib/owcliTerminal";
import { useModo } from "../../lib/mode";
import { navigate } from "../../lib/nav";
import { Button } from "../ui/Button";
import { CopyValue } from "../ui/Copy";
import { Card } from "../ui/Shell";

/** Para o PATH: o que colocar no perfil do shell quando a pasta do link não está nele. */
const LINHA_DO_PATH = 'export PATH="$HOME/.local/bin:$PATH"';

export default function OwcliNoTerminalCard() {
  const { t } = useTranslation();
  const modo = useModo();
  const [e, setE] = useState<OwcliNoTerminal | null>(null);
  const [ocupado, setOcupado] = useState(false);
  const [erro, setErro] = useState<string | null>(null);

  // Lê de novo ao voltar para a janela: a pessoa pode ter renomeado o arquivo
  // que estava no caminho, ou instalado o agente em outra tela.
  const recarregar = useCallback(() => {
    owcliNoTerminal.status().then(setE).catch(() => setE(null));
  }, []);
  useEffect(() => {
    recarregar();
    const aoVoltar = () => document.visibilityState === "visible" && recarregar();
    window.addEventListener("focus", recarregar);
    document.addEventListener("visibilitychange", aoVoltar);
    return () => {
      window.removeEventListener("focus", recarregar);
      document.removeEventListener("visibilitychange", aoVoltar);
    };
  }, [recarregar]);

  // Sem suporte neste sistema, o cartão nem aparece; no Simples, só ligado.
  if (!e || !e.supported || (modo === "simples" && !e.ativo)) return null;

  async function alternar() {
    if (!e || ocupado) return;
    setOcupado(true);
    setErro(null);
    try {
      setE(await (e.ativo ? owcliNoTerminal.desativar() : owcliNoTerminal.ativar()));
    } catch (err) {
      setErro(motivoDoTerminal(err));
      recarregar();
    } finally {
      setOcupado(false);
    }
  }

  const semAgente = !e.installed;
  return (
    <Card title={t("owcliTerminal.title")} hint={t("owcliTerminal.hint")}>
      <div className="mt-3 grid gap-3">
        {semAgente && !e.ativo ? (
          <div className="flex flex-wrap items-center gap-3">
            <p className="text-[13px] text-dim">{t("owcliTerminal.needsAgent")}</p>
            <Button size="sm" icon="terminal" onClick={() => navigate("owcli")}>
              {t("owcliTerminal.openOwcli")}
            </Button>
          </div>
        ) : (
          <>
            <div className="flex flex-wrap items-center gap-3">
              {/* `aria-disabled` e não `disabled`: durante a operação o foco fica
                  no botão em vez de se perder. */}
              <Button
                variant={e.ativo ? "secondary" : "primary"}
                size="sm"
                aria-disabled={ocupado || undefined}
                onClick={() => void alternar()}
              >
                {e.ativo ? t("owcliTerminal.turnOff") : t("owcliTerminal.turnOn")}
              </Button>
              <span role="status" className="text-[12px] text-ok">
                {e.ativo && !semAgente
                  ? e.apontaParaOAtual
                    ? t("owcliTerminal.on")
                    : t("owcliTerminal.onOutdated")
                  : ""}
              </span>
            </div>
            {e.ativo && semAgente && (
              <p className="text-[12px] leading-relaxed text-warn">{t("owcliTerminal.installedOff")}</p>
            )}
            {e.ativo && e.local && (
              <div className="text-[12px] text-dim">
                {t(e.viaPath ? "owcliTerminal.whereWindows" : "owcliTerminal.where")}{" "}
                <CopyValue value={e.local} className="mt-1" />
              </div>
            )}
            {e.conflito && !e.ativo && (
              <p role="status" className="text-[12px] leading-relaxed text-warn">
                {t("owcliTerminal.conflict", { path: e.conflito })}
              </p>
            )}
            {e.ativo && e.viaPath && (
              <p className="text-[12px] leading-relaxed text-dim">{t("owcliTerminal.newTerminal")}</p>
            )}
            {e.ativo && !e.viaPath && !e.noPath && (
              <div className="grid gap-1.5 text-[12px] leading-relaxed text-dim">
                <p>{t("owcliTerminal.notOnPath")}</p>
                <CopyValue value={LINHA_DO_PATH} />
              </div>
            )}
            {!semAgente && (
              <p className="text-[12px] leading-relaxed text-dim">{t("owcliTerminal.appOpen")}</p>
            )}
          </>
        )}
        {erro && (
          <p role="alert" className="text-[12px] leading-relaxed text-bad select-text">
            {erro}
          </p>
        )}
      </div>
    </Card>
  );
}
