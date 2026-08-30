// Os dois convites ao DeepSeek Harness que moram no Chat.
//
// Eles NÃO instalam nem sobem nada: o harness tem tela própria na barra
// lateral, e é lá que a instalação acontece, com progresso, log e o painel
// embutido. Antes o clique aqui disparava minutos de `npm install` por trás
// de um toast flutuante, num lugar em que a pessoa tinha vindo conversar —
// era trabalho pesado escondido atrás de um botão pequeno. Aqui ficou só o
// convite; quem decide é a tela.

import { useEffect, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { harnessStore, refreshStatus } from "../../lib/harness";
import { navigate } from "../../lib/nav";

function irParaOHarness() {
  navigate("harness");
}

/** Card no estado vazio do Chat (hero): o convite explícito. */
export function HarnessHeroCard() {
  const { t } = useTranslation();
  const s = useSyncExternalStore(harnessStore.subscribe, harnessStore.get);
  useEffect(() => {
    void refreshStatus();
  }, []);
  const rodando = s.status?.running ?? false;

  return (
    <div className="mt-4 flex w-full items-center gap-4 rounded-2xl border border-edge bg-panel/80 px-5 py-4 text-left">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-ink">
          {t("chat.harness.title")}
        </div>
        <p className="mt-1 text-[12px] leading-relaxed text-dim">
          {t("chat.harness.body")}
        </p>
      </div>
      <button
        type="button"
        onClick={irParaOHarness}
        className="shrink-0 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-white"
      >
        {rodando ? t("chat.harness.openPanel") : t("chat.harness.open")}
      </button>
    </div>
  );
}

/** Botão compacto do composer — mora onde vivia o toggle de agente. */
export function HarnessComposerButton() {
  const { t } = useTranslation();
  const s = useSyncExternalStore(harnessStore.subscribe, harnessStore.get);
  useEffect(() => {
    void refreshStatus();
  }, []);
  const rodando = s.status?.running ?? false;

  return (
    <button
      type="button"
      onClick={irParaOHarness}
      title={rodando ? t("chat.harness.openPanel") : t("chat.harness.title")}
      className="flex h-8 shrink-0 items-center gap-1 rounded-full border border-edge px-3 text-xs text-dim transition-colors hover:border-accent hover:text-ink"
    >
      <span>{t("chat.harness.agent")}</span>
      <span aria-hidden="true">⧉</span>
    </button>
  );
}
