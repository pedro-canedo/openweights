//! `web_search`: encontrar endereços para depois ler com `web_fetch`.
//!
//! **Por que o padrão é DuckDuckGo pelo HTML.** A ferramenta tem de funcionar
//! no primeiro minuto de uso, sem cadastro nem cartão. O endpoint `lite` do
//! DuckDuckGo devolve uma página simples, sem chave — em troca, pode bloquear
//! quem busca demais. Esse bloqueio não é falha silenciosa: vira erro dizendo
//! para usar `web_fetch` numa URL específica, que é o caminho que continua
//! aberto.
//!
//! **Por que existe provedor com chave.** Quem usa muito precisa de resposta
//! estável: se houver chave configurada (Brave ou Tavily), ela é usada. A
//! chave vive na configuração do app, nunca neste crate, e vai **só** para o
//! provedor configurado — o cliente dos provedores com chave nem segue
//! redirecionamento, para o cabeçalho de autenticação não ser reenviado a
//! outro host.
//!
//! **Por que o resultado é numerado com título, URL e trecho.** É o formato
//! que o modelo consegue citar. Resultado sem URL vira alegação que ninguém
//! confere — e o trecho vem da internet, então entra cercado pelo aviso de
//! conteúdo não confiável.

use std::sync::Arc;

use async_trait::async_trait;
use lr_tools::{Tool, ToolContext, ToolError, ToolOutput, ToolResult, arg_str, arg_u64};
use lr_types::agent::{ToolCategory, ToolPreview};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::extract::{Node, collapse_ws, tokenize};
use crate::net;
use crate::{WebConfig, untrusted_block};

/// Quantos resultados devolver quando o modelo não pede um número.
const DEFAULT_RESULTS: u64 = 5;

/// Teto de resultados: mais que isso só enche o contexto.
const MAX_RESULTS: u64 = 15;

/// Teto de caracteres de cada trecho.
const SNIPPET_CHARS: usize = 300;

pub const DUCKDUCKGO_ENDPOINT: &str = "https://lite.duckduckgo.com/lite/";
pub const BRAVE_ENDPOINT: &str = "https://api.search.brave.com/res/v1/web/search";
pub const TAVILY_ENDPOINT: &str = "https://api.tavily.com/search";

/// Provedor de busca.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchProvider {
    /// Decide sozinho: com chave, um provedor pago; sem chave, DuckDuckGo.
    #[default]
    Auto,
    DuckDuckGo,
    Brave,
    Tavily,
}

impl SearchProvider {
    /// Nome que aparece para a pessoa e para o modelo.
    pub fn label(&self) -> &'static str {
        match self {
            SearchProvider::Auto => "automático",
            SearchProvider::DuckDuckGo => "DuckDuckGo",
            SearchProvider::Brave => "Brave Search",
            SearchProvider::Tavily => "Tavily",
        }
    }

    /// Adivinha o provedor pelo formato da chave.
    ///
    /// Chave da Tavily começa com `tvly-`; qualquer outra assumimos Brave.
    /// Palpite errado dá erro de autenticação claro, que é melhor do que
    /// exigir dois campos para quem só colou uma chave.
    fn sniff(key: &str) -> SearchProvider {
        if key.trim_start().to_ascii_lowercase().starts_with("tvly") {
            SearchProvider::Tavily
        } else {
            SearchProvider::Brave
        }
    }

    fn default_endpoint(&self) -> &'static str {
        match self {
            SearchProvider::Brave => BRAVE_ENDPOINT,
            SearchProvider::Tavily => TAVILY_ENDPOINT,
            _ => DUCKDUCKGO_ENDPOINT,
        }
    }
}

/// Configuração da busca (parte de [`WebConfig`]).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchConfig {
    pub provider: SearchProvider,
    /// Chave do provedor pago. Nunca é gravada por este crate.
    pub api_key: Option<String>,
    /// Endpoint alternativo (instância própria ou testes).
    pub endpoint: Option<String>,
}

