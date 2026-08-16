//! Testes do cliente contra um **servidor HTTP falso** (std::net, sem
//! dependência nova). As respostas SSE são escritas em pedaços, com pausa
//! entre eles, para exercitar de verdade o buffer de linha parcial do
//! cliente — que é onde mora o bug clássico de tool call fragmentada.

use super::*;
use serde_json::json;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

// ------------------------------------------------------- servidor falso ---

#[derive(Debug, Clone)]
struct FakeRequest {
    method: String,
    path: String,
    body: String,
    authorization: Option<String>,
}

struct FakeResponse {
    status: u16,
    content_type: &'static str,
    /// Escritos em sequência (chunked), com pausa entre eles.
    chunks: Vec<String>,
}

impl FakeResponse {
    fn json(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            content_type: "application/json",
            chunks: vec![body.into()],
        }
    }

    fn sse(chunks: Vec<String>) -> Self {
        Self {
            status: 200,
            content_type: "text/event-stream",
            chunks,
        }
    }

    fn error(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "application/json",
            chunks: vec![body.into()],
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

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn client(&self) -> LlamaClient {
        LlamaClient::new(self.url())
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
    let method = start.next()?.to_string();
    let path = start.next()?.to_string();

    let mut len = 0usize;
    let mut authorization = None;
    for line in lines {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_string());
        match k.as_str() {
            "content-length" => len = v.parse().unwrap_or(0),
            "authorization" => authorization = Some(v),
            _ => {}
        }
    }

    let mut body = vec![0u8; len];
    if len > 0 && stream.read_exact(&mut body).is_err() {
        return None;
    }
    Some(FakeRequest {
        method,
        path,
        body: String::from_utf8_lossy(&body).to_string(),
        authorization,
    })
}

fn write_response(stream: &mut TcpStream, res: &FakeResponse) {
    let reason = match res.status {
        200 => "OK",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Status",
    };
    let head = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        res.status, res.content_type
    );
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    for chunk in &res.chunks {
        let framed = format!("{:x}\r\n{chunk}\r\n", chunk.len());
        if stream.write_all(framed.as_bytes()).is_err() {
            return;
        }
        let _ = stream.flush();
        // Pausa curta: força o cliente a ver o stream em pedaços.
        thread::sleep(Duration::from_millis(3));
    }
    let _ = stream.write_all(b"0\r\n\r\n");
    let _ = stream.flush();
}

/// Uma linha SSE completa (`data: {...}` + linha em branco).
fn data(payload: &str) -> String {
    format!("data: {payload}\n\n")
}

fn collect(deltas: &Arc<Mutex<Vec<ChatDelta>>>) -> Vec<ChatDelta> {
    deltas.lock().unwrap().clone()
}

fn req() -> ChatRequest {
    ChatRequest::new("modelo-teste", vec![ChatMessage::user("oi")])
}

// -------------------------------------------------------------- /props ---

#[tokio::test]
async fn props_endpoint_is_parsed() {
    let server = FakeServer::spawn(|r| {
        assert_eq!(r.path, "/props");
        assert_eq!(r.method, "GET");
        FakeResponse::json(
            json!({
                "model_path": "/m/Qwen3-8B-Q4_K_M.gguf",
                "chat_template_caps": {
                    "supports_tools": true,
                    "supports_parallel_tool_calls": false,
                    "supports_system_role": true
                },
                "modalities": { "vision": true },
                "default_generation_settings": { "n_ctx": 16384 }
            })
            .to_string(),
        )
    });
    let props = server.client().props().await.unwrap();
    assert!(props.supports_tools());
    assert!(!props.chat_template_caps.supports_parallel_tool_calls);
    assert_eq!(props.n_ctx, Some(16384));
    assert_eq!(props.modalities, vec!["vision".to_string()]);
}

/// Servidor no ar mas sem `/props` (build antigo) tem que virar erro — um
/// `ServerProps` default seria lido como "não suporta ferramentas".
#[tokio::test]
async fn props_failure_is_an_error_not_a_default() {
    let server = FakeServer::spawn(|_| FakeResponse::error(404, "não encontrado"));
    let err = server.client().props().await.unwrap_err();
    assert!(matches!(err, EngineError::Http { status: 404, .. }));
}

