// O ponto "modificada" de um ajuste e o "↺ padrão" que o desfaz: quando o que
// está valendo difere do padrão do llama.cpp, a linha diz isso e devolve ao
// padrão com um clique (o padrão nunca é gravado: voltar é apagar o ajuste).

import { useTranslation } from "react-i18next";
import { Badge } from "../ui/Button";

export default function Modificada({
  modificada,
  padrao,
  disabled,
  onReset,
}: {
  modificada: boolean;
  /** O padrão do llama.cpp, quando conhecido. */
  padrao?: string | null;
  disabled?: boolean;
  onReset: () => void;
}) {
  const { t } = useTranslation();
  if (!modificada) return null;
  return (
    <span className="ml-auto flex items-center gap-1.5">
      <Badge tone="accent">{t("flags.modified")}</Badge>
      <button
        type="button"
        disabled={disabled}
        onClick={onReset}
        title={padrao ? t("flags.resetTo", { v: padrao }) : t("flags.reset")}
        className="rounded px-1 text-[11px] text-dim transition-colors hover:text-ink disabled:opacity-40"
      >
        ↺ {t("flags.reset")}
      </button>
    </span>
  );
}
