//! Cliente HTTP da API do Hugging Face Hub.

use crate::{HF_BASE, ModelCaps, ModelSummary, ModelsError, RepoFile};
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
    // O `rename_all = "camelCase"` da struct procuraria `pipelineTag`, e o
    // Hub manda `pipeline_tag` — sem este rename a etiqueta chega sempre
    // vazia e nenhum modelo teria o selo de visão.
    #[serde(default, rename = "pipeline_tag")]
    pipeline_tag: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    card_data: Option<ApiCardData>,
}

/// O cabeçalho do README (`cardData`) — daqui sai a licença.
#[derive(Deserialize)]
struct ApiCardData {
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    license_name: Option<String>,
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
            .user_agent(concat!("OpenWeights/", env!("CARGO_PKG_VERSION")))
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
                    "{HF_BASE}/api/models?filter=gguf&sort={}&direction=-1&limit={}&expand[]=gguf&expand[]=gated&expand[]=downloads&expand[]=likes&expand[]=lastModified&expand[]=pipeline_tag&expand[]=tags&expand[]=cardData",
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

    /// O README do repositório, sem o cabeçalho YAML.
    ///
    /// É o texto que o autor escreveu sobre o modelo — na tela de descoberta,
    /// a única fonte de "o que é isto" que não seja o nome do arquivo. Vem do
    /// `raw`, não da API: o cartão inteiro renderizado seria HTML, e aqui o
    /// que se quer é o Markdown.
    pub async fn readme(&self, repo_id: &str) -> Result<String, ModelsError> {
        let url = format!("{HF_BASE}/{repo_id}/raw/main/README.md");
        let resp = self.auth(self.http.get(&url)).send().await?;
        match resp.status().as_u16() {
            200 => {}
            401 | 403 => return Err(ModelsError::Gated),
            404 => return Ok(String::new()),
            s => return Err(ModelsError::Api(format!("README retornou HTTP {s}"))),
        }
        let texto = resp.text().await?;
        Ok(sem_frontmatter(&texto)
            .chars()
            .take(MAX_README_CHARS)
            .collect())
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
        license: m
            .card_data
            .as_ref()
            .and_then(|c| c.license_name.clone().or_else(|| c.license.clone())),
        caps: capacidades(
            m.pipeline_tag.as_deref(),
            &m.tags,
            m.gguf.as_ref().and_then(|g| g.chat_template.as_deref()),
        ),
    }
}

