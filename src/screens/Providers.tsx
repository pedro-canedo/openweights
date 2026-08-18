// Fontes de LLM: onde as conversas podem ser atendidas.
//
// Tela própria, e não um card em Configurações, porque o painel do 9router é
// uma aplicação inteira embutida — precisa da altura toda, que a coluna
// estreita de Configurações não dá.

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import NineRouterCard from "../components/providers/NineRouterCard";
import GatewayCard from "../components/providers/GatewayCard";
import NineRouterPanel from "../components/providers/NineRouterPanel";
import OpenRouterCard from "../components/providers/OpenRouterCard";
import {
  providersList,
  type NineRouterStatus,
  type ProviderView,
} from "../lib/providers";

type Aba = "openrouter" | "9router" | "gateway";

export default function Providers() {
  const { t } = useTranslation();
  const [aba, setAba] = useState<Aba>("openrouter");
  const [estado, setEstado] = useState<ProviderView[] | null>(null);
  const [nove, setNove] = useState<NineRouterStatus | null>(null);

  const recarregar = useCallback(() => {
    void providersList()
      .then(setEstado)
      .catch(() => setEstado([]));
  }, []);

  useEffect(recarregar, [recarregar]);

  const aoMudarNove = useCallback(
    (s: NineRouterStatus) => {
      setNove(s);
      // Ligar ou desligar o 9router muda o que a lista de situação mostra.
      recarregar();
    },
    [recarregar],
  );

  return (
    <div className="mx-auto flex h-full max-w-5xl flex-col px-8 py-8">
      <h1 className="text-lg">{t("providers.title")}</h1>
      <p className="mt-1 text-[12px] text-dim">{t("providers.subtitle")}</p>

      <div className="mt-4 rounded-xl border border-edge bg-panel px-5 py-4">
        <div className="text-sm">{t("providers.statusTitle")}</div>
        {estado == null ? (
          <div className="mt-3 h-5 w-48 animate-pulse rounded bg-panel2" />
        ) : (
          <ul className="mt-3 space-y-1.5">
            {estado.map((p) => (
              <li key={p.id} className="flex items-center gap-2 text-[12px]">
                <span
                  aria-hidden
                  className={`h-2 w-2 shrink-0 rounded-full ${
                    p.ready ? "bg-ok" : "bg-dim"
                  }`}
                />
                <span>{t(`providers.name.${p.id}`)}</span>
                <span className="truncate text-dim">
                  {p.ready ? (p.baseUrl ?? "") : (p.reason ?? "")}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>

      <div className="mt-4 flex gap-1 border-b border-edge">
        {(["openrouter", "9router", "gateway"] as const).map((x) => (
          <button
            key={x}
            onClick={() => setAba(x)}
            className={`-mb-px border-b-2 px-3 py-2 text-sm ${
              aba === x
                ? "border-accent text-ink"
                : "border-transparent text-dim hover:text-ink"
            }`}
          >
            {x === "gateway" ? t("providers.gateway.tab") : t(`providers.name.${x}`)}
          </button>
        ))}
      </div>

      <div className="mt-4 flex min-h-0 flex-1 flex-col gap-4">
        {aba === "openrouter" && <OpenRouterCard />}
        {aba === "gateway" && <GatewayCard />}
        {aba === "9router" && (
          <>
            <NineRouterCard onChanged={aoMudarNove} />
            {nove?.running && nove.dashboardUrl && (
              <NineRouterPanel url={nove.dashboardUrl} />
            )}
          </>
        )}
      </div>
    </div>
  );
}
