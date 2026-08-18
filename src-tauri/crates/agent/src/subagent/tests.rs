//! Testes da delegação contra um **servidor HTTP falso** (std::net, sem
//! dependência nova), o mesmo de `scout/tests.rs`.
//!
//! O que precisa ficar provado aqui é comportamento, não implementação:
//! 1. o ajudante começa com contexto NOVO e devolve só o resumo;
//! 2. o papel manda no cardápio dele, e `agent_delegate` nunca entra nele;
//! 3. um ajudante que não termina não derruba o pai — volta como texto;
//! 4. o que ele escreveu chega aos acumuladores do run;
//! 5. os dois eventos saem, na ordem, com o mesmo `call_id`.

use super::*;
use crate::events::EventSink;
use lr_types::agent::{RunEvent, ToolTier};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

// -------------------------------------------------------- servidor falso ---

#[derive(Debug, Clone)]
struct FakeRequest {
    path: String,
    body: String,
}

struct FakeResponse {
    status: u16,
    content_type: &'static str,
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

    fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Corpos enviados para o endpoint de chat (na ordem).
    fn chat_bodies(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.path == "/v1/chat/completions")
            .map(|r| r.body.clone())
            .collect()
    }

    /// O primeiro pedido de passo que o ajudante mandou.
    fn first_step(&self) -> String {
        self.chat_bodies()
            .into_iter()
            .find(|b| b.contains("\"stream\":true"))
            .expect("o ajudante tinha que ter falado com o modelo")
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
    let head = format!(
        "HTTP/1.1 {} OK\r\nContent-Type: {}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
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
        thread::sleep(Duration::from_millis(2));
    }
    let _ = stream.write_all(b"0\r\n\r\n");
    let _ = stream.flush();
}

fn sse(payload: &str) -> String {
    format!("data: {payload}\n\n")
}

/// Resposta em stream com um texto e o fim do turno.
fn stream_text(text: &str) -> Vec<String> {
    vec![
        sse(&json!({"choices":[{"delta":{"content": text}}]}).to_string()),
        sse(&json!({
            "choices": [{ "delta": {}, "finish_reason": "stop" }],
            "timings": { "prompt_n": 40, "predicted_n": 8 }
        })
        .to_string()),
        sse("[DONE]"),
    ]
}

/// Resposta em stream pedindo UMA ferramenta.
fn stream_tool_call(id: &str, name: &str, args: &str) -> Vec<String> {
    vec![
        sse(&json!({"choices":[{"delta":{"tool_calls":[{
            "index": 0, "id": id, "type": "function",
            "function": { "name": name, "arguments": args }
        }]}}]})
        .to_string()),
        sse(&json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}).to_string()),
        sse("[DONE]"),
    ]
}

/// Servidor que responde com o mesmo texto em qualquer passo.
fn answering_server(text: &'static str) -> FakeServer {
    FakeServer::spawn(move |r| match r.path.as_str() {
        "/v1/chat/completions/input_tokens" => {
            FakeResponse::json(json!({ "prompt_tokens": 120 }).to_string())
        }
        "/v1/chat/completions" => FakeResponse::sse(stream_text(text)),
        _ => FakeResponse::error(404, "{}"),
    })
}

// ------------------------------------------------------ catálogo de teste ---

/// Ferramenta de mentira: registra que rodou e, quando é de edição, diz qual
/// arquivo mexeu.
struct FakeTool {
    name: &'static str,
    category: ToolCategory,
}

#[async_trait]
impl Tool for FakeTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "ferramenta de teste"
    }

    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": { "path": { "type": "string" } } })
    }

    fn category(&self) -> ToolCategory {
        self.category
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("sem-caminho")
            .to_string();
        let out = ToolOutput::text(format!("{} rodou em {path}", self.name));
        Ok(match self.category {
            ToolCategory::Edit => out.with_changed(vec![path]),
            _ => out,
        })
    }
}

