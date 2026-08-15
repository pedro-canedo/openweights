// Badge de compatibilidade de hardware por quantização, no estilo do widget
// "Hardware compatibility" do Hugging Face: verde (GPU total), amarelo
// (parcial), cinza (só CPU) e vermelho (não cabe).

import { useTranslation } from "react-i18next";
import type { FitVerdict } from "../../lib/types";

const STYLES: Record<FitVerdict["kind"], string> = {
  fullGpu: "bg-ok/15 text-ok",
  partial: "bg-warn/15 text-warn",
  cpuOnly: "bg-dim/15 text-dim",
  wontFit: "bg-bad/15 text-bad",
};

export default function VerdictBadge({ verdict }: { verdict: FitVerdict }) {
  const { t } = useTranslation();

  const label =
    verdict.kind === "partial"
      ? t("badge.partial", { ngl: verdict.ngl, total: verdict.layersTotal })
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
