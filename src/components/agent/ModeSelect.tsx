// Dois seletores compactos na barra do composer, complementares:
//
//  - MODO DE TRABALHO (Scout Rule): como o agente encara o pedido —
//    conversar, só planejar, executar a tarefa ou rodar o plano inteiro em
//    laço. É o que decide se o objetivo vira entregas pequenas.
//  - AUTORIZAÇÃO: o que ele pode fazer sozinho (pedir sempre, pedir só para
//    alterações, automático). Some quando o modo de trabalho não executa
//    nada — em conversa e em planejamento não há o que autorizar.
//
// YOLO é o único que muda o contrato de confiança, então exige duas coisas:
// uma pasta de projeto escolhida (é o limite do "automático") e uma
// confirmação explícita do usuário.

import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { canExecute, type WorkMode } from "../../lib/agent/scout";
import type { RunMode } from "../../lib/agent/types";
import type { ChatParams } from "../../lib/types";

type AuthMode = Exclude<RunMode, "chat">;

const AUTH_MODES: AuthMode[] = ["approve", "smart", "yolo"];
const WORK_MODES: WorkMode[] = ["chat", "plan", "agent", "loop"];

const WORK_ICONS: Record<WorkMode, string> = {
  chat: "M21 12a8 8 0 01-8 8H7l-4 3v-5.5A8 8 0 1121 12z",
  plan: "M9 5h9M9 12h9M9 19h9M4 5l1 1 2-2M4 12l1 1 2-2M4 19l1 1 2-2",
  agent: "M12 8V4M9 14h.01M15 14h.01M2 13v3M22 13v3",
  loop: "M3 12a9 9 0 0115.5-6.2M21 12a9 9 0 01-15.5 6.2M18 3v4h-4M6 21v-4h4",
};

function WorkIcon({ mode }: { mode: WorkMode }) {
  return (
    <svg
      className="h-4 w-4 shrink-0"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      viewBox="0 0 24 24"
    >
      {mode === "agent" && <rect x="4" y="8" width="16" height="12" rx="3" />}
      <path d={WORK_ICONS[mode]} />
    </svg>
  );
}

function ModeIcon({ mode }: { mode: RunMode }) {
  const path =
    mode === "approve"
      ? "M9 12l2 2 4-4M12 3l7 4v5c0 4.5-3 8-7 9-4-1-7-4.5-7-9V7z"
      : mode === "smart"
        ? "M12 3l1.9 4.6 4.6 1.9-4.6 1.9L12 16l-1.9-4.6L5.5 9.5l4.6-1.9zM18 15l.9 2.1 2.1.9-2.1.9L18 21l-.9-2.1-2.1-.9 2.1-.9z"
        : "M13 2L4.5 13.5H11l-1 8.5 8.5-11.5H12z";
  return (
    <svg
      className="h-4 w-4 shrink-0"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      viewBox="0 0 24 24"
    >
      <path d={path} />
    </svg>
  );
}

function Check() {
  return (
    <svg
      className="h-4 w-4 shrink-0 text-sky-400"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.4"
      strokeLinecap="round"
      strokeLinejoin="round"
      viewBox="0 0 24 24"
    >
      <path d="M20 6L9 17l-5-5" />
    </svg>
  );
}

function Caret() {
  return (
    <svg
      className="h-3 w-3 text-dim"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      viewBox="0 0 24 24"
    >
      <path d="M6 9l6 6 6-6" />
    </svg>
  );
}

/** Fecha no clique fora e no Esc — o mesmo comportamento dos dois menus. */
function usePopover() {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);
  return { open, setOpen, ref };
}

/** Modo de trabalho: conversa, planejamento, agente ou laço. */
function WorkModeSelect({
  params,
  onChange,
  disabled,
}: {
  params: ChatParams;
  onChange: (p: ChatParams) => void;
  disabled: boolean;
}) {
  const { t } = useTranslation();
  const { open, setOpen, ref } = usePopover();
  const mode: WorkMode = params.workMode ?? "agent";
  // O laço executa o plano inteiro sozinho: só faz sentido com o agente
  // ligado (é ele quem dá as ferramentas).
  const options =
    params.agent === true ? WORK_MODES : WORK_MODES.filter((m) => m !== "loop");

  return (
    <div ref={ref} className="relative shrink-0">
      <button
        type="button"
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
        title={`${t("plan.title")}: ${t(`plan.mode.${mode}`)}`}
        className={`flex items-center gap-1 rounded-full px-2 py-1 text-xs transition-colors hover:bg-panel disabled:opacity-40 ${
          open ? "bg-panel text-ink" : "text-ink"
        }`}
      >
        <WorkIcon mode={mode} />
        <span className="max-w-32 truncate">{t(`plan.mode.${mode}`)}</span>
        <Caret />
      </button>

      {open && (
        <div className="absolute bottom-full left-0 z-30 mb-2 w-80 rounded-xl border border-edge bg-panel py-1.5 shadow-[0_12px_40px_rgba(0,0,0,0.45)]">
          <p className="px-3 pt-1 pb-1.5 text-[11px] tracking-wide text-dim uppercase">
            {t("plan.title")}
          </p>
          {options.map((option) => (
            <button
              key={option}
              type="button"
              onClick={() => {
                onChange({ ...params, workMode: option });
                setOpen(false);
              }}
              className="flex w-full items-start gap-2.5 px-3 py-2.5 text-left hover:bg-panel2"
            >
              <span className="mt-0.5 text-dim">
                <WorkIcon mode={option} />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block text-[13px] text-ink">
                  {t(`plan.mode.${option}`)}
                </span>
                <span className="mt-0.5 block text-[11px] leading-relaxed text-dim">
                  {t(`plan.mode.${option}Hint`)}
                </span>
              </span>
              {mode === option && <Check />}
            </button>
          ))}
          <p className="border-t border-edge px-3 pt-2 pb-1 text-[11px] leading-relaxed text-dim">
            {t("plan.subtitle")}
          </p>
        </div>
      )}
    </div>
  );
}