/// Provedor já decidido, com endpoint e chave prontos.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub provider: SearchProvider,
    pub endpoint: String,
    pub api_key: Option<String>,
    /// Explicação quando a escolha não foi a pedida (falta de chave).
    pub note: Option<String>,
}

impl SearchConfig {
    pub fn resolve(&self) -> Resolved {
        let key = self
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty());

        let (provider, note) = match (self.provider, key) {
            (SearchProvider::Auto, Some(k)) => (SearchProvider::sniff(k), None),
            (SearchProvider::Auto, None) => (SearchProvider::DuckDuckGo, None),
            (SearchProvider::DuckDuckGo, _) => (SearchProvider::DuckDuckGo, None),
            (p, Some(_)) => (p, None),
            (p, None) => (
                SearchProvider::DuckDuckGo,
                Some(format!(
                    "o provedor {} está configurado mas sem chave; usei o DuckDuckGo",
                    p.label()
                )),
            ),
        };

        let endpoint = self
            .endpoint
            .as_deref()
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| provider.default_endpoint().to_string());

        Resolved {
            provider,
            endpoint,
            // Chave nunca acompanha o DuckDuckGo: ele não pede nada e o
            // segredo não tem por que sair da máquina para lá.
            api_key: match provider {
                SearchProvider::DuckDuckGo | SearchProvider::Auto => None,
                _ => key.map(str::to_string),
            },
            note,
        }
    }
}

/// Um resultado de busca.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Busca na internet.
pub struct WebSearch {
    config: Arc<WebConfig>,
}

impl WebSearch {
    pub fn new(config: Arc<WebConfig>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for WebSearch {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Busca na internet e devolve uma lista numerada de título, URL e trecho. Use para \
         descobrir ENDEREÇOS sobre um assunto (documentação, erro, versão de biblioteca) e \
         depois leia a página escolhida com `web_fetch`. Não devolve a resposta pronta: \
         devolve onde procurar."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "O que procurar, como você digitaria num buscador. Ex.: rust tokio timeout stream."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Quantos resultados trazer (1 a 15; padrão 5).",
                    "minimum": 1,
                    "maximum": 15
                }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        // Rede: a consulta sai da máquina, então a política sempre pergunta.
        ToolCategory::Network
    }