#[tokio::test]
async fn api_key_goes_as_bearer() {
    let server = FakeServer::spawn(|_| FakeResponse::json("{}"));
    let client = LlamaClient::new(server.url()).with_api_key("s3cr3t");
    client.props().await.unwrap();
    assert_eq!(
        server.requests()[0].authorization.as_deref(),
        Some("Bearer s3cr3t")
    );

    // String vazia não vira header (senão o servidor recusa por API key ruim).
    let client = LlamaClient::new(server.url()).with_optional_api_key(Some(String::new()));
    client.props().await.unwrap();
    assert_eq!(server.requests()[1].authorization, None);
}

// ------------------------------------------------------------ streaming ---

/// (a) `arguments` partido em 4 fragmentos, com `id`/`name` só no primeiro.
#[tokio::test]
async fn stream_reassembles_fragmented_tool_call_arguments() {
    let server = FakeServer::spawn(|_| {
        FakeResponse::sse(vec![
            data(&json!({"choices":[{"index":0,"delta":{"role":"assistant","content":""}}]}).to_string()),
            data(&json!({"choices":[{"index":0,"delta":{"tool_calls":[
                {"index":0,"id":"call_abc","type":"function","function":{"name":"fs_read","arguments":"{\"pa"}}
            ]}}]}).to_string()),
            data(&json!({"choices":[{"index":0,"delta":{"tool_calls":[
                {"index":0,"function":{"arguments":"th\": \"RE"}}
            ]}}]}).to_string()),
            data(&json!({"choices":[{"index":0,"delta":{"tool_calls":[
                {"index":0,"function":{"arguments":"ADME"}}
            ]}}]}).to_string()),
            data(&json!({"choices":[{"index":0,"delta":{"tool_calls":[
                {"index":0,"function":{"arguments":".md\"}"}}
            ]}}]}).to_string()),
            data(&json!({
                "choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],
                "timings":{"prompt_n":31,"predicted_n":42,"predicted_ms":120.5,"predicted_per_second":348.5}
            }).to_string()),
            data("[DONE]"),
        ])
    });

    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let mut on_delta = move |d: ChatDelta| sink.lock().unwrap().push(d);
    let out = server
        .client()
        .chat_stream(&req(), &mut on_delta)
        .await
        .unwrap();

    assert_eq!(out.finish_reason.as_deref(), Some("tool_calls"));
    assert!(out.wants_tools());
    assert_eq!(out.tool_calls.len(), 1);
    let call = &out.tool_calls[0];
    assert_eq!(call.id, "call_abc");
    assert_eq!(call.name, "fs_read");
    assert_eq!(call.arguments_json, r#"{"path": "README.md"}"#);
    assert_eq!(call.arguments().unwrap()["path"], "README.md");
    assert_eq!(out.content, "");
    assert_eq!(out.timings.as_ref().unwrap().predicted_n, Some(42));
    assert_eq!(out.timings.as_ref().unwrap().prompt_n, Some(31));

    // Os fragmentos também saem ao vivo, todos no mesmo index.
    let frags: Vec<_> = collect(&seen)
        .into_iter()
        .filter_map(|d| match d {
            ChatDelta::ToolCall {
                index,
                args_fragment,
                ..
            } => Some((index, args_fragment)),
            _ => None,
        })
        .collect();
    assert_eq!(frags.len(), 4);
    assert!(frags.iter().all(|(i, _)| *i == 0));

    // E o turno reconstruído para o histórico preserva id/nome/args.
    let msg = out.to_assistant_message();
    assert_eq!(msg.role, "assistant");
    assert_eq!(msg.tool_calls[0].id, "call_abc");
    assert_eq!(msg.tool_calls[0].function.name, "fs_read");
    assert_eq!(
        msg.tool_calls[0].function.arguments,
        r#"{"path": "README.md"}"#
    );
}

/// (b) `reasoning_content` + `content` + tool calls no MESMO stream.
#[tokio::test]
async fn stream_mixes_reasoning_content_and_tool_calls() {
    let server = FakeServer::spawn(|_| {
        FakeResponse::sse(vec![
            data(&json!({"choices":[{"delta":{"reasoning_content":"preciso "}}]}).to_string()),
            data(&json!({"choices":[{"delta":{"reasoning_content":"ler o arquivo"}}]}).to_string()),
            data(&json!({"choices":[{"delta":{"content":"Vou verificar."}}]}).to_string()),
            data(&json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"c1","type":"function","function":{"name":"fs_list","arguments":"{}"}}
            ]}}]}).to_string()),
            data(&json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}).to_string()),
            data("[DONE]"),
        ])
    });

    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let mut on_delta = move |d: ChatDelta| sink.lock().unwrap().push(d);
    let out = server
        .client()
        .chat_stream(&req(), &mut on_delta)
        .await
        .unwrap();

    assert_eq!(out.reasoning, "preciso ler o arquivo");
    assert_eq!(out.content, "Vou verificar.");
    assert_eq!(out.tool_calls.len(), 1);
    assert_eq!(out.tool_calls[0].name, "fs_list");

    let deltas = collect(&seen);
    assert_eq!(
        deltas
            .iter()
            .filter(|d| matches!(d, ChatDelta::Reasoning(_)))
            .count(),
        2
    );
    assert!(
        deltas
            .iter()
            .any(|d| matches!(d, ChatDelta::Text(t) if t == "Vou verificar."))
    );
    assert!(
        deltas
            .iter()
            .any(|d| matches!(d, ChatDelta::ToolCall { .. }))
    );
}

