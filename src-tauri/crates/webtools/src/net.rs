//! Encanamento de rede compartilhado pelas quatro ferramentas.
//!
//! Três decisões moram aqui:
//!
//! **A URL é validada antes de existir conexão.** `file:`, `data:` e
//! `javascript:` são recusados na hora, com mensagem que diz o que fazer —
//! tentar e falhar depois já teria aberto socket para algum lugar.
//!
//! **O corpo é lido em pedaços com teto.** `Response::bytes()` carregaria uma
//! resposta de 2 GB inteira na memória antes de qualquer limite valer. Aqui a
//! leitura para no limite e avisa que cortou.
//!
//! **Erro de rede vira mensagem acionável.** O agente se corrige lendo o
//! resultado: "não resolvi o host X" leva a conferir o endereço, enquanto um
//! `reqwest::Error` cru leva a repetir a mesma chamada.

use lr_tools::{ToolError, ToolResult};
use reqwest::header::HeaderMap;
use reqwest::{Client, Response, Url};
use std::time::Duration;

/// Tempo padrão de uma chamada de rede.
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Teto de tempo: acima disso não é ferramenta de agente, é serviço.
pub const MAX_TIMEOUT_SECS: u64 = 300;

/// Saltos de redirecionamento aceitos.
pub const DEFAULT_MAX_REDIRECTS: usize = 5;

/// Teto de bytes lidos de uma resposta comum (páginas, APIs).
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 5 * 1024 * 1024;

/// Teto de bytes de um download para o disco.
pub const DEFAULT_MAX_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024;

/// Tempo máximo só para abrir a conexão.
const CONNECT_TIMEOUT_SECS: u64 = 15;

/// Como o app se identifica.
///
/// Diz o que é (não finge ser navegador) mas mantém o prefixo `Mozilla/5.0`,
/// que vários servidores exigem para responder qualquer coisa.
pub const USER_AGENT: &str = concat!(
    "Mozilla/5.0 (compatible; OpenWeights/",
    env!("CARGO_PKG_VERSION"),
    "; agente local)"
);

/// Valida a URL e garante que só `http`/`https` passam.
pub fn parse_http_url(raw: &str) -> ToolResult<Url> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ToolError::InvalidArgs(
            "`url` está vazio — mande o endereço completo, ex.: https://exemplo.com/pagina".into(),
        ));
    }

    let url = Url::parse(trimmed).map_err(|e| {
        ToolError::InvalidArgs(format!(
            "`{trimmed}` não é uma URL válida ({e}). Use o endereço completo com esquema, \
             ex.: https://exemplo.com/pagina"
        ))
    })?;

    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(ToolError::InvalidArgs(format!(
                "o esquema `{other}:` não é aceito nesta ferramenta — só `http` e `https`. \
                 Para ler arquivo local use as ferramentas de arquivo."
            )));
        }
    }

    if url.host_str().unwrap_or_default().is_empty() {
        return Err(ToolError::InvalidArgs(format!(
            "`{trimmed}` não tem host. Escreva o endereço completo, ex.: https://exemplo.com/pagina"
        )));
    }

    Ok(url)
}

/// Host (com porta, quando não é a padrão) para mostrar na prévia.
pub fn host_of(url: &Url) -> String {
    match (url.host_str(), url.port()) {
        (Some(h), Some(p)) => format!("{h}:{p}"),
        (Some(h), None) => h.to_string(),
        _ => "(sem host)".to_string(),
    }
}

/// A chamada fica na própria máquina/rede local?
///
/// Não bloqueia nada — só serve para a prévia contar à pessoa que aquele
/// endereço não sai para a internet (o caso do próprio `llama-server`).
pub fn is_local_host(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified()
        }
        Ok(std::net::IpAddr::V6(ip)) => ip.is_loopback() || ip.is_unspecified(),
        Err(_) => false,
    }
}

