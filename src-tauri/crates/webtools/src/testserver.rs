//! Servidor HTTP falso para os testes (mesma receita de `lr_engine`).
//!
//! A suíte normal **nunca** toca a internet: teste que depende de rede real
//! falha no avião, no CI sem saída e no dia em que o site muda o HTML. Aqui o
//! servidor é um `TcpListener` de `std::net` numa porta efêmera — sem
//! dependência nova e sem surpresa.
//!
//! O que ele precisa saber fazer, e por quê:
//! - **guardar a requisição inteira** (método, caminho, query, cabeçalhos,
//!   corpo): é assim que o teste confirma que a chave da busca foi para o
//!   provedor certo — e só para ele;
//! - **redirecionar** (`Location`): exercita o limite de saltos;
//! - **demorar de propósito**: exercita o timeout;
//! - **responder HEAD sem corpo**: senão o cliente espera um corpo que não vem.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct FakeRequest {
    pub method: String,
    /// Caminho sem a query.
    pub path: String,
    /// Query crua (sem o `?`).
    pub query: String,
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

impl FakeRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// Valor de um parâmetro da query (já sem escapes de URL).
    pub fn query_param(&self, key: &str) -> Option<String> {
        self.query.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            (k == key).then(|| percent_decode(v))
        })
    }
}

pub struct FakeResponse {
    pub status: u16,
    pub content_type: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Espera antes de responder (para exercitar timeout).
    pub delay: Duration,
}

impl FakeResponse {
    pub fn html(body: impl Into<String>) -> Self {
        Self::new(200, "text/html; charset=utf-8", body.into().into_bytes())
    }

    pub fn json(body: impl Into<String>) -> Self {
        Self::new(200, "application/json", body.into().into_bytes())
    }

    pub fn text(body: impl Into<String>) -> Self {
        Self::new(200, "text/plain; charset=utf-8", body.into().into_bytes())
    }

    pub fn bytes(content_type: &str, body: Vec<u8>) -> Self {
        Self::new(200, content_type, body)
    }

    pub fn status(status: u16, body: impl Into<String>) -> Self {
        Self::new(
            status,
            "text/plain; charset=utf-8",
            body.into().into_bytes(),
        )
    }

    pub fn redirect(to: &str) -> Self {
        let mut res = Self::new(302, "text/plain", b"redirecionando".to_vec());
        res.headers.push(("Location".into(), to.to_string()));
        res
    }

    fn new(status: u16, content_type: &str, body: Vec<u8>) -> Self {
        Self {
            status,
            content_type: content_type.to_string(),
            headers: Vec::new(),
            body,
            delay: Duration::ZERO,
        }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

pub struct FakeServer {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<FakeRequest>>>,
    stop: Arc<AtomicBool>,
}

impl FakeServer {
    pub fn spawn<F>(handler: F) -> Self
    where
        F: Fn(&FakeRequest) -> FakeResponse + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        listener.set_nonblocking(true).expect("nonblocking");

        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (reqs, flag) = (requests.clone(), stop.clone());

        thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        if let Some(req) = read_request(&mut stream) {
                            reqs.lock().unwrap().push(req.clone());
                            let res = handler(&req);
                            if !res.delay.is_zero() {
                                thread::sleep(res.delay);
                            }
                            write_response(&mut stream, &res, req.method == "HEAD");
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            addr,
            requests,
            stop,
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn url_for(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    pub fn requests(&self) -> Vec<FakeRequest> {
        self.requests.lock().unwrap().clone()
    }

    pub fn last_request(&self) -> FakeRequest {
        self.requests().pop().expect("nenhuma requisição chegou")
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn read_request(stream: &mut TcpStream) -> Option<FakeRequest> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).ok()? == 0 {
            return None;
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).to_string();
    let mut lines = text.lines();
    let mut start = lines.next()?.split_whitespace();
    let method = start.next()?.to_string();
    let target = start.next()?.to_string();
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target, String::new()),
    };

    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }

    let len: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; len];
    if len > 0 && stream.read_exact(&mut body).is_err() {
        return None;
    }

    Some(FakeRequest {
        method,
        path,
        query,
        headers,
        body: String::from_utf8_lossy(&body).to_string(),
    })
}

fn write_response(stream: &mut TcpStream, res: &FakeResponse, head_only: bool) {
    let reason = match res.status {
        200 => "OK",
        302 => "Found",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "Status",
    };
    let mut head = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        res.status,
        res.content_type,
        res.body.len()
    );
    for (name, value) in &res.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");

    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    if !head_only {
        let _ = stream.write_all(&res.body);
    }
    let _ = stream.flush();
}

/// Desfaz `%XX` e `+` de um valor de query.
fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}
