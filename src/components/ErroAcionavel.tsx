// O cartão de erro: o que aconteceu, por quê e o botão que resolve, com o
// texto técnico recolhido (selecionável e copiável). É o molde do cartão da
// PrismML, generalizado: quem mostra o erro passa o que ele mesmo sabe
// refazer (`tentar`, `novaConversa`); para onde ir dentro do app já vem pronto.

import { useTranslation } from "react-i18next";
import { Button } from "./ui/Button";
import { CopyButton } from "./ui/Copy";
import { navigate } from "../lib/nav";
import { classificarErro, type AcaoDoErro, type ErroClassificado } from "../lib/errors";

/** Os botões que só quem mostra o erro sabe fazer. */
export type AcoesDoChamador = Partial<Record<"tentar" | "novaConversa", () => void>>;

const NAVEGACAO: Partial<Record<AcaoDoErro, () => void>> = {
  servidor: () => navigate("server", { serverTab: "overview" }),
  motor: () => navigate("server", { serverTab: "overview" }),
  fontes: () => navigate("providers"),
  descobrir: () => navigate("discover"),
  modelos: () => navigate("models"),
};

export default function ErroAcionavel({
  erro,
  acoes = {},
  className = "",
}: {
  /** O erro cru (qualquer coisa que se possa lançar) ou já classificado. */
  erro: unknown;
  acoes?: AcoesDoChamador;
  className?: string;
}) {
  const { t } = useTranslation();
  const c: ErroClassificado =
    typeof erro === "object" && erro !== null && "acoes" in erro && "titulo" in erro
      ? (erro as ErroClassificado)
      : classificarErro(erro);

  const botoes = c.acoes
    .map((id) => {
      const run = id === "tentar" || id === "novaConversa" ? acoes[id] : NAVEGACAO[id];
      return run ? { id, run } : null;
    })
    .filter((b): b is { id: AcaoDoErro; run: () => void } => b !== null);

  return (
    <div
      role="alert"
      data-tipo={c.tipo}
      className={`rounded-xl border border-bad/40 bg-bad/10 px-4 py-3 text-sm ${className}`}
    >
      <p className="font-medium text-ink">{c.titulo}</p>
      <p className="mt-1 leading-relaxed text-dim">{c.explicacao}</p>

      {botoes.length > 0 && (
        <div className="mt-3 flex flex-wrap gap-2">
          {botoes.map((b, i) => (
            <Button key={b.id} size="sm" variant={i === 0 ? "primary" : "secondary"} onClick={b.run}>
              {t(`actionableError.actions.${b.id}`)}
            </Button>
          ))}
        </div>
      )}

      {c.detalhe && (
        <details className="mt-3 text-[12px] text-dim">
          <summary className="cursor-pointer select-none">{t("actionableError.details")}</summary>
          <pre className="mt-2 max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-panel2 p-2 font-mono text-[11px] select-text">
            {c.detalhe}
          </pre>
          <CopyButton
            value={c.detalhe}
            label={t("actionableError.copy")}
            className="mt-2"
          />
        </details>
      )}
    </div>
  );
}
