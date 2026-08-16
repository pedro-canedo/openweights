// Gaveta com a trilha de uma execução, por cima da tela que a chamou.
//
// Uma execução de automação — ou uma execução antiga aberta pela tela de
// Atividade — não pertence ao chat aberto, então o painel de execução do Chat
// não a alcança. Aqui ela é reaberta a partir dos eventos gravados
// (`run_events_list`) e REATADA ao run vivo: sem isso a gaveta seria uma foto,
// e uma execução parada esperando confirmação continuaria parada.

import { useEffect, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { runStore } from "../../lib/agent/runStore";
import ApprovalBar from "./ApprovalBar";
import RunTimeline from "./RunTimeline";

export default function TraceDrawer({
  runId,
  onClose,
}: {
  runId: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const snap = useSyncExternalStore(runStore.subscribe, runStore.get);
  const live = snap.runs.find((r) => r.runId === runId) ?? null;
  const pending = live?.pendingCallId ? live.tools[live.pendingCallId] : null;

  useEffect(() => {
    void runStore.attachToRun(runId);
  }, [runId]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={t("agent.run.trace")}
      className="fixed inset-0 z-50 flex justify-end bg-black/60"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      {pending && (
        <div className="flex w-96 max-w-[90vw] flex-col justify-end overflow-y-auto border-l border-edge bg-bg px-3 py-3">
          <ApprovalBar
            call={pending}
            workspaceDir={live?.workspaceDir ?? null}
            onDecide={(decision) =>
              void runStore.approve(runId, pending.callId, decision)
            }
          />
        </div>
      )}
      <RunTimeline run={live} runId={runId} onClose={onClose} />
    </div>
  );
}