    async fn preview(&self, args: &Value, _ctx: &ToolContext) -> Option<ToolPreview> {
        let query = arg_str(args, "query").ok()?;
        let resolved = self.config.search.resolve();
        let host = Url::parse(&resolved.endpoint)
            .map(|u| net::host_of(&u))
            .unwrap_or_else(|_| resolved.endpoint.clone());
        let count = arg_u64(args, "max_results", DEFAULT_RESULTS).clamp(1, MAX_RESULTS);
        let mut body = format!(
            "Buscar na internet\nProvedor: {}\nEndereço: {host}\nConsulta enviada: {}\nResultados: até {count}",
            resolved.provider.label(),
            query.trim()
        );
        if resolved.api_key.is_some() {
            // O valor da chave nunca aparece: a prévia fica gravada na trilha.
            body.push_str("\nAutenticação: chave configurada (valor oculto)");
        }
        if let Some(note) = &resolved.note {
            body.push_str(&format!("\nObservação: {note}"));
        }
        Some(ToolPreview::Text { body })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let query = arg_str(&args, "query")?;
        let query = query.trim().to_string();
        if query.is_empty() {
            return Err(ToolError::InvalidArgs(
                "`query` está vazio — diga o que procurar".into(),
            ));
        }
        let count = arg_u64(&args, "max_results", DEFAULT_RESULTS).clamp(1, MAX_RESULTS) as usize;

        let resolved = self.config.search.resolve();
        let timeout = self.config.timeout(None);
        // Provedor com chave não segue redirecionamento: um `Location` para
        // outro host levaria o cabeçalho de autenticação junto.
        let redirects = match resolved.provider {
            SearchProvider::DuckDuckGo | SearchProvider::Auto => self.config.max_redirects,
            _ => 0,
        };
        let client = net::client(timeout, redirects)?;

        let hits = match resolved.provider {
            SearchProvider::Brave => {
                brave(
                    &client,
                    &resolved,
                    &query,
                    count,
                    timeout,
                    self.config.max_response_bytes,
                )
                .await?
            }
            SearchProvider::Tavily => {
                tavily(
                    &client,
                    &resolved,
                    &query,
                    count,
                    timeout,
                    self.config.max_response_bytes,
                )
                .await?
            }
            _ => {
                duckduckgo(
                    &client,
                    &resolved,
                    &query,
                    count,
                    timeout,
                    self.config.max_response_bytes,
                )
                .await?
            }
        };

        let via = resolved.provider.label();
        if hits.is_empty() {
            let mut msg = format!(
                "A busca por \"{query}\" no {via} não trouxe nenhum resultado. Tente outras \
                 palavras (mais específicas ou em inglês), ou use `web_fetch` direto na \
                 documentação oficial se souber o endereço."
            );
            if let Some(note) = &resolved.note {
                msg.push_str(&format!("\n(Observação: {note}.)"));
            }
            return Ok(ToolOutput::text(msg));
        }

        let mut list = String::new();
        for (i, hit) in hits.iter().enumerate() {
            list.push_str(&format!("{}. {}\n   {}\n", i + 1, hit.title, hit.url));
            if !hit.snippet.is_empty() {
                list.push_str(&format!("   {}\n", hit.snippet));
            }
        }

        let mut source = format!(
            "Busca no {via} por \"{query}\" — {} resultado(s).",
            hits.len()
        );
        if let Some(note) = &resolved.note {
            source.push_str(&format!(" ({note})"));
        }

        let body = format!(
            "{}\n\nPara ler qualquer um deles, chame `web_fetch` com a URL. Cite a URL quando \
             usar a informação.",
            untrusted_block(&source, list.trim_end())
        );
        Ok(ToolOutput::text(body).truncated_to(ctx.max_output_bytes))
    }
}

// ------------------------------------------------------------ provedores ---

async fn duckduckgo(
    client: &Client,
    resolved: &Resolved,
    query: &str,
    count: usize,
    timeout: u64,
    max_bytes: usize,
) -> ToolResult<Vec<Hit>> {
    let mut url = net::parse_http_url(&resolved.endpoint)?;
    url.query_pairs_mut().append_pair("q", query);

    let resp = client
        .get(url.clone())
        .header("Accept", "text/html,application/xhtml+xml")
        // Só idioma da resposta. Nada de cookie ou sessão: o cliente não tem
        // armazenamento de cookies, então cada busca sai anônima.
        .header("Accept-Language", "pt-BR,pt;q=0.9,en;q=0.8")
        .send()
        .await
        .map_err(|e| net::send_error(e, &url, timeout))?;

    let status = resp.status().as_u16();
    let (bytes, _) = net::read_body_capped(resp, max_bytes, timeout).await?;
    let html = String::from_utf8_lossy(&bytes);

    if looks_blocked(status, &html) {
        return Err(ToolError::Other(format!(
            "a busca foi bloqueada pelo buscador (HTTP {status}) — normalmente é limite de \
             requisições ou captcha. Tente `web_fetch` numa URL específica (documentação \
             oficial, por exemplo), ou configure uma chave de busca nas configurações."
        )));
    }
    if !(200..300).contains(&status) {
        return Err(ToolError::Other(format!(
            "o buscador respondeu HTTP {status}: {}",
            net::status_hint(status)
        )));
    }

    Ok(parse_duckduckgo(&html, count))
}

