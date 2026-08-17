// Pasta da sessão: ícone no composer, explorador à direita (estilo VS)
// e editor em modal ao abrir um arquivo.

import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
  type UIEvent,
} from "react";
import { useTranslation } from "react-i18next";
import {
  listWorkspace,
  pickWorkspace,
  readWorkspaceFile,
  revealWorkspace,
  writeWorkspaceFile,
} from "../../lib/api";
import type { WorkspaceFile } from "../../lib/types";
import CheckpointList from "../agent/CheckpointList";
import RagPanel from "../agent/RagPanel";
import MemoryPanel from "../agent/MemoryPanel";

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

function revealFolderLabel(t: (key: string) => string): string {
  const p = navigator.platform.toLowerCase();
  if (p.includes("mac")) return t("workspace.revealFinder");
  if (p.includes("win")) return t("workspace.revealExplorer");
  return t("workspace.revealFolder");
}

function ExternalIcon({ className = "h-3.5 w-3.5" }: { className?: string }) {
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
      <path d="M14 5h5v5" />
      <path d="M10 14L19 5" />
      <path d="M19 13v5a1 1 0 01-1 1H6a1 1 0 01-1-1V6a1 1 0 011-1h5" />
    </svg>
  );
}

function SidebarIcon() {
  return (
    <svg
      className="h-4 w-4"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      viewBox="0 0 24 24"
    >
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M9 4v16" />
    </svg>
  );
}

type TreeNode = {
  name: string;
  file?: WorkspaceFile;
  children?: TreeNode[];
};

function buildTree(files: WorkspaceFile[]): TreeNode[] {
  const root: TreeNode[] = [];
  for (const file of files) {
    const parts = file.path.split(/[\\/]/).filter(Boolean);
    let level = root;
    parts.forEach((part, i) => {
      const isFile = i === parts.length - 1;
      let node = level.find((n) => n.name === part);
      if (!node) {
        node = isFile
          ? { name: part, file }
          : { name: part, children: [] };
        level.push(node);
      }
      if (!isFile) level = node.children ?? (node.children = []);
    });
  }
  const sortNodes = (nodes: TreeNode[]) => {
    nodes.sort((a, b) => {
      const ad = a.children ? 0 : 1;
      const bd = b.children ? 0 : 1;
      return ad - bd || a.name.localeCompare(b.name);
    });
    nodes.forEach((n) => n.children && sortNodes(n.children));
  };
  sortNodes(root);
  return root;
}

type WorkspaceCtx = {
  dir: string | null;
  files: WorkspaceFile[];
  disabled?: boolean;
  explorerOpen: boolean;
  setExplorerOpen: (open: boolean) => void;
  /** Muda quando o agente cria um checkpoint — recarrega a lista. */
  checkpointsKey: number;
  addFolder: () => Promise<void>;
  removeFolder: () => void;
  query: string;
  setQuery: (q: string) => void;
  openPath: string | null;
  editorOpen: boolean;
  draft: string;
  setDraft: (s: string) => void;
  dirty: boolean;
  setDirty: (d: boolean) => void;
  busy: boolean;
  error: string | null;
  saved: boolean;
  visible: WorkspaceFile[];
  openFile: (f: WorkspaceFile) => Promise<void>;
  closeEditor: () => void;
  save: () => Promise<void>;
  reveal: (rel?: string | null) => Promise<void>;
  reloadFiles: () => void;
};

const WorkspaceContext = createContext<WorkspaceCtx | null>(null);

function useWorkspace(): WorkspaceCtx {
  const ctx = useContext(WorkspaceContext);
  if (!ctx) {
    throw new Error("Workspace precisa estar em WorkspaceHost");
  }
  return ctx;
}

