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

mod card;
mod download;
mod gguf_local;
mod hf;
mod local;
mod oauth;
pub use download::{
    DownloadEvent, DownloadManager, DownloadRequest, DownloadState, DownloadStatus,
    artifact_on_disk, download_id,
};
pub use gguf_local::{GGML_TYPE_PQ2_0, LocalGgufMeta, expert_slot_bytes, read_local_meta};
pub use hf::{BaseConfig, GgufRepoMeta, HfAccess, HfClient, HfIdentity, HfWhoami, SortBy};
pub use local::{LocalArtifact, scan_local};
pub use oauth::{HfSession, Login, OauthClient, refresh as oauth_refresh};

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
    /// Licença declarada no cartão do repositório (`apache-2.0`, `mit`, …).
    pub license: Option<String>,
    /// O que o modelo sabe fazer, derivado do que o Hub já entrega — a
    /// interface não precisa do chat template inteiro (dezenas de KB por
    /// modelo) para desenhar três selos.
    pub caps: ModelCaps,
}

/// Capacidades de um modelo, cada uma com uma fonte verificável.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCaps {
    /// Entende imagem: o `pipeline_tag` do Hub diz `image-text-to-text`.
    pub vision: bool,
    /// Chama ferramentas: o chat template tem um ramo para `tools`.
    pub tools: bool,
    /// Raciocina antes de responder, e o raciocínio pode ser desligado — a
    /// mesma marca (`enable_thinking`) que o app procura no GGUF baixado.
    pub reasoning: bool,
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
/// Este arquivo é um projetor de visão, e não um modelo?
///
/// Ele é `.gguf` como qualquer outro e o nome costuma carregar um rótulo de
/// quantização (`mmproj-F16.gguf`), então sem esta checagem ele virava uma
/// "quantização recomendada" — pequena, portanto "cabe inteiro na placa" —,
/// entrava no catálogo do motor e falhava ao carregar. É acessório do modelo,
/// não alternativa a ele.
pub fn is_vision_projector(path: &str) -> bool {
    let base = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_lowercase();
    base.starts_with("mmproj") || base.contains("mmproj-") || base.contains("-mmproj")
}

/// GGUF que acompanha o modelo mas não é um: a matriz de importância da
/// quantização (`imatrix*.gguf`) e a cabeça MTP separada (`MTP/mtp-*.gguf`),
/// que só serve de rascunho para o modelo principal. O modelo que embute a
/// cabeça (`…-MTP-Q4_K_M.gguf`) continua sendo modelo.
pub fn is_auxiliary_gguf(path: &str) -> bool {
    let mut partes = path.rsplit(['/', '\\']);
    let base = partes.next().unwrap_or(path).to_lowercase();
    let na_pasta_mtp = partes.next().is_some_and(|p| p.eq_ignore_ascii_case("mtp"));
    base.starts_with("imatrix") || base.starts_with("mtp-") || na_pasta_mtp
}

/// Como a biblioteca vai chamar um artefato do Hub depois de baixado: o
/// agrupamento pelos nomes dos arquivos sem a subpasta do repositório, que é
/// o que [`scan_local`] enxerga em cada pasta. É o nome que o Chat recebe.
pub fn local_name(files: &[RepoFile]) -> Option<String> {
    let soltos: Vec<RepoFile> = files
        .iter()
        .map(|f| RepoFile {
            path: f.path.rsplit('/').next().unwrap_or(&f.path).to_string(),
            size_bytes: f.size_bytes,
        })
        .collect();
    group_artifacts(&soltos).into_iter().next().map(|a| a.name)
}

