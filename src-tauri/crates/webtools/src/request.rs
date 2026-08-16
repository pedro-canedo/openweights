//! `http_request`: falar com uma API.
//!
//! `web_fetch` serve para ler página; isto serve para o resto — consultar um
//! endpoint REST, mandar um POST, conferir um status. Três decisões:
//!
//! **Resposta 4xx/5xx não é erro da ferramenta.** Um `404` de API é
//! *informação*: o agente precisa vê-lo para decidir o próximo passo. Só falha
//! de transporte (DNS, conexão, timeout) vira `Err`. Transformar `422` em erro
//! faria o modelo repetir a mesma chamada esperando outro resultado.
//!
//! **Nenhum cabeçalho é inventado.** Vai exatamente o que o modelo pediu, mais
//! um `Content-Type` deduzido quando há corpo e ninguém declarou o tipo.
//! Autenticação só existe aqui se estiver nos argumentos — e aparece na tela
//! de confirmação antes de sair.
//!
//! **A prévia oculta o valor de cabeçalho sensível.** Ela é gravada na trilha
//! de execução, que fica no disco e aparece na interface; o *nome* do
//! cabeçalho basta para a pessoa saber que existe autenticação na chamada.

use std::sync::Arc;

use async_trait::async_trait;
use lr_tools::{Tool, ToolContext, ToolError, ToolOutput, ToolResult, arg_str, arg_str_opt};
use lr_types::agent::{ToolCategory, ToolPreview};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, Url};
use serde_json::{Value, json};

use crate::net;
use crate::{UNTRUSTED_NOTE, WebConfig};

/// Métodos aceitos. `HEAD` entra porque é o jeito barato de conferir se um
/// endereço existe; `OPTIONS`/`TRACE` ficam de fora por não servirem ao agente.
const METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"];

/// Cabeçalhos cujo valor não aparece na prévia nem no resultado.
const SENSITIVE: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "api-key",
    "apikey",
    "x-auth-token",
    "x-subscription-token",
    "x-access-token",
];

/// Cabeçalhos de resposta que valem contexto para o modelo.
const INTERESTING: &[&str] = &[
    "content-type",
    "content-length",
    "location",
    "retry-after",
    "etag",
    "last-modified",
    "x-request-id",
];

/// Teto de caracteres do corpo mostrado ao modelo.
const BODY_CHARS: usize = 20_000;

/// Faz uma requisição HTTP arbitrária.
pub struct HttpRequest {
    config: Arc<WebConfig>,
}

impl HttpRequest {
    pub fn new(config: Arc<WebConfig>) -> Self {
        Self { config }
    }
}

fn is_sensitive(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SENSITIVE.contains(&lower.as_str())
}

/// Lê o método, aceitando minúsculas.
fn parse_method(args: &Value) -> ToolResult<Method> {
    let raw = arg_str_opt(args, "method").unwrap_or_else(|| "GET".to_string());
    let upper = raw.trim().to_ascii_uppercase();
    if !METHODS.contains(&upper.as_str()) {
        return Err(ToolError::InvalidArgs(format!(
            "método `{raw}` não é aceito. Use um destes: {}.",
            METHODS.join(", ")
        )));
    }
    Method::from_bytes(upper.as_bytes())
        .map_err(|_| ToolError::InvalidArgs(format!("método `{raw}` inválido")))
}

/// Lê `headers` como pares nome→valor.
///
/// Aceita objeto JSON (o normal) ou texto com um objeto dentro — modelo
/// pequeno manda os dois, e recusar o segundo gastaria um passo do run.
fn parse_headers(args: &Value) -> ToolResult<Vec<(String, String)>> {
    let raw = match args.get("headers") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::String(text)) if text.trim().is_empty() => return Ok(Vec::new()),
        Some(Value::String(text)) => serde_json::from_str::<Value>(text).map_err(|e| {
            ToolError::InvalidArgs(format!(
                "`headers` veio como texto e não é um objeto JSON válido ({e}). Mande algo \
                 como {{\"Accept\": \"application/json\"}}."
            ))
        })?,
        Some(other) => other.clone(),
    };

    let object = raw.as_object().ok_or_else(|| {
        ToolError::InvalidArgs(
            "`headers` precisa ser um objeto com nome e valor, ex.: \
             {\"Accept\": \"application/json\"}."
                .into(),
        )
    })?;

    let mut out = Vec::new();
    for (name, value) in object {
        let value = match value {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            _ => {
                return Err(ToolError::InvalidArgs(format!(
                    "o cabeçalho `{name}` precisa ter valor de texto"
                )));
            }
        };
        out.push((name.clone(), value));
    }
    Ok(out)
}

