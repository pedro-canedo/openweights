// Cartão de perguntas do agente: a execução PAROU esperando estas respostas
// (a pausa é durável — sobrevive a reload e reinício). As opções viram
// botões que preenchem o compositor; a pessoa confirma o envio, e a resposta
// retoma o plano de onde parou.

import { useTranslation } from "react-i18next";
import type { PendingQuestion } from "../../lib/agent/scout";

export default function QuestionCard({
  question,
  onPick,
}: {
  question: PendingQuestion;
  /** Leva a resposta escolhida para o compositor. */
  onPick: (texto: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="rounded-xl border border-accent/40 bg-panel px-4 py-3">
      <p className="mb-2 text-[11px] font-medium tracking-wide text-accent uppercase">
        {t("agent.question.title")}
      </p>
      <ul className="flex flex-col gap-2.5">
        {question.items.map((q, i) => (
          <li key={i} className="text-[13px] leading-relaxed text-ink">
            <span className="select-text">
              {question.items.length > 1 ? `${i + 1}. ` : ""}
              {q.text}
            </span>
            {q.options.length > 0 && (
              <span className="mt-1.5 flex flex-wrap gap-1.5">
                {q.options.map((op) => (
                  <button
                    key={op}
                    type="button"
                    onClick={() => onPick(op)}
                    className="rounded-full border border-edge bg-panel2 px-2.5 py-1 text-[11px] text-dim transition-colors hover:border-accent hover:text-ink"
                  >
                    {op}
                  </button>
                ))}
              </span>
            )}
          </li>
        ))}
      </ul>
      <p className="mt-2 text-[11px] text-dim">{t("agent.question.hint")}</p>
    </div>
  );
}
