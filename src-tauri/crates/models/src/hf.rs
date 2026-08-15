//! Cliente HTTP da API do Hugging Face Hub.

use crate::{HF_BASE, ModelSummary, ModelsError, RepoFile};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    Trending,
    Downloads,
    Likes,
    Updated,
}

impl SortBy {
    fn param(self) -> &'static str {
        match self {
            SortBy::Trending => "trendingScore",
            SortBy::Downloads => "downloads",
            SortBy::Likes => "likes",
            SortBy::Updated => "lastModified",
        }
    }
}

pub struct HfClient {
    http: reqwest::Client,
    token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiModel {
    id: String,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    likes: u64,
    #[serde(default)]
    gated: serde_json::Value,
    #[serde(default)]
    last_modified: Option<String>,
    #[serde(default)]
    gguf: Option<ApiGguf>,
}

#[derive(Deserialize)]
struct ApiGguf {
    #[serde(default)]
    total: Option<u64>,
    #[serde(default)]
    architecture: Option<String>,
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    chat_template: Option<String>,
}

/// Metadados GGUF de um repositório (via `expand[]=gguf`), sem baixar nada.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GgufRepoMeta {
    pub params_total: Option<u64>,
    pub architecture: Option<String>,
    pub context_length: Option<u64>,
    pub chat_template: Option<String>,
}

#[derive(Deserialize)]
struct ApiTreeEntry {
    #[serde(rename = "type")]
    kind: String,
    path: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    lfs: Option<ApiLfs>,
}

#[derive(Deserialize)]
struct ApiLfs {
    size: u64,
}

impl HfClient {
    pub fn new(token: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("Rift/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client");
        Self { http, token }
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(t) => req.bearer_auth(t),
            None => req,
        }
    }

    /// Busca modelos GGUF. `query` vazio retorna os em alta.
    pub async fn search(
        &self,
        query: &str,
        sort: SortBy,
        limit: u32,
    ) -> Result<Vec<ModelSummary>, ModelsError> {
        Ok(self.search_cursor(query, sort, limit, None).await?.0)
    }

    /// Busca paginada: devolve a página e o cursor da próxima (se houver).
    ///
    /// A API pagina via header `Link` (`rel="next"`); o cursor é a URL
    /// completa da próxima página — quando presente, `query`/`sort`/`limit`
    /// são ignorados e essa URL é usada direto.
    pub async fn search_cursor(
        &self,
        query: &str,
        sort: SortBy,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<(Vec<ModelSummary>, Option<String>), ModelsError> {
        let url = match cursor {
            Some(next) => {
                // O cursor vem do header Link de uma resposta anterior; só
                // aceitamos URLs do próprio Hub (o bearer token vai junto).
                if !next.starts_with(HF_BASE) {
                    return Err(ModelsError::Api(format!("cursor inválido: {next}")));
                }
                next.to_string()
            }
            None => {
                let mut url = format!(
                    "{HF_BASE}/api/models?filter=gguf&sort={}&direction=-1&limit={}&expand[]=gguf&expand[]=gated&expand[]=downloads&expand[]=likes&expand[]=lastModified",
                    sort.param(),
                    limit.clamp(1, 100),
                );
                if !query.trim().is_empty() {
                    url.push_str(&format!("&search={}", urlencode(query.trim())));
                }
                url
            }
        };

        let resp = self.auth(self.http.get(&url)).send().await?;
        if !resp.status().is_success() {
            return Err(ModelsError::Api(format!(
                "busca retornou HTTP {}",
                resp.status()
            )));
        }
        let next = resp
            .headers()
            .get("link")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_link_next);
        let raw: Vec<ApiModel> = resp.json().await?;
        Ok((raw.into_iter().map(to_summary).collect(), next))
    }

    /// Metadados GGUF de um repositório (nº de parâmetros, arquitetura,
    /// context_length, chat_template). `None` quando o repo não expõe o
    /// bloco `gguf`.
    pub async fn gguf_meta(&self, repo_id: &str) -> Result<Option<GgufRepoMeta>, ModelsError> {
        let url = format!("{HF_BASE}/api/models/{repo_id}?expand[]=gguf");
        let resp = self.auth(self.http.get(&url)).send().await?;
        match resp.status().as_u16() {
            200 => {}
            401 | 403 => return Err(ModelsError::Gated),
            s => return Err(ModelsError::Api(format!("metadados retornaram HTTP {s}"))),
        }
        #[derive(Deserialize)]
        struct Resp {
            #[serde(default)]
            gguf: Option<ApiGguf>,
        }
        let raw: Resp = resp.json().await?;
        Ok(raw.gguf.map(|g| GgufRepoMeta {
            params_total: g.total,
            architecture: g.architecture,
            context_length: g.context_length,
            chat_template: g.chat_template,
        }))
    }

    /// Lista os arquivos (com tamanhos) de um repositório.
    pub async fn repo_files(&self, repo_id: &str) -> Result<Vec<RepoFile>, ModelsError> {
        let url = format!("{HF_BASE}/api/models/{repo_id}/tree/main?recursive=true");
        let resp = self.auth(self.http.get(&url)).send().await?;
        match resp.status().as_u16() {
            200 => {}
            401 | 403 => return Err(ModelsError::Gated),
            s => return Err(ModelsError::Api(format!("tree retornou HTTP {s}"))),
        }
        let raw: Vec<ApiTreeEntry> = resp.json().await?;
        Ok(raw
            .into_iter()
            .filter(|e| e.kind == "file")
            .map(|e| RepoFile {
                size_bytes: e.lfs.as_ref().map(|l| l.size).unwrap_or(e.size),
                path: e.path,
            })
            .collect())
    }
}