/// Deriva as capacidades do que o Hub entrega.
///
/// Nenhuma delas é adivinhada pelo NOME do modelo, que erra nos dois
/// sentidos. Visão é o `pipeline_tag` (a etiqueta que o próprio autor
/// escolheu); ferramentas e raciocínio saem do chat template, que é o
/// arquivo que o llama.cpp de fato executa — se ele tem o ramo, a
/// capacidade existe.
fn capacidades(pipeline: Option<&str>, tags: &[String], chat_template: Option<&str>) -> ModelCaps {
    let visao = |s: &str| s == "image-text-to-text" || s == "visual-question-answering";
    let tpl = chat_template.unwrap_or_default();
    ModelCaps {
        vision: pipeline.is_some_and(visao) || tags.iter().any(|t| visao(t)),
        tools: tpl.contains("tools"),
        reasoning: tpl.contains("enable_thinking"),
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

/// Teto do README trazido para a tela. Cartões de modelo chegam a centenas
/// de KB de tabelas de benchmark; o que interessa está no começo, e o resto
/// só custa memória e tempo de renderização.
const MAX_README_CHARS: usize = 60_000;

/// Remove o bloco YAML de metadados do topo do README.
///
/// O cartão do Hub começa com `---` … `---` (licença, tags, modelo base) —
/// informação que a interface já mostra em campos próprios, e que como texto
/// solto abriria a descrição com uma parede de chaves e traços.
fn sem_frontmatter(texto: &str) -> &str {
    let t = texto.trim_start_matches('\u{feff}');
    let Some(resto) = t.strip_prefix("---") else {
        return t;
    };
    // A abertura precisa ser a linha inteira `---`; um `---abc` não é
    // frontmatter, é texto.
    let resto = match resto
        .strip_prefix('\n')
        .or_else(|| resto.strip_prefix("\r\n"))
    {
        Some(r) => r,
        None => return t,
    };
    for (i, linha) in resto.match_indices('\n') {
        let anterior = &resto[..i];
        let fim = anterior.rsplit('\n').next().unwrap_or(anterior).trim_end();
        if fim == "---" || fim == "..." {
            return resto[i + 1..].trim_start();
        }
        let _ = linha;
    }
    // Abriu e não fechou: o documento inteiro seria comido pelo corte, então
    // devolvê-lo como está é o comportamento menos destrutivo.
    t
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

    /// Uma resposta REAL da busca, recortada: se o Hub mudar o nome de um
    /// campo, é aqui que se descobre — e não numa tela que passou a mostrar
    /// "sem licença" e nenhum selo para todo mundo.
    #[test]
    fn a_real_search_row_fills_every_field_the_screen_shows() {
        let json = r#"[{
            "id": "unsloth/Qwen3.8-27B-GGUF",
            "author": "unsloth",
            "downloads": 8839153,
            "likes": 3237,
            "gated": false,
            "lastModified": "2026-08-20T12:04:25.000Z",
            "pipeline_tag": "image-text-to-text",
            "tags": ["gguf", "conversational"],
            "cardData": { "license": "apache-2.0", "base_model": ["Qwen/Qwen3.8-27B"] },
            "gguf": {
                "total": 27320697856,
                "architecture": "qwen35",
                "context_length": 262144,
                "chat_template": "{%- if tools %}...{%- endif %}{%- if enable_thinking %}<think>{%- endif %}"
            }
        }]"#;
        let raw: Vec<ApiModel> = serde_json::from_str(json).expect("desserializa");
        let m = to_summary(raw.into_iter().next().unwrap());

        assert_eq!(m.author, "unsloth");
        assert_eq!(m.name, "Qwen3.8-27B-GGUF");
        assert_eq!(m.downloads, 8_839_153);
        assert_eq!(m.params_total, Some(27_320_697_856));
        assert_eq!(m.architecture.as_deref(), Some("qwen35"));
        assert_eq!(m.context_length, Some(262_144));
        assert_eq!(m.license.as_deref(), Some("apache-2.0"));
        assert!(!m.gated);
        assert_eq!(
            m.caps,
            ModelCaps {
                vision: true,
                tools: true,
                reasoning: true
            }
        );
    }

    /// `license_name` vence `license` quando existe: é o nome específico da
    /// licença própria de um autor ("qwen-community-1.0"), e "other" não diz
    /// nada a ninguém.
    #[test]
    fn a_named_license_wins_over_the_generic_one() {
        let json = r#"[{
            "id": "a/b",
            "cardData": { "license": "other", "license_name": "qwen-community-1.0" }
        }]"#;
        let raw: Vec<ApiModel> = serde_json::from_str(json).expect("desserializa");
        let m = to_summary(raw.into_iter().next().unwrap());
        assert_eq!(m.license.as_deref(), Some("qwen-community-1.0"));
    }

    /// O cartão do Hub começa com um bloco YAML que a tela já mostra em
    /// campos próprios — como texto, ele só empurraria a descrição para
    /// baixo atrás de uma parede de chaves.
    #[test]
    fn the_card_header_is_stripped_from_the_readme() {
        let com = "---\nlicense: apache-2.0\ntags:\n- unsloth\n---\n\n# Qwen3.8\n\nUm modelo.";
        assert_eq!(sem_frontmatter(com), "# Qwen3.8\n\nUm modelo.");

        // Sem cabeçalho, o texto passa inteiro.
        let sem = "# Qwen3.8\n\nUm modelo.";
        assert_eq!(sem_frontmatter(sem), sem);

        // `---` no meio de uma linha não abre bloco nenhum: é texto.
        let falso = "---abc\n# Título";
        assert_eq!(sem_frontmatter(falso), falso);

        // Abriu e nunca fechou: devolver tudo é menos destrutivo que comer o
        // documento inteiro.
        let aberto = "---\nlicense: mit\n\n# Título sem fim";
        assert_eq!(sem_frontmatter(aberto), aberto);

        // Um fechamento com `...` também é YAML válido.
        let pontos = "---\nlicense: mit\n...\n# Título";
        assert_eq!(sem_frontmatter(pontos), "# Título");
    }

    /// Visão vem da etiqueta que o autor escolheu; ferramentas e raciocínio,
    /// do template que o llama.cpp executa. Nada sai do nome do modelo.
    #[test]
    fn capabilities_come_from_the_hub_not_from_the_name() {
        let tpl_pensante = "{%- if enable_thinking %}<think>{%- endif %}{%- if tools %}...";
        let c = capacidades(
            Some("image-text-to-text"),
            &["gguf".to_string()],
            Some(tpl_pensante),
        );
        assert!(c.vision && c.tools && c.reasoning);

        // Um nome cheio de promessas não vale nada sem as fontes.
        let c = capacidades(
            Some("text-generation"),
            &["gguf".to_string()],
            Some("{{ bos_token }}"),
        );
        assert_eq!(c, ModelCaps::default());

        // Sem template, o que sobra é a etiqueta.
        let c = capacidades(None, &["visual-question-answering".to_string()], None);
        assert!(c.vision);
        assert!(!c.tools && !c.reasoning);
    }

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
                pipeline_tag: None,
                tags: Vec::new(),
                card_data: None,
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
