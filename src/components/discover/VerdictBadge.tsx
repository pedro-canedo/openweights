// Badge de compatibilidade de hardware por quantização, no estilo do widget
// "Hardware compatibility" do Hugging Face: verde (GPU total), azul
// (especialistas na RAM), amarelo (parcial), cinza (só CPU) e vermelho (não
// cabe).
//
// O azul tem cor própria porque a divisão é outra: num MoE, mandar os
// especialistas roteados para a RAM deixa a atenção inteira na placa, e
// pintar isso de amarelo diria "roda muito mais devagar" sobre a
// configuração que faz o arquivo caber.

import { useTranslation } from "react-i18next";
import type { FitVerdict } from "../../lib/types";

const STYLES: Record<FitVerdict["kind"], string> = {
  fullGpu: "bg-ok/15 text-ok",
  moeOffload: "bg-accent/15 text-accent",
  partial: "bg-warn/15 text-warn",
  cpuOnly: "bg-dim/15 text-dim",
  wontFit: "bg-bad/15 text-bad",
};

export default function VerdictBadge({ verdict }: { verdict: FitVerdict }) {
  const { t } = useTranslation();

  const label =
    verdict.kind === "partial"
      ? t("badge.partial", { ngl: verdict.ngl, total: verdict.layersTotal })
      : verdict.kind === "moeOffload"
        ? t("badge.moeOffload", {
            ncmoe: verdict.ncmoe,
            total: verdict.layersTotal,
          })
        : t(`badge.${verdict.kind}`);

  return (
    <span
      className={`inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[11px] font-medium ${STYLES[verdict.kind]}`}
    >
      <span className="h-1.5 w-1.5 rounded-full bg-current" />
      {label}
    </span>
  );
}
