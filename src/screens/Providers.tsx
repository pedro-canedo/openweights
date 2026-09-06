// Fontes de LLM: onde as conversas podem ser atendidas.
//
// Tela própria, e não um card em Configurações: são três provedores com
// ciclo de vida próprio (instalar, subir, parar), catálogo de centenas de
// modelos e credenciais — conteúdo demais para caber como um item numa lista
// de preferências.
//
// A tela tinha o problema clássico de uma lista de estado: três bolinhas
// coloridas em cima, três abas embaixo, e nenhuma ligação entre elas. Quem
// lia "o provedor OpenRouter está desligado" precisava descobrir sozinho que
// a providência estava na primeira aba. Agora cada fonte é um cartão, o
// cartão diz o que está acontecendo e clicar nele leva para onde se resolve.

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import NineRouterCard from "../components/providers/NineRouterCard";
import GatewayCard from "../components/providers/GatewayCard";
import NineRouterPanel from "../components/providers/NineRouterPanel";
import OpenRouterCard from "../components/providers/OpenRouterCard";
import { Page, StatusDot, Tabs, useTab, type TabDef } from "../components/ui/Shell";
import {
  providersList,
  type NineRouterStatus,
  type ProviderId,
  type ProviderView,
} from "../lib/providers";

/** A aba onde cada fonte se resolve. `local` mora em outra tela. */
const ABA_DA_FONTE: Record<string, string | null> = {
  openrouter: "openrouter",
  "9router": "9router",
  local: null,
};

export default function Providers() {
  const { t } = useTranslation();
  const [aba, setAba] = useTab("ow.providers.tab", "openrouter");
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

  const abas: TabDef[] = [
    { id: "openrouter", label: t("providers.name.openrouter") },
    { id: "9router", label: t("providers.name.9router") },
    { id: "gateway", label: t("providers.gateway.tab") },
  ];

  const ativas = estado?.filter((p) => p.ready).length ?? 0;

  return (
    <Page
      title={t("providers.title")}
      subtitle={t("providers.subtitle")}
      wide
    >
      {/* A situação, em cartões: cada um diz o que está no ar e leva para
          onde se muda isso. */}
      <div className="mt-5 grid gap-3 sm:grid-cols-3">
        {estado == null
          ? [0, 1, 2].map((i) => (
              <div
                key={i}
                className="h-[74px] animate-pulse rounded-xl border border-edge bg-panel"
              />
            ))
          : estado.map((p) => (
              <FonteCard
                key={p.id}
                fonte={p}
                onOpen={() => {
                  const destino = ABA_DA_FONTE[p.id];
                  if (destino) setAba(destino);
                }}
              />
            ))}
      </div>
      {estado != null && (
        <p className="mt-2 text-[11px] text-dim">
          {t("providers.readyCount", { n: ativas, total: estado.length })}
        </p>
      )}

      <Tabs tabs={abas} value={aba} onChange={setAba} />

      <div className="mt-4 flex flex-col gap-4">
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
    </Page>
  );
}

/**
 * Uma fonte: nome, estado e o endereço (ou o motivo de não haver um).
 *
 * O cartão inteiro é o alvo do clique quando há aba para onde ir — o motivo
 * de estar desligado e a providência ficam a um clique um do outro.
 */
function FonteCard({
  fonte,
  onOpen,
}: {
  fonte: ProviderView;
  onOpen: () => void;
}) {
  const { t } = useTranslation();
  const clicavel = ABA_DA_FONTE[fonte.id] != null;
  const nome = t(`providers.name.${fonte.id as ProviderId}`);

  const conteudo = (
    <>
      <div className="flex items-center gap-2">
        <StatusDot tone={fonte.ready ? "ok" : "off"} />
        <span className="truncate text-sm">{nome}</span>
        <span
          className={`ml-auto shrink-0 text-[11px] ${
            fonte.ready ? "text-ok" : "text-dim"
          }`}
        >
          {fonte.ready ? t("providers.on") : t("providers.off")}
        </span>
      </div>
      <p
        className="mt-1.5 truncate text-[11px] text-dim"
        title={fonte.ready ? (fonte.baseUrl ?? "") : (fonte.reason ?? "")}
      >
        {fonte.ready ? (fonte.baseUrl ?? "—") : (fonte.reason ?? "—")}
      </p>
    </>
  );

  const classe = `rounded-xl border bg-panel px-4 py-3 text-left ${
    fonte.ready ? "border-ok/25" : "border-edge"
  }`;

  return clicavel ? (
    <button
      type="button"
      onClick={onOpen}
      className={`${classe} transition-colors hover:border-accent`}
    >
      {conteudo}
    </button>
  ) : (
    <div className={classe}>{conteudo}</div>
  );
}
