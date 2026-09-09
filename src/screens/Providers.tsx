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
import Icon from "../components/ui/Icon";
import { navigate } from "../lib/nav";
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
      icon="network"
      title={t("providers.title")}
      subtitle={t("providers.subtitle")}
      wide
      actions={estado != null && estado.length > 0 && <span className="workspace-badge"><StatusDot tone={ativas ? "ok" : "off"} />{t("providers.readyCount", { n: ativas, total: estado.length })}</span>}
    >
      {/* A situação, em cartões: cada um diz o que está no ar e leva para
          onde se muda isso. */}
      <div className="sources-grid mt-5">
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
                selected={ABA_DA_FONTE[p.id] === aba}
                onOpen={() => {
                  const destino = ABA_DA_FONTE[p.id];
                  if (destino) setAba(destino);
                  else navigate("server");
                }}
              />
            ))}
      </div>

      <Tabs tabs={abas} value={aba} onChange={setAba} />

      <div className="provider-details mt-4 flex flex-col gap-4">
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
  selected,
}: {
  fonte: ProviderView;
  onOpen: () => void;
  selected: boolean;
}) {
  const { t } = useTranslation();
  const nome = t(`providers.name.${fonte.id as ProviderId}`);

  const conteudo = (
    <>
      <div className="mb-5 flex items-center justify-between"><span className="source-icon"><Icon name={fonte.id === "local" ? "cpu" : fonte.id === "9router" ? "network" : "layers"} className="h-5 w-5" /></span><Icon name="arrow-right" className="h-4 w-4 text-dim" /></div>
      <div className="flex items-center gap-2">
        <StatusDot tone={fonte.ready ? "ok" : "off"} />
        <span className="min-w-0 text-sm font-medium">{nome}</span>
        <span
          className={`ml-auto shrink-0 text-[11px] ${
            fonte.ready ? "text-ok" : "text-dim"
          }`}
        >
          {fonte.ready ? t("providers.on") : t("providers.off")}
        </span>
      </div>
      <p
        className="mt-3 truncate text-xs text-dim"
        title={fonte.ready ? (fonte.baseUrl ?? "") : (fonte.reason ?? "")}
      >
        {fonte.ready ? (fonte.baseUrl ?? "—") : (fonte.reason ?? t(`interface.sourceHint.${fonte.id}`))}
      </p>
    </>
  );

  const classe = `source-card rounded-2xl border bg-panel p-5 text-left ${
    selected ? "border-accent/60 bg-accent/5" : "border-edge"
  }`;

  return (
    <button
      type="button"
      onClick={onOpen}
      aria-pressed={selected}
      className={`${classe} transition-colors hover:border-accent`}
    >
      {conteudo}
    </button>
  );
}
