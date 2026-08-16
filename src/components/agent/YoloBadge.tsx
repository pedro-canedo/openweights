// Pílula âmbar do modo automático: lembra que o agente está executando
// sozinho dentro da pasta e dá o caminho de volta — o checkpoint criado
// antes da primeira alteração.

import { useTranslation } from "react-i18next";
import type { CheckpointMark } from "../../lib/agent/runStore";

function folderName(dir: string): string {
  const parts = dir.replace(/[\\/]+$/, "").split(/[\\/]/);
  return parts[parts.length - 1] || dir;
}

export default function YoloBadge({
  workspaceDir,
  checkpoint,
  onOpenCheckpoint,
}: {
  workspaceDir: string | null;
  /** Primeiro checkpoint do run (o "desfazer" mais útil). */
  checkpoint?: CheckpointMark | null;
  onOpenCheckpoint?: (checkpointId: string) => void;
}) {
  const { t } = useTranslation();
  const dir = workspaceDir ? folderName(workspaceDir) : "—";

  return (
    <span className="flex items-center gap-1.5 rounded-full border border-warn/40 bg-warn/10 px-2 py-1 text-[11px] text-warn">
      <span className="font-semibold tracking-wide">{t("agent.yolo.badge")}</span>
      <span className="min-w-0 truncate" title={workspaceDir ?? undefined}>
        {t("agent.yolo.banner", { dir })}
      </span>
      {checkpoint && onOpenCheckpoint && (
        <button
          type="button"
          onClick={() => onOpenCheckpoint(checkpoint.id)}
          title={checkpoint.label}
          className="shrink-0 underline underline-offset-2 transition-colors hover:text-ink"
        >
          {t("agent.checkpoint.restore")}
        </button>
      )}
    </span>
  );
}
