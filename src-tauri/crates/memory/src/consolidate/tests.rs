//! Testes da consolidação contra um **servidor HTTP falso** (`std::net`, sem
//! dependência nova) — mesma receita de `crates/engine/src/client/tests.rs`.
//!
//! O que interessa aqui não é o protocolo (isso o `lr_engine` já testa), e sim
//! o comportamento diante de um modelo local de verdade: às vezes ele responde
//! o JSON pedido, às vezes devolve conversa fiada, às vezes o servidor cai no
//! meio. Cada caso tem um desfecho diferente para os episódios pendentes.

use super::*;
use lr_store::Store;
use serde_json::json;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

// -------------------------------------------------------- servidor falso ---

#[derive(Debug, Clone)]
struct FakeRequest {
    path: String,
    body: String,
}

struct FakeResponse {
    status: u16,
    body: String,
}

impl FakeResponse {
    /// Resposta de `/v1/chat/completions` com o texto que o modelo "gerou".
    fn completion(content: &str) -> Self {
        Self {
            status: 200,
            body: json!({
                "choices": [{
                    "finish_reason": "stop",
                    "message": { "role": "assistant", "content": content }
                }]
            })
            .to_string(),
        }
    }

    fn error(status: u16) -> Self {
        Self {
            status,
            body: json!({ "error": { "message": "modelo caiu" } }).to_string(),
        }
    }
}

struct FakeServer {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<FakeRequest>>>,
    stop: Arc<AtomicBool>,
}

impl FakeServer {
    fn spawn<F>(handler: F) -> Self
    where
        F: Fn(&FakeRequest) -> FakeResponse + Send + 'static,
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
                            write_response(&mut stream, &handler(&req));
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

    fn client(&self) -> LlamaClient {
        LlamaClient::new(format!("http://{}", self.addr))
    }

