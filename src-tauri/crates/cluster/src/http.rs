//! HTTP mínimo do plano de controle (JSON). Sem framework: um POST e um GET.

use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const MAX_BODY: usize = 64 * 1024;

pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub body: Vec<u8>,
    pub peer: SocketAddr,
}

pub async fn read_request(
    stream: &mut TcpStream,
    peer: SocketAddr,
) -> Result<HttpRequest, String> {
    let mut buf = Vec::with_capacity(1024);
    let mut tmp = [0u8; 2048];
    loop {
        let n = stream.read(&mut tmp).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.len() > MAX_BODY + 4096 {
            return Err("pedido grande demais".into());
        }
        if let Some(req) = parse_http(&buf, peer) {
            return Ok(req);
        }
        if n < tmp.len() && !buf.windows(4).any(|w| w == b"\r\n\r\n") {
            // Ainda sem cabeçalho completo; continua.
            continue;
        }
    }
    parse_http(&buf, peer).ok_or_else(|| "pedido HTTP incompleto".into())
}

fn parse_http(buf: &[u8], peer: SocketAddr) -> Option<HttpRequest> {
    let header_end = buf.windows(4).position(|w| w == b"\r\n\r\n")?;
    let header = std::str::from_utf8(&buf[..header_end]).ok()?;
    let mut lines = header.split("\r\n");
    let start = lines.next()?;
    let mut parts = start.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut content_length = 0usize;
    for line in lines {
        let (k, v) = line.split_once(':')?;
        if k.eq_ignore_ascii_case("content-length") {
            content_length = v.trim().parse().ok()?;
        }
    }
    if content_length > MAX_BODY {
        return None;
    }
    let body_start = header_end + 4;
    if buf.len() < body_start + content_length {
        return None;
    }
    Some(HttpRequest {
        method,
        path,
        body: buf[body_start..body_start + content_length].to_vec(),
        peer,
    })
}

pub async fn write_json(stream: &mut TcpStream, status: u16, reason: &str, body: &str) {
    let msg = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(msg.as_bytes()).await;
    let _ = stream.flush().await;
}

pub async fn write_empty(stream: &mut TcpStream, status: u16, reason: &str) {
    write_json(stream, status, reason, "{}").await;
}

/// Cliente HTTP JSON sobre o plano de controle de um peer.
pub async fn post_json<T: serde::de::DeserializeOwned>(
    ip: &str,
    port: u16,
    path: &str,
    body: &impl serde::Serialize,
) -> Result<T, String> {
    let raw = serde_json::to_string(body).map_err(|e| e.to_string())?;
    let text = exchange(ip, port, "POST", path, Some(&raw)).await?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

async fn exchange(
    ip: &str,
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> Result<String, String> {
    let addr = format!("{ip}:{port}");
    let mut stream = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        TcpStream::connect(&addr),
    )
    .await
    .map_err(|_| format!("tempo esgotado ao falar com {addr}"))?
    .map_err(|e| format!("não alcancei {addr}: {e}"))?;

    let body_s = body.unwrap_or("");
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {ip}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body_s}",
        body_s.len()
    );
    stream
        .write_all(req.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    stream.flush().await.map_err(|e| e.to_string())?;

    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .await
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&buf);
    let (_, rest) = text.split_once("\r\n\r\n").ok_or("resposta HTTP vazia")?;
    if let Some(status_line) = text.lines().next()
        && !status_line.contains(" 200 ")
        && !status_line.contains(" 202 ")
    {
        return Err(format!("HTTP {status_line}: {rest}"));
    }
    Ok(rest.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_get_without_body() {
        let raw = b"GET /v1/hello HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\n\r\n";
        let req = parse_http(raw, "127.0.0.1:1".parse().unwrap()).unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/v1/hello");
        assert!(req.body.is_empty());
    }

    #[test]
    fn parse_post_json() {
        let body = "{\"id\":\"a\"}";
        let raw = format!(
            "POST /v1/pair HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let req = parse_http(raw.as_bytes(), "127.0.0.1:1".parse().unwrap()).unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(std::str::from_utf8(&req.body).unwrap(), body);
    }
}