/// URL sem usuário/senha, para aparecer na prévia e no resultado.
///
/// Credencial embutida (`https://user:senha@host`) não pode vazar para a
/// trilha de execução, que fica gravada em disco.
pub fn display_url(url: &Url) -> String {
    if url.username().is_empty() && url.password().is_none() {
        return url.to_string();
    }
    let mut clean = url.clone();
    let _ = clean.set_username("");
    let _ = clean.set_password(None);
    format!("{clean} (credencial na URL ocultada)")
}

/// Cliente para requisições comuns: teto de tempo no total da chamada.
pub fn client(timeout_secs: u64, max_redirects: usize) -> ToolResult<Client> {
    build(timeout_secs, max_redirects, /* total_timeout = */ true)
}

/// Cliente para download: o teto vale por leitura, não pelo total.
///
/// Um arquivo de 80 MB numa conexão lenta passa de 30s com facilidade; o que
/// não pode acontecer é a transferência *parar* e o run ficar preso.
pub fn streaming_client(idle_timeout_secs: u64, max_redirects: usize) -> ToolResult<Client> {
    build(
        idle_timeout_secs,
        max_redirects,
        /* total_timeout = */ false,
    )
}

fn build(timeout_secs: u64, max_redirects: usize, total_timeout: bool) -> ToolResult<Client> {
    let redirect = if max_redirects == 0 {
        reqwest::redirect::Policy::none()
    } else {
        reqwest::redirect::Policy::limited(max_redirects)
    };
    let mut builder = Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(
            CONNECT_TIMEOUT_SECS.min(timeout_secs.max(1)),
        ))
        .redirect(redirect);
    builder = if total_timeout {
        builder.timeout(Duration::from_secs(timeout_secs))
    } else {
        builder.read_timeout(Duration::from_secs(timeout_secs))
    };
    builder.build().map_err(|e| {
        ToolError::Other(format!(
            "não foi possível preparar o cliente HTTP ({e}). Isso é um problema local, não da URL."
        ))
    })
}

/// Traduz a falha de envio em algo que o agente consiga usar.
pub fn send_error(e: reqwest::Error, url: &Url, timeout_secs: u64) -> ToolError {
    let host = host_of(url);
    if e.is_timeout() {
        return ToolError::Timeout(timeout_secs);
    }
    if e.is_redirect() {
        return ToolError::Other(format!(
            "`{host}` redirecionou vezes demais e a chamada parou no limite de saltos. \
             Descubra o endereço final e chame direto."
        ));
    }
    if e.is_connect() {
        return ToolError::Other(format!(
            "não consegui conectar em `{host}` ({e}). Confira o endereço e se há conexão; \
             se o host for interno, ele pode não existir nesta máquina."
        ));
    }
    if e.is_builder() {
        return ToolError::InvalidArgs(format!("a requisição não pôde ser montada ({e})"));
    }
    ToolError::Other(format!(
        "a chamada para `{host}` falhou ({e}). Tente de novo ou use outra fonte."
    ))
}

/// Lê o corpo em pedaços, parando no teto.
///
/// Devolve `(bytes, cortado)`.
pub async fn read_body_capped(
    mut resp: Response,
    max_bytes: usize,
    timeout_secs: u64,
) -> ToolResult<(Vec<u8>, bool)> {
    let mut buf: Vec<u8> = Vec::new();
    let mut truncated = false;
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                if buf.len() + chunk.len() > max_bytes {
                    let room = max_bytes.saturating_sub(buf.len());
                    buf.extend_from_slice(&chunk[..room]);
                    truncated = true;
                    break;
                }
                buf.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(e) if e.is_timeout() => return Err(ToolError::Timeout(timeout_secs)),
            Err(e) => {
                return Err(ToolError::Other(format!(
                    "a resposta foi interrompida no meio ({e}). Tente de novo ou peça \
                     menos conteúdo."
                )));
            }
        }
    }
    Ok((buf, truncated))
}

/// Valor de um cabeçalho como texto.
pub fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