fn registry(tools: &[(&'static str, ToolCategory)]) -> Arc<ToolRegistry> {
    let mut reg = ToolRegistry::new();
    for (name, category) in tools {
        reg.register(Arc::new(FakeTool {
            name,
            category: *category,
        }));
    }
    Arc::new(reg)
}

/// Catálogo pequeno: cabe inteiro numa janela de 8k.
fn catalog() -> Arc<ToolRegistry> {
    registry(&[
        ("fs_read", ToolCategory::Read),
        ("fs_list", ToolCategory::Read),
        ("fs_grep", ToolCategory::Read),
        ("fs_edit", ToolCategory::Edit),
        ("fs_write", ToolCategory::Edit),
        ("terminal_run", ToolCategory::Execute),
    ])
}

// ----------------------------------------------------------------- harness ---

/// Um run montado à mão: só o que a delegação precisa em volta.
struct Harness {
    store: Arc<Store>,
    handle: Arc<RunHandle>,
    config: Arc<AgentConfig>,
    events: Arc<Mutex<Vec<RunEvent>>>,
    written: Arc<Mutex<Vec<String>>>,
    commands: Arc<Mutex<Vec<CommandRecord>>>,
    steps: Arc<AtomicU32>,
    _dir: tempfile::TempDir,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_in_memory().unwrap());
    store
        .create_run("r1", None, "m", RunMode::Yolo, true, None, "objetivo")
        .unwrap();

    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = events.clone();
    let sink = EventSink::new("r1", Some(store.clone()))
        .with_listener(Arc::new(move |ev| seen.lock().unwrap().push(ev)));

    Harness {
        handle: RunHandle::new("r1".into(), sink),
        config: Arc::new(AgentConfig::new(dir.path().to_path_buf())),
        store,
        events,
        written: Arc::new(Mutex::new(Vec::new())),
        commands: Arc::new(Mutex::new(Vec::new())),
        steps: Arc::new(AtomicU32::new(0)),
        _dir: dir,
    }
}

/// A instrução da conversa é da PESSOA, não do pai: ela vale para quem
/// trabalha, inclusive o ajudante. O que não pode atravessar é o histórico —
/// e isso o teste do contexto novo prova pela contagem de mensagens.
fn options() -> RunOptions {
    serde_json::from_value(json!({
        "chatId": 0,
        "model": "m",
        "mode": "yolo",
        "maxSteps": 40,
        "systemPrompt": "INSTRUCAO-DA-PESSOA: fale como um pirata."
    }))
    .unwrap()
}

fn delegate(h: &Harness, server: &FakeServer, registry: Arc<ToolRegistry>) -> AgentDelegate {
    AgentDelegate::new(SubagentDeps {
        base_url: server.base_url(),
        api_key: None,
        headers: Vec::new(),
        dialect: lr_engine::Dialect::LlamaCpp,
        registry,
        store: h.store.clone(),
        config: h.config.clone(),
        handle: h.handle.clone(),
        sink: h.handle.sink(),
        opts: options(),
        workspace: None,
        n_ctx: Some(8_192),
        groups: ToolGroup::ALL.to_vec(),
        overrides: Vec::new(),
        mode: RunMode::Yolo,
        written: h.written.clone(),
        commands: h.commands.clone(),
        steps: h.steps.clone(),
        tool_calls: Arc::new(AtomicU32::new(0)),
        memory: Vec::new(),
        user_system: options().system_prompt.clone(),
    })
}

fn call(id: &str) -> ToolContext {
    ToolContext::new(None, id)
}

/// Os dois eventos da delegação, na ordem em que saíram.
fn subagent_events(h: &Harness) -> Vec<RunEventKind> {
    h.events
        .lock()
        .unwrap()
        .iter()
        .map(|e| e.event.clone())
        .filter(|e| {
            matches!(
                e,
                RunEventKind::SubagentStarted { .. } | RunEventKind::SubagentFinished { .. }
            )
        })
        .collect()
}

// ------------------------------------------------------------------ testes ---

#[tokio::test]
async fn a_helper_starts_with_a_clean_context() {
    let server = answering_server("O roteamento é decidido em src/router.rs, na função pick().");
    let h = harness();
    let tool = delegate(&h, &server, catalog());

    let out = tool
        .execute(
            json!({ "objective": "MARCA-DA-MISSAO: descubra onde o roteamento é decidido" }),
            &call("c1"),
        )
        .await
        .unwrap();

    assert!(out.content.contains("src/router.rs"), "{}", out.content);

    let body = server.first_step();
    let sent: Value = serde_json::from_str(&body).unwrap();
    let messages = sent["messages"].as_array().unwrap();
    assert_eq!(
        messages.len(),
        2,
        "contexto novo: só o prompt de sistema e a missão"
    );
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[1]["role"], "user");
    assert!(body.contains("MARCA-DA-MISSAO"), "a missão vai junto");
    assert!(
        body.contains("INSTRUCAO-DA-PESSOA"),
        "o que a pessoa pediu na conversa vale para o ajudante: {body}"
    );
    assert!(
        body.contains("até 10 linhas"),
        "o contrato do resumo tem que estar no prompt dele"
    );
}