/// Corpo da requisição como texto, com o tipo deduzido quando falta.
fn parse_body(args: &Value) -> Option<(String, &'static str)> {
    match args.get("body") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) if text.is_empty() => None,
        Some(Value::String(text)) => {
            let trimmed = text.trim_start();
            let guess = if trimmed.starts_with('{') || trimmed.starts_with('[') {
                "application/json"
            } else {
                "text/plain; charset=utf-8"
            };
            Some((text.clone(), guess))
        }
        // Objeto/array direto: o modelo quis mandar JSON.
        Some(other) => Some((other.to_string(), "application/json")),
    }
}

fn build_header_map(pairs: &[(String, String)]) -> ToolResult<HeaderMap> {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        let header = HeaderName::from_bytes(name.trim().as_bytes()).map_err(|_| {
            ToolError::InvalidArgs(format!(
                "`{name}` não é um nome de cabeçalho válido (use letras, números e hífen)"
            ))
        })?;
        let value = HeaderValue::from_str(value.trim()).map_err(|_| {
            ToolError::InvalidArgs(format!(
                "o valor do cabeçalho `{name}` tem caractere que não pode ser enviado"
            ))
        })?;
        map.insert(header, value);
    }
    Ok(map)
}

#[async_trait]
impl Tool for HttpRequest {
    fn name(&self) -> &str {
        "http_request"
    }

