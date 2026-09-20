// A faixa de estado do servidor — a única coisa desta tela que não rola.
//
// Antes, "ligado ou desligado?" era um card entre onze, e a resposta saía do
// campo de visão no primeiro rolar. Só que TODAS as outras perguntas da tela
// dependem dela: um benchmark não roda com o motor parado, um endereço não
// serve para nada se ninguém está atendendo nele. Então o estado gruda no
// topo, e leva junto as três coisas que se procura quando ele está no ar: o
// endereço para colar em outro programa, o modelo que está carregado e a
// velocidade que ele está entregando.

import { useTranslation } from "react-i18next";
import type { ServerStatus } from "../../lib/types";
import { StatusDot } from "../ui/Shell";
import { CopyValue } from "../ui/Copy";

export default function ServerHeader({
  status,
  busy,
  onToggle,
  model,
  genTps,
  error,
}: {
  status: ServerStatus | null;
  busy: boolean;
  onToggle: () => void;
  /** O modelo carregado no Router, quando há um. */
  model: string | null;
  /** Geração média do recorte de sessão — `null` enquanto não serviu nada. */
  genTps: number | null;
  error: string | null;
}) {
  const { t } = useTranslation();
  const running = !!status?.running;

  return (
    <div className="sticky top-0 z-10 -mx-8 mb-2 bg-bg/95 px-8 pb-2 pt-4 backdrop-blur">
      <div
        className={`rounded-xl border bg-panel p-4 ${
          running ? "border-ok/30" : "border-edge"
        }`}
      >
        <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
          <StatusDot tone={busy ? "busy" : running ? "ok" : "off"} pulse={busy} />
          <span className="text-sm font-medium">
            {running ? t("server.running") : t("server.stopped")}
          </span>

          {running && status?.baseUrl && (
            <CopyValue value={status.baseUrl} title={t("server.connect.baseUrl")} />
          )}

          <button
            onClick={onToggle}
            disabled={busy || !status}
            className={`ml-auto rounded-lg px-4 py-2 text-sm font-medium transition-opacity disabled:opacity-50 ${
              running
                ? "border border-edge text-ink hover:border-bad hover:text-bad"
                : "bg-accent text-white"
            }`}
          >
            {busy
              ? t("common.loading")
              : running
                ? t("server.stop")
                : t("server.start")}
          </button>
        </div>

        {/* A linha do "e agora, o que está acontecendo" — só com o motor de
            pé, porque parado nenhum destes números quer dizer nada. */}
        {running && (
          <div className="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-[12px] text-dim">
            <span className="truncate" title={model ?? undefined}>
              {model ?? t("server.header.noModel")}
            </span>
            {genTps != null && (
              <>
                <span aria-hidden>·</span>
                <span className="tabular-nums">
                  {t("server.header.tps", { n: genTps.toFixed(1) })}
                </span>
              </>
            )}
            {status?.lan && (
              <>
                <span aria-hidden>·</span>
                <span>{t("server.header.lanOn")}</span>
              </>
            )}
            {/* Só quando NÃO é o oficial: é a resposta a "por que o /v1/models
                diz outra build?" — o modelo selecionado pediu outro motor. */}
            {status?.engine === "prism" && (
              <>
                <span aria-hidden>·</span>
                <span className="text-accent">{t("server.header.enginePrism")}</span>
              </>
            )}
            {status?.engine === "moeCache" && (
              <>
                <span aria-hidden>·</span>
                <span className="text-accent">{t("server.header.engineMoe")}</span>
              </>
            )}
            {status?.keyStale && (
              <>
                <span aria-hidden>·</span>
                <span className="text-warn">{t("server.header.keyStale")}</span>
              </>
            )}
          </div>
        )}

        {error && <p className="mt-2 text-[12px] text-bad">{error}</p>}
      </div>
    </div>
  );
}