/// A página está dizendo "você não" em vez de trazer resultado?
fn looks_blocked(status: u16, html: &str) -> bool {
    if matches!(status, 202 | 403 | 429) {
        return true;
    }
    let lower = html.to_ascii_lowercase();
    [
        "anomaly",
        "captcha",
        "unusual traffic",
        "blocked",
        "are you a robot",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
        && !lower.contains("result-link")
        && !lower.contains("result__a")
}

/// Lê os resultados do HTML do DuckDuckGo (`lite` e `html`).
///
/// Os dois layouts existem e mudam sem aviso; casar pela classe do link
/// (`result-link` / `result__a`) é o que sobrevive a mudança de tabela.
pub fn parse_duckduckgo(html: &str, count: usize) -> Vec<Hit> {
    const LINK_CLASSES: &[&str] = &["result-link", "result__a"];
    const SNIPPET_CLASSES: &[&str] = &["result-snippet", "result__snippet"];

    let nodes = tokenize(html);
    let mut hits: Vec<Hit> = Vec::new();
    let mut i = 0usize;

    while i < nodes.len() {
        let node = &nodes[i];
        match node {
            Node::Open { name, .. } if name == "a" && node.has_class(LINK_CLASSES) => {
                let href = node.attr("href").unwrap_or_default().to_string();
                let (title, next) = text_until_close(&nodes, i + 1, "a");
                i = next;
                if let Some(url) = clean_result_url(&href)
                    && !title.trim().is_empty()
                    && !hits.iter().any(|h| h.url == url)
                {
                    hits.push(Hit {
                        title: collapse_ws(&title),
                        url,
                        snippet: String::new(),
                    });
                }
                continue;
            }
            Node::Open { name, .. } if node.has_class(SNIPPET_CLASSES) => {
                let tag = name.clone();
                let (snippet, next) = text_until_close(&nodes, i + 1, &tag);
                i = next;
                if let Some(last) = hits.last_mut()
                    && last.snippet.is_empty()
                {
                    let (cut, truncated) =
                        crate::extract::truncate_chars(&collapse_ws(&snippet), SNIPPET_CHARS);
                    last.snippet = if truncated { format!("{cut}…") } else { cut };
                }
                continue;
            }
            _ => i += 1,
        }
    }

    hits.truncate(count);
    hits
}

/// Junta o texto até a tag fechar; devolve o texto e a posição seguinte.
fn text_until_close(nodes: &[Node], from: usize, tag: &str) -> (String, usize) {
    let mut text = String::new();
    let mut depth = 1usize;
    let mut i = from;
    while i < nodes.len() {
        match &nodes[i] {
            Node::Open {
                name,
                self_closing: false,
                ..
            } if name == tag => depth += 1,
            Node::Close { name } if name == tag => {
                depth -= 1;
                if depth == 0 {
                    return (text, i + 1);
                }
            }
            Node::Text(t) => {
                if !text.is_empty() && !text.ends_with(' ') {
                    text.push(' ');
                }
                text.push_str(t);
            }
            _ => {}
        }
        i += 1;
    }
    (text, i)
}

/// Desembrulha o link de saída do DuckDuckGo (`/l/?uddg=…`) e descarta anúncio.
pub fn clean_result_url(href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() || href.starts_with('#') {
        return None;
    }
    // Link relativo do próprio buscador vira absoluto para poder ser lido.
    let absolute = if href.starts_with("//") {
        format!("https:{href}")
    } else if href.starts_with('/') {
        format!("https://duckduckgo.com{href}")
    } else {
        href.to_string()
    };

    let url = Url::parse(&absolute).ok()?;
    // Anúncio: sempre passa pelo redirecionador `y.js`.
    if url.path().contains("y.js") {
        return None;
    }
    if let Some((_, target)) = url.query_pairs().find(|(k, _)| k == "uddg") {
        let target = Url::parse(&target).ok()?;
        return matches!(target.scheme(), "http" | "https").then(|| target.to_string());
    }
    matches!(url.scheme(), "http" | "https").then(|| url.to_string())
}

async fn brave(
    client: &Client,
    resolved: &Resolved,
    query: &str,
    count: usize,
    timeout: u64,
    max_bytes: usize,
) -> ToolResult<Vec<Hit>> {
    let mut url = net::parse_http_url(&resolved.endpoint)?;
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("count", &count.to_string());

    let key = resolved.api_key.clone().unwrap_or_default();
    let resp = client
        .get(url.clone())
        .header("Accept", "application/json")
        .header("X-Subscription-Token", key)
        .send()
        .await
        .map_err(|e| net::send_error(e, &url, timeout))?;

    let value = json_body(resp, timeout, max_bytes, SearchProvider::Brave).await?;
    let results = value
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    Ok(results
        .iter()
        .filter_map(|r| {
            Some(Hit {
                title: collapse_ws(r.get("title")?.as_str().unwrap_or_default()),
                url: r.get("url")?.as_str()?.to_string(),
                snippet: snippet_of(r, &["description", "snippet"]),
            })
        })
        .take(count)
        .collect())
}

async fn tavily(
    client: &Client,
    resolved: &Resolved,
    query: &str,
    count: usize,
    timeout: u64,
    max_bytes: usize,
) -> ToolResult<Vec<Hit>> {
    let url = net::parse_http_url(&resolved.endpoint)?;
    let key = resolved.api_key.clone().unwrap_or_default();
    let resp = client
        .post(url.clone())
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {key}"))
        .json(&json!({
            "query": query,
            "max_results": count,
            "include_answer": false,
        }))
        .send()
        .await
        .map_err(|e| net::send_error(e, &url, timeout))?;

    let value = json_body(resp, timeout, max_bytes, SearchProvider::Tavily).await?;
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    Ok(results
        .iter()
        .filter_map(|r| {
            Some(Hit {
                title: collapse_ws(r.get("title")?.as_str().unwrap_or_default()),
                url: r.get("url")?.as_str()?.to_string(),
                snippet: snippet_of(r, &["content", "snippet"]),
            })
        })
        .take(count)
        .collect())
}

fn snippet_of(value: &Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(text) = value.get(*key).and_then(Value::as_str) {
            let (cut, truncated) =
                crate::extract::truncate_chars(&collapse_ws(text), SNIPPET_CHARS);
            return if truncated { format!("{cut}…") } else { cut };
        }
    }
    String::new()
}

