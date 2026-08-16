//! Varredura da pasta do projeto: o que entra no índice e, principalmente,
//! o que NUNCA entra.
//!
//! A lista de exclusão de segurança não é conveniência, é contenção: um
//! `.env` indexado vira trecho recuperável, o trecho vira contexto do modelo e
//! o segredo sai pelo prompt. Como o índice também alimenta a resposta que o
//! usuário lê e copia, credencial indexada é credencial vazada. Por isso a
//! recusa acontece aqui, antes de qualquer leitura, e por nome — sem depender
//! de o projeto ter um `.gitignore` correto.
//!
//! O resto das exclusões é economia: binário não tem texto para casar,
//! `node_modules` não é código do usuário, e arquivo acima de 5 MB é
//! quase sempre dado, não fonte.

use ignore::WalkBuilder;
use std::path::{Path, PathBuf};

use crate::RagError;

/// Acima disto o arquivo é dado, não código. Ler 5 MB para chunkar em 3 mil
/// trechos custa caro e polui o índice.
pub const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

/// Quanto se lê do começo do arquivo para decidir se é binário.
const SNIFF_BYTES: usize = 8192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Caminho relativo à raiz do projeto, sempre com `/`.
    pub rel_path: String,
    pub abs_path: PathBuf,
    pub size: u64,
    /// Modificação em segundos desde a época.
    pub mtime: i64,
}

/// Pastas que não são código do usuário. O crate `ignore` já respeita o
/// `.gitignore`, mas nem todo projeto ignora tudo isso (e há projeto sem git).
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".jj",
    "node_modules",
    "bower_components",
    "target",
    "dist",
    "build",
    "out",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".turbo",
    ".parcel-cache",
    ".venv",
    "venv",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".tox",
    ".gradle",
    ".idea",
    "Pods",
    "DerivedData",
    "vendor",
    ".terraform",
    ".cargo",
    ".ssh",
    ".gnupg",
    ".aws",
];

/// Nomes de arquivo que guardam credencial. Comparação exata, minúsculas.
const SECRET_NAMES: &[&str] = &[
    ".env",
    ".envrc",
    ".netrc",
    "_netrc",
    ".npmrc",
    ".yarnrc",
    ".pypirc",
    ".dockercfg",
    ".docker-config.json",
    ".pgpass",
    ".htpasswd",
    ".git-credentials",
    "credentials",
    "credentials.json",
    "client_secret.json",
    "service-account.json",
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
    "identity",
];

/// Extensões de material criptográfico. Nunca indexadas.
const SECRET_EXTS: &[&str] = &[
    "key",
    "pem",
    "p12",
    "pfx",
    "pkcs12",
    "jks",
    "keystore",
    "truststore",
    "ppk",
    "kdbx",
    "gpg",
    "asc",
    "der",
    "crt",
    "cer",
    "csr",
    "keychain",
];

/// Extensões binárias conhecidas: nem vale abrir para farejar.
const BINARY_EXTS: &[&str] = &[
    "png",
    "jpg",
    "jpeg",
    "gif",
    "bmp",
    "ico",
    "icns",
    "webp",
    "avif",
    "tiff",
    "psd",
    "ai",
    "eps",
    "pdf",
    "zip",
    "gz",
    "tgz",
    "bz2",
    "xz",
    "zst",
    "7z",
    "rar",
    "jar",
    "war",
    "ear",
    "exe",
    "dll",
    "so",
    "dylib",
    "a",
    "lib",
    "o",
    "obj",
    "pdb",
    "class",
    "pyc",
    "pyo",
    "wasm",
    "node",
    "bin",
    "dat",
    "db",
    "sqlite",
    "sqlite3",
    "mdb",
    "iso",
    "dmg",
    "img",
    "mp3",
    "mp4",
    "m4a",
    "aac",
    "avi",
    "mov",
    "mkv",
    "webm",
    "wav",
    "flac",
    "ogg",
    "ttf",
    "otf",
    "woff",
    "woff2",
    "eot",
    "gguf",
    "safetensors",
    "onnx",
    "pt",
    "pth",
    "ckpt",
    "npy",
    "npz",
    "parquet",
    "arrow",
    "feather",
    "pack",
    "idx",
    "lockb",
];

fn lower_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_lowercase()
}

fn extension_of(name: &str) -> Option<&str> {
    let idx = name.rfind('.')?;
    if idx == 0 || idx + 1 >= name.len() {
        return None;
    }
    Some(&name[idx + 1..])
}

