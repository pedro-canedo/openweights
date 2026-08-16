// Liga/desliga o modo agente da conversa (persistido em `ChatParams.agent`).
// A capacidade de ferramentas vem do chat template do modelo carregado
// (GET /props) — nunca do nome do modelo. Sem suporte, o botão fica
// desabilitado com a explicação no tooltip.

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { getServerProps } from "../../lib/api";
import type { ChatParams } from "../../lib/types";

export default function AgentToggle({
  params,
  onChange,
  model,
  disabled = false,
}: {
  params: ChatParams;
  onChange: (p: ChatParams) => void;
  /** Modelo selecionado: trocar de modelo revalida o suporte a ferramentas. */
  model: string;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  // null = desconhecido (servidor fora do ar): não bloqueia o usuário.
  const [supportsTools, setSupportsTools] = useState<boolean | null>(null);

  useEffect(() => {
    let cancelled = false;
    setSupportsTools(null);
    if (!model) return;
    void getServerProps()
      .then((props) => {
        if (!cancelled) setSupportsTools(props.chatTemplateCaps.supportsTools);
      })
      .catch(() => {
        if (!cancelled) setSupportsTools(null);
      });
    return () => {
      cancelled = true;
    };
  }, [model]);

  const on = params.agent === true;
  const unsupported = supportsTools === false;

  const title = unsupported
    ? t("agent.unsupported")
    : on
      ? t("agent.toggleOn")
      : t("agent.toggleOff");

  return (
    <button
      type="button"
      disabled={disabled || unsupported}
      title={title}
      aria-pressed={on}
      onClick={() =>
        onChange({
          ...params,
          agent: !on,
          // Primeira vez: entra no modo conservador (lê sozinho, pede para
          // alterar) em vez de já sair executando.
          mode: params.mode ?? "smart",
        })
      }
      className={`flex shrink-0 items-center gap-1.5 rounded-full px-2 py-1 text-xs transition-colors disabled:opacity-40 ${
        on
          ? "bg-accent/15 text-accent"
          : "text-ink hover:bg-panel"
      }`}
    >
      <svg
        className="h-4 w-4 shrink-0"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
        viewBox="0 0 24 24"
      >
        <rect x="4" y="8" width="16" height="12" rx="3" />
        <path d="M12 8V4M9 14h.01M15 14h.01M2 13v3M22 13v3" />
      </svg>
      <span>{t("agent.toggle")}</span>
      {unsupported && (
        <span
          aria-label={t("agent.unsupportedShort")}
          className="h-1.5 w-1.5 shrink-0 rounded-full bg-warn"
        />
      )}
    </button>
  );
}
