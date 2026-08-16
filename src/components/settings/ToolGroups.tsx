// Preferências das famílias de ferramentas.
//
// Modelo local tem janela pequena, e as 37 ferramentas do harness não cabem
// todas na frente dele. A pessoa liga e desliga FAMÍLIAS inteiras aqui; o
// cardápio que o agente monta a cada run sai só do que ficou ligado, e o que
// não coube o modelo pede quando precisar.
//
// A contagem por família vem do catálogo vivo do backend (conectores MCP
// incluídos): o número existe para a pessoa dimensionar o que está
// desligando, e uma lista escrita aqui mentiria na primeira ferramenta nova.

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  toolGroupCounts,
  toolGroupsGet,
  toolGroupsSet,
} from "../../lib/agent/agentApi";
import { TOOL_GROUPS, type ToolGroup } from "../../lib/agent/types";

/** Descarta chaves desconhecidas e normaliza a ordem de exibição. */
function parseGroups(keys: string[]): ToolGroup[] {
  return TOOL_GROUPS.filter((group) => keys.includes(group));
}

export default function ToolGroups() {
  const { t } = useTranslation();
  // null = ainda carregando (lista vazia é um estado possível do backend).
  const [enabled, setEnabled] = useState<ToolGroup[] | null>(null);
  const [counts, setCounts] = useState<Record<string, number>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void toolGroupsGet().then((keys) => setEnabled(parseGroups(keys)));
  }, []);

  useEffect(() => {
    void toolGroupCounts().then(setCounts);
  }, []);

  /** Otimista: a chavinha vira na hora e volta sozinha se o backend recusar. */
  async function toggle(group: ToolGroup): Promise<void> {
    if (!enabled || busy) return;
    const next = enabled.includes(group)
      ? enabled.filter((g) => g !== group)
      : TOOL_GROUPS.filter((g) => g === group || enabled.includes(g));
    setEnabled(next);
    setBusy(true);
    setError(null);
    try {
      // Vale o que o backend guardou, não o que a tela pediu (ele descarta
      // chave que não conhece). Resposta vazia seria "voltou ao padrão" —
      // nesse caso a tela não tem o que mostrar de novo.
      const saved = await toolGroupsSet(next);
      if (Array.isArray(saved) && saved.length > 0) setEnabled(parseGroups(saved));
    } catch (e) {
      setEnabled(enabled);
      setError(t("agent.toolGroups.saveFailed", { error: String(e) }));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div
      id="tool-groups"
      className="rounded-xl border border-edge bg-panel px-5 py-4"
    >
      <div className="text-sm">{t("agent.toolGroups.title")}</div>
      <div className="mt-1 text-[12px] text-dim">
        {t("agent.toolGroups.subtitle")}
      </div>

      {error && (
        <p className="mt-3 rounded-lg border border-bad/40 bg-bad/10 px-3 py-2 text-[12px] text-bad">
          {error}
        </p>
      )}

      <div className="mt-4 flex flex-col gap-1.5">
        {!enabled && (
          <p className="text-[12px] text-dim">{t("common.loading")}</p>
        )}
        {enabled?.length === 0 && (
          <p className="text-[12px] text-warn">{t("agent.toolGroups.lastOne")}</p>
        )}
        {enabled &&
          TOOL_GROUPS.map((group) => (
            <GroupRow
              key={group}
              group={group}
              on={enabled.includes(group)}
              count={counts[group] ?? 0}
              // Sem nenhuma família ligada não existe "agente sem ferramenta":
              // a preferência vazia é lida como o padrão, ou seja, TODAS. Para
              // não mentir na tela, a última ligada fica travada.
              locked={enabled.length === 1 && enabled.includes(group)}
              busy={busy}
              onToggle={() => void toggle(group)}
            />
          ))}
      </div>

      <p className="mt-3 text-[11px] text-dim">
        {t("agent.toolGroups.windowHint")}
      </p>
    </div>
  );
}

function GroupRow({
  group,
  on,
  count,
  locked,
  busy,
  onToggle,
}: {
  group: ToolGroup;
  on: boolean;
  count: number;
  /** Última família ligada: não dá para desligar. */
  locked: boolean;
  busy: boolean;
  onToggle: () => void;
}) {
  const { t } = useTranslation();
  const label = t(`agent.toolGroups.${group}.label`);
  const frozen = locked || busy;

  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      // `aria-disabled` em vez de `disabled`: a linha travada continua
      // alcançável pelo teclado, senão a explicação fica inacessível.
      aria-disabled={frozen}
      onClick={() => {
        if (!frozen) onToggle();
      }}
      className={`flex w-full items-start gap-3 rounded-lg border border-edge bg-panel2 px-3 py-2.5 text-left transition-colors ${
        frozen ? "cursor-default" : "hover:border-accent"
      }`}
    >
      <span className="min-w-0 flex-1">
        <span className="flex flex-wrap items-center gap-2">
          <span className={`text-[13px] ${on ? "text-ink" : "text-dim"}`}>
            {label}
          </span>
          <span className="shrink-0 rounded-md bg-panel px-1.5 py-0.5 text-[10px] text-dim">
            {/* Só a família de conectores chega a zero (nenhum MCP ligado), e
                "0 ferramenta" não é frase que se escreva. */}
            {count === 0
              ? t("agent.toolGroups.countNone")
              : t("agent.toolGroups.count", { count })}
          </span>
        </span>
        <span className="mt-0.5 block text-[11px] leading-relaxed text-dim">
          {t(`agent.toolGroups.${group}.desc`)}
        </span>
        {locked && (
          <span className="mt-1 block text-[11px] leading-relaxed text-warn">
            {t("agent.toolGroups.lastOne")}
          </span>
        )}
      </span>
      <Switch on={on} />
    </button>
  );
}

/** Chavinha só visual: quem recebe foco e clique é a linha inteira. */
function Switch({ on }: { on: boolean }) {
  return (
    <span
      aria-hidden
      className={`mt-0.5 flex h-4 w-7 shrink-0 items-center rounded-full p-0.5 transition-colors ${
        on ? "bg-accent" : "bg-edge"
      }`}
    >
      <span
        className={`h-3 w-3 rounded-full bg-panel transition-transform ${
          on ? "translate-x-3" : ""
        }`}
      />
    </span>
  );
}