pub fn group_artifacts(files: &[RepoFile]) -> Vec<GgufArtifact> {
    use std::collections::BTreeMap;

    let mut singles = Vec::new();
    let mut sharded: BTreeMap<(String, u32), Vec<(u32, RepoFile)>> = BTreeMap::new();

    for f in files {
        if !f.path.to_lowercase().ends_with(".gguf")
            || is_vision_projector(&f.path)
            || is_auxiliary_gguf(&f.path)
        {
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

/// Projetores de visão do repositório, do menor para o maior.
///
/// Ficam separados dos modelos: são o que a visão sob demanda vai carregar
/// junto quando houver imagem.
pub fn vision_projectors(files: &[RepoFile]) -> Vec<RepoFile> {
    let mut out: Vec<RepoFile> = files
        .iter()
        .filter(|f| f.path.to_lowercase().ends_with(".gguf") && is_vision_projector(&f.path))
        .cloned()
        .collect();
    out.sort_by_key(|f| f.size_bytes);
    out
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

    /// O Hub guarda algumas quantizações numa subpasta; a biblioteca vê a
    /// pasta e chama o artefato pelo nome dos arquivos dela. O nome que o
    /// Chat recebe tem de ser esse.
    #[test]
    fn o_nome_local_ignora_a_subpasta_do_repositorio() {
        let na_subpasta = [f("UD-Q4_K_XL/Qwen3-8B-UD-Q4_K_XL.gguf", 5)];
        let na_raiz = [f("Qwen3-8B-UD-Q4_K_XL.gguf", 5)];
        assert!(local_name(&na_subpasta).is_some());
        assert_eq!(local_name(&na_subpasta), local_name(&na_raiz));
        let shards = [
            f("Q2_K/Qwen3-235B-Q2_K-00001-of-00002.gguf", 5),
            f("Q2_K/Qwen3-235B-Q2_K-00002-of-00002.gguf", 5),
        ];
        let local = scan_names(&[
            f("Qwen3-235B-Q2_K-00001-of-00002.gguf", 5),
            f("Qwen3-235B-Q2_K-00002-of-00002.gguf", 5),
        ]);
        assert_eq!(local_name(&shards), local);
        assert_eq!(local_name(&[]), None);
    }

    fn scan_names(files: &[RepoFile]) -> Option<String> {
        group_artifacts(files).into_iter().next().map(|a| a.name)
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

    /// O projetor de visão é `.gguf`, é pequeno e o nome traz um rótulo de
    /// quantização: sem separá-lo, ele virava a "quantização recomendada" de
    /// todo repositório multimodal e um modelo que não carrega.
    #[test]
    fn the_vision_projector_is_not_a_model() {
        let arquivos = [
            f("gemma-3-27b-it-Q4_K_M.gguf", 17_000_000_000),
            f("mmproj-F16.gguf", 931_000_000),
            f("mmproj-model-f16.gguf", 900_000_000),
            f("subpasta/gemma-3-mmproj-BF16.gguf", 800_000_000),
        ];
        let modelos = group_artifacts(&arquivos);
        assert_eq!(modelos.len(), 1, "{modelos:?}");
        assert_eq!(modelos[0].name, "gemma-3-27b-it-Q4_K_M.gguf");

        let projetores = vision_projectors(&arquivos);
        assert_eq!(projetores.len(), 3);
        // Do menor para o maior: é o que a visão sob demanda vai preferir.
        assert_eq!(projetores[0].path, "subpasta/gemma-3-mmproj-BF16.gguf");
    }

    /// A cabeça MTP separada e a imatrix também são `.gguf` com rótulo de
    /// quantização: sem separá-las, o Qwen3.8-27B oferecia um "Q4_0 de 1,3
    /// GB" que não é modelo nenhum — e o primeiro uso o sugeria para a CPU.
    #[test]
    fn the_mtp_head_and_the_imatrix_are_not_models() {
        let arquivos = [
            f("Qwen3.8-27B-Q4_0.gguf", 14_950_000_000),
            f("MTP/mtp-Qwen3.8-27B-Q4_0.gguf", 1_280_000_000),
            f("imatrix_unsloth.gguf", 10_000_000),
            f("Qwen3.6-35B-A3B-MTP-UD-Q4_K_M.gguf", 21_000_000_000),
        ];
        let mut nomes: Vec<String> = group_artifacts(&arquivos)
            .into_iter()
            .map(|a| a.name)
            .collect();
        nomes.sort();
        assert_eq!(
            nomes,
            [
                "Qwen3.6-35B-A3B-MTP-UD-Q4_K_M.gguf",
                "Qwen3.8-27B-Q4_0.gguf"
            ]
        );
        assert!(is_auxiliary_gguf("mtp\\mtp-x-Q8_0.gguf"));
        assert!(!is_auxiliary_gguf("modelos/Qwen3-MTP-Q4_K_M.gguf"));
    }

    #[test]
    fn a_model_named_after_something_else_is_still_a_model() {
        assert!(!is_vision_projector("Qwen3-8B-Q4_K_M.gguf"));
        assert!(!is_vision_projector("modelo-projecao.gguf"));
        assert!(is_vision_projector("MMPROJ-F16.GGUF"));
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