/// `content-type` sem os parâmetros e em minúsculas.
pub fn content_type(headers: &HeaderMap) -> String {
    header_value(headers, "content-type")
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

/// O tipo é HTML/XML (vale extrair texto legível)?
pub fn is_html(content_type: &str) -> bool {
    content_type.contains("html") || content_type.contains("xml")
}

/// O tipo é texto que o modelo consegue ler direto?
pub fn is_textual(content_type: &str) -> bool {
    content_type.is_empty()
        || content_type.starts_with("text/")
        || content_type.contains("json")
        || content_type.contains("xml")
        || content_type.contains("javascript")
        || content_type.contains("csv")
        || content_type.contains("yaml")
        || content_type.contains("x-ndjson")
}

/// Tamanho em unidade legível, para prévia e resultado.
pub fn human_bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    match n {
        b if b >= GB => format!("{:.1} GB", b as f64 / GB as f64),
        b if b >= MB => format!("{:.1} MB", b as f64 / MB as f64),
        b if b >= KB => format!("{:.1} KB", b as f64 / KB as f64),
        b => format!("{b} B"),
    }
}

/// Explica um status HTTP que não deu certo, com o próximo passo.
pub fn status_hint(status: u16) -> &'static str {
    match status {
        401 | 403 => {
            "o servidor recusou o acesso — pode exigir login ou bloquear robôs; \
                      tente outra fonte."
        }
        404 | 410 => "a página não existe nesse endereço; confira o link ou busque de novo.",
        429 => {
            "o servidor pediu para esperar (limite de requisições). Tente mais tarde ou \
                use outra fonte."
        }
        s if s >= 500 => {
            "o problema é do servidor, não da chamada. Tente de novo depois ou \
                          use outra fonte."
        }
        _ => "confira o endereço e os parâmetros da chamada.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_http_and_https_only() {
        assert!(parse_http_url("https://exemplo.com/a?b=1").is_ok());
        assert!(parse_http_url("  http://127.0.0.1:8080/x  ").is_ok());

        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,<b>x</b>",
            "ftp://exemplo.com/a",
        ] {
            let err = parse_http_url(bad).unwrap_err();
            assert!(matches!(err, ToolError::InvalidArgs(_)), "{bad}: {err:?}");
            assert!(
                err.to_model_message().contains("http"),
                "a mensagem tem de dizer o que é aceito: {}",
                err.to_model_message()
            );
        }
    }

    #[test]
    fn rejects_garbage_and_empty_urls() {
        for bad in ["", "   ", "exemplo.com/sem-esquema", "http://"] {
            let err = parse_http_url(bad).unwrap_err();
            assert!(matches!(err, ToolError::InvalidArgs(_)), "{bad}: {err:?}");
            // A mensagem precisa mostrar o formato esperado.
            assert!(err.to_model_message().contains("https://exemplo.com"));
        }
    }

    #[test]
    fn local_hosts_are_recognized_for_the_preview() {
        for local in [
            "http://localhost:8080/",
            "http://127.0.0.1/",
            "http://192.168.0.10/",
            "http://[::1]:3000/",
        ] {
            assert!(is_local_host(&parse_http_url(local).unwrap()), "{local}");
        }
        assert!(!is_local_host(
            &parse_http_url("https://exemplo.com").unwrap()
        ));
    }

    #[test]
    fn display_url_hides_embedded_credentials() {
        let url = parse_http_url("https://ana:segredo@exemplo.com/x").unwrap();
        let shown = display_url(&url);
        assert!(!shown.contains("segredo"), "{shown}");
        assert!(shown.contains("exemplo.com"), "{shown}");
        // URL sem credencial sai igualzinha.
        let plain = parse_http_url("https://exemplo.com/x").unwrap();
        assert_eq!(display_url(&plain), "https://exemplo.com/x");
    }

    #[test]
    fn content_type_helpers_classify_bodies() {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", "TEXT/HTML; charset=UTF-8".parse().unwrap());
        assert_eq!(content_type(&headers), "text/html");
        assert!(is_html(&content_type(&headers)));
        assert!(is_textual("application/json"));
        assert!(is_textual(""), "sem content-type tratamos como texto");
        assert!(!is_textual("application/pdf"));
        assert!(!is_html("text/plain"));
    }

    #[test]
    fn human_bytes_is_readable() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
    }
}
