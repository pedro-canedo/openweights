//! Biblioteca local: varre `<models_dir>` em busca de GGUFs baixados.
//!
//! Layout esperado: `<models_dir>/<author>/<repo>/<arquivo>.gguf`, com
//! suporte a arquivos soltos na raiz (importados manualmente pelo usuário)
//! e a subpastas dentro do repositório.
//!
//! As subpastas não são um detalhe: repositórios grandes do Hugging Face
//! guardam cada quantização na sua (`UD-Q2_K_XL/`, `UD-Q4_K_XL/`), o
//! download preserva esse caminho, e uma varredura de dois níveis deixaria
//! setenta gigabytes íntegros no disco sem aparecer em lugar nenhum.

use crate::{RepoFile, group_artifacts, vision_projectors};
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
    /// Projetor de visão que veio junto no mesmo repositório, quando há.
    ///
    /// Não é um modelo (por isso não aparece na lista), mas é o que dá olhos
    /// a este aqui — guardar o caminho agora evita ter de varrer o disco de
    /// novo quando a visão sob demanda existir.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vision_projector: Option<PathBuf>,
    pub total_bytes: u64,
    pub files: Vec<PathBuf>,
}

/// Varre a biblioteca local: a raiz, cada `<autor>/<repo>` e as subpastas
/// dentro dele.
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
            scan_tree(&repo.path(), &repo_id, PROFUNDIDADE_MAXIMA, &mut out);
        }
    }

    out.sort_by(|a, b| (&a.repo_id, &a.name).cmp(&(&b.repo_id, &b.name)));
    out
}

/// Quantos níveis descer DENTRO de um repositório.
///
/// Três cobre o que o Hub usa (uma pasta por quantização, às vezes com uma
/// segunda dentro) sem transformar a varredura em caminhada pelo disco
/// inteiro caso alguém aponte a biblioteca para uma pasta grande.
const PROFUNDIDADE_MAXIMA: u32 = 3;

/// Varre `dir` e suas subpastas, mantendo o `repo_id` do repositório: quem
/// identifica o modelo é o arquivo, não a pasta de quantização em que ele mora.
fn scan_tree(dir: &Path, repo_id: &str, restante: u32, out: &mut Vec<LocalArtifact>) {
    scan_dir(dir, repo_id, out);
    if restante == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_tree(&path, repo_id, restante - 1, out);
        }
    }
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
    // O menor projetor da pasta serve a todas as quantizações do mesmo
    // modelo — elas dividem o mesmo repositório.
    let projetor = vision_projectors(&files).first().map(|f| dir.join(&f.path));

    for art in group_artifacts(&files) {
        out.push(LocalArtifact {
            repo_id: repo_id.to_string(),
            name: art.name.clone(),
            primary_path: dir.join(&art.files[0].path),
            total_bytes: art.total_bytes,
            files: art.files.iter().map(|f| dir.join(&f.path)).collect(),
            vision_projector: projetor.clone(),
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

    /// O projetor não entra na lista de modelos, mas fica anotado no modelo
    /// que ele acompanha.
    #[test]
    fn the_projector_rides_along_without_becoming_a_model() {
        let dir = std::env::temp_dir().join(format!("lr-mmproj-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        touch(&dir.join("google/gemma-3-GGUF/gemma-3-Q4_K_M.gguf"), 30);
        touch(&dir.join("google/gemma-3-GGUF/mmproj-F16.gguf"), 5);

        let achados = scan_local(&dir);
        assert_eq!(achados.len(), 1, "{achados:?}");
        assert_eq!(achados[0].name, "gemma-3-Q4_K_M.gguf");
        assert!(
            achados[0]
                .vision_projector
                .as_ref()
                .is_some_and(|p| p.ends_with("mmproj-F16.gguf")),
            "{:?}",
            achados[0].vision_projector
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// O caso real: `unsloth/Qwen3.8-Flash-Next-GGUF` guarda cada quantização
    /// numa subpasta, o download preserva esse caminho, e a varredura de dois
    /// níveis fazia 73 GB baixados sumirem da biblioteca.
    #[test]
    fn a_quantization_subfolder_is_still_part_of_its_repository() {
        let dir = std::env::temp_dir().join(format!("lr-subdir-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let base = dir.join("unsloth/Flash-Next-GGUF");
        touch(
            &base.join("UD-Q2_K_XL/Flash-UD-Q2_K_XL-00001-of-00002.gguf"),
            3,
        );
        touch(
            &base.join("UD-Q2_K_XL/Flash-UD-Q2_K_XL-00002-of-00002.gguf"),
            9,
        );
        touch(&base.join("UD-Q4_K_XL/Flash-UD-Q4_K_XL.gguf"), 20);

        let arts = scan_local(&dir);
        assert_eq!(arts.len(), 2, "{arts:?}");
        // A pasta da quantização não vira repositório: quem identifica o
        // modelo é o repo, e o nome do arquivo já carrega a quantização.
        assert!(arts.iter().all(|a| a.repo_id == "unsloth/Flash-Next-GGUF"));
        let dividido = arts
            .iter()
            .find(|a| a.name == "Flash-UD-Q2_K_XL.gguf")
            .expect("shards agrupados dentro da subpasta");
        assert_eq!(dividido.total_bytes, 12);
        assert!(
            dividido
                .primary_path
                .ends_with("UD-Q2_K_XL/Flash-UD-Q2_K_XL-00001-of-00002.gguf")
        );
        let _ = std::fs::remove_dir_all(&dir);
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