export function WorkspaceHost({
  dir,
  onDirChange,
  files,
  onFiles,
  disabled,
  checkpointsKey = 0,
  children,
}: {
  dir: string | null;
  onDirChange: (dir: string | null) => void;
  files: WorkspaceFile[];
  onFiles: (files: WorkspaceFile[]) => void;
  disabled?: boolean;
  /** Contador de checkpoints do run corrente (força recarga da lista). */
  checkpointsKey?: number;
  children: ReactNode;
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [openPath, setOpenPath] = useState<string | null>(null);
  const [editorOpen, setEditorOpen] = useState(false);
  const [explorerOpen, setExplorerOpen] = useState(false);
  const [draft, setDraft] = useState("");
  const [dirty, setDirty] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const reloadFiles = () => {
    if (!dir) {
      onFiles([]);
      return;
    }
    void listWorkspace(dir)
      .then(onFiles)
      .catch(() => onFiles([]));
  };

  useEffect(() => {
    if (!dir) {
      onFiles([]);
      setOpenPath(null);
      setEditorOpen(false);
      return;
    }
    reloadFiles();
    // Recarrega quando a pasta muda, o agente grava um checkpoint, ou o
    // explorador abre — senão index.html / MEMORY.md nascem e a lista fica vazia.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dir, checkpointsKey, explorerOpen]);

  const addFolder = async () => {
    setError(null);
    const picked = await pickWorkspace().catch(() => null);
    if (picked) {
      onDirChange(picked);
      setExplorerOpen(true);
    }
  };

  const removeFolder = () => {
    if (dirty && !window.confirm(t("workspace.discard"))) return;
    onDirChange(null);
    setOpenPath(null);
    setEditorOpen(false);
    setDraft("");
    setDirty(false);
  };

  const openFile = async (file: WorkspaceFile) => {
    if (!dir) return;
    if (dirty && openPath !== file.path && !window.confirm(t("workspace.discard"))) {
      return;
    }
    setError(null);
    try {
      const text = await readWorkspaceFile(dir, file.path);
      setOpenPath(file.path);
      setDraft(text);
      setDirty(false);
      setEditorOpen(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const closeEditor = () => {
    if (dirty && !window.confirm(t("workspace.discard"))) return;
    setEditorOpen(false);
    setOpenPath(null);
    setDraft("");
    setDirty(false);
  };

  const reveal = async (rel?: string | null) => {
    if (!dir) return;
    setError(null);
    try {
      await revealWorkspace(dir, rel);
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
        explorerOpen,
        setExplorerOpen,
        checkpointsKey,
        addFolder,
        removeFolder,
        query,
        setQuery,
        openPath,
        editorOpen,
        draft,
        setDraft,
        dirty,
        setDirty,
        busy,
        error,
        saved,
        visible,
        openFile,
        closeEditor,
        save,
        reveal,
        reloadFiles,
      }}
    >
      {children}
      <WorkspaceEditor />
    </WorkspaceContext.Provider>
  );
}

/**
 * Acesso ao explorador de dentro do `WorkspaceHost` — usado pelo atalho de
 * checkpoint do agente, que precisa abrir a coluna onde a lista mora.
 */
export function useWorkspacePanel(): {
  dir: string | null;
  explorerOpen: boolean;
  openExplorer: () => void;
} {
  const { dir, explorerOpen, setExplorerOpen } = useWorkspace();
  return { dir, explorerOpen, openExplorer: () => setExplorerOpen(true) };
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

export function WorkspaceToggle() {
  const { t } = useTranslation();
  const { explorerOpen, setExplorerOpen, dir } = useWorkspace();

  return (
    <button
      type="button"
      onClick={() => setExplorerOpen(!explorerOpen)}
      title={
        explorerOpen ? t("workspace.closeExplorer") : t("workspace.openExplorer")
      }
      className={`flex h-8 w-8 items-center justify-center rounded-full border transition-colors ${
        explorerOpen
          ? "border-accent text-accent"
          : dir
            ? "border-edge text-ink hover:border-accent"
            : "border-edge text-dim hover:border-accent hover:text-ink"
      }`}
    >
      <SidebarIcon />
    </button>
  );
}

function FileTree({
  nodes,
  depth,
}: {
  nodes: TreeNode[];
  depth: number;
}) {
  const { openPath, openFile } = useWorkspace();
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});

  return (
    <>
      {nodes.map((node) => {
        const key = `${depth}-${node.name}`;
        if (node.children) {
          const closed = collapsed[key] === true;
          return (
            <div key={key}>
              <button
                type="button"
                onClick={() =>
                  setCollapsed((cur) => ({ ...cur, [key]: !closed }))
                }
                style={{ paddingLeft: 8 + depth * 12 }}
                className="flex w-full items-center gap-1.5 py-1 pr-2 text-left text-[12px] text-dim hover:bg-panel2 hover:text-ink"
              >
                <span className="w-3 shrink-0 text-[10px]">
                  {closed ? "▸" : "▾"}
                </span>
                <span className="truncate">{node.name}</span>
              </button>
              {!closed && (
                <FileTree nodes={node.children} depth={depth + 1} />
              )}
            </div>
          );
        }
        const active = openPath === node.file?.path;
        return (
          <button
            key={node.file?.path ?? key}
            type="button"
            onClick={() => node.file && void openFile(node.file)}
            style={{ paddingLeft: 20 + depth * 12 }}
            className={`flex w-full items-center py-1 pr-2 text-left text-[12px] hover:bg-panel2 ${
              active ? "bg-panel2 text-ink" : "text-dim"
            }`}
          >
            <span className="truncate">{node.name}</span>
          </button>
        );
      })}
    </>
  );
}

export function WorkspaceExplorer() {
  const { t } = useTranslation();
  const {
    dir,
    explorerOpen,
    setExplorerOpen,
    checkpointsKey,
    addFolder,
    removeFolder,
    reveal,
    query,
    setQuery,
    visible,
    error,
    openFile,
    reloadFiles,
  } = useWorkspace();

  const tree = useMemo(() => buildTree(visible), [visible]);

  if (!explorerOpen) return null;

  return (
    <aside className="flex w-64 shrink-0 flex-col border-l border-edge bg-panel">
      <div className="flex items-center gap-2 border-b border-edge px-3 py-2">
        <span className="min-w-0 flex-1 truncate text-[11px] font-medium tracking-wide text-dim uppercase">
          {dir ? folderName(dir) : t("workspace.explorer")}
        </span>
        {dir && (
          <>
            <button
              type="button"
              onClick={() => void reveal()}
              title={revealFolderLabel(t)}
              className="flex h-5 w-5 items-center justify-center rounded text-dim hover:text-ink"
            >
              <ExternalIcon />
            </button>
            <button
              type="button"
              onClick={removeFolder}
              title={t("workspace.remove")}
              className="flex h-5 w-5 items-center justify-center rounded text-dim hover:bg-bad/20 hover:text-bad"
            >
              ×
            </button>
          </>
        )}
        <button
          type="button"
          onClick={() => setExplorerOpen(false)}
          title={t("workspace.closeExplorer")}
          className="flex h-5 w-5 items-center justify-center rounded text-dim hover:text-ink"
        >
          ▸
        </button>
      </div>

      {dir ? (
        <>
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("workspace.search")}
            className="w-full border-b border-edge bg-transparent px-3 py-2 text-xs outline-none placeholder:text-dim"
          />
          <div className="min-h-0 flex-1 overflow-y-auto py-1">
            {visible.length === 0 ? (
              <p className="px-3 py-6 text-center text-[11px] text-dim">
                {t("workspace.empty")}
              </p>
            ) : (
              <FileTree nodes={tree} depth={0} />
            )}
          </div>
          {/* Desfazer das alterações do agente, junto dos arquivos. */}
          <CheckpointList workspaceDir={dir} reloadKey={checkpointsKey} />
          {/* Índice do projeto: clicar num resultado abre o arquivo no mesmo
              editor da árvore (o trecho traz caminho e linha). */}
          <RagPanel
            workspaceDir={dir}
            onOpenFile={(path) =>
              void openFile({
                path,
                name: path.split("/").pop() ?? path,
                bytes: 0,
              })
            }
          />
          <MemoryPanel
            workspaceDir={dir}
            reloadKey={checkpointsKey}
            onChanged={reloadFiles}
          />
        </>
      ) : (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 px-4 text-center">
          <p className="text-[12px] text-dim">{t("workspace.noFolder")}</p>
          <button
            type="button"
            onClick={() => void addFolder()}
            className="rounded-lg bg-accent px-3 py-1.5 text-xs font-medium text-white"
          >
            {t("workspace.add")}
          </button>
        </div>
      )}

      {/* Memória global continua visível mesmo sem pasta escolhida. */}
      {!dir && (
        <MemoryPanel
          workspaceDir={dir}
          reloadKey={checkpointsKey}
          onChanged={reloadFiles}
        />
      )}

      {error && (
        <p className="border-t border-edge px-3 py-2 text-[11px] text-bad">
          {error}
        </p>
      )}
    </aside>
  );
}

function WorkspaceEditor() {
  const { t } = useTranslation();
  const {
    editorOpen,
    openPath,
    draft,
    setDraft,
    dirty,
    setDirty,
    busy,
    saved,
    save,
    closeEditor,
    reveal,
  } = useWorkspace();
  const taRef = useRef<HTMLTextAreaElement>(null);
  const gutterRef = useRef<HTMLDivElement>(null);
  const [cursor, setCursor] = useState({ line: 1, col: 1 });

  const lines = useMemo(() => draft.split("\n"), [draft]);

  useEffect(() => {
    if (!editorOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        void save();
      }
      if (e.key === "Escape") closeEditor();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [editorOpen, save, closeEditor]);

  useEffect(() => {
    if (editorOpen) taRef.current?.focus();
  }, [editorOpen, openPath]);

  if (!editorOpen || !openPath) return null;

  const syncScroll = (e: UIEvent<HTMLTextAreaElement>) => {
    if (gutterRef.current) {
      gutterRef.current.scrollTop = e.currentTarget.scrollTop;
    }
  };

  const updateCursor = (el: HTMLTextAreaElement) => {
    const before = el.value.slice(0, el.selectionStart);
    const line = before.split("\n").length;
    const col = before.length - before.lastIndexOf("\n");
    setCursor({ line, col });
  };

  const onKeyDown = (e: ReactKeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key !== "Tab") return;
    e.preventDefault();
    const el = e.currentTarget;
    const start = el.selectionStart;
    const end = el.selectionEnd;
    const next = `${draft.slice(0, start)}  ${draft.slice(end)}`;
    setDraft(next);
    setDirty(true);
    requestAnimationFrame(() => {
      el.selectionStart = el.selectionEnd = start + 2;
      updateCursor(el);
    });
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) closeEditor();
      }}
    >
      <div className="flex h-[min(86vh,820px)] w-full max-w-5xl flex-col overflow-hidden rounded-xl border border-edge bg-[#1e1e1e] shadow-[0_24px_80px_rgba(0,0,0,0.55)]">
        <div className="flex items-center gap-2 border-b border-white/10 px-3 py-2">
          <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-white/90">
            {openPath}
            {dirty ? " •" : ""}
          </span>
          <button
            type="button"
            onClick={() => void reveal(openPath)}
            title={t("workspace.revealFile")}
            className="flex h-6 w-6 items-center justify-center rounded text-white/50 hover:bg-white/10 hover:text-white"
          >
            <ExternalIcon className="h-3.5 w-3.5" />
          </button>
          <button
            type="button"
            onClick={() => void save()}
            disabled={busy || !dirty}
            className="rounded-md bg-accent px-2.5 py-1 text-[11px] font-medium text-white disabled:opacity-40"
          >
            {saved ? t("workspace.saved") : t("common.save")}
          </button>
          <button
            type="button"
            onClick={closeEditor}
            title={t("common.close")}
            className="flex h-6 w-6 items-center justify-center rounded text-white/50 hover:bg-white/10 hover:text-white"
          >
            ×
          </button>
        </div>

        <div className="flex min-h-0 flex-1">
          <div
            ref={gutterRef}
            className="shrink-0 overflow-hidden border-r border-white/10 bg-[#1e1e1e] py-3 pr-3 pl-3 text-right font-mono text-[12px] leading-relaxed text-white/25 select-none"
          >
            {lines.map((_, i) => (
              <div key={i}>{i + 1}</div>
            ))}
          </div>
          <textarea
            ref={taRef}
            value={draft}
            spellCheck={false}
            wrap="off"
            onScroll={syncScroll}
            onKeyDown={onKeyDown}
            onClick={(e) => updateCursor(e.currentTarget)}
            onKeyUp={(e) => updateCursor(e.currentTarget)}
            onChange={(e) => {
              setDraft(e.target.value);
              setDirty(true);
              updateCursor(e.target);
            }}
            className="h-full min-h-0 w-full resize-none bg-[#1e1e1e] px-3 py-3 font-mono text-[12px] leading-relaxed text-[#d4d4d4] outline-none select-text"
          />
        </div>

        <div className="flex items-center justify-between border-t border-white/10 px-3 py-1.5 text-[11px] text-white/40">
          <span>{t("workspace.editor")}</span>
          <span>
            {t("workspace.lineCol", {
              line: cursor.line,
              col: cursor.col,
            })}
            {dirty ? ` · ${t("workspace.unsaved")}` : ""}
          </span>
        </div>
      </div>
    </div>
  );
}