/// (c) `finish_reason: "stop"` — resposta puramente textual.
#[tokio::test]
async fn stream_plain_answer_finishes_with_stop() {
    let server = FakeServer::spawn(|_| {
        FakeResponse::sse(vec![
            data(&json!({"choices":[{"delta":{"content":"Olá"}}]}).to_string()),
            data(&json!({"choices":[{"delta":{"content":", mundo"}}]}).to_string()),
            data(
                &json!({"choices":[{"delta":{},"finish_reason":"stop"}],
                        "timings":{"predicted_n":7,"predicted_per_second":50.0}})
                .to_string(),
            ),
            data("[DONE]"),
        ])
    });
    let mut noop = |_: ChatDelta| {};
    let out = server
        .client()
        .chat_stream(&req(), &mut noop)
        .await
        .unwrap();
    assert_eq!(out.content, "Olá, mundo");
    assert_eq!(out.finish_reason.as_deref(), Some("stop"));
    assert!(!out.wants_tools());
    assert_eq!(
        out.timings.as_ref().unwrap().predicted_per_second,
        Some(50.0)
    );
    // Sem tool calls o histórico volta como assistente simples.
    assert!(out.to_assistant_message().tool_calls.is_empty());
}

/// Duas chamadas em paralelo, com fragmentos intercalados entre os índices.
#[tokio::test]
async fn stream_keeps_parallel_tool_calls_separate() {
    let server = FakeServer::spawn(|_| {
        FakeResponse::sse(vec![
            data(&json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"a","type":"function","function":{"name":"fs_read","arguments":"{\"p\":"}}
            ]}}]}).to_string()),
            data(&json!({"choices":[{"delta":{"tool_calls":[
                {"index":1,"id":"b","type":"function","function":{"name":"fs_list","arguments":"{\"d\":"}}
            ]}}]}).to_string()),
            data(&json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"function":{"arguments":"\"a.md\"}"}}
            ]}}]}).to_string()),
            data(&json!({"choices":[{"delta":{"tool_calls":[
                {"index":1,"function":{"arguments":"\".\"}"}}
            ]}}]}).to_string()),
            data(&json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}).to_string()),
            data("[DONE]"),
        ])
    });
    let mut noop = |_: ChatDelta| {};
    let out = server
        .client()
        .chat_stream(&req(), &mut noop)
        .await
        .unwrap();
    assert_eq!(out.tool_calls.len(), 2);
    assert_eq!(out.tool_calls[0].id, "a");
    assert_eq!(out.tool_calls[0].arguments_json, r#"{"p":"a.md"}"#);
    assert_eq!(out.tool_calls[1].id, "b");
    assert_eq!(out.tool_calls[1].arguments_json, r#"{"d":"."}"#);
}

/// (f) Linha partida no meio do JSON, comentários SSE, ruído não-JSON,
/// `[DONE]` e lixo depois do `[DONE]` (que precisa ser ignorado).
#[tokio::test]
async fn stream_survives_partial_lines_and_noise() {
    let server = FakeServer::spawn(|_| {
        FakeResponse::sse(vec![
            ": keep-alive\n\n".to_string(),
            "\n".to_string(),
            // O mesmo `data:` cortado ao meio, em dois writes TCP.
            "data: {\"choices\":[{\"delta\":{\"cont".to_string(),
            "ent\":\"parte-inteira\"}}]}\n\n".to_string(),
            "data: isto-nao-e-json\n\n".to_string(),
            "evento-solto-sem-prefixo\n".to_string(),
            data(
                &json!({"choices":[{"delta":{"content":"!"},"finish_reason":"stop"}]}).to_string(),
            ),
            data("[DONE]"),
            // Nada depois do [DONE] pode entrar no resultado.
            data(&json!({"choices":[{"delta":{"content":"IGNORAR"}}]}).to_string()),
        ])
    });
    let mut noop = |_: ChatDelta| {};
    let out = server
        .client()
        .chat_stream(&req(), &mut noop)
        .await
        .unwrap();
    assert_eq!(out.content, "parte-inteira!");
    assert_eq!(out.finish_reason.as_deref(), Some("stop"));
}

/// Stream que termina sem `[DONE]` e sem newline final ainda entrega tudo.
#[tokio::test]
async fn stream_without_done_flushes_trailing_line() {
    let server = FakeServer::spawn(|_| {
        FakeResponse::sse(vec![
            data(&json!({"choices":[{"delta":{"content":"a"}}]}).to_string()),
            // Sem "\n" no fim: só o EOF encerra.
            format!(
                "data: {}",
                json!({"choices":[{"delta":{"content":"b"},"finish_reason":"stop"}]})
            ),
        ])
    });
    let mut noop = |_: ChatDelta| {};
    let out = server
        .client()
        .chat_stream(&req(), &mut noop)
        .await
        .unwrap();
    assert_eq!(out.content, "ab");
    assert_eq!(out.finish_reason.as_deref(), Some("stop"));
}

/// `<think>` inline (modelo sem parser de reasoning no servidor), partido
/// entre chunks — vai para `reasoning`, não para `content`.
#[tokio::test]
async fn stream_splits_inline_think_tags() {
    let server = FakeServer::spawn(|_| {
        FakeResponse::sse(vec![
            data(&json!({"choices":[{"delta":{"content":"<thi"}}]}).to_string()),
            data(&json!({"choices":[{"delta":{"content":"nk>plano</think"}}]}).to_string()),
            data(&json!({"choices":[{"delta":{"content":">resposta"}}]}).to_string()),
            data("[DONE]"),
        ])
    });
    let mut noop = |_: ChatDelta| {};
    let out = server
        .client()
        .chat_stream(&req(), &mut noop)
        .await
        .unwrap();
    assert_eq!(out.reasoning, "plano");
    assert_eq!(out.content, "resposta");
}

#[tokio::test]
async fn stream_reports_http_errors_with_body() {
    let server = FakeServer::spawn(|_| {
        FakeResponse::error(
            500,
            json!({"error":{"message":"modelo não carregado"}}).to_string(),
        )
    });
    let mut noop = |_: ChatDelta| {};
    let err = server
        .client()
        .chat_stream(&req(), &mut noop)
        .await
        .unwrap_err();
    match err {
        EngineError::Http { status, body } => {
            assert_eq!(status, 500);
            assert!(body.contains("modelo não carregado"), "corpo: {body}");
        }
        other => panic!("erro inesperado: {other}"),
    }
}

/// Erro no meio do stream (o servidor manda `data: {"error":...}`).
#[tokio::test]
async fn stream_surfaces_mid_stream_error() {
    let server = FakeServer::spawn(|_| {
        FakeResponse::sse(vec![
            data(&json!({"choices":[{"delta":{"content":"oi"}}]}).to_string()),
            data(&json!({"error":{"message":"contexto estourado"}}).to_string()),
        ])
    });
    let mut noop = |_: ChatDelta| {};
    let err = server
        .client()
        .chat_stream(&req(), &mut noop)
        .await
        .unwrap_err();
    assert!(matches!(err, EngineError::Protocol(m) if m.contains("contexto estourado")));
}

// -------------------------------------------------------- complete_once ---

#[tokio::test]
async fn complete_once_reads_message_and_tool_calls() {
    let server = FakeServer::spawn(|r| {
        assert_eq!(r.path, "/v1/chat/completions");
        let body: Value = serde_json::from_str(&r.body).unwrap();
        assert_eq!(body["stream"], false);
        FakeResponse::json(
            json!({
                "choices":[{
                    "finish_reason":"tool_calls",
                    "message":{
                        "role":"assistant",
                        "content":"vou listar",
                        "reasoning_content":"pensando",
                        "tool_calls":[{"id":"x1","type":"function",
                                       "function":{"name":"fs_list","arguments":"{\"dir\":\".\"}"}}]
                    }
                }],
                "timings":{"predicted_n":9}
            })
            .to_string(),
        )
    });
    let out = server.client().complete_once(&req()).await.unwrap();
    assert_eq!(out.content, "vou listar");
    assert_eq!(out.reasoning, "pensando");
    assert_eq!(out.finish_reason.as_deref(), Some("tool_calls"));
    assert_eq!(out.tool_calls[0].id, "x1");
    assert_eq!(out.tool_calls[0].arguments_json, r#"{"dir":"."}"#);
    assert_eq!(out.timings.unwrap().predicted_n, Some(9));
}

#[tokio::test]
async fn complete_once_splits_inline_think() {
    let server = FakeServer::spawn(|_| {
        FakeResponse::json(
            json!({"choices":[{"message":{"content":"<think>hmm</think>Título"},
                               "finish_reason":"stop"}]})
            .to_string(),
        )
    });
    let out = server.client().complete_once(&req()).await.unwrap();
    assert_eq!(out.content, "Título");
    assert_eq!(out.reasoning, "hmm");
}

// ----------------------------------------------------------- embeddings ---

#[tokio::test]
async fn embeddings_returns_one_vector_per_input() {
    let server = FakeServer::spawn(|r| {
        assert_eq!(r.path, "/v1/embeddings");
        let body: Value = serde_json::from_str(&r.body).unwrap();
        assert_eq!(body["model"], "bge-m3");
        assert_eq!(body["input"][1], "b");
        FakeResponse::json(
            json!({"data":[
                {"embedding":[0.5, -0.25]},
                // pooling=none devolve por token: ficamos com a primeira linha.
                {"embedding":[[1.0, 2.0],[3.0, 4.0]]}
            ]})
            .to_string(),
        )
    });
    let out = server
        .client()
        .embeddings("bge-m3", &["a".to_string(), "b".to_string()])
        .await
        .unwrap();
    assert_eq!(out, vec![vec![0.5, -0.25], vec![1.0, 2.0]]);
}

#[tokio::test]
async fn embeddings_short_circuits_on_empty_input() {
    let server = FakeServer::spawn(|_| FakeResponse::json("{}"));
    let out = server.client().embeddings("m", &[]).await.unwrap();
    assert!(out.is_empty());
    assert!(server.requests().is_empty());
}

// ---------------------------------------------------------- input_tokens ---

#[tokio::test]
async fn input_tokens_prefers_the_dedicated_endpoint() {
    let server = FakeServer::spawn(|r| {
        assert_eq!(r.path, "/v1/chat/completions/input_tokens");
        FakeResponse::json(json!({"prompt_tokens": 1234}).to_string())
    });
    assert_eq!(server.client().input_tokens(&req()).await.unwrap(), 1234);
}

#[tokio::test]
async fn input_tokens_falls_back_to_template_plus_tokenize() {
    let server = FakeServer::spawn(|r| match r.path.as_str() {
        "/v1/chat/completions/input_tokens" => FakeResponse::error(404, "{}"),
        "/apply-template" => FakeResponse::json(json!({"prompt":"<|im_start|>oi"}).to_string()),
        "/tokenize" => FakeResponse::json(json!({"tokens":[1,2,3,4,5]}).to_string()),
        other => panic!("caminho inesperado: {other}"),
    });
    assert_eq!(server.client().input_tokens(&req()).await.unwrap(), 5);
    let paths: Vec<_> = server.requests().iter().map(|r| r.path.clone()).collect();
    assert_eq!(
        paths,
        vec![
            "/v1/chat/completions/input_tokens",
            "/apply-template",
            "/tokenize"
        ]
    );
}

#[tokio::test]
async fn input_tokens_falls_back_to_char_heuristic() {
    let server = FakeServer::spawn(|_| FakeResponse::error(404, "{}"));
    let request = ChatRequest::new(
        "m",
        vec![
            ChatMessage::system("você é um agente"),
            ChatMessage::user("liste os arquivos"),
        ],
    );
    let got = server.client().input_tokens(&request).await.unwrap();
    assert_eq!(got, estimate_tokens(&request.messages));
    assert!(got > 0);
}

// ------------------------------------------------- serialização do corpo ---

#[test]
fn request_body_omits_absent_fields() {
    let body = serde_json::to_value(ChatRequest::new("m", vec![ChatMessage::user("oi")])).unwrap();
    for absent in [
        "tools",
        "tool_choice",
        "parallel_tool_calls",
        "temperature",
        "top_p",
        "top_k",
        "max_tokens",
    ] {
        assert!(
            body.get(absent).is_none(),
            "campo `{absent}` não deveria ir no corpo (o llama-server é chato com null)"
        );
    }
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][0]["content"], "oi");
    assert_eq!(body["stream"], false);
    // Mensagem simples não carrega os campos de ferramenta.
    assert!(body["messages"][0].get("tool_calls").is_none());
    assert!(body["messages"][0].get("tool_call_id").is_none());
}

