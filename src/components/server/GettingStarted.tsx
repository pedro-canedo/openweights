// "Como usar" — três passos, e só enquanto fazem falta.
//
// A tela do servidor responde bem a quem já sabe o que é uma API compatível
// com OpenAI. Quem não sabe olha um endereço, uma chave e um botão e não tem
// como adivinhar que a sequência é: ligar, copiar, colar no outro programa.
// São três frases; escondê-las atrás de uma documentação seria pedir que a
// pessoa saia do app para descobrir o que fazer dentro dele.
//
// O bloco desaparece sozinho assim que o servidor atende a primeira
// requisição de qualquer cliente — a partir daí ele é ruído. Quem dispensar
// antes disso dispensa de vez: a escolha fica no `localStorage`.

import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { CopyButton } from "../ui/Copy";

/** Onde mora a dispensa do guia. */
export const CHAVE_GUIA = "ow.server.guide.hidden";

function Passo({
  n,
  titulo,
  detalhe,
  feito,
  acao,
}: {
  n: number;
  titulo: string;
  detalhe?: string;
  feito: boolean;
  acao?: ReactNode;
}) {
  return (
    <li className="flex items-start gap-3">
      <span
        className={`mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-[11px] font-medium ${
          feito
            ? "bg-ok/15 text-ok"
            : "border border-edge text-dim"
        }`}
      >
        {feito ? "✓" : n}
      </span>
      <span className="min-w-0 flex-1">
        <span className={`text-sm ${feito ? "text-dim" : "text-ink"}`}>
          {titulo}
        </span>
        {detalhe && (
          <span className="mt-0.5 block text-[11px] leading-relaxed text-dim">
            {detalhe}
          </span>
        )}
      </span>
      {acao && <span className="shrink-0">{acao}</span>}
    </li>
  );
}

export default function GettingStarted({
  running,
  baseUrl,
  served,
  onDismiss,
}: {
  running: boolean;
  baseUrl: string | null;
  /** O servidor já atendeu alguma requisição — de qualquer cliente. */
  served: boolean;
  onDismiss: () => void;
}) {
  const { t } = useTranslation();

  return (
    <section className="mt-4 rounded-xl border border-accent/30 bg-panel p-5">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h2 className="text-sm font-medium">{t("server.guide.title")}</h2>
          <p className="mt-0.5 text-[12px] leading-relaxed text-dim">
            {t("server.guide.subtitle")}
          </p>
        </div>
        <button
          type="button"
          onClick={onDismiss}
          className="shrink-0 text-[11px] text-dim hover:text-ink"
        >
          {t("server.guide.dismiss")}
        </button>
      </div>

      <ol className="mt-4 flex flex-col gap-3">
        <Passo
          n={1}
          feito={running}
          titulo={t("server.guide.step1")}
          detalhe={running ? undefined : t("server.guide.step1Hint")}
        />
        <Passo
          n={2}
          feito={running && !!baseUrl}
          titulo={t("server.guide.step2")}
          detalhe={baseUrl ?? undefined}
          acao={
            baseUrl ? (
              <CopyButton value={baseUrl} label={t("server.copy")} />
            ) : undefined
          }
        />
        <Passo
          n={3}
          feito={served}
          titulo={t("server.guide.step3")}
          detalhe={t("server.guide.step3Hint")}
        />
      </ol>
    </section>
  );
}
