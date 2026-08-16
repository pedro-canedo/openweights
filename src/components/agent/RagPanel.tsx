// Índice do projeto (RAG): estado, indexação com progresso e busca por
// significado. Mora no explorador de pasta, ao lado dos checkpoints — que é
// onde o usuário já pensa em "os arquivos deste projeto".
//
// Duas coisas o painel precisa deixar óbvias, porque mudam o que o agente
// consegue fazer:
//   1. **Se há índice.** Sem índice, `workspace_search` não responde e o
//      agente volta a depender de adivinhar nome de arquivo.
//   2. **Se há vetor.** Sem modelo de embedding o índice existe, mas só com
//      busca textual — o usuário precisa saber que a parte "por significado"
//      está desligada (`agent.rag.vectorOff`), e não achar que está quebrada.
//
// A seção fica recolhida por padrão: quem abriu o explorador quer ver
// arquivos; o índice é uma ação deliberada.

import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ragClear,
  ragIndexCancel,
  ragIndexStart,
  ragSearch,
  ragStatus,
  type RagHit,
  type RagProgress,
  type RagStatus,
} from "../../lib/agent/rag";
import { errorMessage } from "../../lib/serverSession";

/**
 * Evento disparado no botão "baixar modelo de embedding". A tela de Descobrir
 * não é acessível daqui (o painel vive dentro do Chat), então o pedido sai
 * como evento de janela e quem sabe navegar decide o que fazer.
 */
export const OPEN_DISCOVER_EVENT = "openweights:open-discover";

function SearchIcon() {
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
      <circle cx="11" cy="11" r="7" />
      <path d="M20 20l-3.5-3.5" />
    </svg>
  );
}

/** Rótulo de origem do trecho: veio do texto, do vetor ou dos dois. */
function sourceKey(source: RagHit["source"]): string {
  if (source === "both") return "agent.rag.sourceBoth";
  return source === "vector" ? "agent.rag.sourceVector" : "agent.rag.sourceText";
}