#[test]
fn request_body_carries_tools_and_sampling() {
    let request = ChatRequest {
        temperature: Some(0.7),
        top_k: Some(40),
        max_tokens: Some(512),
        parallel_tool_calls: Some(false),
        ..ChatRequest::new("m", vec![ChatMessage::user("oi")])
    }
    .with_tools(vec![json!({"type":"function"})])
    .with_extra("reasoning_effort", json!("high"));
    let body = serde_json::to_value(&request).unwrap();
    // f32 → JSON alarga a mantissa (0.699999988…); o que importa é o valor.
    assert!((body["temperature"].as_f64().unwrap() - 0.7).abs() < 1e-6);
    assert_eq!(body["top_k"], 40);
    assert_eq!(body["max_tokens"], 512);
    assert_eq!(body["parallel_tool_calls"], false);
    assert_eq!(body["tool_choice"], "auto");
    assert_eq!(body["reasoning_effort"], "high");
    assert!(body.get("top_p").is_none());

    let named = serde_json::to_value(ToolChoice::function("fs_read")).unwrap();
    assert_eq!(named["type"], "function");
    assert_eq!(named["function"]["name"], "fs_read");
    assert_eq!(serde_json::to_value(ToolChoice::none()).unwrap(), "none");
}

/// O ciclo pedido→resultado tem que voltar íntegro ao modelo no passo
/// seguinte: sem isto o template não pareia a resposta com a chamada.
#[test]
fn tool_call_history_roundtrip() {
    let outcome = ChatOutcome {
        content: String::new(),
        tool_calls: vec![ToolCallReq {
            id: "call_1".into(),
            name: "fs_read".into(),
            arguments_json: r#"{"path":"a.md"}"#.into(),
        }],
        ..Default::default()
    };
    let history = vec![
        ChatMessage::user("leia a.md"),
        outcome.to_assistant_message(),
        ChatMessage::tool_result("call_1", "fs_read", "conteúdo do arquivo"),
    ];
    let body = serde_json::to_value(ChatRequest::new("m", history)).unwrap();
    let msgs = &body["messages"];
    assert_eq!(msgs[1]["role"], "assistant");
    assert_eq!(msgs[1]["tool_calls"][0]["id"], "call_1");
    assert_eq!(msgs[1]["tool_calls"][0]["type"], "function");
    assert_eq!(msgs[1]["tool_calls"][0]["function"]["name"], "fs_read");
    assert_eq!(
        msgs[1]["tool_calls"][0]["function"]["arguments"],
        r#"{"path":"a.md"}"#
    );
    assert_eq!(msgs[2]["role"], "tool");
    assert_eq!(msgs[2]["toolCallId"], Value::Null);
    assert_eq!(msgs[2]["tool_call_id"], "call_1");
    assert_eq!(msgs[2]["name"], "fs_read");
}