#[tokio::test]
async fn the_summary_comes_back_as_the_tool_result() {
    let server = answering_server("Li três arquivos: a decisão está em src/router.rs.");
    let h = harness();
    let tool = delegate(&h, &server, catalog());

    let out = tool
        .execute(
            json!({ "objective": "descubra onde o roteamento é decidido" }),
            &call("c1"),
        )
        .await
        .unwrap();

    // O pai recebe o resumo e o preço dele — nada do que o ajudante leu.
    assert!(
        out.content.starts_with("Ajudante (explorador"),
        "{}",
        out.content
    );
    assert!(out.content.contains("1 passo(s)"), "{}", out.content);
    assert!(out.content.contains("a decisão está em src/router.rs"));
    assert_eq!(h.steps.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_explorer_only_gets_tools_that_read() {
    let server = answering_server("não achei nada");
    let h = harness();
    let tool = delegate(&h, &server, catalog());

    tool.execute(json!({ "objective": "leia o README" }), &call("c1"))
        .await
        .unwrap();

    let body = server.first_step();
    assert!(body.contains("fs_read"), "{body}");
    assert!(
        !body.contains("fs_write") && !body.contains("fs_edit"),
        "explorador não recebe ferramenta de escrita: {body}"
    );
    assert!(
        !body.contains("terminal_run"),
        "explorador não roda comando: {body}"
    );
}

#[tokio::test]
async fn a_coder_gets_the_tools_that_change_the_project() {
    let server = answering_server("escrevi o arquivo");
    let h = harness();
    let tool = delegate(&h, &server, catalog());

    tool.execute(
        json!({ "objective": "crie o arquivo notas.md", "role": "coder" }),
        &call("c1"),
    )
    .await
    .unwrap();

    let body = server.first_step();
    assert!(body.contains("fs_write"), "{body}");
    assert!(body.contains("terminal_run"), "{body}");
}

/// Sem recursão: o ajudante não pode delegar de novo, e a garantia não pode
/// depender de o modelo ser sensato.
#[tokio::test]
async fn a_helper_never_sees_the_delegate_tool() {
    let server = answering_server("pronto");
    let h = harness();
    let catalog = registry(&[
        ("fs_read", ToolCategory::Read),
        ("fs_list", ToolCategory::Read),
        (AGENT_DELEGATE, ToolCategory::Meta),
    ]);
    let tool = delegate(&h, &server, catalog);

    tool.execute(json!({ "objective": "leia o README" }), &call("c1"))
        .await
        .unwrap();

    let body = server.first_step();
    assert!(body.contains("fs_read"), "{body}");
    assert!(
        !body.contains(AGENT_DELEGATE),
        "delegar de dentro de um ajudante nunca é oferecido: {body}"
    );
}

/// Cardápio recortado: a porta de saída entra, igual ao pai faz.
#[tokio::test]
async fn a_partial_menu_still_gets_the_escape_hatch() {
    let server = answering_server("pronto");
    let h = harness();
    // Mais ferramentas de leitura do que cabem numa janela de 8k.
    let muitas: Vec<(&'static str, ToolCategory)> = [
        "fs_read",
        "fs_list",
        "fs_grep",
        "fs_glob",
        "git_status",
        "git_diff",
        "git_log",
        "csv_preview",
        "csv_query",
        "data_summary",
        "sql_schema",
        "project_info",
        "workspace_search",
    ]
    .into_iter()
    .map(|name| (name, ToolCategory::Read))
    .collect();
    let tool = delegate(&h, &server, registry(&muitas));

    tool.execute(json!({ "objective": "leia o README" }), &call("c1"))
        .await
        .unwrap();

    assert!(
        server.first_step().contains(menu::TOOLS_FIND),
        "com cardápio parcial o ajudante precisa da porta de saída"
    );
}

/// Teto de passos: um ajudante em looping para sozinho e o pai recebe um
/// texto que dá para agir em cima, não um erro genérico.
#[tokio::test]
async fn a_looping_helper_stops_at_the_step_ceiling_without_breaking_the_parent() {
    let voltas = Arc::new(AtomicU32::new(0));
    let contador = voltas.clone();
    let server = FakeServer::spawn(move |r| match r.path.as_str() {
        "/v1/chat/completions/input_tokens" => {
            FakeResponse::json(json!({ "prompt_tokens": 120 }).to_string())
        }
        // Argumentos diferentes a cada volta: é laço de verdade, não a
        // repetição idêntica que o detector já pega.
        "/v1/chat/completions" => {
            let n = contador.fetch_add(1, Ordering::SeqCst);
            FakeResponse::sse(stream_tool_call(
                &format!("c{n}"),
                "fs_read",
                &json!({ "path": format!("arquivo{n}.rs") }).to_string(),
            ))
        }
        _ => FakeResponse::error(404, "{}"),
    });

    let h = harness();
    let tool = delegate(&h, &server, catalog());
    let out = tool
        .execute(
            json!({ "objective": "leia o projeto inteiro" }),
            &call("c1"),
        )
        .await
        .expect("um ajudante que não termina não derruba o pai");

    assert!(
        out.content
            .contains(&format!("limite de {MAX_SUBAGENT_STEPS} passos")),
        "{}",
        out.content
    );
    assert!(
        out.content.contains("Divida a missão"),
        "o pai precisa saber o que fazer em seguida: {}",
        out.content
    );
    assert_eq!(h.steps.load(Ordering::SeqCst), MAX_SUBAGENT_STEPS);

    match subagent_events(&h).last() {
        Some(RunEventKind::SubagentFinished { status, steps, .. }) => {
            assert_eq!(*status, RunStatus::MaxSteps);
            assert_eq!(*steps, MAX_SUBAGENT_STEPS);
        }
        other => panic!("faltou o fim do ajudante: {other:?}"),
    }
}

#[tokio::test]
async fn a_file_written_by_a_coder_helper_reaches_the_shared_ledger() {
    let voltas = Arc::new(AtomicU32::new(0));
    let contador = voltas.clone();
    let server = FakeServer::spawn(move |r| match r.path.as_str() {
        "/v1/chat/completions/input_tokens" => {
            FakeResponse::json(json!({ "prompt_tokens": 120 }).to_string())
        }
        "/v1/chat/completions" => match contador.fetch_add(1, Ordering::SeqCst) {
            0 => FakeResponse::sse(stream_tool_call(
                "c1",
                "fs_write",
                &json!({ "path": "notas.md" }).to_string(),
            )),
            _ => FakeResponse::sse(stream_text("criei notas.md com o resumo")),
        },
        _ => FakeResponse::error(404, "{}"),
    });

    let h = harness();
    let tool = delegate(&h, &server, catalog());
    let out = tool
        .execute(
            json!({ "objective": "crie notas.md", "role": "coder" }),
            &call("c1"),
        )
        .await
        .unwrap();

    assert_eq!(
        h.written.lock().unwrap().as_slice(),
        ["notas.md".to_string()],
        "o que o ajudante escreveu tem que chegar na verificação final do pai"
    );
    assert_eq!(out.changed_files, vec!["notas.md".to_string()]);
    assert!(out.content.contains("Arquivos alterados: notas.md"));
    assert_eq!(h.steps.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn both_events_come_out_in_order_with_the_same_call_id() {
    let server = answering_server("achei em src/router.rs");
    let h = harness();
    let tool = delegate(&h, &server, catalog());

    tool.execute(
        json!({ "objective": "descubra onde fica o roteamento" }),
        &call("chamada-7"),
    )
    .await
    .unwrap();

    let eventos = subagent_events(&h);
    assert_eq!(eventos.len(), 2, "abre e fecha, uma vez cada");
    match (&eventos[0], &eventos[1]) {
        (
            RunEventKind::SubagentStarted {
                call_id: abriu,
                objective,
                role,
            },
            RunEventKind::SubagentFinished {
                call_id: fechou,
                status,
                steps,
                summary,
            },
        ) => {
            assert_eq!(abriu, "chamada-7");
            assert_eq!(fechou, "chamada-7", "a interface aninha a trilha pelo id");
            assert_eq!(objective, "descubra onde fica o roteamento");
            assert_eq!(*role, SubagentRole::Explorer);
            assert_eq!(*status, RunStatus::Done);
            assert_eq!(*steps, 1);
            assert!(summary.contains("src/router.rs"));
        }
        other => panic!("eventos fora de ordem: {other:?}"),
    }
}

/// Cancelar o run atravessa para dentro do ajudante — e vira erro, para o
/// laço do pai parar junto em vez de seguir com um resumo vazio.
#[tokio::test]
async fn cancelling_the_run_stops_the_helper_and_the_parent() {
    let server = answering_server("nunca chega aqui");
    let h = harness();
    let tool = delegate(&h, &server, catalog());
    h.handle.cancel();

    let err = tool
        .execute(json!({ "objective": "leia o projeto" }), &call("c1"))
        .await
        .unwrap_err();

    assert!(matches!(err, ToolError::Cancelled), "{err:?}");
    assert!(
        server.chat_bodies().is_empty(),
        "o ajudante nem chegou a falar com o modelo"
    );
    match subagent_events(&h).last() {
        Some(RunEventKind::SubagentFinished { status, .. }) => {
            assert_eq!(*status, RunStatus::Cancelled, "a trilha fecha mesmo assim");
        }
        other => panic!("faltou o fim do ajudante: {other:?}"),
    }
}

#[tokio::test]
async fn a_mission_without_text_is_refused_before_anything_starts() {
    let server = answering_server("nunca chega aqui");
    let h = harness();
    let tool = delegate(&h, &server, catalog());

    let err = tool
        .execute(json!({ "objective": "   " }), &call("c1"))
        .await
        .unwrap_err();

    assert!(err.to_model_message().contains("objective"));
    assert!(
        subagent_events(&h).is_empty(),
        "nada começou, nada a anunciar"
    );
}

#[test]
fn an_unknown_role_falls_back_to_the_one_that_changes_nothing() {
    assert_eq!(role_from(&json!({})), SubagentRole::Explorer);
    assert_eq!(
        role_from(&json!({ "role": "explorer" })),
        SubagentRole::Explorer
    );
    assert_eq!(role_from(&json!({ "role": "Coder" })), SubagentRole::Coder);
    assert_eq!(
        role_from(&json!({ "role": "escritor" })),
        SubagentRole::Explorer
    );
}

/// O schema é plano e com enum: o conversor de gramática do llama.cpp não
/// aceita aninhamento, e modelo pequeno erra em cada nível.
#[test]
fn the_schema_stays_flat_for_a_small_model() {
    let tool = AgentDelegate::new(SubagentDeps {
        base_url: "http://127.0.0.1:1".into(),
        api_key: None,
        headers: Vec::new(),
        dialect: lr_engine::Dialect::LlamaCpp,
        registry: Arc::new(ToolRegistry::new()),
        store: Arc::new(Store::open_in_memory().unwrap()),
        config: Arc::new(AgentConfig::new(std::env::temp_dir())),
        handle: RunHandle::new("r1".into(), EventSink::new("r1", None)),
        sink: Arc::new(EventSink::new("r1", None)),
        opts: options(),
        workspace: None,
        n_ctx: Some(8_192),
        groups: ToolGroup::ALL.to_vec(),
        overrides: Vec::new(),
        mode: RunMode::Yolo,
        written: Arc::new(Mutex::new(Vec::new())),
        commands: Arc::new(Mutex::new(Vec::new())),
        steps: Arc::new(AtomicU32::new(0)),
        tool_calls: Arc::new(AtomicU32::new(0)),
        memory: Vec::new(),
        user_system: options().system_prompt.clone(),
    });

    let spec = tool.spec();
    assert_eq!(spec.name, AGENT_DELEGATE);
    assert_eq!(spec.category, ToolCategory::Meta);
    assert_eq!(spec.tier, ToolTier::Safe);
    let props = &spec.parameters["properties"];
    assert_eq!(props["objective"]["type"], "string");
    assert_eq!(props["role"]["enum"], json!(["explorer", "coder"]));
    assert_eq!(spec.parameters["required"], json!(["objective"]));
    for (_, value) in props.as_object().unwrap() {
        assert_ne!(value["type"], "object", "nada aninhado no schema");
        assert_ne!(value["type"], "array", "nada aninhado no schema");
    }
}