    fn description(&self) -> &str {
        "Faz uma chamada HTTP a uma API e devolve status, cabeçalhos principais e corpo. Use \
         para consultar ou enviar dados a um serviço (REST, webhook, health check). Para ler \
         uma página como texto prefira `web_fetch`."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"],
                    "description": "Método HTTP. Padrão GET."
                },
                "url": {
                    "type": "string",
                    "description": "Endereço completo, começando com https:// ou http://."
                },
                "headers": {
                    "type": "object",
                    "description": "Cabeçalhos como objeto, ex.: {\"Accept\": \"application/json\"}. Só mande autenticação se o usuário pediu."
                },
                "body": {
                    "type": "string",
                    "description": "Corpo da requisição (texto ou JSON). Não use com GET nem HEAD."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Tempo máximo em segundos. Padrão 30, teto 300.",
                    "minimum": 1,
                    "maximum": 300
                }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Network
    }

    async fn preview(&self, args: &Value, _ctx: &ToolContext) -> Option<ToolPreview> {
        let raw = arg_str(args, "url").ok()?;
        let method = match parse_method(args) {
            Ok(m) => m.to_string(),
            Err(e) => {
                return Some(ToolPreview::Text {
                    body: e.to_model_message(),
                });
            }
        };
        let url = match net::parse_http_url(&raw) {
            Ok(u) => u,
            Err(e) => {
                return Some(ToolPreview::Text {
                    body: e.to_model_message(),
                });
            }
        };

        let mut body = format!(
            "Chamada HTTP\nMétodo: {method}\nURL: {}\nHost: {}",
            net::display_url(&url),
            net::host_of(&url)
        );
        if net::is_local_host(&url) {
            body.push_str("\nObservação: endereço local — não sai desta máquina.");
        }
        if let Ok(headers) = parse_headers(args)
            && !headers.is_empty()
        {
            body.push_str("\nCabeçalhos:");
            for (name, value) in &headers {
                if is_sensitive(name) {
                    body.push_str(&format!("\n  {name}: (valor oculto)"));
                } else {
                    body.push_str(&format!("\n  {name}: {value}"));
                }
            }
        }
        if let Some((content, kind)) = parse_body(args) {
            let (shown, cut) = crate::extract::truncate_chars(&content, 500);
            body.push_str(&format!(
                "\nCorpo ({kind}, {}):\n{shown}{}",
                net::human_bytes(content.len() as u64),
                if cut { "\n…" } else { "" }
            ));
        }
        Some(ToolPreview::Text { body })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let method = parse_method(&args)?;
        let url = net::parse_http_url(&arg_str(&args, "url")?)?;
        let headers = parse_headers(&args)?;
        let body = parse_body(&args);

        if body.is_some() && matches!(method, Method::GET | Method::HEAD) {
            return Err(ToolError::InvalidArgs(format!(
                "{method} não leva corpo. Mande os dados na query da URL, ou use POST."
            )));
        }

        let timeout = self
            .config
            .timeout(args.get("timeout_secs").and_then(Value::as_u64));
        let client = net::client(timeout, self.config.max_redirects)?;

        let mut header_map = build_header_map(&headers)?;
        let mut request = client.request(method.clone(), url.clone());
        if let Some((content, guessed)) = &body {
            if !header_map.contains_key("content-type") {
                header_map.insert(
                    HeaderName::from_static("content-type"),
                    HeaderValue::from_static(guessed),
                );
            }
            request = request.body(content.clone());
        }
        let resp = request
            .headers(header_map)
            .send()
            .await
            .map_err(|e| net::send_error(e, &url, timeout))?;

        let status = resp.status();
        let final_url = resp.url().clone();
        let resp_headers = resp.headers().clone();
        let (bytes, capped) =
            net::read_body_capped(resp, self.config.max_response_bytes, timeout).await?;

        Ok(ToolOutput::text(format_response(&Received {
            method,
            requested: url,
            final_url,
            status: status.as_u16(),
            reason: status.canonical_reason().unwrap_or(""),
            headers: resp_headers,
            body: bytes,
            capped,
        }))
        .truncated_to(ctx.max_output_bytes))
    }
}

/// O que sobrou da resposta depois de o corpo ter sido lido.
struct Received {
    method: Method,
    /// URL pedida (pode diferir da final, se houve redirecionamento).
    requested: Url,
    final_url: Url,
    status: u16,
    reason: &'static str,
    headers: HeaderMap,
    body: Vec<u8>,
    /// O corpo bateu no teto de bytes e foi cortado.
    capped: bool,
}