#[test]
fn multimodal_parts_serialize_in_openai_shape() {
    let msg = ChatMessage::user_parts(vec![
        ContentPart::text("o que é isto?"),
        ContentPart::image("data:image/png;base64,AAA"),
    ]);
    let v = serde_json::to_value(&msg).unwrap();
    assert_eq!(v["content"][0]["type"], "text");
    assert_eq!(v["content"][1]["type"], "image_url");
    assert_eq!(
        v["content"][1]["image_url"]["url"],
        "data:image/png;base64,AAA"
    );
    assert_eq!(msg.text(), "o que é isto?");
}

// -------------------------------------------------- sanitização de schema ---

fn spec(parameters: Value) -> ToolSpec {
    ToolSpec {
        name: "fs_read".into(),
        description: "lê um arquivo".into(),
        parameters,
        category: lr_types::agent::ToolCategory::Read,
        tier: lr_types::agent::ToolTier::Safe,
        origin: lr_types::agent::ToolOrigin::Builtin,
        read_only: true,
    }
}

/// (e) O conversor GBNF do llama.cpp rejeita atalhos PCRE e referências —
/// tudo isso sai antes de chegar ao servidor.
#[test]
fn tool_specs_are_sanitized_for_the_gbnf_converter() {
    let tools = tool_specs_to_api(&[spec(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "urn:fs_read",
        "type": "object",
        "definitions": { "Path": { "type": "string" } },
        "properties": {
            "path":    { "type": "string", "pattern": "^\\d+$" },
            "iso":     { "type": "string", "pattern": "^[A-Za-z_.-]+$" },
            "ref":     { "$ref": "#/definitions/Path" },
            "mode":    { "type": "string", "enum": ["text", "bytes"] },
            "limit":   { "type": "integer", "minimum": 1, "maximum": 100 },
            "nested":  {
                "type": "object",
                "properties": { "deep": { "type": "string", "pattern": "\\w+" } }
            },
            "list":    { "type": "array", "items": { "type": "string", "pattern": "\\s" } }
        },
        "required": ["path"],
        "not": { "required": ["ref"] },
        "if": { "properties": { "mode": { "const": "bytes" } } },
        "patternProperties": { "^x-": { "type": "string" } },
        "additionalProperties": false
    }))]);

    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["type"], "function");
    assert_eq!(tools[0]["function"]["name"], "fs_read");
    let p = &tools[0]["function"]["parameters"];

    for gone in [
        "$schema",
        "$id",
        "definitions",
        "not",
        "if",
        "patternProperties",
    ] {
        assert!(p.get(gone).is_none(), "`{gone}` deveria ter sido removido");
    }
    // `pattern` com atalho PCRE sai; regex simples fica.
    assert!(p["properties"]["path"].get("pattern").is_none());
    assert_eq!(p["properties"]["iso"]["pattern"], "^[A-Za-z_.-]+$");
    // `$ref` some, mas a propriedade continua existindo (vira livre).
    assert!(p["properties"]["ref"].is_object());
    assert!(p["properties"]["ref"].get("$ref").is_none());
    // Recursão em objetos aninhados e em `items`.
    assert!(
        p["properties"]["nested"]["properties"]["deep"]
            .get("pattern")
            .is_none()
    );
    assert!(p["properties"]["list"]["items"].get("pattern").is_none());
    // O que o conversor entende continua intacto.
    assert_eq!(p["type"], "object");
    assert_eq!(p["properties"]["mode"]["enum"][1], "bytes");
    assert_eq!(p["properties"]["limit"]["minimum"], 1);
    assert_eq!(p["required"][0], "path");
    assert_eq!(p["additionalProperties"], false);
}