fn to_summary(m: ApiModel) -> ModelSummary {
    let (author, name) =
        m.id.split_once('/')
            .map(|(a, n)| (a.to_string(), n.to_string()))
            .unwrap_or_else(|| (String::new(), m.id.clone()));
    ModelSummary {
        gated: match &m.gated {
            serde_json::Value::Bool(b) => *b,
            serde_json::Value::String(_) => true,
            _ => false,
        },
        id: m.id,
        author,
        name,
        downloads: m.downloads,
        likes: m.likes,
        params_total: m.gguf.as_ref().and_then(|g| g.total),
        architecture: m.gguf.as_ref().and_then(|g| g.architecture.clone()),
        context_length: m.gguf.as_ref().and_then(|g| g.context_length),
        updated_at: m.last_modified,
    }
}

/// Extrai a URL `rel="next"` de um header `Link` (subset da RFC 8288:
/// entradas separadas por vírgula, parâmetros por ponto e vírgula).
fn parse_link_next(link: &str) -> Option<String> {
    for entry in link.split(',') {
        let mut parts = entry.split(';');
        let url_part = parts.next()?.trim();
        if !(url_part.starts_with('<') && url_part.ends_with('>')) {
            continue;
        }
        let is_next = parts.any(|p| {
            let p = p.trim();
            p.eq_ignore_ascii_case("rel=\"next\"") || p.eq_ignore_ascii_case("rel=next")
        });
        if is_next {
            return Some(url_part[1..url_part.len() - 1].to_string());
        }
    }
    None
}

fn urlencode(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => vec![c],
            ' ' => vec!['+'],
            other => format!("%{:02X}", other as u32).chars().collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencode_basics() {
        assert_eq!(urlencode("qwen 8b"), "qwen+8b");
        assert_eq!(urlencode("c++"), "c%2B%2B");
    }

    #[test]
    fn parses_link_next_header() {
        // formato real da API do Hub: uma entrada só, rel="next"
        let l = "<https://huggingface.co/api/models?filter=gguf&cursor=eyJfaWQiOnt9fQ%3D%3D>; rel=\"next\"";
        assert_eq!(
            parse_link_next(l).as_deref(),
            Some("https://huggingface.co/api/models?filter=gguf&cursor=eyJfaWQiOnt9fQ%3D%3D")
        );
        // múltiplas entradas: escolhe a rel="next"
        let multi = "<https://x/prev>; rel=\"prev\", <https://x/next>; rel=\"next\"";
        assert_eq!(parse_link_next(multi).as_deref(), Some("https://x/next"));
        // rel sem aspas também é aceito
        assert_eq!(
            parse_link_next("<https://x/n>; rel=next").as_deref(),
            Some("https://x/n")
        );
        // sem rel="next" → None
        assert_eq!(parse_link_next("<https://x/prev>; rel=\"prev\""), None);
        assert_eq!(parse_link_next(""), None);
    }

    #[test]
    fn gated_value_variants() {
        for (v, expected) in [
            (serde_json::json!(false), false),
            (serde_json::json!(true), true),
            (serde_json::json!("manual"), true),
            (serde_json::json!("auto"), true),
        ] {
            let m = ApiModel {
                id: "a/b".into(),
                downloads: 0,
                likes: 0,
                gated: v,
                last_modified: None,
                gguf: None,
            };
            assert_eq!(to_summary(m).gated, expected);
        }
    }

    /// Teste live (rede): metadados GGUF + paginação por cursor.
    #[tokio::test]
    #[ignore = "acessa a rede (Hugging Face)"]
    async fn live_gguf_meta_and_search_cursor() {
        let c = HfClient::new(None);

        let meta = c.gguf_meta("Qwen/Qwen3-0.6B-GGUF").await.unwrap();
        let meta = meta.expect("repo GGUF deveria expor o bloco gguf");
        assert!(meta.params_total.unwrap_or(0) > 100_000_000);
        assert!(meta.architecture.is_some());

        let (page1, next) = c
            .search_cursor("qwen", SortBy::Downloads, 5, None)
            .await
            .unwrap();
        assert_eq!(page1.len(), 5);
        let next = next.expect("deveria haver próxima página");
        let (page2, _) = c
            .search_cursor("", SortBy::Downloads, 5, Some(&next))
            .await
            .unwrap();
        assert!(!page2.is_empty());
        assert_ne!(page1[0].id, page2[0].id);
    }
}
