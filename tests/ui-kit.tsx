// O kit de `src/components/ui` montado sozinho, sem Tauri, para o Playwright
// provar o que não dá para ver num print: para onde vai o foco, o que o Esc
// fecha, o que o leitor de tela recebe. `?tema=claro` troca o tema.
import React, { useState } from "react";
import { createRoot } from "react-dom/client";
import "../src/styles.css";
import "../src/i18n";
import { Badge, Button, IconButton, Input } from "../src/components/ui/Button";
import { ConfirmHost, Dialog, confirmar } from "../src/components/ui/Dialog";
import { Menu, Popover, usePopover } from "../src/components/ui/Popover";
import { Tooltip } from "../src/components/ui/Tooltip";
import { ToastHost, toast } from "../src/components/ui/Toast";
import { Split } from "../src/components/ui/Split";
import { HelpTip } from "../src/components/ui/HelpTip";

if (new URLSearchParams(location.search).get("tema") === "claro") {
  document.documentElement.dataset.theme = "light";
}

function Kit() {
  const [dialogo, setDialogo] = useState(false);
  const [resposta, setResposta] = useState("nenhuma");
  const pop = usePopover();
  const menu = usePopover("menu");
  const [escolha, setEscolha] = useState("nenhuma");
  return (
    <main className="flex h-full flex-col gap-6 overflow-auto bg-bg p-8 text-ink">
      <h1 className="text-lg font-semibold">Kit de componentes</h1>

      <section className="flex flex-wrap items-center gap-2">
        <Button variant="primary">Primário</Button>
        <Button>Secundário</Button>
        <Button variant="ghost">Discreto</Button>
        <Button variant="danger">Apagar</Button>
        <Button variant="primary" size="sm" icon="download" busy>
          Baixando
        </Button>
        <IconButton icon="copy" label="Copiar endereço" />
        <Badge tone="ok">Cabe na placa</Badge>
        <Badge tone="warn">Parcial</Badge>
        <Badge tone="bad">Não cabe</Badge>
        <Badge tone="accent">Recomendado</Badge>
      </section>

      <section className="grid max-w-md gap-3">
        <Input label="Nome do preset" hint="Aparece na lista de presets." />
        <Input label="Buscar modelos" hideLabel placeholder="Buscar modelos" />
        <Input label="Porta" error="A porta precisa ficar entre 1024 e 65535." defaultValue="80" />
      </section>

      <section className="flex flex-wrap items-center gap-3">
        <Button id="abrir-dialogo" onClick={() => setDialogo(true)}>
          Abrir diálogo
        </Button>
        <Button
          id="abrir-confirmacao"
          onClick={async () =>
            setResposta(
              (await confirmar({
                title: "Apagar o modelo?",
                message: "Os 14 GB saem do disco.",
                confirmLabel: "Apagar",
                tone: "danger",
              }))
                ? "sim"
                : "não",
            )
          }
        >
          Confirmar algo
        </Button>
        <span data-testid="resposta">{resposta}</span>

        <span className="relative">
          <Button id="abrir-popover" {...pop.triggerProps}>
            Abrir popover
          </Button>
          <Popover {...pop.popoverProps} label="Detalhes" className="absolute left-0 top-full mt-2 w-64 p-3">
            <p className="text-sm">Conteúdo do popover.</p>
            <Button size="sm" className="mt-2">
              Ação dentro
            </Button>
          </Popover>
        </span>

        <span className="relative">
          <Button id="abrir-menu" {...menu.triggerProps}>
            Mais ações
          </Button>
          <Menu
            {...menu.popoverProps}
            label="Mais ações"
            className="absolute left-0 top-full mt-2"
            items={[
              { id: "renomear", label: "Renomear", onSelect: () => setEscolha("renomear") },
              { id: "exportar", label: "Exportar", onSelect: () => setEscolha("exportar") },
              { id: "apagar", label: "Apagar", danger: true, onSelect: () => setEscolha("apagar") },
            ]}
          />
        </span>
        <span data-testid="escolha">{escolha}</span>

        <Tooltip content="Copia o endereço do servidor.">
          <button id="com-dica" type="button" className="rounded-lg border border-edge px-3 py-1.5 text-sm">
            Com dica
          </button>
        </Tooltip>

        <span className="flex items-center gap-1 text-sm">
          Quantização
          <HelpTip label="O que é quantização?">
            Uma versão comprimida do modelo: menos bits por número, arquivo menor, alguma perda de qualidade.
          </HelpTip>
        </span>

        <Button
          id="avisar"
          onClick={() =>
            toast({
              message: "Modelo pronto.",
              tone: "ok",
              action: { label: "Conversar", run: () => setEscolha("conversar") },
            })
          }
        >
          Avisar
        </Button>
      </section>

      <section className="h-40 rounded-xl border border-edge">
        <Split
          label="Largura da lista"
          defaultSize={240}
          min={120}
          minSecond={120}
          first={<div className="h-full bg-panel p-3 text-sm">Lista</div>}
          second={<div className="h-full bg-panel2 p-3 text-sm">Detalhe</div>}
        />
      </section>

      <Dialog
        open={dialogo}
        onClose={() => setDialogo(false)}
        title="Diálogo de teste"
        description="Tab e Shift+Tab não saem daqui."
        footer={
          <>
            <Button onClick={() => setDialogo(false)}>Cancelar</Button>
            <Button variant="primary" onClick={() => setDialogo(false)}>
              Continuar
            </Button>
          </>
        }
      >
        <Input label="Campo no diálogo" className="mt-4" />
      </Dialog>

      <ToastHost />
      <ConfirmHost />
    </main>
  );
}

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Kit />
  </React.StrictMode>,
);