export default function RagPanel({
  workspaceDir,
  onOpenFile,
}: {
  workspaceDir: string | null;
  /** Abre um arquivo do projeto no editor (caminho relativo). */
  onOpenFile?: (path: string) => void;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [status, setStatus] = useState<RagStatus | null>(null);
  const [progress, setProgress] = useState<RagProgress | null>(null);
  const [busy, setBusy] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<RagHit[] | null>(null);
  const [searching, setSearching] = useState(false);
  // Um `run` só pode escrever no estado se ainda for o mais recente: trocar de
  // pasta no meio de uma busca não pode fazer o resultado antigo aparecer.
  const runId = useRef(0);

  const refresh = useCallback(async () => {
    if (!workspaceDir) {
      setStatus(null);
      return;
    }
    const mine = ++runId.current;
    try {
      const next = await ragStatus(workspaceDir);
      if (runId.current === mine) setStatus(next);
    } catch {
      if (runId.current === mine) setStatus(null);
    }
  }, [workspaceDir]);

  useEffect(() => {
    setHits(null);
    setQuery("");
    setProgress(null);
    void refresh();
  }, [refresh]);

  const index = async () => {
    if (!workspaceDir) return;
    setBusy(true);
    setCancelling(false);
    setError(null);
    setProgress({ phase: "scanning", done: 0, total: 0, path: "" });
    try {
      await ragIndexStart(workspaceDir, setProgress);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
      setCancelling(false);
      setProgress(null);
      void refresh();
    }
  };

  const cancel = async () => {
    setCancelling(true);
    await ragIndexCancel().catch(() => {});
  };

  const clear = async () => {
    if (!workspaceDir) return;
    if (!window.confirm(t("agent.rag.confirmClear"))) return;
    setError(null);
    try {
      await ragClear(workspaceDir);
      setHits(null);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      void refresh();
    }
  };

  const search = async () => {
    if (!workspaceDir || !query.trim()) return;
    const mine = ++runId.current;
    setSearching(true);
    setError(null);
    try {
      const found = await ragSearch(workspaceDir, query.trim(), 10);
      if (runId.current === mine) setHits(found);
    } catch (e) {
      if (runId.current === mine) setError(errorMessage(e));
    } finally {
      if (runId.current === mine) setSearching(false);
    }
  };

  if (!workspaceDir) return null;

  const indexed = status?.indexed === true;
  const needModel = status != null && !status.embedModelConfigured;
  // Índice existe mas sem nenhum vetor: a parte "por significado" está fora.
  const vectorOff =
    indexed && (status.vectors === 0 || !status.capabilities.vector);

  const progressLabel = () => {
    if (!progress) return null;
    if (cancelling) return t("agent.rag.cancelling");
    if (progress.phase === "scanning") return t("agent.rag.scanning");
    if (progress.phase === "embedding") {
      return t("agent.rag.embedding", {
        done: progress.done,
        total: progress.total,
      });
    }
    return t("agent.rag.indexing", {
      done: progress.done,
      total: progress.total,
    });
  };

  const percent =
    progress && progress.total > 0
      ? Math.min(100, Math.round((progress.done / progress.total) * 100))
      : null;

  return (
    <div className="border-t border-edge">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-panel2"
      >
        <span className="w-3 shrink-0 text-[10px] text-dim">
          {open ? "▾" : "▸"}
        </span>
        <span className="flex-1 text-[11px] font-medium tracking-wide text-dim uppercase">
          {t("agent.rag.title")}
        </span>
        {indexed && (
          <span className="rounded-full bg-panel2 px-1.5 py-0.5 text-[10px] tabular-nums text-dim">
            {status.files}
          </span>
        )}
      </button>

      {open && (
        <div className="px-3 pb-3">
          <p className="pb-2 text-[11px] leading-snug text-dim">
            {t("agent.rag.subtitle")}
          </p>

          {indexed ? (
            <p className="pb-2 text-[11px] text-ink">
              {t("agent.rag.indexed", {
                files: status.files,
                chunks: status.chunks,
              })}
            </p>
          ) : (
            <p className="pb-2 text-[11px] text-dim">{t("agent.rag.empty")}</p>
          )}

          {needModel && (
            <div className="mb-2 rounded-md border border-edge bg-panel2 p-2">
              <p className="text-[11px] leading-snug text-dim">
                {t("agent.rag.needModel")}
              </p>
              <button
                type="button"
                onClick={() =>
                  window.dispatchEvent(new CustomEvent(OPEN_DISCOVER_EVENT))
                }
                className="mt-1.5 rounded-md border border-edge px-2 py-1 text-[10px] text-dim transition-colors hover:border-accent hover:text-ink"
              >
                {t("agent.rag.openDiscover")}
              </button>
            </div>
          )}

          {vectorOff && !needModel && (
            <p className="mb-2 rounded-md border border-edge bg-panel2 p-2 text-[11px] leading-snug text-dim">
              {t("agent.rag.vectorOff")}
            </p>
          )}

          {busy && (
            <div className="mb-2">
              <p className="pb-1 text-[11px] text-dim">{progressLabel()}</p>
              <div className="h-1 w-full overflow-hidden rounded-full bg-panel2">
                <div
                  className="h-full bg-accent transition-[width] duration-200"
                  style={{ width: `${percent ?? 8}%` }}
                />
              </div>
              {progress?.path && (
                <p className="truncate pt-1 font-mono text-[10px] text-dim">
                  {progress.path}
                </p>
              )}
            </div>
          )}

          <div className="flex flex-wrap gap-1.5">
            {busy ? (
              <button
                type="button"
                onClick={() => void cancel()}
                disabled={cancelling}
                className="rounded-md border border-edge px-2 py-1 text-[10px] text-dim transition-colors hover:border-bad hover:text-bad disabled:opacity-40"
              >
                {t("common.cancel")}
              </button>
            ) : (
              <button
                type="button"
                onClick={() => void index()}
                className="rounded-md bg-accent px-2.5 py-1 text-[10px] font-medium text-white"
              >
                {indexed ? t("agent.rag.reindex") : t("agent.rag.index")}
              </button>
            )}
            {indexed && !busy && (
              <button
                type="button"
                onClick={() => void clear()}
                className="rounded-md border border-edge px-2 py-1 text-[10px] text-dim transition-colors hover:border-bad hover:text-bad"
              >
                {t("agent.rag.clear")}
              </button>
            )}
          </div>

          {error && <p className="pt-2 text-[11px] text-bad">{error}</p>}

          {indexed && (
            <div className="pt-3">
              <div className="flex items-center gap-1.5 rounded-md border border-edge px-2 py-1">
                <span className="text-dim">
                  <SearchIcon />
                </span>
                <input
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void search();
                  }}
                  placeholder={t("agent.rag.searchPlaceholder")}
                  className="min-w-0 flex-1 bg-transparent text-[11px] outline-none placeholder:text-dim"
                />
              </div>

              {searching && (
                <p className="pt-2 text-[11px] text-dim">
                  {t("common.loading")}
                </p>
              )}

              {!searching && hits != null && hits.length === 0 && (
                <p className="pt-2 text-[11px] text-dim">
                  {t("agent.rag.noResults")}
                </p>
              )}

              {!searching && hits != null && hits.length > 0 && (
                <ul className="max-h-64 overflow-y-auto pt-2">
                  {hits.map((hit) => (
                    <li key={hit.chunkId}>
                      <button
                        type="button"
                        onClick={() => onOpenFile?.(hit.path)}
                        title={t("agent.rag.openFile")}
                        className="w-full rounded-md px-1.5 py-1 text-left hover:bg-panel2"
                      >
                        <span className="flex items-baseline gap-1.5">
                          <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-ink">
                            {hit.path}:{hit.startLine}
                          </span>
                          <span className="shrink-0 text-[9px] text-dim">
                            {t(sourceKey(hit.source))}
                          </span>
                        </span>
                        <span className="mt-0.5 block max-h-12 overflow-hidden font-mono text-[10px] leading-tight whitespace-pre-wrap text-dim">
                          {hit.snippet.trim().slice(0, 220)}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
