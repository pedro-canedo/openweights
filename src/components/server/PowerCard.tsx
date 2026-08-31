// Energia da placa: quanto ela pode gastar, e quanto está gastando agora.
//
// Isto não é um controle de overclock disfarçado. Gerar tokens é limitado
// pela BANDA de memória da placa, e banda não sobe com watts — então, nesta
// carga específica, cortar o limite de energia costuma custar quase nada em
// velocidade e tirar bastante calor e consumo. "Costuma" é a palavra certa:
// o número que circula foi medido noutra pilha de software, então o card
// oferece medir aqui em vez de repetir a promessa.
//
// Duas honestidades que o card não esconde:
// - aplicar exige administrador (o driver não deixa de outro jeito), e o
//   sistema vai perguntar;
// - o limite NÃO sobrevive a reiniciar o computador. Quem diz isso é a
//   documentação do próprio NVML, e um app não tem como contornar.

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { gpuPowerSet, gpuPowerStatus, type GpuPower } from "../../lib/power";
import { chipClass } from "../form/controls";

/** Alvo econômico sugerido: ~70% do padrão, arredondado a 10 W. */
function alvoEconomico(p: GpuPower): number {
  const base = p.defaultW ?? p.limitW;
  const alvo = Math.round((base * 0.7) / 10) * 10;
  return Math.max(p.minW ?? alvo, Math.min(alvo, p.maxW ?? alvo));
}

function Barra({ atual, limite }: { atual: number; limite: number }) {
  const p = Math.max(0, Math.min(100, (atual / Math.max(1, limite)) * 100));
  return (
    <div className="h-1.5 w-full overflow-hidden rounded-full bg-panel2">
      <div
        className={`h-full rounded-full transition-[width] duration-700 ${
          p > 90 ? "bg-warn" : "bg-accent"
        }`}
        style={{ width: `${p}%` }}
      />
    </div>
  );
}

export default function PowerCard() {
  const { t } = useTranslation();
  const [gpus, setGpus] = useState<GpuPower[] | null>(null);
  const [aberto, setAberto] = useState(false);
  const [aplicando, setAplicando] = useState<number | null>(null);
  const [erro, setErro] = useState<string | null>(null);

  const recarregar = useCallback(() => {
    gpuPowerStatus()
      .then(setGpus)
      .catch(() => setGpus([]));
  }, []);

  useEffect(() => recarregar(), [recarregar]);

  // Consumo é um número vivo: enquanto o card está aberto ele acompanha.
  useEffect(() => {
    if (!aberto) return;
    const id = window.setInterval(recarregar, 2000);
    return () => window.clearInterval(id);
  }, [aberto, recarregar]);

  // Sem placa NVIDIA não há o que mostrar — e isso não é erro.
  if (!gpus || gpus.length === 0) return null;

  const aplicar = async (g: GpuPower, watts: number) => {
    setAplicando(g.index);
    setErro(null);
    try {
      await gpuPowerSet(g.index, watts);
      recarregar();
    } catch (e) {
      setErro(String(e));
    } finally {
      setAplicando(null);
    }
  };

  return (
    <div className="mt-4 rounded-xl border border-edge bg-panel">
      <button
        onClick={() => setAberto((v) => !v)}
        className="flex w-full items-center gap-3 px-5 py-3 text-left"
      >
        <span className="min-w-0 flex-1">
          <span className="block text-sm">{t("power.title")}</span>
          <span className="mt-0.5 block text-[11px] text-dim">
            {gpus
              .map((g) =>
                g.usageW != null
                  ? `${g.usageW} W / ${g.limitW} W`
                  : `${t("power.limit")} ${g.limitW} W`,
              )
              .join(" · ")}
          </span>
        </span>
        <span className="shrink-0 text-dim">{aberto ? "▾" : "▸"}</span>
      </button>

      {aberto && (
        <div className="border-t border-edge px-5 py-4">
          <p className="text-[12px] leading-relaxed text-dim">
            {t("power.subtitle")}
          </p>

          {gpus.map((g) => {
            const economico = alvoEconomico(g);
            const noEconomico = g.limitW === economico;
            const noPadrao = g.defaultW != null && g.limitW === g.defaultW;
            return (
              <div
                key={g.index}
                className="mt-4 rounded-xl border border-edge bg-panel2/40 p-4"
              >
                <div className="flex flex-wrap items-baseline gap-2">
                  <span className="text-[13px] font-medium text-ink">
                    {g.name}
                  </span>
                  {g.usageW != null && (
                    <span className="ml-auto text-[12px] tabular-nums text-dim">
                      {g.usageW} W / {g.limitW} W
                    </span>
                  )}
                </div>

                {g.usageW != null && (
                  <div className="mt-2">
                    <Barra atual={g.usageW} limite={g.limitW} />
                  </div>
                )}

                <div className="mt-3 flex flex-wrap items-center gap-2">
                  {/* Ligar e desligar o limite é alternar entre estes dois
                      valores — ambos vindos do driver, nenhum inventado. */}
                  <button
                    type="button"
                    disabled={aplicando != null || g.defaultW == null}
                    onClick={() => g.defaultW && void aplicar(g, g.defaultW)}
                    className={chipClass(noPadrao)}
                  >
                    {t("power.standard", { w: g.defaultW ?? g.maxW ?? g.limitW })}
                  </button>
                  <button
                    type="button"
                    disabled={aplicando != null}
                    onClick={() => void aplicar(g, economico)}
                    className={chipClass(noEconomico)}
                  >
                    {t("power.eco", { w: economico })}
                  </button>
                  {aplicando === g.index && (
                    <span className="text-[11px] text-dim">
                      {t("power.applying")}
                    </span>
                  )}
                </div>

                {g.minW != null && g.maxW != null && (
                  <p className="mt-2 text-[11px] tabular-nums text-dim">
                    {t("power.range", { min: g.minW, max: g.maxW })}
                  </p>
                )}
              </div>
            );
          })}

          <p className="mt-3 text-[11px] leading-relaxed text-warn/90">
            {t("power.notPersistent")}
          </p>
          <p className="mt-1 text-[11px] leading-relaxed text-dim">
            {t("power.needsAdmin")}
          </p>

          {erro && (
            <p className="mt-2 rounded-lg border border-bad/40 bg-bad/10 px-3 py-2 text-[12px] text-bad">
              {erro}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
