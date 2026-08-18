//! A ponte entre o script e o harness.
//!
//! É um servidor HTTP mínimo em `127.0.0.1`, porta efêmera, que atende um
//! único caminho: `POST /call {"tool": "...", "args": {...}}`. Cada
//! requisição vira um [`BridgeRequest`] entregue a quem hospeda a ponte (o
//! laço do agente), que responde por um canal de mão única.
//!
//! ## Por que HTTP, e não stdin/stdout
//!
//! O script precisa imprimir o resultado dele em stdout — é assim que o
//! modelo recebe a resposta. Multiplexar as chamadas de ferramenta no mesmo
//! cano exigiria um protocolo de enquadramento que o modelo teria que
//! respeitar ao escrever `console.log`, e ele não vai. Descritor extra (fd 3)
//! resolveria no Unix e não existe no Windows do mesmo jeito. Sobrou o
//! loopback, que é igual nos três sistemas e o Node fala nativamente com
//! `fetch`.
//!
//! ## Uma chamada por vez, de propósito
//!
//! O laço de `accept` atende uma conexão até o fim antes de aceitar a
//! próxima. Isso serializa as chamadas mesmo que o script use `Promise.all` —
//! e é justamente o que permite ao hospedeiro reentrar no seu estado mutável
//! (política, contadores, foto do projeto) sem lock nenhum. Quem espera é o
//! script, que não tem pressa: o TCP enfileira as conexões seguintes.
//!
//! ## O token
//!
//! Loopback não é privado: qualquer processo da máquina alcança a porta. O
//! token de 32 bytes que sai daqui vai para o script pelo **ambiente** (nunca
//! por argumento, que aparece na lista de processos) e é exigido em toda
//! requisição.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

/// Teto do corpo de uma requisição da ponte.
///
/// Generoso porque `fs_write` manda o arquivo inteiro por aqui, e apertado o
/// bastante para um script em pânico não comer a memória do app.
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// Espera entre tentativas de `accept` (o listener é não-bloqueante para
/// conseguir ver o pedido de parada).
const ACCEPT_IDLE: Duration = Duration::from_millis(2);

/// Uma chamada de ferramenta pedida por um script em execução.
#[derive(Debug)]
pub struct BridgeRequest {
    pub tool: String,
    pub args: Value,
    /// Por onde a resposta volta. Largar isto sem responder faz o script
    /// receber um erro — nunca um travamento.
    pub reply: oneshot::Sender<CallReply>,
}

/// O que o hospedeiro devolve para uma chamada.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallReply {
    /// `false` faz o SDK levantar `ToolError` do lado do script.
    pub ok: bool,
    pub content: String,
}

impl CallReply {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            ok: true,
            content: content.into(),
        }
    }

    pub fn err(content: impl Into<String>) -> Self {
        Self {
            ok: false,
            content: content.into(),
        }
    }
}

/// Servidor vivo enquanto este valor existir.
pub struct Bridge {
    addr: SocketAddr,
    token: String,
    stop: Arc<AtomicBool>,
    calls: Arc<AtomicU32>,
}

impl Bridge {
    /// Sobe a ponte e devolve a fila por onde as chamadas chegam.
    pub fn start() -> std::io::Result<(Bridge, mpsc::UnboundedReceiver<BridgeRequest>)> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        listener.set_nonblocking(true)?;

