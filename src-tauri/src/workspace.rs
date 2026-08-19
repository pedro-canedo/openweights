//! Pasta da sessão: listar / ler / gravar arquivos sob um diretório
//! escolhido pelo usuário. Todo caminho é resolvido contra a raiz —
//! `..` e prefixos absolutos são rejeitados.

use serde::Serialize;
use std::path::{Component, Path, PathBuf};

const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    "dist",
    ".svn",
    "__pycache__",
    ".next",
    "build",
];
/// Pastas que começam com `.` e mesmo assim entram na árvore (memória do projeto).
const VISIBLE_DOT_DIRS: &[&str] = &[".openweights"];
const MAX_FILES: usize = 400;
const MAX_DEPTH: usize = 5;
const MAX_BYTES: u64 = 300 * 1024;

const TEXT_EXTS: &[&str] = &[
    "txt", "md", "markdown", "json", "js", "ts", "tsx", "jsx", "py", "rs", "toml", "yaml", "yml",
    "css", "html", "htm", "c", "cpp", "h", "hpp", "java", "go", "rb", "php", "sh", "sql", "csv",
    "log", "xml", "svg", "env", "ini", "cfg",
];

#[derive(Debug)]
pub enum WorkspaceError {
    Msg(String),
    Io(std::io::Error),
}

impl std::fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Msg(m) => f.write_str(m),
            Self::Io(e) => write!(f, "falha de E/S: {e}"),
        }
    }
}

impl From<std::io::Error> for WorkspaceError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

pub type WsResult<T> = Result<T, WorkspaceError>;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFile {
    pub path: String,
    pub name: String,
    pub bytes: u64,
}

/// Abre o seletor de pastas.
///
/// `parent` amarra o diálogo à janela do app: sem isso, no Windows ele abre
/// ATRÁS da janela principal e dá a impressão de que nada aconteceu.
pub async fn pick_folder(parent: Option<tauri::WebviewWindow>) -> Option<String> {
    let mut dialog = rfd::AsyncFileDialog::new().set_title("Adicionar pasta à sessão");
    if let Some(window) = parent.as_ref() {
        dialog = dialog.set_parent(window);
    }
    dialog
        .pick_folder()
        .await
        .map(|h| h.path().to_string_lossy().into_owned())
}

pub fn list_files(root: &str) -> WsResult<Vec<WorkspaceFile>> {
    let root = PathBuf::from(root);
    if !root.is_dir() {
        return Err(WorkspaceError::Msg("pasta não encontrada".into()));
    }
    let mut out = Vec::new();
    walk(&root, &root, 0, &mut out)?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

pub fn read_file(root: &str, rel: &str) -> WsResult<String> {
    let path = resolve_under(Path::new(root), rel)?;
    let meta = std::fs::metadata(&path)?;
    if meta.len() > MAX_BYTES {
        return Err(WorkspaceError::Msg(format!(
            "arquivo grande demais (máx. {} KB)",
            MAX_BYTES / 1024
        )));
    }
    Ok(std::fs::read_to_string(&path)?)
}

pub fn write_file(root: &str, rel: &str, content: &str) -> WsResult<()> {
    if content.len() as u64 > MAX_BYTES {
        return Err(WorkspaceError::Msg(format!(
            "conteúdo grande demais (máx. {} KB)",
            MAX_BYTES / 1024
        )));
    }
    let path = resolve_under(Path::new(root), rel)?;
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        return Err(WorkspaceError::Msg("pasta pai não existe".into()));
    }
    std::fs::write(&path, content)?;
    Ok(())
}

/// Abre a pasta (ou revela o arquivo) no Explorer / Finder / xdg-open.
pub fn reveal(root: &str, rel: Option<&str>) -> WsResult<()> {
    let path = match rel.filter(|s| !s.is_empty()) {
        Some(rel) => resolve_under(Path::new(root), rel)?,
        None => {
            let root = PathBuf::from(root);
            if !root.is_dir() {
                return Err(WorkspaceError::Msg("pasta não encontrada".into()));
            }
            std::fs::canonicalize(&root)?
        }
    };
    open_in_file_manager(&path)
}

fn open_in_file_manager(path: &Path) -> WsResult<()> {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = std::process::Command::new("explorer");
        lr_proc::no_window_std(&mut cmd);
        if path.is_file() {
            cmd.arg(format!("/select,{}", path.display()));
        } else {
            cmd.arg(path);
        }
        cmd.spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new("open");
        lr_proc::no_window_std(&mut cmd);
        if path.is_file() {
            cmd.args(["-R", &path.to_string_lossy()]);
        } else {
            cmd.arg(path);
        }
        cmd.spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        let target = if path.is_file() {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        let mut cmd = std::process::Command::new("xdg-open");
        lr_proc::no_window_std(&mut cmd).arg(target).spawn()?;
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
        return Err(WorkspaceError::Msg(
            "abrir no gerenciador de arquivos não é suportado nesta plataforma".into(),
        ));
    }
    Ok(())
}