/** Nível de autorização: pedir sempre, pedir para alterar ou automático. */
function AuthSelect({
  params,
  onChange,
  disabled,
}: {
  params: ChatParams;
  onChange: (p: ChatParams) => void;
  disabled: boolean;
}) {
  const { t } = useTranslation();
  const { open, setOpen, ref } = usePopover();
  const [confirmYolo, setConfirmYolo] = useState(false);
  const [needWorkspace, setNeedWorkspace] = useState(false);

  const mode: RunMode = params.mode ?? "smart";

  const pick = (next: RunMode) => {
    if (next === "yolo") {
      if (!params.workspaceDir) {
        setNeedWorkspace(true);
        return;
      }
      setNeedWorkspace(false);
      setConfirmYolo(true);
      return;
    }
    setNeedWorkspace(false);
    onChange({ ...params, mode: next });
    setOpen(false);
  };

  return (
    <div ref={ref} className="relative shrink-0">
      <button
        type="button"
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
        title={`${t("agent.mode.label")}: ${t(`agent.mode.${mode}`)}`}
        className={`flex items-center gap-1 rounded-full px-2 py-1 text-xs transition-colors hover:bg-panel disabled:opacity-40 ${
          mode === "yolo" ? "text-warn" : open ? "bg-panel text-ink" : "text-ink"
        }`}
      >
        <ModeIcon mode={mode} />
        <span className="max-w-32 truncate">{t(`agent.mode.${mode}`)}</span>
        <Caret />
      </button>

      {open && (
        <div className="absolute bottom-full left-0 z-30 mb-2 w-80 rounded-xl border border-edge bg-panel py-1.5 shadow-[0_12px_40px_rgba(0,0,0,0.45)]">
          <p className="px-3 pt-1 pb-1.5 text-[11px] tracking-wide text-dim uppercase">
            {t("agent.mode.label")}
          </p>
          {AUTH_MODES.map((option) => (
            <button
              key={option}
              type="button"
              onClick={() => pick(option)}
              className="flex w-full items-start gap-2.5 px-3 py-2.5 text-left hover:bg-panel2"
            >
              <span
                className={`mt-0.5 ${option === "yolo" ? "text-warn" : "text-dim"}`}
              >
                <ModeIcon mode={option} />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block text-[13px] text-ink">
                  {t(`agent.mode.${option}`)}
                </span>
                <span className="mt-0.5 block text-[11px] leading-relaxed text-dim">
                  {t(`agent.mode.${option}Hint`)}
                </span>
              </span>
              {mode === option && <Check />}
            </button>
          ))}
          {needWorkspace && (
            <p className="border-t border-edge px-3 py-2 text-[11px] text-warn">
              {t("agent.yolo.needWorkspace")}
            </p>
          )}
        </div>
      )}

      {confirmYolo && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6"
          onMouseDown={(e) => {
            if (e.target === e.currentTarget) setConfirmYolo(false);
          }}
        >
          <div className="w-full max-w-md rounded-2xl border border-warn/40 bg-panel p-5 shadow-[0_24px_80px_rgba(0,0,0,0.55)]">
            <h2 className="flex items-center gap-2 text-sm font-semibold text-ink">
              <span className="rounded-full bg-warn/15 px-2 py-0.5 text-[11px] font-semibold text-warn">
                {t("agent.yolo.badge")}
              </span>
              {t("agent.yolo.confirmTitle")}
            </h2>
            <p className="mt-2.5 text-[12px] leading-relaxed text-dim select-text">
              {t("agent.yolo.confirmBody")}
            </p>
            {params.workspaceDir && (
              <p className="mt-2 truncate font-mono text-[11px] text-dim">
                {params.workspaceDir}
              </p>
            )}
            <div className="mt-4 flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setConfirmYolo(false)}
                className="rounded-lg border border-edge px-3 py-1.5 text-[12px] text-dim transition-colors hover:text-ink"
              >
                {t("common.cancel")}
              </button>
              <button
                type="button"
                onClick={() => {
                  onChange({ ...params, mode: "yolo" });
                  setConfirmYolo(false);
                  setOpen(false);
                }}
                className="rounded-lg bg-warn px-3 py-1.5 text-[12px] font-medium text-black"
              >
                {t("agent.yolo.confirmOk")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export default function ModeSelect({
  params,
  onChange,
  disabled = false,
}: {
  params: ChatParams;
  onChange: (p: ChatParams) => void;
  disabled?: boolean;
}) {
  const workMode: WorkMode = params.workMode ?? "agent";
  return (
    <>
      <WorkModeSelect
        params={params}
        onChange={onChange}
        disabled={disabled}
      />
      {canExecute(workMode) && (
        <AuthSelect params={params} onChange={onChange} disabled={disabled} />
      )}
    </>
  );
}
