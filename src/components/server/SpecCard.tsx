// Especulação: o que foi medido nesta máquina, e o que o app fez com isso.
//
// O card da EVIDÊNCIA — a configuração em si mora no card de cima. A ordem do
// conteúdo é a ordem das perguntas: está ligado sozinho? o que ele decidiu?
// com base em quais números? e a resposta continuou a mesma?
//
// Essa última pergunta é a razão de o card existir. Velocidade sem conferir a
// saída é armadilha conhecida: um kernel de 8 bits já rendeu números lindos
// por uma hora enquanto servia texto sem sentido. Como a especulação é
// *lossless* — o modelo grande confere cada rascunho —, a resposta com e sem
// ela tem de ser idêntica, e o que não bate é mostrado aqui, trecho a trecho,
// em vez de virar uma promessa que ninguém pode verificar.

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  onTuneSpec,
  tuneSpecCancel,
  type SpecArm,
  type SpecOutcome,
  type SpecQuality,
} from "../../lib/tuning";

const SELO: Record<SpecQuality, string> = {
  match: "text-ok",
  truncated: "text-dim",
  diverged: "text-bad",
  unverifiable: "text-warn",
};

/** O rótulo composto: "MTP + N-grama". */
function nome(spec: SpecArm["spec"], t: (k: string) => string): string {
  if (spec.length === 0) return t("tune.spec.type.none");
  return spec.map((s) => t(`tune.spec.type.${s}`)).join(" + ");
}

function Linha({
  arm,
  melhor,
  t,
}: {
  arm: SpecArm;
  melhor: boolean;
  t: (k: string) => string;
}) {
  return (
    <div
      className={`rounded-lg border px-3 py-2 ${
        melhor ? "border-ok/50 bg-ok/5" : "border-edge"
      }`}
    >
      <div className="flex flex-wrap items-center gap-2">
        <span className="min-w-0 flex-1 truncate text-[12px] text-ink">
          {nome(arm.spec, t)}
        </span>
        {/* Por tipo de texto, e não só a média: o n-grama ganha muito em
            código e perde em prosa — a média esconderia exatamente isso. */}
        {arm.byPrompt.map(([tipo, tps]) => (
          <span key={tipo} className="shrink-0 text-[11px] tabular-nums text-dim">
            {t(`tune.spec.prompt.${tipo}`)} {tps.toFixed(1)}
          </span>
        ))}
        <span className={`shrink-0 text-[11px] ${SELO[arm.quality]}`}>
          {t(`tune.spec.quality.${arm.quality}`)}
        </span>
      </div>

      {arm.divergence && (
        <details className="mt-1.5">
          <summary className="cursor-pointer text-[11px] text-bad">
            {t("tune.spec.quality.showDiff")}
          </summary>
          <div className="mt-1.5 flex flex-col gap-1 text-[10px]">
            <div className="rounded border border-edge bg-panel2 p-2">
              <span className="text-dim">{t("tune.spec.quality.expected")}: </span>
              <span className="font-mono text-ok">{arm.divergence.expected}</span>
            </div>
            <div className="rounded border border-edge bg-panel2 p-2">
              <span className="text-dim">{t("tune.spec.quality.got")}: </span>
              <span className="font-mono text-bad">{arm.divergence.got}</span>
            </div>
          </div>
        </details>
      )}
    </div>
  );
}

export default function SpecCard({ model }: { model: string }) {
  const { t } = useTranslation();
  const [aberto, setAberto] = useState(false);
  const [medindo, setMedindo] = useState(false);
  const [outcome, setOutcome] = useState<SpecOutcome | null>(null);
  const [veredito, setVeredito] = useState<string | null>(null);

  // A medição roda sozinha em segundo plano; o card é a janela para ela.
  useEffect(() => {
    const parar = onTuneSpec((p) => {
      if (p.phase === "start") {
        setMedindo(true);
        setAberto(true);
        return;
      }
      setMedindo(false);
      if (p.outcome) setOutcome(p.outcome);
      if (p.verdict) setVeredito(p.verdict);
    });
    return () => {
      void parar.then((f) => f());
    };
  }, []);

  if (!model) return null;

  const naoVerificavel = outcome?.qualityGate === "unverifiable";

  return (
    <div className="mt-4 rounded-xl border border-edge bg-panel">
      <button
        onClick={() => setAberto((v) => !v)}
        className="flex w-full items-center gap-3 px-5 py-3 text-left"
      >
        <span className="min-w-0 flex-1">
          <span className="block text-sm">{t("tune.spec.card.title")}</span>
          <span className="mt-0.5 block text-[11px] leading-relaxed text-dim">
            {medindo
              ? t("tune.spec.card.measuring")
              : veredito
                ? t(`tune.spec.verdict.${veredito}`)
                : t("tune.spec.card.never")}
          </span>
        </span>
        {medindo && (
          <span className="shrink-0 rounded-full bg-accent/15 px-2 py-0.5 text-[10px] text-accent">
            {t("tune.spec.card.running")}
          </span>
        )}
        <span className="shrink-0 text-dim">{aberto ? "▾" : "▸"}</span>
      </button>

      {aberto && (
        <div className="border-t border-edge px-5 py-4">
          <p className="text-[12px] leading-relaxed text-dim">
            {t("tune.spec.card.subtitle")}
          </p>

          {medindo && (
            <div className="mt-3 flex items-center gap-3">
              <span className="text-[12px] text-dim">
                {t("tune.spec.card.restartWarning")}
              </span>
              <button
                onClick={() => void tuneSpecCancel()}
                className="ml-auto shrink-0 rounded-lg border border-edge px-3 py-1.5 text-[12px] text-dim hover:text-ink"
              >
                {t("tune.spec.card.stop")}
              </button>
            </div>
          )}

          {naoVerificavel && (
            <p className="mt-3 rounded-lg border border-warn/40 bg-warn/10 px-3 py-2 text-[11px] leading-relaxed text-warn">
              {t("tune.spec.quality.unverifiableHint")}
            </p>
          )}

          {outcome && (
            <div className="mt-3 flex flex-col gap-1.5">
              {outcome.arms.map((a, i) => (
                <Linha
                  key={a.spec.join("+") || "none"}
                  arm={a}
                  melhor={i === outcome.best && !outcome.inconclusive}
                  t={t}
                />
              ))}
            </div>
          )}

          {outcome?.inconclusive && (
            <p className="mt-2 text-[11px] leading-relaxed text-dim">
              {t("tune.spec.inconclusive")}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
