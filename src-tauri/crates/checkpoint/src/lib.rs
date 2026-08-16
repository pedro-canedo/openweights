//! Pontos de restauração dos arquivos do projeto.
//!
//! Antes de o agente alterar qualquer coisa, tiramos uma "foto" do estado
//! atual para que a pessoa possa voltar atrás com um clique.
//!
//! Duas implementações:
//! - [`GitShadow`]: um repositório git **paralelo** (em `<data_dir>`), que
//!   versiona a pasta do projeto sem NUNCA tocar no `.git` do usuário — nada
//!   de renomear, mover ou commitar no repositório dele. Conseguimos isso
//!   passando `GIT_DIR`/`GIT_WORK_TREE` para o git.
//! - [`CopySnapshot`]: cópia dos arquivos que serão alterados. Sempre
//!   disponível (não exige git instalado) e barata quando o run mexe em
//!   poucos arquivos.
//!
//! A escolha é automática: git quando existe e o projeto não é gigante;
//! cópia caso contrário.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum CheckpointError {
    #[error("falha de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("git falhou: {0}")]
    Git(String),
    #[error("checkpoint não encontrado: {0}")]
    NotFound(String),
}

/// Pastas que nunca entram num checkpoint: pesadas e reconstruíveis.
pub const EXCLUDES: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    ".next",
    ".venv",
    "venv",
    "__pycache__",
    ".cache",
    ".turbo",
    ".gradle",
    "vendor",
    ".pnpm-store",
];

/// Acima disto, o git-sombra fica lento demais e caímos para cópia.
const MAX_FILES_FOR_GIT: usize = 20_000;

/// Identificação de um ponto de restauração.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Checkpoint {
    pub id: String,
    /// `git` ou `copy`.
    pub backend: String,
    /// Commit (git) ou nome da pasta de snapshot (cópia).
    pub ref_id: String,
    pub label: String,
    /// Caminhos relativos salvos (só no backend de cópia).
    pub files: Vec<String>,
}

pub trait CheckpointEngine: Send + Sync {
    fn backend_name(&self) -> &'static str;

    /// Tira uma foto antes de alterar `files` (relativos ao projeto).
    /// Backends que versionam tudo podem ignorar a lista.
    fn snapshot(&self, label: &str, files: &[String]) -> Result<Checkpoint, CheckpointError>;

    /// Devolve os arquivos ao estado do checkpoint.
    fn restore(&self, cp: &Checkpoint) -> Result<Vec<String>, CheckpointError>;
}

/// Escolhe o backend apropriado para o projeto.
pub fn engine_for(
    workspace: &Path,
    store_dir: &Path,
) -> Result<Box<dyn CheckpointEngine>, CheckpointError> {
    if git_available() && !is_huge(workspace) {
        match GitShadow::new(workspace, store_dir) {
            Ok(g) => return Ok(Box::new(g)),
            Err(e) => log::warn!("git-sombra indisponível ({e}); usando cópia de arquivos"),
        }
    }
    Ok(Box::new(CopySnapshot::new(workspace, store_dir)))
}

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Contagem com early-exit: só queremos saber se passa do teto.
fn is_huge(root: &Path) -> bool {
    fn walk(dir: &Path, count: &mut usize, depth: u32) -> bool {
        if *count > MAX_FILES_FOR_GIT || depth > 12 {
            return true;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if EXCLUDES.contains(&name.as_str()) || name == ".git" {
                continue;
            }
            let path = e.path();
            if path.is_dir() {
                if walk(&path, count, depth + 1) {
                    return true;
                }
            } else {
                *count += 1;
                if *count > MAX_FILES_FOR_GIT {
                    return true;
                }
            }
        }
        false
    }
    let mut n = 0;
    walk(root, &mut n, 0)
}

/// Nome estável de pasta para um projeto (evita colisão entre caminhos).
fn workspace_key(workspace: &Path) -> String {
    let s = workspace.to_string_lossy().to_lowercase();
    let mut hash: u64 = 1469598103934665603;
    for b in s.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    let last = workspace
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "ws".into());
    let safe: String = last
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .take(24)
        .collect();
    format!("{safe}-{hash:016x}")
}

fn unique_id(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{prefix}-{nanos:x}")
}

// ------------------------------------------------------------- git sombra ---

/// Repositório git paralelo. O `.git` do usuário permanece intocado porque
/// todo comando roda com `GIT_DIR` apontando para o nosso repositório.
pub struct GitShadow {
    workspace: PathBuf,
    git_dir: PathBuf,
}

impl GitShadow {
    pub fn new(workspace: &Path, store_dir: &Path) -> Result<Self, CheckpointError> {
        let git_dir = store_dir
            .join("checkpoints")
            .join(workspace_key(workspace))
            .join("shadow.git");
        let me = Self {
            workspace: workspace.to_path_buf(),
            git_dir,
        };
        me.init()?;
        Ok(me)
    }

