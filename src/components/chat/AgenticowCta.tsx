// Os dois convites ao AgenticOw que moram no Chat.
//
// Eles NÃO preparam nem sobem nada: o AgenticOw tem tela própria na barra
// lateral, e é lá que a preparação acontece, com o progresso à vista. Aqui
// fica só o convite; quem decide é a tela.

import { useEffect, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { agenticowStore, refreshStatus } from "../../lib/agenticow";
import { navigate } from "../../lib/nav";
import Icon from "../ui/Icon";

function irParaOAgenticow() {
  navigate("agenticow");
}

/** Card no estado vazio do Chat (hero): o convite explícito. */
export function AgenticowHeroCard() {
  const { t } = useTranslation();
  const s = useSyncExternalStore(agenticowStore.subscribe, agenticowStore.get);
  useEffect(() => {
    void refreshStatus();
  }, []);
  const rodando = s.status?.ready ?? false;

  return (
    <div className="mt-4 flex w-full items-center gap-4 rounded-2xl border border-edge bg-panel/80 px-5 py-4 text-left">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-ink">
          {t("chat.agenticow.title")}
        </div>
        <p className="mt-1 text-[12px] leading-relaxed text-dim">
          {t("chat.agenticow.body")}
        </p>
      </div>
      <button
        type="button"
        onClick={irParaOAgenticow}
        className="shrink-0 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-white"
      >
        {rodando ? t("chat.agenticow.openPanel") : t("chat.agenticow.open")}
      </button>
    </div>
  );
}

/** Botão compacto do composer — mora onde vivia o toggle de agente. */
export function AgenticowComposerButton() {
  const { t } = useTranslation();
  const s = useSyncExternalStore(agenticowStore.subscribe, agenticowStore.get);
  useEffect(() => {
    void refreshStatus();
  }, []);
  const rodando = s.status?.ready ?? false;

  return (
    <button
      type="button"
      onClick={irParaOAgenticow}
      title={rodando ? t("chat.agenticow.openPanel") : t("chat.agenticow.title")}
      className="flex h-8 shrink-0 items-center gap-1 rounded-full border border-edge px-3 text-xs text-dim transition-colors hover:border-accent hover:text-ink"
    >
      <span>{t("chat.agenticow.agent")}</span>
      <Icon name="external" className="h-3 w-3" />
    </button>
  );
}