fn resolve_under(root: &Path, rel: &str) -> WsResult<PathBuf> {
    if rel.is_empty() || rel.contains('\0') {
        return Err(WorkspaceError::Msg("caminho inválido".into()));
    }
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return Err(WorkspaceError::Msg("caminho absoluto recusado".into()));
    }
    for c in rel_path.components() {
        match c {
            Component::Normal(_) => {}
            Component::CurDir => {}
            _ => return Err(WorkspaceError::Msg("caminho inválido".into())),
        }
    }
    let root = std::fs::canonicalize(root)
        .map_err(|_| WorkspaceError::Msg("pasta da sessão não encontrada".into()))?;
    let joined = root.join(rel_path);
    let canon = if joined.exists() {
        std::fs::canonicalize(&joined)?
    } else {
        let parent = joined
            .parent()
            .ok_or_else(|| WorkspaceError::Msg("caminho inválido".into()))?;
        let parent = std::fs::canonicalize(parent)?;
        if !parent.starts_with(&root) {
            return Err(WorkspaceError::Msg(
                "arquivo fora da pasta da sessão".into(),
            ));
        }
        parent.join(joined.file_name().unwrap_or_default())
    };
    if !canon.starts_with(&root) {
        return Err(WorkspaceError::Msg(
            "arquivo fora da pasta da sessão".into(),
        ));
    }
    Ok(canon)
}

fn skip_entry(name: &str, is_dir: bool) -> bool {
    if !name.starts_with('.') {
        return false;
    }
    if is_dir {
        return !VISIBLE_DOT_DIRS
            .iter()
            .any(|d| name.eq_ignore_ascii_case(d));
    }
    name != ".env" && name != ".gitignore"
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<WorkspaceFile>) -> WsResult<()> {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return Ok(());
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for ent in entries.flatten() {
        if out.len() >= MAX_FILES {
            break;
        }
        let path = ent.path();
        let name = ent.file_name().to_string_lossy().into_owned();
        if skip_entry(&name, path.is_dir()) {
            continue;
        }
        if path.is_dir() {
            if SKIP_DIRS.iter().any(|s| name.eq_ignore_ascii_case(s)) {
                continue;
            }
            // Rascunhos do code_run: não poluem o explorador.
            if name.eq_ignore_ascii_case("scratch")
                && dir.file_name().is_some_and(|n| n == ".openweights")
            {
                continue;
            }
            walk(root, &path, depth + 1, out)?;
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !TEXT_EXTS.contains(&ext.as_str()) {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = ent.metadata().map(|m| m.len()).unwrap_or(0);
        out.push(WorkspaceFile {
            name,
            path: rel,
            bytes,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ow-ws-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn rejects_escape() {
        let dir = tmp("esc");
        std::fs::write(dir.join("ok.txt"), "x").unwrap();
        assert!(resolve_under(&dir, "../secret.txt").is_err());
        assert!(resolve_under(&dir, "C:/Windows/notepad.exe").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lists_and_reads_nested() {
        let dir = tmp("list");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.join("readme.md"), "hi").unwrap();
        std::fs::create_dir_all(dir.join("node_modules/x")).unwrap();
        std::fs::write(dir.join("node_modules/x/a.js"), "no").unwrap();

        let files = list_files(dir.to_str().unwrap()).unwrap();
        let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"readme.md"));
        assert!(paths.contains(&"src/main.rs"));
        assert!(!paths.iter().any(|p| p.contains("node_modules")));

        let body = read_file(dir.to_str().unwrap(), "src/main.rs").unwrap();
        assert_eq!(body, "fn main() {}");
        write_file(dir.to_str().unwrap(), "readme.md", "ok").unwrap();
        assert_eq!(read_file(dir.to_str().unwrap(), "readme.md").unwrap(), "ok");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lists_memory_fact_files_and_html() {
        let dir = tmp("facts");
        std::fs::write(dir.join("index.html"), "<h1>oi</h1>").unwrap();
        let mem = dir.join(".openweights").join("memory");
        std::fs::create_dir_all(&mem).unwrap();
        std::fs::write(mem.join("MEMORY.md"), "# memória").unwrap();
        std::fs::write(mem.join("estilo.md"), "- tema dark").unwrap();
        let scratch = dir.join(".openweights").join("scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        std::fs::write(scratch.join("tmp.py"), "print(1)").unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        std::fs::write(dir.join(".hidden").join("secret.md"), "no").unwrap();

        let files = list_files(dir.to_str().unwrap()).unwrap();
        let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"index.html"));
        assert!(paths.contains(&".openweights/memory/MEMORY.md"));
        assert!(paths.contains(&".openweights/memory/estilo.md"));
        assert!(!paths.iter().any(|p| p.contains("scratch")));
        assert!(!paths.iter().any(|p| p.contains(".hidden")));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
