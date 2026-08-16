// Checkpoints do projeto: o "desfazer" das alterações feitas pelo agente.
// Plugado no explorador de pasta (WorkspacePanel), que é onde o usuário
// pensa em arquivos.

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { checkpointRestore, checkpointsList } from "../../lib/agent/agentApi";
import { errorMessage } from "../../lib/serverSession";
import type { CheckpointRow } from "../../lib/agent/types";

/** O backend grava segundos ou milissegundos conforme a origem da linha. */
function toDate(value: number): Date {
  return new Date(value < 1e12 ? value * 1000 : value);
}

function timeLabel(value: number): string {
  const d = toDate(value);
  const today = new Date();
  const sameDay = d.toDateString() === today.toDateString();
  return sameDay
    ? d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })
    : d.toLocaleString(undefined, {
        day: "2-digit",
        month: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
      });
}

export default function CheckpointList({
  workspaceDir,
  reloadKey = 0,
}: {
  workspaceDir: string | null;
  /** Muda para forçar recarga (ex.: checkpoint criado no run corrente). */
  reloadKey?: number;
}) {
  const { t } = useTranslation();
  const [rows, setRows] = useState<CheckpointRow[]>([]);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [restored, setRestored] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    if (!workspaceDir) {
      setRows([]);
      return;
    }
    void checkpointsList(workspaceDir)
      .then(setRows)
      .catch(() => setRows([]));
  }, [workspaceDir]);

  useEffect(() => {
    refresh();
  }, [refresh, reloadKey]);

  const restore = async (row: CheckpointRow) => {
    if (!window.confirm(t("agent.checkpoint.confirmRestore"))) return;
    setBusyId(row.id);
    setError(null);
    try {
      const files = await checkpointRestore(row.id);
      setRestored(files.length);
      window.setTimeout(() => setRestored(null), 3000);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusyId(null);
      refresh();
    }
  };

  if (!workspaceDir) return null;

  return (
    <div className="border-t border-edge">
      <div className="flex items-center gap-2 px-3 py-2">
        <span className="flex-1 text-[11px] font-medium tracking-wide text-dim uppercase">
          {t("agent.checkpoint.list")}
        </span>
        {rows.length > 0 && (
          <span className="rounded-full bg-panel2 px-1.5 py-0.5 text-[10px] tabular-nums text-dim">
            {rows.length}
          </span>
        )}
      </div>

      {restored != null && (
        <p className="px-3 pb-2 text-[11px] text-ok">
          {t("agent.checkpoint.restored")} ({restored})
        </p>
      )}
      {error && <p className="px-3 pb-2 text-[11px] text-bad">{error}</p>}

      {rows.length === 0 ? (
        <p className="px-3 pb-3 text-[11px] text-dim">
          {t("agent.checkpoint.empty")}
        </p>
      ) : (
        <ul className="max-h-48 overflow-y-auto pb-1">
          {rows.map((row) => (
            <li
              key={row.id}
              className="flex items-center gap-2 px-3 py-1.5 hover:bg-panel2"
            >
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[12px] text-ink">
                  {row.label || t("agent.checkpoint.created")}
                </span>
                <span className="block truncate text-[10px] text-dim">
                  {timeLabel(row.createdAt)} ·{" "}
                  {t(
                    row.backend.toLowerCase().includes("git")
                      ? "agent.checkpoint.backendGit"
                      : "agent.checkpoint.backendCopy",
                  )}
                </span>
              </span>
              <button
                type="button"
                disabled={busyId != null}
                onClick={() => void restore(row)}
                className="shrink-0 rounded-md border border-edge px-2 py-1 text-[10px] text-dim transition-colors hover:border-accent hover:text-ink disabled:opacity-40"
              >
                {t("agent.checkpoint.restore")}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