/// Este caminho pode conter credencial? Recusa por nome, extensão ou por estar
/// numa pasta sensível — em qualquer nível do caminho relativo.
pub fn is_sensitive_path(rel_path: &str) -> bool {
    let rel = rel_path.replace('\\', "/").to_lowercase();
    // Pastas que existem justamente para guardar credencial: tudo lá dentro
    // é suspeito, independentemente do nome do arquivo.
    for segment in rel.split('/').filter(|s| !s.is_empty()) {
        if matches!(
            segment,
            ".ssh" | ".gnupg" | ".aws" | ".azure" | ".kube" | "secrets" | ".secrets"
        ) {
            return true;
        }
    }

    let name = lower_name(&rel);
    if SECRET_NAMES.contains(&name.as_str()) {
        return true;
    }
    // `.env.local`, `.env.production`, `.env.test.local`...
    if name.starts_with(".env") {
        return true;
    }
    // `id_rsa.pub`, `id_ed25519_work`, ...
    if name.starts_with("id_rsa") || name.starts_with("id_ed25519") || name.starts_with("id_ecdsa")
    {
        return true;
    }
    if name.starts_with("secrets.") || name.starts_with("secret.") {
        return true;
    }
    if let Some(ext) = extension_of(&name)
        && SECRET_EXTS.contains(&ext)
    {
        return true;
    }
    false
}

/// Extensão notoriamente binária?
pub fn is_binary_extension(rel_path: &str) -> bool {
    let name = lower_name(&rel_path.replace('\\', "/"));
    extension_of(&name).is_some_and(|e| BINARY_EXTS.contains(&e))
}

/// Heurística de binário pelo conteúdo: byte nulo é o sinal mais confiável
/// (nenhum texto UTF-8 válido tem um), e um excesso de bytes de controle
/// denuncia o resto.
pub fn looks_binary(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(SNIFF_BYTES)];
    if sample.is_empty() {
        return false;
    }
    if sample.contains(&0) {
        return true;
    }
    let control = sample
        .iter()
        .filter(|b| **b < 0x09 || (**b > 0x0d && **b < 0x20))
        .count();
    control * 100 / sample.len() > 10
}

/// Este arquivo pode ser indexado? Decide só pelo caminho e pelo tamanho —
/// o conteúdo é conferido na hora da leitura.
pub fn should_index(rel_path: &str, size: u64) -> bool {
    !is_sensitive_path(rel_path)
        && !is_binary_extension(rel_path)
        && size <= MAX_FILE_BYTES
        && size > 0
}

/// Modificação em NANOSSEGUNDOS desde a época.
///
/// Segundos seriam suficientes para quase tudo, mas duas edições no mesmo
/// segundo com o mesmo tamanho passariam batido pela comparação rápida. O
/// nanossegundo elimina esse ponto cego onde o sistema de arquivos oferece
/// resolução para tanto (e, onde não oferece, o hash ainda segura a barra).
fn mtime_of(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

/// Lista os arquivos indexáveis do projeto, em ordem estável.
///
/// Respeita `.gitignore` (via crate `ignore`) e ainda assim aplica as
/// exclusões próprias: um projeto sem git, ou com `.gitignore` incompleto, não
/// pode virar vazamento.
pub fn scan_workspace(root: &Path) -> Result<Vec<FileEntry>, RagError> {
    if !root.is_dir() {
        return Err(RagError::NoWorkspace);
    }
    let mut out = Vec::new();

    let walker = WalkBuilder::new(root)
        .hidden(false) // Vemos os ocultos de propósito — e recusamos por nome.
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        // Sem isto o `.gitignore` só valeria dentro de um repositório de fato.
        // Projeto ainda não versionado também merece ter seu `dist/` fora.
        .require_git(false)
        .parents(false)
        .follow_links(false) // Link simbólico pode sair do projeto (ou dar laço).
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy().to_lowercase();
            let is_dir = e.file_type().is_some_and(|t| t.is_dir());
            !(is_dir && SKIP_DIRS.contains(&name.as_str()))
        })
        .build();

    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let abs = entry.path();
        let rel = match abs.strip_prefix(root) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        let Ok(meta) = entry.metadata() else { continue };
        if !should_index(&rel, meta.len()) {
            continue;
        }
        out.push(FileEntry {
            rel_path: rel,
            abs_path: abs.to_path_buf(),
            size: meta.len(),
            mtime: mtime_of(&meta),
        });
    }

    out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(out)
}

