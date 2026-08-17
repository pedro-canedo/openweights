//! Varredura de arquivos-fonte — o plano B do `files_at_risk`.
//!
//! O checkpoint tira a foto ANTES de a ferramenta rodar, e para isso precisa
//! saber quais arquivos podem mudar. Quando o formatador tem modo conferência,
//! a resposta é exata (ele mesmo lista o que reescreveria). Quando não tem — um
//! `npm run format` que só sabe escrever, um `rubocop -a` — a alternativa
//! honesta é assumir que qualquer fonte da linguagem pode mudar.
//!
//! Errar por excesso aqui é barato (o snapshot guarda arquivos a mais) e errar
//! por falta é caro (o usuário perde a versão anterior de um arquivo
//! reformatado e não tem como voltar). Daí o teto: sem limite, um repositório
//! grande faria a foto demorar minutos; com ele, o pior caso é um checkpoint
//! parcial num projeto gigante — que também é o caso em que o motor de git-
//! sombra, o mais usado, ignora a lista e salva tudo de qualquer jeito.

use ignore::WalkBuilder;
use std::path::Path;

/// Teto de arquivos devolvidos.
pub const MAX_FILES: usize = 400;

/// Pastas que nunca contêm fonte que o formatador do projeto vá reescrever.
const HEAVY_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "vendor",
    ".git",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    ".svelte-kit",
    ".openweights",
];

/// Arquivos com essas extensões dentro de `dir`, em caminho relativo à raiz.
///
/// Respeita `.gitignore` (é o mesmo motor que o `fs_glob` usa), então
/// `node_modules` e `target` ficam de fora sem lista manual.
pub fn source_files(root: &Path, dir: &Path, extensions: &[String], cap: usize) -> Vec<String> {
    if extensions.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();

    let mut walker = WalkBuilder::new(dir);
    walker
        .hidden(true)
        .git_ignore(true)
        // Sem isto, o `.gitignore` só valeria dentro de um repositório git —
        // e projeto recém-criado (ou baixado em zip) cairia com
        // `node_modules` inteiro na lista.
        .require_git(false)
        .parents(true);
    // Cinto e suspensório: pasta pesada conhecida sai mesmo sem `.gitignore`.
    walker.filter_entry(|entry| {
        !entry
            .file_name()
            .to_str()
            .map(|name| HEAVY_DIRS.contains(&name))
            .unwrap_or(false)
    });

    for entry in walker.build().flatten() {
        if out.len() >= cap.min(MAX_FILES) {
            break;
        }
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        let matches = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|ext| extensions.iter().any(|want| want.eq_ignore_ascii_case(ext)))
            .unwrap_or(false);
        if !matches {
            continue;
        }
        if let Some(rel) = relativize(root, path) {
            out.push(rel);
        }
    }
    out.sort();
    out
}

/// Caminho relativo à raiz do projeto, sempre com `/`.
pub fn relativize(root: &Path, path: &Path) -> Option<String> {
    if let Some(rel) = tenta_relativizar(root, path) {
        return Some(rel);
    }
    // Symlink no meio do caminho: no macOS o `TempDir` nasce em
    // `/var/folders/…`, que é link para `/private/var/folders/…`, e as
    // ferramentas (rustfmt, prettier) reportam o caminho JÁ RESOLVIDO. Sem
    // esta segunda tentativa, todo arquivo apontado por elas era descartado
    // como "fora do projeto" — e o modo conferência dizia que não sabia
    // quais arquivos estavam tortos.
    let root_real = root.canonicalize().ok()?;
    let path_real = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    tenta_relativizar(&root_real, &path_real)
}

fn tenta_relativizar(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let text = relative.to_string_lossy().replace('\\', "/");
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// O caso do macOS: a raiz chega pelo symlink (`/var/…`) e a ferramenta
    /// reporta o caminho resolvido (`/private/var/…`). Antes, o arquivo era
    /// descartado como se fosse de fora do projeto.
    #[test]
    fn a_symlinked_root_still_matches_the_resolved_path() {
        let base = tempfile::tempdir().unwrap();
        let real = base.path().join("projeto");
        fs::create_dir_all(real.join("src")).unwrap();
        fs::write(real.join("src/lib.rs"), "").unwrap();

        let link = base.path().join("atalho");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();
        #[cfg(windows)]
        if std::os::windows::fs::symlink_dir(&real, &link).is_err() {
            return; // criar symlink no Windows exige privilégio; sem ele, pula
        }

        // Raiz pelo atalho, caminho pelo destino real: tem de casar.
        let resolvido = real.join("src/lib.rs");
        assert_eq!(relativize(&link, &resolvido).as_deref(), Some("src/lib.rs"));
        // E o caminho de sempre (sem symlink) continua igual.
        assert_eq!(relativize(&real, &resolvido).as_deref(), Some("src/lib.rs"));
        // Fora do projeto continua fora.
        assert!(relativize(&link, Path::new("/outro/x.rs")).is_none());
    }

    #[test]
    fn finds_sources_and_skips_ignored_folders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("node_modules/pacote")).unwrap();
        fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
        fs::write(root.join("src/a.ts"), "").unwrap();
        fs::write(root.join("src/b.tsx"), "").unwrap();
        fs::write(root.join("src/leiame.md"), "").unwrap();
        fs::write(root.join("node_modules/pacote/c.ts"), "").unwrap();

        let found = source_files(root, root, &["ts".into(), "tsx".into()], 100);
        assert_eq!(found, vec!["src/a.ts", "src/b.tsx"], "{found:?}");
    }

    #[test]
    fn the_cap_is_respected_and_no_extension_means_no_files() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..20 {
            fs::write(dir.path().join(format!("f{i}.rs")), "").unwrap();
        }
        assert_eq!(
            source_files(dir.path(), dir.path(), &["rs".into()], 5).len(),
            5
        );
        assert!(source_files(dir.path(), dir.path(), &[], 5).is_empty());
    }

    #[test]
    fn relativize_uses_forward_slashes() {
        let root = Path::new("/projeto");
        assert_eq!(
            relativize(root, Path::new("/projeto/src/a.rs")).as_deref(),
            Some("src/a.rs")
        );
        assert_eq!(relativize(root, Path::new("/outro/a.rs")), None);
    }
}
