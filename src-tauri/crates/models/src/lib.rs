//! Cliente do Hugging Face Hub para modelos GGUF + gerenciador de downloads.
//!
//! Fatos da API (verificados ao vivo em 2026-08-15):
//! - Busca: `GET /api/models?filter=gguf` (`library=gguf` é IGNORADO pela API
//!   JSON!) + `search`, `author`, `sort` (downloads|likes|trendingScore|...),
//!   `limit`, `expand[]=gguf` (nº de parâmetros, arquitetura, context_length),
//!   `expand[]=gated`. Paginação via header `Link` (cursor opaco).
//! - Arquivos e tamanhos: `GET /api/models/{id}/tree/main?recursive=true`
//!   (campos `size` e `lfs.size`).
//! - Download: `GET /{repo}/resolve/main/{file}` → 302 para CDN assinada;
//!   aceita `Range` (resume); URL expira → re-resolver ao retomar.
//! - Shards `-00001-of-0000N.gguf`: baixar todos, apontar llama.cpp para o
//!   primeiro.
//! - Rate limits por janelas de 5 min; token gratuito dobra os limites e
//!   destrava modelos gated (401 → aceitar licença no site).

use serde::{Deserialize, Serialize};

mod download;
mod hf;
mod local;
pub use download::{
    DownloadEvent, DownloadManager, DownloadRequest, DownloadState, DownloadStatus, download_id,
};
pub use hf::{GgufRepoMeta, HfClient, SortBy};
pub use local::{LocalArtifact, scan_local};

pub const HF_BASE: &str = "https://huggingface.co";

#[derive(Debug, thiserror::Error)]
pub enum ModelsError {
    #[error("falha de rede: {0}")]
    Network(#[from] reqwest::Error),
    #[error("resposta inesperada da API: {0}")]
    Api(String),
    #[error("falha de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("modelo gated: aceite a licença em huggingface.co e informe um token")]
    Gated,
}

/// Resumo de um repositório de modelo, para os cards da tela Descobrir.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSummary {
    pub id: String,
    pub author: String,
    pub name: String,
    pub downloads: u64,
    pub likes: u64,
    pub params_total: Option<u64>,
    pub architecture: Option<String>,
    pub context_length: Option<u64>,
    pub gated: bool,
    pub updated_at: Option<String>,
}

/// Um arquivo do repositório (da árvore), com tamanho real.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoFile {
    pub path: String,
    pub size_bytes: u64,
}

/// Um item baixável: um GGUF único ou um conjunto completo de shards.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GgufArtifact {
    /// Nome de exibição (arquivo único ou base do conjunto de shards).
    pub name: String,
    /// Arquivos a baixar, em ordem; o primeiro é o que o llama.cpp abre.
    pub files: Vec<RepoFile>,
    pub total_bytes: u64,
}

/// Agrupa a árvore de um repo em artefatos GGUF baixáveis: arquivos soltos
/// viram artefatos de 1 arquivo; shards `-NNNNN-of-NNNNN` viram um artefato
/// com todos os pedaços (incompletos são descartados).
pub fn group_artifacts(files: &[RepoFile]) -> Vec<GgufArtifact> {
    use std::collections::BTreeMap;

    let mut singles = Vec::new();
    let mut sharded: BTreeMap<(String, u32), Vec<(u32, RepoFile)>> = BTreeMap::new();

    for f in files {
        if !f.path.to_lowercase().ends_with(".gguf") {
            continue;
        }
        match parse_shard(&f.path) {
            Some((base, idx, total)) => {
                sharded
                    .entry((base, total))
                    .or_default()
                    .push((idx, f.clone()));
            }
            None => singles.push(GgufArtifact {
                name: f.path.clone(),
                files: vec![f.clone()],
                total_bytes: f.size_bytes,
            }),
        }
    }

    for ((base, total), mut parts) in sharded {
        parts.sort_by_key(|(idx, _)| *idx);
        let complete = parts.len() as u32 == total
            && parts
                .iter()
                .enumerate()
                .all(|(i, (idx, _))| *idx == i as u32 + 1);
        if !complete {
            log::warn!(
                "conjunto de shards incompleto ignorado: {base} ({}/{total})",
                parts.len()
            );
            continue;
        }
        let files: Vec<RepoFile> = parts.into_iter().map(|(_, f)| f).collect();
        let total_bytes = files.iter().map(|f| f.size_bytes).sum();
        singles.push(GgufArtifact {
            name: format!("{base}.gguf"),
            files,
            total_bytes,
        });
    }

    singles.sort_by(|a, b| a.name.cmp(&b.name));
    singles
}

/// Reconhece `<base>-00001-of-00003.gguf` → (base, 1, 3).
fn parse_shard(path: &str) -> Option<(String, u32, u32)> {
    let stem = path
        .strip_suffix(".gguf")
        .or_else(|| path.strip_suffix(".GGUF"))?;
    let (rest, total_str) = stem.rsplit_once("-of-")?;
    let total: u32 = total_str.parse().ok()?;
    let (base, idx_str) = rest.rsplit_once('-')?;
    if idx_str.len() != 5 || total_str.len() != 5 {
        return None;
    }
    let idx: u32 = idx_str.parse().ok()?;
    (idx >= 1 && idx <= total).then(|| (base.to_string(), idx, total))
}

/// URL de download (resolve) de um arquivo.
pub fn resolve_url(repo_id: &str, filename: &str) -> String {
    format!("{HF_BASE}/{repo_id}/resolve/main/{filename}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(path: &str, size: u64) -> RepoFile {
        RepoFile {
            path: path.into(),
            size_bytes: size,
        }
    }

    #[test]
    fn parses_shard_names() {
        assert_eq!(
            parse_shard("Qwen3-235B-UD-Q2_K_XL-00001-of-00002.gguf"),
            Some(("Qwen3-235B-UD-Q2_K_XL".into(), 1, 2))
        );
        assert_eq!(parse_shard("model-Q4_K_M.gguf"), None);
        assert_eq!(parse_shard("model-00001-of-abc.gguf"), None);
    }

    #[test]
    fn groups_singles_and_complete_shards() {
        let files = vec![
            f("m-Q4_K_M.gguf", 100),
            f("big-00001-of-00002.gguf", 50),
            f("big-00002-of-00002.gguf", 60),
            f("README.md", 1),
        ];
        let arts = group_artifacts(&files);
        assert_eq!(arts.len(), 2);
        let big = arts.iter().find(|a| a.name == "big.gguf").unwrap();
        assert_eq!(big.total_bytes, 110);
        assert_eq!(big.files[0].path, "big-00001-of-00002.gguf");
    }

    #[test]
    fn incomplete_shard_sets_are_dropped() {
        let files = vec![f("big-00001-of-00003.gguf", 50)];
        assert!(group_artifacts(&files).is_empty());
    }
}
