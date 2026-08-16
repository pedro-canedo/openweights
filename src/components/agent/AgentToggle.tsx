// Liga/desliga o modo agente da conversa (persistido em `ChatParams.agent`).
// A capacidade de ferramentas vem do chat template do modelo carregado
// (GET /props) — nunca do nome do modelo. Sem suporte, o botão fica
// desabilitado com a explicação no tooltip.
//
// É só o ícone, no tamanho dos outros botões redondos da barra: o seletor ao
// lado já escreve em que modo o agente está ("Agente", "Plano", "Laço"), e
// duas palavras iguais lado a lado não informavam nada. Ligado, ele acende.

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { getServerProps } from "../../lib/api";
import { describesModel } from "../../lib/agent/types";
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
    void getServerProps(model)
      .then((props) => {
        // Resposta do roteador não fala deste modelo: continua desconhecido,
        // e desconhecido NÃO desabilita o botão.
        if (cancelled) return;
        setSupportsTools(
          describesModel(props) ? props.chatTemplateCaps.supportsTools : null,
        );
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
      className={`relative flex h-8 w-8 shrink-0 items-center justify-center rounded-full transition-colors disabled:opacity-40 ${
        on
          ? "bg-accent/15 text-accent"
          : "text-dim hover:bg-panel hover:text-ink"
      }`}
    >
      <svg
        className="h-[18px] w-[18px] shrink-0"
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
      <span className="sr-only">{t("agent.toggle")}</span>
      {unsupported && (
        <span
          aria-label={t("agent.unsupportedShort")}
          className="absolute top-1 right-1 h-1.5 w-1.5 rounded-full bg-warn"
        />
      )}
    </button>
  );
}