/// Lê a resposta de um provedor com chave, traduzindo os erros típicos.
async fn json_body(
    resp: reqwest::Response,
    timeout: u64,
    max_bytes: usize,
    provider: SearchProvider,
) -> ToolResult<Value> {
    let status = resp.status().as_u16();
    let (bytes, _) = net::read_body_capped(resp, max_bytes, timeout).await?;
    let text = String::from_utf8_lossy(&bytes).to_string();

    if matches!(status, 401 | 403) {
        return Err(ToolError::Other(format!(
            "o {} recusou a chave de busca (HTTP {status}). Confira a chave nas configurações \
             do app, ou remova-a para voltar ao DuckDuckGo.",
            provider.label()
        )));
    }
    if !(200..300).contains(&status) {
        let (preview, _) = crate::extract::truncate_chars(text.trim(), 300);
        return Err(ToolError::Other(format!(
            "o {} respondeu HTTP {status}: {}. Resposta: {preview}",
            provider.label(),
            net::status_hint(status)
        )));
    }

    serde_json::from_str(&text).map_err(|e| {
        ToolError::Other(format!(
            "não entendi a resposta do {} ({e}). Tente de novo ou remova a chave para usar o \
             DuckDuckGo.",
            provider.label()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testserver::{FakeResponse, FakeServer};
    use crate::{CONTENT_START, UNTRUSTED_NOTE};

    /// Página no formato do endpoint `lite`, com anúncio e link de saída.
    fn ddg_page() -> String {
        r#"<html><body><table>
        <tr><td><a rel="nofollow" href="//duckduckgo.com/y.js?ad=1" class="result-link">Anúncio</a></td></tr>
        <tr><td><a rel="nofollow" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdoc.rust%2Dlang.org%2Fbook%2F&amp;rut=abc" class="result-link">O Livro do Rust</a></td></tr>
        <tr><td class="result-snippet">Introdu&ccedil;&atilde;o    completa   à linguagem.</td></tr>
        <tr><td><a rel="nofollow" href="https://tokio.rs/tokio/tutorial" class="result-link">Tutorial do <b>Tokio</b></a></td></tr>
        <tr><td class="result-snippet">Async em Rust na prática.</td></tr>
        </table></body></html>"#
            .to_string()
    }

    fn config(endpoint: &str, provider: SearchProvider, key: Option<&str>) -> Arc<WebConfig> {
        Arc::new(WebConfig {
            search: SearchConfig {
                provider,
                api_key: key.map(str::to_string),
                endpoint: Some(endpoint.to_string()),
            },
            timeout_secs: 5,
            ..WebConfig::default()
        })
    }

    fn ctx() -> ToolContext {
        ToolContext::new(None, "call-web")
    }

    #[tokio::test]
    async fn duckduckgo_results_are_numbered_and_citable() {
        let server = FakeServer::spawn(|_| FakeResponse::html(ddg_page()));
        let tool = WebSearch::new(config(&server.url(), SearchProvider::Auto, None));

        let out = tool
            .execute(json!({"query": "rust async"}), &ctx())
            .await
            .unwrap();

        assert!(
            out.content.contains("1. O Livro do Rust"),
            "{}",
            out.content
        );
        assert!(
            out.content.contains("https://doc.rust-lang.org/book/"),
            "o link de saída tem de ser desembrulhado: {}",
            out.content
        );
        assert!(
            out.content.contains("Introdução completa"),
            "{}",
            out.content
        );
        assert!(
            out.content.contains("2. Tutorial do Tokio"),
            "{}",
            out.content
        );
        assert!(!out.content.contains("Anúncio"), "anúncio: {}", out.content);
        assert!(out.content.contains("web_fetch"), "{}", out.content);
        // A consulta chegou ao servidor como parâmetro `q`.
        assert_eq!(
            server.last_request().query_param("q").as_deref(),
            Some("rust async")
        );
    }

    #[tokio::test]
    async fn search_results_carry_the_untrusted_warning() {
        let server = FakeServer::spawn(|_| FakeResponse::html(ddg_page()));
        let tool = WebSearch::new(config(&server.url(), SearchProvider::DuckDuckGo, None));
        let out = tool
            .execute(json!({"query": "rust"}), &ctx())
            .await
            .unwrap();
        assert!(out.content.contains(UNTRUSTED_NOTE), "{}", out.content);
        assert!(out.content.contains(CONTENT_START), "{}", out.content);
    }

    #[tokio::test]
    async fn max_results_is_respected_and_clamped() {
        let server = FakeServer::spawn(|_| FakeResponse::html(ddg_page()));
        let tool = WebSearch::new(config(&server.url(), SearchProvider::Auto, None));

        let out = tool
            .execute(json!({"query": "rust", "max_results": 1}), &ctx())
            .await
            .unwrap();
        assert!(out.content.contains("1 resultado(s)"), "{}", out.content);
        assert!(!out.content.contains("2. "), "{}", out.content);

        // Pedido absurdo não vira erro: entra no teto.
        let out = tool
            .execute(json!({"query": "rust", "max_results": 999}), &ctx())
            .await
            .unwrap();
        assert!(out.content.contains("2 resultado(s)"), "{}", out.content);
    }

    #[tokio::test]
    async fn a_blocked_search_says_what_to_do_next() {
        let server = FakeServer::spawn(|_| {
            FakeResponse::status(403, "<html><body>Unusual traffic detected</body></html>")
        });
        let tool = WebSearch::new(config(&server.url(), SearchProvider::Auto, None));
        let err = tool
            .execute(json!({"query": "rust"}), &ctx())
            .await
            .unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("bloqueada"), "{msg}");
        assert!(
            msg.contains("web_fetch"),
            "a saída tem de estar na mensagem: {msg}"
        );
    }

    #[tokio::test]
    async fn no_results_is_an_answer_not_an_error() {
        let server =
            FakeServer::spawn(|_| FakeResponse::html("<html><body>nada aqui</body></html>"));
        let tool = WebSearch::new(config(&server.url(), SearchProvider::Auto, None));
        let out = tool
            .execute(json!({"query": "termo inexistente xyz"}), &ctx())
            .await
            .unwrap();
        assert!(out.content.contains("nenhum resultado"), "{}", out.content);
        assert!(out.content.contains("web_fetch"), "{}", out.content);
    }

    #[tokio::test]
    async fn brave_is_used_when_a_key_exists() {
        let server = FakeServer::spawn(|_| {
            FakeResponse::json(
                json!({"web": {"results": [
                    {"title": "Docs do Brave", "url": "https://brave.com/a", "description": "trecho"}
                ]}})
                .to_string(),
            )
        });
        let tool = WebSearch::new(config(
            &server.url(),
            SearchProvider::Brave,
            Some("BSA-chave"),
        ));

        let out = tool
            .execute(json!({"query": "assunto", "max_results": 3}), &ctx())
            .await
            .unwrap();
        assert!(
            out.content.contains("Busca no Brave Search"),
            "{}",
            out.content
        );
        assert!(
            out.content.contains("https://brave.com/a"),
            "{}",
            out.content
        );

        let req = server.last_request();
        assert_eq!(req.method, "GET");
        assert_eq!(req.header("x-subscription-token"), Some("BSA-chave"));
        assert_eq!(req.query_param("count").as_deref(), Some("3"));
    }

    #[tokio::test]
    async fn tavily_is_used_when_the_key_looks_like_one() {
        let server = FakeServer::spawn(|_| {
            FakeResponse::json(
                json!({"results": [
                    {"title": "Página", "url": "https://exemplo.com/x", "content": "trecho tavily"}
                ]})
                .to_string(),
            )
        });
        // Provedor automático + chave `tvly-…` tem de escolher a Tavily.
        let tool = WebSearch::new(config(
            &server.url(),
            SearchProvider::Auto,
            Some("tvly-123"),
        ));

        let out = tool
            .execute(json!({"query": "assunto"}), &ctx())
            .await
            .unwrap();
        assert!(out.content.contains("Busca no Tavily"), "{}", out.content);
        assert!(out.content.contains("trecho tavily"), "{}", out.content);

        let req = server.last_request();
        assert_eq!(req.method, "POST");
        assert_eq!(req.header("authorization"), Some("Bearer tvly-123"));
        assert!(req.body.contains("\"query\":\"assunto\""), "{}", req.body);
    }

    #[tokio::test]
    async fn a_keyed_provider_without_a_key_falls_back_to_duckduckgo() {
        let server = FakeServer::spawn(|_| FakeResponse::html(ddg_page()));
        let cfg = Arc::new(WebConfig {
            search: SearchConfig {
                provider: SearchProvider::Brave,
                api_key: Some("   ".into()), // chave em branco não conta
                endpoint: Some(server.url()),
            },
            timeout_secs: 5,
            ..WebConfig::default()
        });
        let out = WebSearch::new(cfg)
            .execute(json!({"query": "rust"}), &ctx())
            .await
            .unwrap();
        assert!(
            out.content.contains("Busca no DuckDuckGo"),
            "{}",
            out.content
        );
        assert!(out.content.contains("sem chave"), "{}", out.content);
        // E nenhum cabeçalho de autenticação foi enviado.
        assert!(
            server
                .last_request()
                .header("x-subscription-token")
                .is_none()
        );
    }

    #[tokio::test]
    async fn the_key_never_travels_to_duckduckgo() {
        let server = FakeServer::spawn(|_| FakeResponse::html(ddg_page()));
        let tool = WebSearch::new(config(
            &server.url(),
            SearchProvider::DuckDuckGo,
            Some("chave-secreta"),
        ));
        tool.execute(json!({"query": "rust"}), &ctx())
            .await
            .unwrap();

        let req = server.last_request();
        for (name, value) in &req.headers {
            assert!(
                !value.contains("chave-secreta"),
                "a chave vazou no cabeçalho {name}"
            );
        }
        assert!(!req.query.contains("chave-secreta"), "{}", req.query);
    }

    #[tokio::test]
    async fn a_refused_key_explains_how_to_recover() {
        let server = FakeServer::spawn(|_| FakeResponse::status(401, "{\"error\":\"bad key\"}"));
        let tool = WebSearch::new(config(&server.url(), SearchProvider::Brave, Some("errada")));
        let err = tool
            .execute(json!({"query": "rust"}), &ctx())
            .await
            .unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("401"), "{msg}");
        assert!(msg.contains("DuckDuckGo"), "precisa dizer o plano B: {msg}");
    }

    #[tokio::test]
    async fn preview_shows_provider_and_query_without_the_key() {
        let tool = WebSearch::new(config(
            "https://api.search.brave.com/res/v1/web/search",
            SearchProvider::Brave,
            Some("chave-secreta"),
        ));
        let preview = tool
            .preview(&json!({"query": "  rust async  "}), &ctx())
            .await
            .unwrap();
        match preview {
            ToolPreview::Text { body } => {
                assert!(body.contains("Brave Search"), "{body}");
                assert!(body.contains("api.search.brave.com"), "{body}");
                assert!(body.contains("rust async"), "{body}");
                assert!(!body.contains("chave-secreta"), "a chave vazou: {body}");
                assert!(body.contains("valor oculto"), "{body}");
            }
            other => panic!("esperava prévia de texto, veio {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_empty_query_is_refused_before_the_network() {
        let tool = WebSearch::new(Arc::new(WebConfig::default()));
        let err = tool
            .execute(json!({"query": "   "}), &ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)), "{err:?}");
    }

    #[test]
    fn provider_resolution_covers_every_combination() {
        let auto = SearchConfig::default().resolve();
        assert_eq!(auto.provider, SearchProvider::DuckDuckGo);
        assert!(auto.api_key.is_none());
        assert_eq!(auto.endpoint, DUCKDUCKGO_ENDPOINT);

        let sniffed = SearchConfig {
            provider: SearchProvider::Auto,
            api_key: Some("tvly-abc".into()),
            endpoint: None,
        }
        .resolve();
        assert_eq!(sniffed.provider, SearchProvider::Tavily);
        assert_eq!(sniffed.endpoint, TAVILY_ENDPOINT);

        let guessed_brave = SearchConfig {
            provider: SearchProvider::Auto,
            api_key: Some("BSAxyz".into()),
            endpoint: None,
        }
        .resolve();
        assert_eq!(guessed_brave.provider, SearchProvider::Brave);
        assert_eq!(guessed_brave.api_key.as_deref(), Some("BSAxyz"));
    }

    #[test]
    fn result_urls_are_unwrapped_and_ads_dropped() {
        assert_eq!(
            clean_result_url("//duckduckgo.com/l/?uddg=https%3A%2F%2Fexemplo.com%2Fa&rut=x")
                .as_deref(),
            Some("https://exemplo.com/a")
        );
        assert_eq!(
            clean_result_url("https://exemplo.com/b").as_deref(),
            Some("https://exemplo.com/b")
        );
        assert!(clean_result_url("//duckduckgo.com/y.js?ad_provider=x").is_none());
        assert!(clean_result_url("#").is_none());
        assert!(clean_result_url("").is_none());
    }

    /// Contra a internet de verdade. Fora da suíte normal de propósito:
    /// `cargo test -p lr_webtools -- --ignored busca_real`.
    #[tokio::test]
    #[ignore = "usa a internet de verdade"]
    async fn busca_real_no_duckduckgo() {
        let tool = WebSearch::new(Arc::new(WebConfig::default()));
        let out = tool
            .execute(json!({"query": "rust programming language"}), &ctx())
            .await
            .unwrap();
        assert!(out.content.contains("http"), "{}", out.content);
    }
}
