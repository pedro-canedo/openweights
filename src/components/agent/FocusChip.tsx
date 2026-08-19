// Chip do plano corrente (Focus Chain): mostra a tarefa em aberto e o
// progresso; clicar abre a lista inteira. A fonte preferida é o plano
// ESTRUTURADO do run (a mesma verdade do quadro de tarefas); o markdown do
// evento `focus.updated` é só o fallback legado.

import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { isTaskFinished, type TaskPlan } from "../../lib/agent/scout";

export interface FocusTask {
  done: boolean;
  text: string;
}

/**
 * Extrai as tarefas do markdown. Aceita as duas formas que o app produz:
 * checkbox (`- [ ] item` / `- [x] item`, da ferramenta todo_update) e lista
 * numerada do plano (`1. [ ] Título`, com marcadores `>`/`!`/`e`/`-` para
 * rodando/bloqueada/falhou/pulada) — a regex antiga só entendia a primeira,
 * e o chip dizia "Sem plano definido" com o plano cheio na tela.
 */
export function parseFocus(todoMd: string): FocusTask[] {
  const out: FocusTask[] = [];
  for (const line of todoMd.split("\n")) {
    const m = /^\s*(?:\d+[.)]|[-*])\s*\[(.)\]\s*(.+?)\s*$/.exec(line);
    if (m) out.push({ done: m[1].toLowerCase() === "x", text: m[2] });
  }
  return out;
}

export default function FocusChip({
  todoMd,
  plan = null,
}: {
  todoMd: string;
  /** Plano estruturado do run — quando existe, é a fonte da verdade. */
  plan?: TaskPlan | null;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const tasks = useMemo<FocusTask[]>(() => {
    if (plan && plan.tasks.length > 0) {
      return plan.tasks.map((task) => ({
        done: isTaskFinished(task.status),
        text: task.title,
      }));
    }
    return parseFocus(todoMd);
  }, [plan, todoMd]);

  const done = tasks.filter((task) => task.done).length;
  const current = tasks.find((task) => !task.done);
  const label = current?.text ?? (tasks.length > 0 ? t("agent.run.finished") : t("agent.focus.empty"));

  return (
    <span className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        title={t("agent.focus.title")}
        className="flex max-w-72 items-center gap-1.5 rounded-full border border-edge bg-panel px-2 py-1 text-[11px] text-dim transition-colors hover:text-ink"
      >
        <svg
          className="h-3.5 w-3.5 shrink-0 text-accent"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          strokeLinecap="round"
          strokeLinejoin="round"
          viewBox="0 0 24 24"
        >
          <path d="M9 6h11M9 12h11M9 18h11M4 6l1 1 2-2M4 12l1 1 2-2M4 18l1 1 2-2" />
        </svg>
        <span className="min-w-0 truncate">{label}</span>
        {tasks.length > 0 && (
          <span className="shrink-0 tabular-nums text-dim">
            {done}/{tasks.length}
          </span>
        )}
      </button>

      {open && (
        <div className="absolute bottom-full left-0 z-30 mb-2 w-80 rounded-xl border border-edge bg-panel p-3 shadow-[0_12px_40px_rgba(0,0,0,0.45)]">
          <p className="mb-2 text-[11px] tracking-wide text-dim uppercase">
            {t("agent.focus.title")}
          </p>
          {tasks.length === 0 ? (
            <p className="text-[12px] text-dim">{t("agent.focus.empty")}</p>
          ) : (
            <ul className="flex flex-col gap-1.5">
              {tasks.map((task, i) => (
                <li
                  key={i}
                  className={`flex items-start gap-2 text-[12px] leading-relaxed select-text ${
                    task.done ? "text-dim line-through" : "text-ink"
                  }`}
                >
                  <span
                    className={`mt-1 h-2 w-2 shrink-0 rounded-full ${
                      task.done ? "bg-ok" : "bg-edge"
                    }`}
                  />
                  <span className="min-w-0">{task.text}</span>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </span>
  );
}