#[test]
fn sanitizer_never_touches_property_names_or_enum_data() {
    // Uma propriedade LEGÍTIMA chamada `if` (ou `$ref`) não pode sumir só
    // porque o nome coincide com uma palavra-chave.
    let tools = tool_specs_to_api(&[spec(json!({
        "type": "object",
        "properties": {
            "if":   { "type": "string" },
            "$ref": { "type": "string" },
            "kind": { "enum": [{ "$ref": "isto-e-dado" }, "outro"] }
        }
    }))]);
    let p = &tools[0]["function"]["parameters"];
    assert_eq!(p["properties"]["if"]["type"], "string");
    assert_eq!(p["properties"]["$ref"]["type"], "string");
    assert_eq!(p["properties"]["kind"]["enum"][0]["$ref"], "isto-e-dado");
}

#[test]
fn parameters_are_normalized_to_an_object_schema() {
    let tools = tool_specs_to_api(&[spec(Value::Null), spec(json!({"properties": {}}))]);
    assert_eq!(tools[0]["function"]["parameters"]["type"], "object");
    assert!(tools[0]["function"]["parameters"]["properties"].is_object());
    assert_eq!(tools[1]["function"]["parameters"]["type"], "object");
}

#[test]
fn regex_safety_matches_the_gbnf_converter_limits() {
    for safe in ["^[a-z]+$", "a|b", "x{2,4}", "\\.", "^/tmp/.*$"] {
        assert!(regex_is_gbnf_safe(safe), "deveria ser aceita: {safe}");
    }
    for unsafe_p in ["\\d+", "\\w", "\\s*", "\\bfoo", "(?i)x", "(?:a)", "\\p{L}"] {
        assert!(
            !regex_is_gbnf_safe(unsafe_p),
            "deveria ser rejeitada: {unsafe_p}"
        );
    }
}

