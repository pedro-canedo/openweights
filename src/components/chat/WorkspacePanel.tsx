// Pasta da sessão: anexar diretório, listar arquivos, abrir no editor
// e gravar de volta no disco.

import {
  createContext,
  useContext,
  useEffect,
  useState,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import {
  listWorkspace,
  pickWorkspace,
  readWorkspaceFile,
  writeWorkspaceFile,
} from "../../lib/api";
import { formatBytes } from "../../lib/format";
import type { WorkspaceFile } from "../../lib/types";

function folderName(dir: string): string {
  const parts = dir.replace(/[\\/]+$/, "").split(/[\\/]/);
  return parts[parts.length - 1] || dir;
}

function FolderPlusIcon({ className = "h-4 w-4" }: { className?: string }) {
  return (
    <svg
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      viewBox="0 0 24 24"
    >
      <path d="M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v9a2 2 0 01-2 2H5a2 2 0 01-2-2z" />
      <path d="M12 11v6M9 14h6" />
    </svg>
  );
}

type WorkspaceCtx = {
  dir: string | null;
  files: WorkspaceFile[];
  disabled?: boolean;
  addFolder: () => Promise<void>;
  removeFolder: () => void;
  query: string;
  setQuery: (q: string) => void;
  openPath: string | null;
  draft: string;
  setDraft: (s: string) => void;
  dirty: boolean;
  setDirty: (d: boolean) => void;
  busy: boolean;
  error: string | null;
  saved: boolean;
  visible: WorkspaceFile[];
  openFile: (f: WorkspaceFile) => Promise<void>;
  save: () => Promise<void>;
};

const WorkspaceContext = createContext<WorkspaceCtx | null>(null);

function useWorkspace(): WorkspaceCtx {
  const ctx = useContext(WorkspaceContext);
  if (!ctx) {
    throw new Error("WorkspaceTrigger/Files precisam estar em WorkspaceHost");
  }
  return ctx;
}

export function WorkspaceHost({
  dir,
  onDirChange,
  files,
  onFiles,
  disabled,
  children,
}: {
  dir: string | null;
  onDirChange: (dir: string | null) => void;
  files: WorkspaceFile[];
  onFiles: (files: WorkspaceFile[]) => void;
  disabled?: boolean;
  children: ReactNode;
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [openPath, setOpenPath] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [dirty, setDirty] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    if (!dir) {
      onFiles([]);
      setOpenPath(null);
      return;
    }
    void listWorkspace(dir)
      .then(onFiles)
      .catch(() => onFiles([]));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dir]);

  const addFolder = async () => {
    setError(null);
    const picked = await pickWorkspace().catch(() => null);
    if (picked) onDirChange(picked);
  };

  const removeFolder = () => {
    if (dirty && !window.confirm(t("workspace.discard"))) return;
    onDirChange(null);
    setOpenPath(null);
    setDraft("");
    setDirty(false);
  };

  const openFile = async (file: WorkspaceFile) => {
    if (!dir) return;
    if (dirty && !window.confirm(t("workspace.discard"))) return;
    setError(null);
    try {
      const text = await readWorkspaceFile(dir, file.path);
      setOpenPath(file.path);
      setDraft(text);
      setDirty(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const save = async () => {
    if (!dir || !openPath) return;
    setBusy(true);
    setError(null);
    try {
      await writeWorkspaceFile(dir, openPath, draft);
      setDirty(false);
      setSaved(true);
      window.setTimeout(() => setSaved(false), 1500);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const q = query.trim().toLowerCase();
  const visible = q
    ? files.filter(
        (f) =>
          f.name.toLowerCase().includes(q) || f.path.toLowerCase().includes(q),
      )
    : files;

  return (
    <WorkspaceContext.Provider
      value={{
        dir,
        files,
        disabled,
        addFolder,
        removeFolder,
        query,
        setQuery,
        openPath,
        draft,
        setDraft,
        dirty,
        setDirty,
        busy,
        error,
        saved,
        visible,
        openFile,
        save,
      }}
    >
      {children}
    </WorkspaceContext.Provider>
  );
}

export function WorkspaceTrigger() {
  const { t } = useTranslation();
  const { dir, disabled, addFolder } = useWorkspace();

  return (
    <button
      type="button"
      onClick={() => void addFolder()}
      disabled={disabled}
      title={dir ? `${folderName(dir)} — ${t("workspace.change")}` : t("workspace.add")}
      className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-full transition-colors disabled:opacity-40 ${
        dir
          ? "text-accent hover:bg-panel hover:text-ink"
          : "text-dim hover:bg-panel hover:text-ink"
      }`}
    >
      <FolderPlusIcon className="h-[18px] w-[18px]" />
    </button>
  );
}

export function WorkspaceFiles() {
  const { t } = useTranslation();
  const {
    dir,
    removeFolder,
    query,
    setQuery,
    openPath,
    draft,
    setDraft,
    dirty,
    setDirty,
    busy,
    error,
    saved,
    visible,
    openFile,
    save,
  } = useWorkspace();

  if (!dir && !error) return null;

  return (
    <div className="px-6">
    <div className="mx-auto mb-2 flex w-full max-w-2xl flex-col gap-2">
      {dir && (
        <div className="overflow-hidden rounded-2xl border border-edge bg-panel">
          <div className="flex items-center gap-2 border-b border-edge px-3 py-1.5">
            <span className="min-w-0 flex-1 truncate text-xs text-ink" title={dir}>
              {folderName(dir)}
            </span>
            <button
              type="button"
              onClick={removeFolder}
              title={t("workspace.remove")}
              className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-dim hover:bg-bad/20 hover:text-bad"
            >
              ×
            </button>
          </div>
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("workspace.search")}
            className="w-full border-b border-edge bg-transparent px-3 py-2 text-xs outline-none placeholder:text-dim"
          />
          <div className="max-h-36 overflow-y-auto">
            {visible.length === 0 ? (
              <p className="px-3 py-3 text-center text-[11px] text-dim">
                {t("workspace.empty")}
              </p>
            ) : (
              visible.map((f) => (
                <button
                  key={f.path}
                  type="button"
                  onClick={() => void openFile(f)}
                  className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs hover:bg-panel2 ${
                    openPath === f.path ? "bg-panel2 text-ink" : "text-dim"
                  }`}
                >
                  <span className="min-w-0 flex-1 truncate">{f.path}</span>
                  <span className="shrink-0 tabular-nums text-[10px]">
                    {formatBytes(f.bytes)}
                  </span>
                </button>
              ))
            )}
          </div>

          {openPath && (
            <div className="border-t border-edge">
              <div className="flex items-center gap-2 px-3 py-1.5">
                <span className="min-w-0 flex-1 truncate text-[11px] text-ink">
                  {openPath}
                  {dirty ? " ·" : ""}
                </span>
                <button
                  type="button"
                  onClick={() => void save()}
                  disabled={busy || !dirty}
                  className="rounded-md bg-accent px-2 py-0.5 text-[11px] font-medium text-white disabled:opacity-40"
                >
                  {saved ? t("workspace.saved") : t("common.save")}
                </button>
              </div>
              <textarea
                value={draft}
                onChange={(e) => {
                  setDraft(e.target.value);
                  setDirty(true);
                }}
                spellCheck={false}
                className="max-h-48 min-h-28 w-full resize-y bg-panel2 px-3 py-2 font-mono text-[12px] leading-relaxed outline-none select-text"
              />
            </div>
          )}
        </div>
      )}

      {error && <p className="text-[11px] text-bad">{error}</p>}
    </div>
    </div>
  );
}
