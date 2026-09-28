// O modelo na barra de status, e o que o Servidor Local tem para servir.
//
// Clicar no nome abre os modelos do Router com o estado de cada um: carregar
// e descarregar ficam a um clique, sem sair da tela em que se está.

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ServerLive } from "../lib/api";
import { routerLoadModel, routerModels, routerUnloadModel, type RouterModelView } from "../lib/flags";
import { navigate } from "../lib/nav";
import { errorMessage } from "../lib/serverSession";
import { Button } from "./ui/Button";
import { Popover, usePopover } from "./ui/Popover";
import { toast } from "./ui/Toast";

const semGguf = (id: string) => id.replace(/\.gguf$/i, "");

export default function ModelosDoServidor({ live }: { live: ServerLive | null }) {
  const { t } = useTranslation();
  const pop = usePopover("dialog");
  const [modelos, setModelos] = useState<RouterModelView[] | null>(null);
  const [pedidos, setPedidos] = useState<Set<string>>(new Set());

  const reler = () =>
    routerModels().then(setModelos, () => setModelos((m) => m ?? []));

  useEffect(() => {
    if (pop.open) void reler();
  }, [pop.open]);

  // Carregar um modelo grande leva minutos: enquanto houver um subindo, a
  // lista se relê sozinha.
  const subindo = pedidos.size > 0 || !!modelos?.some((m) => m.state === "loading");
  useEffect(() => {
    if (!pop.open || !subindo) return;
    const id = window.setInterval(() => void reler(), 1500);
    return () => window.clearInterval(id);
  }, [pop.open, subindo]);

  if (!live?.running) return null;

  async function alternar(m: RouterModelView) {
    setPedidos((p) => new Set(p).add(m.id));
    try {
      if (m.state === "loaded") await routerUnloadModel(m.id);
      else await routerLoadModel(m.id);
    } catch (e) {
      toast({ message: errorMessage(e), tone: "bad" });
    } finally {
      setPedidos((p) => {
        const n = new Set(p);
        n.delete(m.id);
        return n;
      });
      void reler();
    }
  }

  const rotulo = live.model ?? t("status.models.none");

  return (
    <span className="relative">
      <button
        type="button"
        {...pop.triggerProps}
        title={live.model ?? undefined}
        aria-label={t("status.models.open", { model: rotulo })}
        className="flex max-w-56 items-center gap-1.5 rounded px-1 text-[11px] text-dim transition-colors hover:text-ink"
      >
        <span
          className={`h-1.5 w-1.5 shrink-0 rounded-full ${
            live.generating ? "animate-pulse bg-accent" : live.model ? "bg-ok" : "bg-edge-strong"
          }`}
        />
        <span className="truncate">{rotulo}</span>
      </button>
      <Popover
        {...pop.popoverProps}
        label={t("status.models.title")}
        className="absolute right-0 bottom-full mb-2 w-80 p-2"
      >
        <div className="px-2 pt-1 pb-2 text-[11px] font-semibold tracking-wide text-dim uppercase">
          {t("status.models.title")}
        </div>
        {modelos == null ? (
          <p role="status" className="px-2 py-3 text-[12px] text-dim">
            {t("common.loading")}
          </p>
        ) : modelos.length === 0 ? (
          <p className="px-2 py-3 text-[12px] text-dim">{t("status.models.empty")}</p>
        ) : (
          <ul className="max-h-72 overflow-y-auto">
            {modelos.map((m) => {
              const carregando = m.state === "loading" || pedidos.has(m.id);
              const carregado = m.state === "loaded";
              return (
                <li key={m.id} className="flex items-center gap-2 rounded-lg px-2 py-1.5 hover:bg-panel2">
                  <span
                    aria-hidden
                    className={`h-1.5 w-1.5 shrink-0 rounded-full ${
                      carregando ? "animate-pulse bg-accent" : carregado ? "bg-ok" : "bg-edge-strong"
                    }`}
                  />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-[12px] text-ink" title={m.id}>
                      {semGguf(m.id)}
                    </span>
                    <span className="block text-[10.5px] text-dim">
                      {t(
                        carregando
                          ? "status.models.loading"
                          : carregado
                            ? "status.models.loaded"
                            : "status.models.unloaded",
                      )}
                    </span>
                  </span>
                  <Button
                    size="sm"
                    disabled={carregando}
                    aria-label={t(carregado ? "status.models.unloadOf" : "status.models.loadOf", {
                      model: semGguf(m.id),
                    })}
                    onClick={() => void alternar(m)}
                  >
                    {t(carregado ? "status.models.unload" : "status.models.load")}
                  </Button>
                </li>
              );
            })}
          </ul>
        )}
        <div className="mt-2 flex justify-end border-t border-edge px-1 pt-2">
          <Button
            size="sm"
            variant="ghost"
            onClick={() => {
              pop.close();
              navigate("server", {
                serverTab: "performance",
                serverModel: live.model ?? undefined,
              });
            }}
          >
            {t("status.models.configure")}
          </Button>
        </div>
      </Popover>
    </span>
  );
}