// ------------------------------------------------------------- unitários ---

#[test]
fn token_count_extraction_accepts_known_shapes() {
    assert_eq!(extract_token_count(&json!(12)), Some(12));
    assert_eq!(extract_token_count(&json!([1, 2, 3])), Some(3));
    assert_eq!(extract_token_count(&json!({"tokens":[1,2]})), Some(2));
    assert_eq!(extract_token_count(&json!({"prompt_tokens":5})), Some(5));
    assert_eq!(
        extract_token_count(&json!({"usage":{"prompt_tokens":7}})),
        Some(7)
    );
    assert_eq!(extract_token_count(&json!({"nada":true})), None);
}

#[test]
fn think_splitter_holds_partial_tags() {
    let mut s = ThinkSplitter::default();
    assert!(s.feed("olá <thi").iter().all(|(r, _)| !*r));
    let segs = s.feed("nk>segredo</think>fim");
    assert_eq!(
        segs,
        vec![(true, "segredo".to_string()), (false, "fim".to_string())]
    );
    assert_eq!(s.flush(), None);
}

#[test]
fn base_url_is_normalized() {
    let c = LlamaClient::new("http://127.0.0.1:11711/");
    assert_eq!(c.base_url(), "http://127.0.0.1:11711");
    assert_eq!(c.url("/props"), "http://127.0.0.1:11711/props");
}

/// O roteador recusa por memória com "failed to load". Repassar isso cru
/// manda a pessoa procurar erro de rede quando o problema é a placa cheia.
#[test]
fn a_model_that_does_not_fit_is_recognized_as_such() {
    assert_eq!(
        failed_to_load("model name=Qwen3-27B-UD-Q2_K_XL.gguf failed to load"),
        Some("Qwen3-27B-UD-Q2_K_XL.gguf".to_string())
    );
    // Sem o nome ainda é o mesmo problema.
    assert_eq!(failed_to_load("failed to load"), Some("escolhido".into()));
    assert_eq!(failed_to_load("context window exceeded"), None);

    let erro = EngineError::ModelLoad {
        model: "m.gguf".into(),
    };
    let texto = erro.to_string();
    assert!(texto.contains("m.gguf"), "{texto}");
    assert!(texto.contains("memória de vídeo"), "{texto}");
}
