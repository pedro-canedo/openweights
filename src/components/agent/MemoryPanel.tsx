// Memória de longo prazo: o que o agente lembra entre conversas.
//
// Fica no explorador da pasta, ao lado dos checkpoints, porque memória é
// contexto do projeto — e porque é aqui que a pessoa já vem quando quer
// entender o que o agente sabe. A lista é curta de propósito: cada fato
// volta no prompt de toda execução seguinte, então "esquecer" é uma ação
// tão importante quanto "lembrar" e mora ao lado de cada linha.

import { useCallback, useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import {
  isGlobalFact,
  memoryAddFact,
  memoryConsolidateNow,
  memoryFacts,
  memoryForgetFact,
  memoryOpenFolder,
} from "../../lib/agent/memory";
import { errorMessage } from "../../lib/serverSession";
import type { MemoryFact } from "../../lib/agent/memory";

function FolderIcon() {
  return (
    <svg
      className="h-3.5 w-3.5"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      viewBox="0 0 24 24"
    >
      <path d="M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v9a2 2 0 01-2 2H5a2 2 0 01-2-2z" />
    </svg>
  );
}

function TidyIcon() {
  return (
    <svg
      className="h-3.5 w-3.5"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      viewBox="0 0 24 24"
    >
      <path d="M12 3l1.6 4.4L18 9l-4.4 1.6L12 15l-1.6-4.4L6 9l4.4-1.6z" />
      <path d="M18 15l.8 2.2L21 18l-2.2.8L18 21l-.8-2.2L15 18l2.2-.8z" />
    </svg>
  );
}

/** Projeto antes de global: o específico é o que a pessoa veio conferir. */
function sortFacts(facts: MemoryFact[]): MemoryFact[] {
  return [...facts].sort((a, b) => {
    const ag = isGlobalFact(a) ? 1 : 0;
    const bg = isGlobalFact(b) ? 1 : 0;
    return ag - bg || b.createdAt - a.createdAt;
  });
}

export default function MemoryPanel({
  workspaceDir,
  reloadKey = 0,
}: {
  workspaceDir: string | null;
  /** Muda para forçar recarga (ex.: o agente memorizou algo no run). */
  reloadKey?: number;
}) {
  const { t } = useTranslation();
  const [facts, setFacts] = useState<MemoryFact[]>([]);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [tidying, setTidying] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    void memoryFacts(workspaceDir)
      .then((rows) => setFacts(sortFacts(rows)))
      .catch(() => setFacts([]));
  }, [workspaceDir]);

  useEffect(() => {
    refresh();
  }, [refresh, reloadKey]);

  const flash = (message: string) => {
    setNote(message);
    window.setTimeout(() => setNote(null), 2500);
  };

  const add = async (e: FormEvent) => {
    e.preventDefault();
    const content = draft.trim();
    if (!content || busy) return;
    setBusy(true);
    setError(null);
    try {
      await memoryAddFact(workspaceDir, content);
      setDraft("");
      flash(t("agent.memory.saved"));
      refresh();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  const forget = async (fact: MemoryFact) => {
    setError(null);
    try {
      await memoryForgetFact(fact.id, workspaceDir);
      refresh();
    } catch (err) {
      setError(errorMessage(err));
    }
  };

  const tidy = async () => {
    setTidying(true);
    setError(null);
    try {
      const report = await memoryConsolidateNow(workspaceDir);
      flash(`${t("agent.memory.saved")} (${report.added.length})`);
      refresh();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setTidying(false);
    }
  };

  const openFolder = async () => {
    if (!workspaceDir) return;
    setError(null);
    try {
      await memoryOpenFolder(workspaceDir);
    } catch (err) {
      setError(errorMessage(err));
    }
  };

  return (
    <div className="border-t border-edge">
      <div className="flex items-center gap-2 px-3 py-2">
        <span
          title={t("agent.memory.subtitle")}
          className="flex-1 text-[11px] font-medium tracking-wide text-dim uppercase"
        >
          {t("agent.memory.title")}
        </span>
        {facts.length > 0 && (
          <span className="rounded-full bg-panel2 px-1.5 py-0.5 text-[10px] tabular-nums text-dim">
            {facts.length}
          </span>
        )}
        <button
          type="button"
          onClick={() => void tidy()}
          disabled={tidying}
          title={tidying ? t("agent.memory.consolidating") : t("agent.memory.consolidate")}
          className="flex h-5 w-5 items-center justify-center rounded text-dim hover:text-ink disabled:opacity-40"
        >
          <TidyIcon />
        </button>
        {workspaceDir && (
          <button
            type="button"
            onClick={() => void openFolder()}
            title={t("agent.memory.openFolder")}
            className="flex h-5 w-5 items-center justify-center rounded text-dim hover:text-ink"
          >
            <FolderIcon />
          </button>
        )}
      </div>

      {tidying && (
        <p className="px-3 pb-2 text-[11px] text-dim">
          {t("agent.memory.consolidating")}
        </p>
      )}
      {note && <p className="px-3 pb-2 text-[11px] text-ok">{note}</p>}
      {error && <p className="px-3 pb-2 text-[11px] text-bad">{error}</p>}

      {facts.length === 0 ? (
        <p className="px-3 pb-2 text-[11px] text-dim">
          {t("agent.memory.empty")}
        </p>
      ) : (
        <ul className="max-h-44 overflow-y-auto pb-1">
          {facts.map((fact) => (
            <li
              key={fact.id}
              className="group flex items-start gap-2 px-3 py-1.5 hover:bg-panel2"
            >
              <span className="min-w-0 flex-1">
                <span className="block text-[12px] break-words text-ink">
                  {fact.content}
                </span>
                <span className="block text-[10px] text-dim">
                  {isGlobalFact(fact)
                    ? t("agent.memory.scopeGlobal")
                    : t("agent.memory.scopeWorkspace")}
                </span>
              </span>
              <button
                type="button"
                onClick={() => void forget(fact)}
                title={t("agent.memory.delete")}
                className="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded text-dim opacity-0 transition-opacity group-hover:opacity-100 hover:bg-bad/20 hover:text-bad focus:opacity-100"
              >
                ×
              </button>
            </li>
          ))}
        </ul>
      )}

      <form onSubmit={(e) => void add(e)} className="flex items-center gap-1 px-3 pb-3">
        <input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder={t("agent.memory.factPlaceholder")}
          className="min-w-0 flex-1 rounded-md border border-edge bg-transparent px-2 py-1 text-[11px] outline-none placeholder:text-dim focus:border-accent"
        />
        <button
          type="submit"
          disabled={busy || draft.trim().length === 0}
          title={t("agent.memory.addFact")}
          className="shrink-0 rounded-md border border-edge px-2 py-1 text-[11px] text-dim transition-colors hover:border-accent hover:text-ink disabled:opacity-40"
        >
          +
        </button>
      </form>
    </div>
  );
}
