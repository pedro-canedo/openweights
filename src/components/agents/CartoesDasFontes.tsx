// "Escolha o cérebro": o que cada fonte do OpenWeights tem e o próximo passo
// de cada uma.
//
// O AgenticOw e o OwCLI pensam só com os modelos do app. Quando não há
// nenhum, é isto que a tela deles mostra. Nada se configura aqui: as ações
// levam às telas do app, ou sobem o Servidor Local direto quando já há
// modelo na biblioteca. Os textos de cada fonte nasceram no AgenticOw e
// moram em `agenticow.brain.*`.

import { useState } from "react";
import { useTranslation } from "react-i18next";
import { startServer } from "../../lib/api";
import type { EstadoDasFontes } from "../../lib/fontes";
import { navigate } from "../../lib/nav";
import { errorMessage } from "../../lib/serverSession";
import { Button } from "../ui/Button";
import Icon, { type IconName } from "../ui/Icon";

interface Passo {
  estado: string;
  acao: string;
  aoClicar: () => void;
  primaria?: boolean;
  ocupada?: boolean;
}

export default function CartoesDasFontes({
  fontes: f,
  aoMudar,
  aoSair,
}: {
  fontes: EstadoDasFontes | null | undefined;
  /** Depois de subir o Servidor Local: quem mostra os cartões relê o que precisa. */
  aoMudar?: () => Promise<unknown> | void;
  /** Antes de levar a pessoa a outra tela (um diálogo se fecha). */
  aoSair?: () => void;
}) {
  const { t } = useTranslation();
  const [subindo, setSubindo] = useState(false);
  const [erro, setErro] = useState<string | null>(null);

  const ir = (...args: Parameters<typeof navigate>) => {
    aoSair?.();
    navigate(...args);
  };
  const emFontes = (tab: "openrouter" | "9router") => ir("providers", { providersTab: tab });

  const subirServidor = async () => {
    setSubindo(true);
    setErro(null);
    try {
      await startServer();
      await aoMudar?.();
    } catch (e) {
      setErro(errorMessage(e));
    } finally {
      setSubindo(false);
    }
  };

  let local: Passo;
  if (!f || f.localModels === 0) {
    local = {
      estado: t("agenticow.brain.local.none"),
      acao: t("agenticow.brain.local.find"),
      aoClicar: () => ir("discover"),
    };
  } else if (!f.serverRunning) {
    local = {
      estado: t("agenticow.brain.local.stopped", { count: f.localModels }),
      acao: t("agenticow.brain.local.start"),
      aoClicar: () => void subirServidor(),
      primaria: true,
      ocupada: subindo,
    };
  } else {
    local = {
      estado: t("agenticow.brain.local.running"),
      acao: t("agenticow.brain.local.open"),
      aoClicar: () => ir("server"),
    };
  }

  const configurar = t("agenticow.brain.configure");
  const openrouter: Passo = !f?.openrouterKey
    ? { estado: t("agenticow.brain.openrouter.noKey"), acao: configurar, aoClicar: () => emFontes("openrouter") }
    : f.openrouterFavorites === 0
      ? { estado: t("agenticow.brain.openrouter.noFavorites"), acao: t("agenticow.brain.openrouter.pick"), aoClicar: () => emFontes("openrouter") }
      : { estado: t("agenticow.brain.openrouter.ready", { count: f.openrouterFavorites }), acao: configurar, aoClicar: () => emFontes("openrouter") };

  const nove: Passo = !f?.ninerouterInstalled
    ? { estado: t("agenticow.brain.ninerouter.notInstalled"), acao: t("agenticow.brain.ninerouter.install"), aoClicar: () => emFontes("9router") }
    : !f.ninerouterRunning
      ? { estado: t("agenticow.brain.ninerouter.stopped"), acao: configurar, aoClicar: () => emFontes("9router") }
      : { estado: t("agenticow.brain.ninerouter.noModels"), acao: configurar, aoClicar: () => emFontes("9router") };

  return (
    <div className="space-y-3">
      <CartaoFonte
        icone="cpu"
        nome={t("agenticow.brain.local.name")}
        descricao={t("agenticow.brain.local.hint")}
        passo={local}
      />
      <CartaoFonte
        icone="network"
        nome={t("agenticow.brain.openrouter.name")}
        descricao={t("agenticow.brain.openrouter.hint")}
        passo={openrouter}
      />
      <CartaoFonte
        icone="layers"
        nome={t("agenticow.brain.ninerouter.name")}
        descricao={t("agenticow.brain.ninerouter.hint")}
        passo={nove}
      />
      {erro && (
        <p role="alert" className="text-sm text-bad">
          {erro}
        </p>
      )}
    </div>
  );
}

function CartaoFonte({
  icone,
  nome,
  descricao,
  passo,
}: {
  icone: IconName;
  nome: string;
  descricao: string;
  passo: Passo;
}) {
  return (
    <div className="flex flex-wrap items-center gap-4 rounded-2xl border border-edge bg-panel px-5 py-4">
      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-panel2 text-accent-ink">
        <Icon name={icone} className="h-4 w-4" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium">{nome}</div>
        <div className="mt-0.5 text-xs leading-relaxed text-dim">{descricao}</div>
        <div className="mt-1.5 text-[12px] text-ink">{passo.estado}</div>
      </div>
      <Button
        variant={passo.primaria ? "primary" : "secondary"}
        size="sm"
        busy={passo.ocupada}
        onClick={passo.aoClicar}
      >
        {passo.acao}
      </Button>
    </div>
  );
}