        let token = new_token();
        let stop = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicU32::new(0));
        let (tx, rx) = mpsc::unbounded_channel::<BridgeRequest>();

        {
            let (token, stop, calls) = (token.clone(), stop.clone(), calls.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut stream, peer)) => {
                            // Cinto e suspensório: o bind já é de loopback.
                            if !peer.ip().is_loopback() {
                                continue;
                            }
                            let _ = stream.set_nonblocking(false);
                            let _ = stream.set_nodelay(true);
                            serve(&mut stream, &token, &tx, &calls);
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(ACCEPT_IDLE);
                        }
                        Err(_) => break,
                    }
                }
            });
        }

        Ok((
            Bridge {
                addr,
                token,
                stop,
                calls,
            },
            rx,
        ))
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// Quantas chamadas autenticadas chegaram até agora.
    pub fn calls(&self) -> u32 {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// 32 bytes do sistema em hexadecimal.
///
/// Se a fonte de aleatoriedade falhar (não deveria), a ponte não vira um
/// servidor sem senha: o token passa a ser impossível de acertar por outro
/// motivo — ninguém consegue produzi-lo — porque o script recebe o mesmo
/// valor de volta pelo ambiente.
fn new_token() -> String {
    let mut buf = [0u8; 32];
    if getrandom::fill(&mut buf).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        buf[..16].copy_from_slice(&nanos.to_le_bytes());
        log::warn!("sem fonte de aleatoriedade do sistema; o token da ponte usa o relógio");
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Atende uma conexão do começo ao fim.
fn serve(
    stream: &mut TcpStream,
    token: &str,
    tx: &mpsc::UnboundedSender<BridgeRequest>,
    calls: &AtomicU32,
) {
    let Some(req) = read_request(stream) else {
        return;
    };

    if req.auth.as_deref() != Some(token) {
        // Sem detalhe no corpo: quem não tem o token não merece pista.
        respond(stream, 401, &CallReply::err("não autorizado"));
        return;
    }
    if req.path != "/call" {
        respond(stream, 404, &CallReply::err("caminho desconhecido"));
        return;
    }

    let parsed: Result<Incoming, _> = serde_json::from_str(&req.body);
    let Ok(incoming) = parsed else {
        respond(
            stream,
            400,
            &CallReply::err("corpo inválido: esperado {\"tool\": \"...\", \"args\": {...}}"),
        );
        return;
    };

    calls.fetch_add(1, Ordering::SeqCst);
    let (reply_tx, reply_rx) = oneshot::channel();
    let enviado = tx.send(BridgeRequest {
        tool: incoming.tool,
        args: incoming.args.unwrap_or(Value::Null),
        reply: reply_tx,
    });
    if enviado.is_err() {
        respond(stream, 503, &CallReply::err("o agente encerrou a execução"));
        return;
    }

    // Sem prazo aqui de propósito: do outro lado pode estar uma pessoa
    // decidindo se autoriza. Quem tem prazo é o processo do script, e matá-lo
    // derruba esta conexão junto.
    match reply_rx.blocking_recv() {
        Ok(reply) => respond(stream, 200, &reply),
        Err(_) => respond(
            stream,
            500,
            &CallReply::err("o agente não respondeu a esta chamada"),
        ),
    }
}

#[derive(Deserialize)]
struct Incoming {
    tool: String,
    #[serde(default)]
    args: Option<Value>,
}

struct RawRequest {
    path: String,
    auth: Option<String>,
    body: String,
}

/// Lê a requisição inteira: linha inicial, cabeçalhos e corpo por
/// `Content-Length`. Não há `chunked` aqui porque o SDK sempre manda um JSON
/// com tamanho conhecido.
fn read_request(stream: &mut TcpStream) -> Option<RawRequest> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 16 * 1024 {
            return None;
        }
        if stream.read(&mut byte).ok()? == 0 {
            return None;
        }
        head.push(byte[0]);
    }

    let text = String::from_utf8_lossy(&head).to_string();
    let mut lines = text.lines();
    let mut start = lines.next()?.split_whitespace();
    let _method = start.next()?;
    let path = start.next()?.to_string();

    let mut auth = None;
    let mut length = 0usize;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "authorization" => auth = value.strip_prefix("Bearer ").map(str::to_string),
            "content-length" => length = value.parse().unwrap_or(0),
            _ => {}
        }
    }

    if length > MAX_BODY_BYTES {
        return None;
    }
    let mut body = vec![0u8; length];
    if length > 0 {
        stream.read_exact(&mut body).ok()?;
    }

    Some(RawRequest {
        path,
        auth,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn respond(stream: &mut TcpStream, status: u16, reply: &CallReply) {
    let body = serde_json::to_vec(reply).unwrap_or_else(|_| b"{\"ok\":false}".to_vec());
    let head = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&body);
    let _ = stream.flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    /// Cliente HTTP de duas linhas: a suíte não precisa de dependência nova.
    fn post(url: &str, token: Option<&str>, body: &str) -> (u16, String) {
        let addr = url.trim_start_matches("http://");
        let mut stream = TcpStream::connect(addr).expect("conectar");
        let auth = match token {
            Some(t) => format!("Authorization: Bearer {t}\r\n"),
            None => String::new(),
        };
        let req = format!(
            "POST /call HTTP/1.1\r\nHost: localhost\r\n{auth}Content-Length: {}\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(req.as_bytes()).expect("enviar");

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).expect("status");
        let status: u16 = line.split_whitespace().nth(1).unwrap().parse().unwrap();

        let mut length = 0usize;
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).expect("cabeçalho");
            if header.trim().is_empty() {
                break;
            }
            if let Some((name, value)) = header.split_once(':')
                && name.trim().eq_ignore_ascii_case("content-length")
            {
                length = value.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0u8; length];
        reader.read_exact(&mut body).expect("corpo");
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn chamada_com_token_chega_ao_hospedeiro_e_a_resposta_volta() {
        let (ponte, mut rx) = Bridge::start().unwrap();
        let url = ponte.url();
        let token = ponte.token().to_string();

        let cliente = std::thread::spawn(move || {
            post(
                &url,
                Some(&token),
                r#"{"tool":"fs_read","args":{"path":"a.txt"}}"#,
            )
        });

        let pedido = rx.recv().await.expect("a chamada chegou");
        assert_eq!(pedido.tool, "fs_read");
        assert_eq!(pedido.args["path"], "a.txt");
        pedido.reply.send(CallReply::ok("conteúdo")).unwrap();

        let (status, corpo) = cliente.join().unwrap();
        assert_eq!(status, 200);
        assert!(corpo.contains("conteúdo"), "{corpo}");
        assert_eq!(ponte.calls(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sem_token_e_com_token_errado_a_ponte_recusa() {
        let (ponte, mut rx) = Bridge::start().unwrap();
        let url = ponte.url();

        let (u1, u2) = (url.clone(), url.clone());
        let sem = std::thread::spawn(move || post(&u1, None, r#"{"tool":"fs_read"}"#));
        let errado = std::thread::spawn(move || post(&u2, Some("outro"), r#"{"tool":"fs_read"}"#));

        assert_eq!(sem.join().unwrap().0, 401);
        assert_eq!(errado.join().unwrap().0, 401);
        // Nada disso pode ter virado chamada de ferramenta.
        assert_eq!(ponte.calls(), 0);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn erro_do_hospedeiro_volta_como_falha_para_o_script() {
        let (ponte, mut rx) = Bridge::start().unwrap();
        let url = ponte.url();
        let token = ponte.token().to_string();

        let cliente =
            std::thread::spawn(move || post(&url, Some(&token), r#"{"tool":"fs_write"}"#));
        let pedido = rx.recv().await.unwrap();
        pedido
            .reply
            .send(CallReply::err("negado pela política"))
            .unwrap();

        let (status, corpo) = cliente.join().unwrap();
        assert_eq!(status, 200);
        assert!(corpo.contains("\"ok\":false"), "{corpo}");
        assert!(corpo.contains("negado pela política"), "{corpo}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn hospedeiro_que_sumiu_no_meio_nao_pendura_o_script() {
        let (ponte, rx) = Bridge::start().unwrap();
        let url = ponte.url();
        let token = ponte.token().to_string();
        drop(rx); // o run acabou enquanto o script ainda rodava

        let (status, corpo) =
            std::thread::spawn(move || post(&url, Some(&token), r#"{"tool":"fs_read"}"#))
                .join()
                .unwrap();
        assert_eq!(status, 503);
        assert!(corpo.contains("encerrou"), "{corpo}");
    }
}
