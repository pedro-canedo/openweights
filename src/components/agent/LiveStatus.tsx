// A linha viva: o que o agente está fazendo AGORA, em uma frase.
//
// Enquanto o run trabalha, a conversa mostrava "Analisando…" e a trilha,
// "Pensou por 60,6s". Nenhum dos dois responde a pergunta que a pessoa tem na
// cabeça — *em que pé está isso?* —, e num modelo local um passo leva minuto,
// então o silêncio dura o suficiente para parecer travamento.
//
// Aqui o estado é DERIVADO do que já chega ao runStore; nada de evento novo.
// A ordem de prioridade é a da pergunta: primeiro o que está acontecendo neste
// instante (uma ferramenta rodando, um ajudante trabalhando), depois o que o
// modelo está fazendo entre ferramentas (pensando, escrevendo). O contexto —
// passo e etapa do plano — vem junto, porque "criando hud.js" sem saber que é
// a etapa 2 de 5 conta metade da história.
//
// O que esta linha NÃO faz: repetir a barra de aprovação. Quando o run para
// esperando uma decisão, quem manda no espaço é ela, e duas mensagens sobre a
// mesma coisa competem em vez de informar.
import type { TFunction } from "i18next";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import type { RunView } from "../../lib/agent/runStore";
import { isRunActive } from "../../lib/agent/runStore";
import { toolTarget } from "./ToolCallCard";
import { toolLabel } from "./toolMeta";

/** Relógio de 1 Hz, ligado só enquanto há o que contar. */
function useSeconds(since: number | null): number {
  const [agora, setAgora] = useState(() => Date.now());
  useEffect(() => {
    if (since == null) return;
    setAgora(Date.now());
    const id = window.setInterval(() => setAgora(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [since]);
  return since == null ? 0 : Math.max(0, Math.round((agora - since) / 1000));
}

interface Estado {
  /** A frase principal. */
  texto: string;
  /** Alvo em monoespaçada (caminho, comando) — vazio quando não há. */
  alvo: string;
  /** Desde quando isto está acontecendo, para o relógio. */
  desde: number | null;
}

/**
 * O que está acontecendo agora, na ordem em que a pessoa perguntaria.
 *
 * Devolve `null` quando não há o que dizer — e isso é uma resposta: linha viva
 * que inventa atividade para não ficar vazia é pior que linha nenhuma.
 */
function estadoDe(run: RunView, t: TFunction): Estado | null {
  // Um ajudante trabalhando é o fato mais específico disponível: ele roda
  // dentro da chamada do agente, então enquanto durar é dele que se trata.
  for (let i = run.items.length - 1; i >= 0; i--) {
    const item = run.items[i];
    if (item.kind !== "subagent") continue;
    if (item.phase === "end") break; // o último par fechou; nada em curso
    return {
      texto: t(`agent.run.subagentStarted.${item.role}`, {
        objective: item.objective,
      }) as string,
      alvo: "",
      desde: item.tsMs,
    };
  }

  // O passo aberto é a origem do relógio em todos os casos: a chamada de
  // ferramenta não guarda quando começou, e o que a pessoa está esperando é o
  // passo terminar de qualquer maneira.
  const passo = [...run.items].reverse().find((i) => i.kind === "step");
  const desde = passo?.kind === "step" ? passo.startedAtMs : null;

  const rodando = Object.values(run.tools).find((c) => c.state === "running");
  if (rodando) {
    return {
      texto: toolLabel(t, rodando.tool),
      alvo: toolTarget(rodando),
      desde,
    };
  }

  if (passo?.kind === "step" && passo.finishedAtMs == null) {
    const pensando = passo.thoughtMs == null;
    return {
      texto: t(pensando ? "agent.live.thinking" : "agent.live.writing"),
      alvo: "",
      desde,
    };
  }
  return null;
}

export default function LiveStatus({ run }: { run: RunView | null | undefined }) {
  const { t } = useTranslation();
  const ativo = run != null && isRunActive(run.status);
  const estado = ativo ? estadoDe(run, t) : null;
  const segundos = useSeconds(estado?.desde ?? null);

  // A aprovação tem barra própria e ela é o assunto; duplicar aqui competiria.
  if (!run || !ativo || run.pendingCallId || !estado) return null;

  const passo = [...run.items].reverse().find((i) => i.kind === "step");
  const etapa = run.plan?.tasks.length
    ? {
        n: Math.min(run.plan.current + 1, run.plan.tasks.length),
        total: run.plan.tasks.length,
        titulo: run.plan.tasks[run.plan.current]?.title ?? "",
      }
    : null;

  return (
    <div
      className="mx-auto flex max-w-3xl items-center gap-2 px-1 text-[11px] text-dim"
      role="status"
      aria-live="polite"
    >
      <span
        aria-hidden
        className="h-1.5 w-1.5 shrink-0 animate-pulse rounded-full bg-accent"
      />

      {etapa && (
        <span className="min-w-0 max-w-[45%] truncate text-ink" title={etapa.titulo}>
          {t("agent.live.task", {
            n: etapa.n,
            total: etapa.total,
            title: etapa.titulo,
          })}
        </span>
      )}

      <span className="shrink-0">{estado.texto}</span>

      {estado.alvo && (
        <span className="min-w-0 truncate font-mono text-[10.5px]" title={estado.alvo}>
          {estado.alvo}
        </span>
      )}

      <span className="ml-auto shrink-0 tabular-nums">
        {passo?.kind === "step" ? t("agent.live.step", { n: passo.index }) : ""}
        {segundos > 0 ? ` · ${segundos}s` : ""}
      </span>
    </div>
  );
}
