// Seletor segmentado Chat | Agente (persistido em `ChatParams.agent`).
// O app é agent-first: o modo agente é o padrão, então as duas opções
// precisam estar visíveis lado a lado — um toggle escondido atrás de um
// ícone fazia sentido quando o agente era exceção, não agora que é regra.
//
// A capacidade de ferramentas vem do chat template do modelo carregado
// (GET /props) — nunca do nome do modelo. Sem suporte declarado, o segmento
// Agente NÃO bloqueia: o backend já degrada com aviso visível na trilha
// (evento `tools.off`); aqui só avisamos no tooltip e no pontinho de alerta.

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
        // e desconhecido NÃO gera aviso.
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

  const agentOn = params.agent === true;
  const unsupported = supportsTools === false;

  // Mesmo idioma visual dos botões da barra: ativo acende em accent,
  // inativo fica apagado até o hover.
  const segment = (active: boolean) =>
    `flex h-full items-center rounded-full px-2.5 text-xs transition-colors disabled:opacity-40 ${
      active ? "bg-accent/15 text-accent" : "text-dim hover:text-ink"
    }`;

  return (
    <div
      role="group"
      aria-label={t("agent.toggle")}
      className="flex h-8 shrink-0 items-center rounded-full border border-edge bg-panel p-0.5"
    >
      <button
        type="button"
        disabled={disabled}
        title={t("agent.toggleOff")}
        aria-pressed={!agentOn}
        onClick={() => onChange({ ...params, agent: false })}
        className={segment(!agentOn)}
      >
        {t("agent.modeChat")}
      </button>
      <button
        type="button"
        disabled={disabled}
        title={unsupported ? t("agent.unsupported") : t("agent.toggleOn")}
        aria-pressed={agentOn}
        onClick={() =>
          onChange({
            ...params,
            agent: true,
            // Primeira vez: entra no modo conservador (lê sozinho, pede para
            // alterar) em vez de já sair executando.
            mode: params.mode ?? "smart",
          })
        }
        className={`relative ${segment(agentOn)}`}
      >
        {t("agent.modeAgent")}
        {unsupported && (
          <span
            aria-label={t("agent.unsupportedShort")}
            className="absolute top-0.5 right-0.5 h-1.5 w-1.5 rounded-full bg-warn"
          />
        )}
      </button>
    </div>
  );
}
