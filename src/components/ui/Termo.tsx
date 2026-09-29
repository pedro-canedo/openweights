// O "?" de um termo do glossário (`src/lib/glossario.ts`).

import { useTranslation } from "react-i18next";
import { HelpTip } from "./HelpTip";
import type { TermoId } from "../../lib/glossario";

export default function Termo({ id, align }: { id: TermoId; align?: "left" | "right" }) {
  const { t } = useTranslation();
  return (
    <HelpTip label={t("glossary.ask", { term: t(`glossary.${id}.name`) })} align={align}>
      {t(`glossary.${id}.text`)}
    </HelpTip>
  );
}