    fn git(&self, args: &[&str]) -> Result<String, CheckpointError> {
        let out = Command::new("git")
            .env("GIT_DIR", &self.git_dir)
            .env("GIT_WORK_TREE", &self.workspace)
            // Isola de configurações e hooks do usuário.
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", &self.git_dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .args(args)
            .current_dir(&self.workspace)
            .output()?;
        if !out.status.success() {
            return Err(CheckpointError::Git(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Comando sem `GIT_WORK_TREE` — `init` e `config` recusam a variável.
    fn git_bare(&self, args: &[&str]) -> Result<String, CheckpointError> {
        let out = Command::new("git")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", &self.git_dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .args(args)
            .output()?;
        if !out.status.success() {
            return Err(CheckpointError::Git(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn init(&self) -> Result<(), CheckpointError> {
        if self.git_dir.join("HEAD").exists() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.git_dir)?;
        let dir = self.git_dir.to_string_lossy().into_owned();
        // Repositório sem árvore própria; a árvore é a pasta do projeto,
        // informada em cada comando por GIT_WORK_TREE.
        self.git_bare(&["init", "--bare", "--quiet", &dir])?;
        // `core.bare=false` é o que permite add/commit contra a árvore
        // externa (mesmo truque dos repositórios de dotfiles).
        self.git_bare(&["--git-dir", &dir, "config", "core.bare", "false"])?;
        // Identidade só deste repositório (não mexe na global do usuário).
        self.git(&["config", "user.email", "agent@openweights.local"])?;
        self.git(&["config", "user.name", "OpenWeights"])?;
        self.git(&["config", "core.autocrlf", "false"])?;
        self.git(&["config", "core.safecrlf", "false"])?;
        // `core.excludesFile` no próprio git-dir: não cria arquivo no projeto.
        let exclude = self.git_dir.join("info").join("exclude");
        std::fs::create_dir_all(exclude.parent().unwrap())?;
        let mut body = String::from("# gerado pelo OpenWeights\n.git/\n");
        for e in EXCLUDES {
            body.push_str(e);
            body.push_str("/\n");
        }
        std::fs::write(exclude, body)?;
        Ok(())
    }
}

impl CheckpointEngine for GitShadow {
    fn backend_name(&self) -> &'static str {
        "git"
    }

    fn snapshot(&self, label: &str, _files: &[String]) -> Result<Checkpoint, CheckpointError> {
        self.git(&["add", "-A", "--", "."])?;
        // `commit` falha quando nada mudou; nesse caso reaproveitamos o HEAD.
        let commit = match self.git(&["commit", "-m", label, "--quiet", "--allow-empty"]) {
            Ok(_) => self.git(&["rev-parse", "HEAD"])?,
            Err(e) => {
                log::debug!("commit do checkpoint sem mudanças: {e}");
                self.git(&["rev-parse", "HEAD"])?
            }
        };
        Ok(Checkpoint {
            id: unique_id("cp"),
            backend: "git".into(),
            ref_id: commit,
            label: label.to_string(),
            files: Vec::new(),
        })
    }

    fn restore(&self, cp: &Checkpoint) -> Result<Vec<String>, CheckpointError> {
        let changed = self
            .git(&["diff", "--name-only", &cp.ref_id])
            .unwrap_or_default();
        self.git(&["checkout", "--force", &cp.ref_id, "--", "."])?;
        Ok(changed
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(str::to_string)
            .collect())
    }
}

// ------------------------------------------------------- cópia de arquivos ---

/// Guarda uma cópia dos arquivos que estão prestes a mudar. Simples,
/// previsível e sem dependência externa.
pub struct CopySnapshot {
    workspace: PathBuf,
    base: PathBuf,
}

impl CopySnapshot {
    pub fn new(workspace: &Path, store_dir: &Path) -> Self {
        Self {
            workspace: workspace.to_path_buf(),
            base: store_dir
                .join("checkpoints")
                .join(workspace_key(workspace))
                .join("copies"),
        }
    }
}

impl CheckpointEngine for CopySnapshot {
    fn backend_name(&self) -> &'static str {
        "copy"
    }

    fn snapshot(&self, label: &str, files: &[String]) -> Result<Checkpoint, CheckpointError> {
        let id = unique_id("cp");
        let dir = self.base.join(&id);
        std::fs::create_dir_all(&dir)?;

        let mut saved = Vec::new();
        for rel in files {
            let src = self.workspace.join(rel);
            // Arquivo ainda não existe (vai ser criado): registramos a
            // ausência para que restaurar signifique apagá-lo.
            if !src.is_file() {
                saved.push(rel.clone());
                continue;
            }
            let dst = dir.join(rel);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&src, &dst)?;
            saved.push(rel.clone());
        }
        std::fs::write(
            dir.join("_checkpoint.json"),
            serde_json::to_string(&saved).unwrap_or_else(|_| "[]".into()),
        )?;

        Ok(Checkpoint {
            id,
            backend: "copy".into(),
            ref_id: dir.to_string_lossy().into_owned(),
            label: label.to_string(),
            files: saved,
        })
    }

    fn restore(&self, cp: &Checkpoint) -> Result<Vec<String>, CheckpointError> {
        let dir = PathBuf::from(&cp.ref_id);
        if !dir.is_dir() {
            return Err(CheckpointError::NotFound(cp.id.clone()));
        }
        let mut restored = Vec::new();
        for rel in &cp.files {
            let backup = dir.join(rel);
            let target = self.workspace.join(rel);
            if backup.is_file() {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(&backup, &target)?;
                restored.push(rel.clone());
            } else if target.exists() {
                // Não havia arquivo no momento da foto: desfazer é remover.
                std::fs::remove_file(&target)?;
                restored.push(rel.clone());
            }
        }
        Ok(restored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn read(root: &Path, rel: &str) -> String {
        std::fs::read_to_string(root.join(rel)).unwrap()
    }

    #[test]
    fn copy_snapshot_restores_modified_file() {
        let ws = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        write(ws.path(), "a.txt", "original");

        let eng = CopySnapshot::new(ws.path(), store.path());
        let cp = eng.snapshot("antes", &["a.txt".into()]).unwrap();

        write(ws.path(), "a.txt", "estragado");
        assert_eq!(read(ws.path(), "a.txt"), "estragado");

        let restored = eng.restore(&cp).unwrap();
        assert_eq!(restored, vec!["a.txt".to_string()]);
        assert_eq!(read(ws.path(), "a.txt"), "original");
    }

    #[test]
    fn copy_snapshot_undo_of_created_file_removes_it() {
        let ws = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();

        let eng = CopySnapshot::new(ws.path(), store.path());
        // Foto tirada ANTES de o arquivo existir.
        let cp = eng.snapshot("antes", &["novo.md".into()]).unwrap();
        write(ws.path(), "novo.md", "conteúdo");
        assert!(ws.path().join("novo.md").exists());

        eng.restore(&cp).unwrap();
        assert!(!ws.path().join("novo.md").exists(), "deve remover o criado");
    }

    #[test]
    fn copy_snapshot_handles_nested_paths() {
        let ws = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        write(ws.path(), "src/deep/x.rs", "v1");

        let eng = CopySnapshot::new(ws.path(), store.path());
        let cp = eng.snapshot("antes", &["src/deep/x.rs".into()]).unwrap();
        write(ws.path(), "src/deep/x.rs", "v2");
        eng.restore(&cp).unwrap();
        assert_eq!(read(ws.path(), "src/deep/x.rs"), "v1");
    }

    #[test]
    fn git_shadow_never_touches_user_repo() {
        if !git_available() {
            eprintln!("git ausente — teste ignorado");
            return;
        }
        let ws = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        write(ws.path(), "a.txt", "v1");

        // Repositório do usuário, com um commit próprio.
        let user_git = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(ws.path())
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("HOME", store.path())
                .output()
                .unwrap()
        };
        user_git(&["init", "--quiet"]);
        user_git(&["config", "user.email", "u@x"]);
        user_git(&["config", "user.name", "u"]);
        user_git(&["add", "."]);
        user_git(&["commit", "-m", "commit do usuário", "--quiet"]);
        let head_before = String::from_utf8_lossy(&user_git(&["rev-parse", "HEAD"]).stdout)
            .trim()
            .to_string();

        let eng = GitShadow::new(ws.path(), store.path()).unwrap();
        let cp = eng.snapshot("antes do agente", &[]).unwrap();
        write(ws.path(), "a.txt", "v2");
        write(ws.path(), "b.txt", "novo");
        eng.snapshot("depois", &[]).unwrap();
        eng.restore(&cp).unwrap();

        assert_eq!(read(ws.path(), "a.txt"), "v1", "arquivo deve voltar");

        // O repositório do usuário continua exatamente como estava.
        let head_after = String::from_utf8_lossy(&user_git(&["rev-parse", "HEAD"]).stdout)
            .trim()
            .to_string();
        assert_eq!(head_before, head_after, "HEAD do usuário não pode mudar");
        assert!(ws.path().join(".git").is_dir(), ".git do usuário intacto");
        // E nosso repositório vive fora do projeto.
        assert!(!ws.path().join("shadow.git").exists());
    }

    #[test]
    fn workspace_key_is_stable_and_distinct() {
        let a = workspace_key(Path::new("/home/u/projeto"));
        let b = workspace_key(Path::new("/home/u/projeto"));
        let c = workspace_key(Path::new("/home/u/outro"));
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.starts_with("projeto-"));
    }

    #[test]
    fn engine_for_always_returns_something() {
        let ws = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        let eng = engine_for(ws.path(), store.path()).unwrap();
        assert!(matches!(eng.backend_name(), "git" | "copy"));
    }
}