/// Lê o arquivo como texto. Devolve `None` para binário — a decisão final,
/// tomada com o conteúdo em mãos.
pub fn read_text(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if looks_binary(&bytes) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// Hash do conteúdo. É o que separa "arquivo salvo de novo" (mtime muda, hash
/// não) de "arquivo alterado" — e evita reindexar o primeiro caso.
pub fn content_hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(text.as_bytes());
    format!("{:x}", h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn secrets_are_never_indexable() {
        for bad in [
            ".env",
            ".env.local",
            ".env.production",
            "config/.env",
            "server.key",
            "certs/server.key",
            "cert.pem",
            "keys/private.pem",
            "id_rsa",
            "home/.ssh/id_rsa",
            "id_rsa.pub",
            "id_ed25519",
            "bundle.p12",
            "store.pfx",
            "app.keystore",
            ".npmrc",
            ".netrc",
            ".pgpass",
            "credentials",
            "aws/credentials",
            ".aws/config",
            ".kube/config",
            ".git-credentials",
            "secrets.json",
            "secrets/api.txt",
            "client_secret.json",
        ] {
            assert!(is_sensitive_path(bad), "deveria recusar: {bad}");
            assert!(!should_index(bad, 10), "deveria recusar: {bad}");
        }
    }

    #[test]
    fn normal_source_files_are_indexable() {
        for ok in [
            "src/main.rs",
            "src/lib/agent/types.ts",
            "README.md",
            "Cargo.toml",
            ".github/workflows/ci.yml",
            "docs/keyboard.md",
            "src/monkey.ts",
        ] {
            assert!(!is_sensitive_path(ok), "não deveria recusar: {ok}");
            assert!(should_index(ok, 1024), "não deveria recusar: {ok}");
        }
    }

    #[test]
    fn big_and_binary_files_are_skipped() {
        assert!(!should_index("data/dump.json", MAX_FILE_BYTES + 1));
        assert!(should_index("data/dump.json", MAX_FILE_BYTES));
        assert!(!should_index("assets/logo.png", 100));
        assert!(!should_index("target/app.exe", 100));
        assert!(!should_index("model.gguf", 100));
        // Arquivo vazio não gera trecho nenhum.
        assert!(!should_index("src/vazio.rs", 0));
    }

    #[test]
    fn binary_sniffing_catches_nul_bytes() {
        assert!(looks_binary(&[0x7f, 0x45, 0x4c, 0x46, 0x00, 0x01]));
        assert!(!looks_binary(b"fn main() {}\n"));
        assert!(!looks_binary("acentuação e emoji \u{1F600}".as_bytes()));
        assert!(!looks_binary(b""));
    }

    #[test]
    fn scan_respects_gitignore_and_skip_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        fs::create_dir_all(root.join("build")).unwrap();
        fs::write(root.join(".gitignore"), "ignorado.txt\n").unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();
        fs::write(root.join("ignorado.txt"), "não deve entrar").unwrap();
        fs::write(root.join("node_modules/pkg/index.js"), "module.exports={}").unwrap();
        fs::write(root.join("build/out.js"), "var a=1").unwrap();
        fs::write(root.join(".env"), "TOKEN=segredo").unwrap();
        fs::write(root.join("server.key"), "-----BEGIN PRIVATE KEY-----").unwrap();

        let files = scan_workspace(root).unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.rel_path.as_str()).collect();

        assert!(paths.contains(&"src/main.rs"));
        assert!(paths.contains(&".gitignore"));
        assert!(!paths.contains(&"ignorado.txt"), "gitignore ignorado");
        assert!(!paths.iter().any(|p| p.starts_with("node_modules/")));
        assert!(!paths.iter().any(|p| p.starts_with("build/")));
        assert!(!paths.contains(&".env"), "SEGREDO INDEXADO: .env");
        assert!(!paths.contains(&"server.key"), "SEGREDO INDEXADO: .key");
    }

    #[test]
    fn scan_rejects_a_path_that_is_not_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        fs::write(&file, "x").unwrap();
        assert!(scan_workspace(&file).is_err());
    }

    #[test]
    fn read_text_refuses_binary_content() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("dados.out");
        fs::write(&bin, [0x00u8, 0x01, 0x02, 0x03]).unwrap();
        assert!(read_text(&bin).is_none());

        let txt = dir.path().join("a.rs");
        fs::write(&txt, "fn main() {}").unwrap();
        assert_eq!(read_text(&txt).as_deref(), Some("fn main() {}"));
    }

    #[test]
    fn content_hash_changes_with_content() {
        let a = content_hash("um");
        assert_eq!(a, content_hash("um"));
        assert_ne!(a, content_hash("dois"));
    }
}