/// Monta o resultado: status, cabeçalhos que importam e corpo.
fn format_response(r: &Received) -> String {
    let (status, headers) = (r.status, &r.headers);
    let mut out = format!(
        "HTTP {status} {} — {} {}",
        r.reason,
        r.method,
        net::display_url(&r.final_url)
    );
    if r.final_url != r.requested {
        out.push_str(&format!(
            "\n(redirecionado de {})",
            net::display_url(&r.requested)
        ));
    }
    if !(200..400).contains(&status) {
        out.push_str(&format!(
            "\nO servidor recusou: {}",
            net::status_hint(status)
        ));
    }

    for name in INTERESTING {
        if let Some(value) = net::header_value(headers, name) {
            out.push_str(&format!("\n{name}: {value}"));
        }
    }
    for (name, value) in headers {
        let name = name.as_str();
        if name.starts_with("x-ratelimit") || name.starts_with("ratelimit") {
            out.push_str(&format!(
                "\n{name}: {}",
                value.to_str().unwrap_or("(binário)")
            ));
        }
    }

    let body = r.body.as_slice();
    let capped = r.capped;
    let content_type = net::content_type(headers);
    out.push_str("\n\n");
    if body.is_empty() {
        out.push_str("(resposta sem corpo)");
        return out;
    }
    if !net::is_textual(&content_type) {
        out.push_str(&format!(
            "(corpo de {} em `{content_type}`, não é texto — use `web_download` se quiser \
             guardar o arquivo)",
            net::human_bytes(body.len() as u64)
        ));
        return out;
    }

    let text = String::from_utf8_lossy(body);
    let (shown, cut) = crate::extract::truncate_chars(text.trim(), BODY_CHARS);
    out.push_str(UNTRUSTED_NOTE);
    out.push_str(&format!(
        "\n{}\n{shown}\n{}",
        crate::CONTENT_START,
        crate::CONTENT_END
    ));
    if cut || capped {
        out.push_str("\n[...corpo cortado; peça um recurso menor ou pagine a API...]");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testserver::{FakeResponse, FakeServer};
    use std::time::Duration;

    fn tool() -> HttpRequest {
        HttpRequest::new(Arc::new(WebConfig {
            timeout_secs: 5,
            ..WebConfig::default()
        }))
    }

    fn ctx() -> ToolContext {
        ToolContext::new(None, "call-http")
    }

    #[tokio::test]
    async fn get_returns_status_headers_and_body() {
        let server = FakeServer::spawn(|_| {
            FakeResponse::json(r#"{"ok":true}"#)
                .with_header("X-RateLimit-Remaining", "42")
                .with_header("X-Ruido", "nao interessa")
        });
        let out = tool()
            .execute(json!({"url": server.url_for("/v1/status")}), &ctx())
            .await
            .unwrap();

        assert!(
            out.content.starts_with("HTTP 200 OK — GET"),
            "{}",
            out.content
        );
        assert!(
            out.content.contains("content-type: application/json"),
            "{}",
            out.content
        );
        assert!(
            out.content.contains("x-ratelimit-remaining: 42"),
            "{}",
            out.content
        );
        assert!(!out.content.contains("nao interessa"), "{}", out.content);
        assert!(out.content.contains(r#"{"ok":true}"#), "{}", out.content);
        // Corpo de terceiros também vem cercado pelo aviso.
        assert!(out.content.contains(UNTRUSTED_NOTE), "{}", out.content);

        let req = server.last_request();
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/v1/status");
    }

    #[tokio::test]
    async fn post_sends_headers_and_body_with_a_guessed_content_type() {
        let server = FakeServer::spawn(|_| FakeResponse::json(r#"{"criado":1}"#));
        let out = tool()
            .execute(
                json!({
                    "method": "post",
                    "url": server.url_for("/v1/itens"),
                    "headers": {"X-Origem": "openweights"},
                    "body": "{\"nome\":\"caneca\"}"
                }),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(out.content.contains("HTTP 200"), "{}", out.content);

        let req = server.last_request();
        assert_eq!(req.method, "POST");
        assert_eq!(req.header("x-origem"), Some("openweights"));
        assert_eq!(req.header("content-type"), Some("application/json"));
        assert_eq!(req.body, r#"{"nome":"caneca"}"#);
    }

    #[tokio::test]
    async fn an_object_body_is_serialized_as_json() {
        let server = FakeServer::spawn(|_| FakeResponse::json("{}"));
        tool()
            .execute(
                json!({"method": "PUT", "url": server.url(), "body": {"n": 2}}),
                &ctx(),
            )
            .await
            .unwrap();
        let req = server.last_request();
        assert_eq!(req.body, r#"{"n":2}"#);
        assert_eq!(req.header("content-type"), Some("application/json"));
    }

    #[tokio::test]
    async fn an_error_status_is_information_not_a_failure() {
        let server = FakeServer::spawn(|_| FakeResponse::status(404, r#"{"erro":"sem item"}"#));
        let out = tool()
            .execute(json!({"url": server.url_for("/v1/itens/9")}), &ctx())
            .await
            .expect("4xx tem de voltar como resultado, não como erro");
        assert!(out.content.contains("HTTP 404"), "{}", out.content);
        assert!(out.content.contains("sem item"), "{}", out.content);
        assert!(out.content.contains("não existe"), "{}", out.content);
    }

    #[tokio::test]
    async fn head_asks_for_no_body_and_reports_none() {
        let server = FakeServer::spawn(|_| FakeResponse::text("corpo que não deve vir"));
        let out = tool()
            .execute(json!({"method": "HEAD", "url": server.url()}), &ctx())
            .await
            .unwrap();
        assert!(out.content.contains("HTTP 200"), "{}", out.content);
        assert!(
            out.content.contains("(resposta sem corpo)"),
            "{}",
            out.content
        );
        assert_eq!(server.last_request().method, "HEAD");
    }

    #[tokio::test]
    async fn invalid_method_body_and_headers_are_refused_with_guidance() {
        let bad_method = tool()
            .execute(
                json!({"method": "TRACE", "url": "https://exemplo.com"}),
                &ctx(),
            )
            .await
            .unwrap_err();
        assert!(
            bad_method.to_model_message().contains("GET"),
            "{bad_method:?}"
        );

        let body_on_get = tool()
            .execute(json!({"url": "https://exemplo.com", "body": "x=1"}), &ctx())
            .await
            .unwrap_err();
        assert!(
            body_on_get.to_model_message().contains("POST"),
            "{body_on_get:?}"
        );

        let bad_headers = tool()
            .execute(
                json!({"url": "https://exemplo.com", "headers": ["Accept: */*"]}),
                &ctx(),
            )
            .await
            .unwrap_err();
        assert!(
            bad_headers.to_model_message().contains("objeto"),
            "{bad_headers:?}"
        );

        let bad_name = tool()
            .execute(
                json!({"url": "https://exemplo.com", "headers": {"Acce pt": "x"}}),
                &ctx(),
            )
            .await
            .unwrap_err();
        assert!(
            bad_name.to_model_message().contains("cabeçalho"),
            "{bad_name:?}"
        );
    }

    #[tokio::test]
    async fn nothing_is_sent_that_the_model_did_not_ask_for() {
        let server = FakeServer::spawn(|_| FakeResponse::text("ok"));
        tool()
            .execute(json!({"url": server.url()}), &ctx())
            .await
            .unwrap();
        let req = server.last_request();
        assert!(req.header("authorization").is_none(), "{:?}", req.headers);
        assert!(req.header("cookie").is_none(), "{:?}", req.headers);
    }

    #[tokio::test]
    async fn a_timeout_is_actionable() {
        let server =
            FakeServer::spawn(|_| FakeResponse::text("tarde").with_delay(Duration::from_secs(3)));
        let err = tool()
            .execute(json!({"url": server.url(), "timeout_secs": 1}), &ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Timeout(1)), "{err:?}");
    }

    #[tokio::test]
    async fn preview_shows_method_url_host_and_masks_secrets() {
        match tool()
            .preview(
                &json!({
                    "method": "post",
                    "url": "https://api.exemplo.com/v1/itens",
                    "headers": {"Authorization": "Bearer segredo", "Accept": "application/json"},
                    "body": "{\"n\":1}"
                }),
                &ctx(),
            )
            .await
            .unwrap()
        {
            ToolPreview::Text { body } => {
                assert!(body.contains("Método: POST"), "{body}");
                assert!(
                    body.contains("URL: https://api.exemplo.com/v1/itens"),
                    "{body}"
                );
                assert!(body.contains("Host: api.exemplo.com"), "{body}");
                assert!(body.contains("Accept: application/json"), "{body}");
                assert!(body.contains("Authorization: (valor oculto)"), "{body}");
                assert!(!body.contains("Bearer segredo"), "segredo vazou: {body}");
                assert!(body.contains("{\"n\":1}"), "{body}");
            }
            other => panic!("esperava prévia de texto, veio {other:?}"),
        }
    }

    #[tokio::test]
    async fn binary_responses_are_described_not_dumped() {
        let server =
            FakeServer::spawn(|_| FakeResponse::bytes("image/png", vec![0x89, 0x50, 0x4e, 0x47]));
        let out = tool()
            .execute(json!({"url": server.url_for("/logo.png")}), &ctx())
            .await
            .unwrap();
        assert!(out.content.contains("image/png"), "{}", out.content);
        assert!(out.content.contains("web_download"), "{}", out.content);
    }
}