    fn requests(&self) -> Vec<FakeRequest> {
        self.requests.lock().unwrap().clone()
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
    let _method = start.next()?;
    let path = start.next()?.to_string();

    let mut len = 0usize;
    for line in lines {
        if let Some((k, v)) = line.split_once(':')
            && k.trim().eq_ignore_ascii_case("content-length")
        {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; len];
    if len > 0 && stream.read_exact(&mut body).is_err() {
        return None;
    }
    Some(FakeRequest {
        path,
        body: String::from_utf8_lossy(&body).to_string(),
    })
}

fn write_response(stream: &mut TcpStream, res: &FakeResponse) {
    let reason = if res.status == 200 {
        "OK"
    } else {
        "Internal Server Error"
    };
    let head = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        res.status,
        res.body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(res.body.as_bytes());
    let _ = stream.flush();
}

// ---------------------------------------------------------------- cenário ---

/// Memória de um projeto temporário, com dois episódios pendentes.
fn scenario() -> (TempDir, MemoryStore) {
    let dir = TempDir::new().unwrap();
    let mem = MemoryStore::new(
        Arc::new(Store::open_in_memory().unwrap()),
        Some(dir.path().to_path_buf()),
    );
    mem.record_episode(
        Some(1),
        Some("run-1"),
        "instalei as dependências com pnpm install",
    )
    .unwrap();
    mem.record_episode(
        Some(1),
        Some("run-2"),
        "rodei pnpm test e 3 testes falharam",
    )
    .unwrap();
    (dir, mem)
}

fn pending(mem: &MemoryStore) -> usize {
    let key = mem.workspace().map(|p| p.to_string_lossy().into_owned());
    mem.store()
        .pending_episodes(key.as_deref(), 50)
        .unwrap()
        .len()
}

// ------------------------------------------------------------------ puros ---

#[test]
fn the_request_forces_json_and_carries_the_episodes() {
    let req = consolidation_request(
        "qwen3-8b",
        &["rodei pnpm test".to_string()],
        &["usa pnpm".to_string()],
    );
    let body = serde_json::to_value(&req).unwrap();

    assert_eq!(body["model"], "qwen3-8b");
    assert_eq!(body["response_format"]["type"], "json_schema");
    assert_eq!(
        body["response_format"]["json_schema"]["schema"]["required"][0],
        "facts"
    );
    let user = body["messages"][1]["content"].as_str().unwrap();
    assert!(user.contains("rodei pnpm test"));
    assert!(user.contains("não repita"), "{user}");
    assert!(user.contains("usa pnpm"));
    // Prompt de tarefa auxiliar tem que ser curto.
    assert!(body["messages"][0]["content"].as_str().unwrap().len() < 700);
}

#[test]
fn parse_reply_reads_every_shape_a_small_model_emits() {
    let expected = vec!["usa pnpm".to_string(), "roda vitest".to_string()];

    assert_eq!(
        parse_reply(r#"{"facts":["usa pnpm","roda vitest"]}"#),
        expected
    );
    assert_eq!(
        parse_reply("```json\n{\"facts\": [\"usa pnpm\", \"roda vitest\"]}\n```"),
        expected
    );
    assert_eq!(parse_reply(r#"["usa pnpm","roda vitest"]"#), expected);
    assert_eq!(
        parse_reply(r#"{"fatos":[{"fato":"usa pnpm"},{"text":"roda vitest"}]}"#),
        expected
    );
    assert_eq!(parse_reply("- usa pnpm\n- roda vitest"), expected);
    assert_eq!(
        parse_reply("Claro! Aqui vai: {\"facts\": [\"usa pnpm\", \"roda vitest\"]} — pronto."),
        expected
    );

    // Resposta ruim não vira fato nenhum, e não explode.
    assert!(parse_reply("").is_empty());
    assert!(parse_reply("Desculpe, não entendi o pedido.").is_empty());
    assert!(parse_reply("{\"facts\": ").is_empty());
    assert!(parse_reply(r#"{"outra_coisa": 42}"#).is_empty());
    // Teto por rodada.
    let many = json!({ "facts": (0..20).map(|i| format!("fato {i}")).collect::<Vec<_>>() });
    assert_eq!(parse_reply(&many.to_string()).len(), MAX_NEW_FACTS);
}

#[test]
fn plan_keeps_the_durable_and_drops_the_noise() {
    let existing = vec!["este projeto usa pnpm".to_string()];
    let reply = json!({"facts": [
        "os testes rodam com pnpm test",
        "Este projeto usa pnpm!",
        "ok",
        "",
        "os testes rodam com pnpm test"
    ]})
    .to_string();

    let (approved, skipped) = plan(&reply, &existing, true);
    assert_eq!(approved.len(), 1);
    assert_eq!(approved[0].content, "os testes rodam com pnpm test");
    assert_eq!(approved[0].topic, "testes");
    // Duplicata do que já sabíamos, fato curto, vazio e repetido no lote.
    assert_eq!(skipped, 4);
}

// ----------------------------------------------------------- com servidor ---

#[tokio::test]
async fn a_good_reply_becomes_memory_and_clears_the_episodes() {
    let (dir, mem) = scenario();
    let server = FakeServer::spawn(|r| {
        assert_eq!(r.path, "/v1/chat/completions");
        FakeResponse::completion(
            &json!({"facts": [
                "este projeto instala dependências com pnpm install",
                "os testes rodam com pnpm test"
            ]})
            .to_string(),
        )
    });

    let report = mem
        .consolidate_now(&server.client(), "qwen3-8b")
        .await
        .unwrap();

    // Os dois episódios pendentes foram parar no prompt.
    let sent = &server.requests()[0].body;
    assert!(sent.contains("pnpm install"), "{sent}");
    assert!(sent.contains("3 testes falharam"), "{sent}");

    assert_eq!(report.episodes, 2);
    assert_eq!(report.added.len(), 2);
    assert_eq!(report.skipped, 0);
    assert_eq!(mem.fact_texts().unwrap().len(), 2);
    assert_eq!(pending(&mem), 0, "episódios lidos não voltam");

    // A face legível também foi atualizada.
    let index = crate::files::read_index(dir.path()).unwrap();
    assert!(index.contains("build.md"), "{index}");
    assert!(index.contains("testes.md"), "{index}");
}

#[tokio::test]
async fn a_bad_reply_adds_nothing_but_still_closes_the_episodes() {
    let (dir, mem) = scenario();
    let server = FakeServer::spawn(|_| {
        FakeResponse::completion("Claro! Posso ajudar com mais alguma coisa?")
    });

    let report = mem
        .consolidate_now(&server.client(), "qwen3-8b")
        .await
        .unwrap();

    assert_eq!(report.episodes, 2);
    assert!(report.added.is_empty());
    assert!(mem.fact_texts().unwrap().is_empty());
    // Nada durável não é erro: os episódios foram vistos e não voltam.
    assert_eq!(pending(&mem), 0);
    // E nenhuma pasta foi criada à toa.
    assert!(!crate::memory_dir(dir.path()).exists());
}

#[tokio::test]
async fn a_reply_repeating_what_we_know_adds_nothing() {
    let (_dir, mem) = scenario();
    mem.save("este projeto usa pnpm", None, None).unwrap();

    let server =
        FakeServer::spawn(|_| FakeResponse::completion(r#"{"facts":["Este projeto usa pnpm."]}"#));
    let report = mem
        .consolidate_now(&server.client(), "qwen3-8b")
        .await
        .unwrap();

    assert!(report.added.is_empty());
    assert_eq!(report.skipped, 1);
    assert_eq!(mem.fact_texts().unwrap().len(), 1);
}

#[tokio::test]
async fn a_server_failure_keeps_the_episodes_for_the_next_round() {
    let (_dir, mem) = scenario();
    let server = FakeServer::spawn(|_| FakeResponse::error(500));

    let err = mem
        .consolidate_now(&server.client(), "qwen3-8b")
        .await
        .unwrap_err();
    assert!(matches!(err, MemoryError::Engine(_)), "{err}");
    assert_eq!(pending(&mem), 2, "falha de rede não pode perder episódio");
}

#[tokio::test]
async fn with_nothing_pending_the_model_is_never_woken_up() {
    let dir = TempDir::new().unwrap();
    let mem = MemoryStore::new(
        Arc::new(Store::open_in_memory().unwrap()),
        Some(dir.path().to_path_buf()),
    );
    let server =
        FakeServer::spawn(|_| FakeResponse::completion(r#"{"facts":["não devia rodar"]}"#));

    let report = mem
        .consolidate_now(&server.client(), "qwen3-8b")
        .await
        .unwrap();

    assert_eq!(report, ConsolidateReport::default());
    assert!(
        server.requests().is_empty(),
        "não pode chamar o modelo à toa"
    );
}
