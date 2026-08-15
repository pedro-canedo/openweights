//! Biblioteca local: varre `<models_dir>` em busca de GGUFs baixados.
//!
//! Layout esperado: `<models_dir>/<author>/<repo>/<arquivo>.gguf`, com
//! suporte a arquivos soltos na raiz (importados manualmente pelo usuário).

use crate::{RepoFile, group_artifacts};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalArtifact {
    /// `author/repo` quando veio de um download nosso; vazio para soltos.
    pub repo_id: String,
    pub name: String,
    /// Arquivo que o llama.cpp abre (primeiro shard ou único).
    pub primary_path: PathBuf,
    pub total_bytes: u64,
    pub files: Vec<PathBuf>,
}

/// Varre a biblioteca local (raiz + dois níveis de diretórios).
pub fn scan_local(models_dir: &Path) -> Vec<LocalArtifact> {
    let mut out = Vec::new();
    scan_dir(models_dir, "", &mut out);

    let Ok(authors) = std::fs::read_dir(models_dir) else {
        return out;
    };
    for author in authors.flatten() {
        if !author.path().is_dir() {
            continue;
        }
        let author_name = author.file_name().to_string_lossy().into_owned();
        let Ok(repos) = std::fs::read_dir(author.path()) else {
            continue;
        };
        for repo in repos.flatten() {
            if !repo.path().is_dir() {
                continue;
            }
            let repo_id = format!("{author_name}/{}", repo.file_name().to_string_lossy());
            scan_dir(&repo.path(), &repo_id, &mut out);
        }
    }

    out.sort_by(|a, b| (&a.repo_id, &a.name).cmp(&(&b.repo_id, &b.name)));
    out
}

fn scan_dir(dir: &Path, repo_id: &str, out: &mut Vec<LocalArtifact>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files = Vec::new();
    for e in entries.flatten() {
        let path = e.path();
        if path.is_file()
            && path
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("gguf"))
        {
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            files.push(RepoFile {
                path: e.file_name().to_string_lossy().into_owned(),
                size_bytes: size,
            });
        }
    }
    for art in group_artifacts(&files) {
        out.push(LocalArtifact {
            repo_id: repo_id.to_string(),
            name: art.name.clone(),
            primary_path: dir.join(&art.files[0].path),
            total_bytes: art.total_bytes,
            files: art.files.iter().map(|f| dir.join(&f.path)).collect(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path, size: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![0u8; size]).unwrap();
    }

    #[test]
    fn scans_nested_and_loose_files() {
        let dir = std::env::temp_dir().join(format!("lr-scan-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        touch(&dir.join("solto-Q4_K_M.gguf"), 10);
        touch(&dir.join("unsloth/Qwen3-8B-GGUF/Qwen3-8B-Q4_K_M.gguf"), 20);
        touch(&dir.join("unsloth/Big-GGUF/big-00001-of-00002.gguf"), 5);
        touch(&dir.join("unsloth/Big-GGUF/big-00002-of-00002.gguf"), 7);
        touch(&dir.join("unsloth/Qwen3-8B-GGUF/README.md"), 1);

        let arts = scan_local(&dir);
        assert_eq!(arts.len(), 3);

        let loose = arts.iter().find(|a| a.repo_id.is_empty()).unwrap();
        assert_eq!(loose.name, "solto-Q4_K_M.gguf");

        let big = arts.iter().find(|a| a.name == "big.gguf").unwrap();
        assert_eq!(big.total_bytes, 12);
        assert!(big.primary_path.ends_with("big-00001-of-00002.gguf"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
