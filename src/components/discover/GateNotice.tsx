// O aviso de modelo com licença — e o caminho mais curto para sair dele.
//
// Antes, este bloco era uma frase fixa presa ao campo `gated` do repositório.
// Esse campo diz "aqui é preciso aceitar uma licença" e NUNCA muda: continua
// verdadeiro depois que a pessoa aceitou. O resultado é que quem já tinha
// acesso via para sempre o mesmo aviso amarelo mandando aceitar de novo, e
// quem não tinha recebia a mesma frase para três problemas diferentes — não
// há token, o token não é reconhecido, ou a licença não foi aceita por ESTA
// conta.
//
// Agora cada estado tem a sua ação, e nenhuma delas é copiar e colar nada:
// sem conta, entra com o Hugging Face; sem licença, abre a página e o app
// espera o aceite sozinho. O único passo humano que sobrou é o clique de
// aceitar a licença, e ele é humano de propósito: é um contrato.

import { useTranslation } from "react-i18next";
import type { AccessReport } from "../../lib/types";

export default function GateNotice({
  report,
  checando,
  falhou,
  aguardando,
  entrando,
  onOpenHub,
  onRecheck,
  onLogin,
}: {
  report: AccessReport | null;
  checando: boolean;
  falhou: boolean;
  /** O app está esperando o aceite acontecer na aba do navegador. */
  aguardando: boolean;
  /** O login está aberto no navegador, esperando a autorização. */
  entrando: boolean;
  onOpenHub: () => void;
  onRecheck: () => void;
  onLogin: () => void;
}) {
  const { t } = useTranslation();

  // Liberado é o caso silencioso: quem já pode baixar não precisa de aviso
  // nenhum, e era exatamente isso que o aviso antigo não sabia fazer.
  if (report?.access === "granted") return null;

  const semConta = report?.access === "noToken";

  const corpo = (() => {
    if (entrando) return t("discover.gate.waitingLogin");
    if (aguardando) return t("discover.gate.waitingAccept");
    if (falhou) return t("discover.gate.error");
    if (!report) return t("discover.gate.checking");
    switch (report.access) {
      case "noToken":
        return t("discover.gate.noToken");
      case "badToken":
        return t("discover.gate.badToken");
      case "needsLicense":
        return report.who?.name
          ? t("discover.gate.needsLicenseAs", { user: report.who.name })
          : t("discover.gate.needsLicense");
      default:
        return t("discover.gate.checking");
    }
  })();

  // O token fine-grained sem alcance sobre repositórios com licença é a
  // armadilha silenciosa: a licença está aceita, a conta está certa, e o
  // download responde 403 sem dizer por quê. Só avisamos quando o Hub
  // confirma que a permissão falta — um alarme falso aqui manda a pessoa
  // refazer um token que estava correto.
  const escopoFaltando = report?.who?.canReadGated === false;
  const ocupado = entrando || aguardando;

  return (
    <div className="mt-3 rounded-xl border border-warn/40 bg-warn/10 p-3">
      <p className="text-xs text-ink">{corpo}</p>

      {escopoFaltando && (
        <p className="mt-1.5 text-[11px] text-dim">
          {t("discover.gate.scopeWarn")}
        </p>
      )}

      <div className="mt-2 flex flex-wrap gap-2">
        {semConta ? (
          <button
            onClick={onLogin}
            disabled={ocupado}
            className={`rounded-lg px-3 py-1.5 text-xs font-medium transition-colors ${
              ocupado
                ? "cursor-default bg-panel2 text-dim"
                : "bg-accent text-white hover:opacity-90"
            }`}
          >
            {t("settings.hfLogin")}
          </button>
        ) : (
          <button
            onClick={onOpenHub}
            disabled={ocupado}
            className={`rounded-lg border border-edge bg-panel px-3 py-1.5 text-xs font-medium transition-colors ${
              ocupado ? "cursor-default text-dim" : "text-ink hover:border-accent"
            }`}
          >
            {t("discover.gate.accept")} ↗
          </button>
        )}

        {/* A verificação manual continua existindo para quem aceitou em
            outra janela, ou depois de a espera automática ter desistido. */}
        <button
          onClick={onRecheck}
          disabled={checando || ocupado}
          className={`rounded-lg border border-edge px-3 py-1.5 text-xs font-medium transition-colors ${
            checando || ocupado
              ? "cursor-default text-dim"
              : "text-ink hover:border-accent"
          }`}
        >
          {checando ? t("discover.gate.checking") : t("discover.gate.recheck")}
        </button>
      </div>
    </div>
  );
}
