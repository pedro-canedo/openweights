//! Teste de ponta a ponta do agente: do pedido da pessoa ao arquivo no disco.
//!
//! Os testes de unidade cobrem cada peça (política, ferramentas, guard-rails,
//! plano). Falta provar que elas funcionam JUNTAS pelo caminho real —
//! `AgentHost::start` com um llama-server falso, ferramentas de verdade e
//! banco de verdade. É este teste que pega o erro de integração: evento que
//! não sai, ordem trocada, arquivo que não aparece, run que nunca termina.
//!
//! O servidor falso responde exatamente como o llama-server responderia:
//! streaming SSE, `tool_calls` fragmentados entre chunks e `finish_reason`.

use lr_agent::{AgentConfig, AgentHost, Endpoint, StartRun};
use lr_store::Store;
use lr_tools::{Tool, ToolContext, ToolOutput, ToolResult};
use lr_types::agent::{
    ApprovalDecision, RunEvent, RunEventKind, RunMode, RunOptions, RunStatus, ToolCategory,
};
use lr_types::scout::WorkMode;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// -------------------------------------------------------- servidor falso ---

/// Sobe um servidor HTTP local que devolve as respostas na ordem dada.
/// Cada chamada ao endpoint de chat consome a próxima resposta.
struct FakeLlama {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
    /// Corpo de cada pedido de chat, na ordem — é onde se vê se as
    /// ferramentas foram mesmo oferecidas ao modelo.
    bodies: Arc<Mutex<Vec<String>>>,
}

/// `/props` de um llama-server comum: fala do modelo e suporta ferramentas.
const PROPS_MODELO: &str = r#"{"model_path":"/m/a.gguf",
    "chat_template_caps":{"supports_tools":true},
    "default_generation_settings":{"n_ctx":8192}}"#;

impl FakeLlama {
    fn spawn(chat_replies: Vec<Vec<String>>) -> Self {
        Self::spawn_with_props(chat_replies, PROPS_MODELO)
    }

    fn spawn_with_props(chat_replies: Vec<Vec<String>>, props: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        listener.set_nonblocking(true).expect("nonblocking");

        let stop = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let (flag, counter) = (stop.clone(), calls.clone());
        let replies = Arc::new(Mutex::new(chat_replies));
        let bodies: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let vistos = bodies.clone();
        let props_json = props.to_string();

        std::thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let Some((path, body)) = read_request(&mut stream) else {
                            continue;
                        };
                        if path.contains("/chat/completions") && !path.contains("input_tokens") {
                            vistos.lock().unwrap().push(body);
                            // Roteiro pode pedir uma recusa do servidor: o
                            // primeiro pedaço vira o corpo do erro.
                            let recusa = replies
                                .lock()
                                .unwrap()
                                .get(counter.load(Ordering::SeqCst))
                                .and_then(|r| r.first().cloned())
                                .filter(|c| c.starts_with(HTTP_500));
                            if let Some(corpo) = recusa {
                                counter.fetch_add(1, Ordering::SeqCst);
                                write_error(&mut stream, corpo.trim_start_matches(HTTP_500));
                                continue;
                            }
                            let idx = counter.fetch_add(1, Ordering::SeqCst);
                            let chunks = replies
                                .lock()
                                .unwrap()
                                .get(idx)
                                .cloned()
                                // Depois do roteiro, encerra sempre com texto.
                                .unwrap_or_else(|| vec![text_chunk("Pronto."), done()]);
                            write_sse(&mut stream, &chunks);
                        } else if path.contains("/props") {
                            write_json(&mut stream, &props_json);
                        } else {
                            write_json(&mut stream, r#"{"count":10}"#);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2))
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            addr,
            stop,
            calls,
            bodies,
        }
    }

    /// Corpo do n-ésimo pedido de chat.
    fn body(&self, n: usize) -> String {
        self.bodies
            .lock()
            .unwrap()
            .get(n)
            .cloned()
            .unwrap_or_default()
    }

    fn endpoint(&self) -> Endpoint {
        Endpoint {
            base_url: format!("http://{}", self.addr),
            api_key: None,
        }
    }
}

impl Drop for FakeLlama {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn read_request(stream: &mut TcpStream) -> Option<(String, String)> {
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
    let path = lines.next()?.split_whitespace().nth(1)?.to_string();
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
    Some((path, String::from_utf8_lossy(&body).to_string()))
}

fn write_sse(stream: &mut TcpStream, chunks: &[String]) {
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    for c in chunks {
        let framed = format!("{:x}\r\n{c}\r\n", c.len());
        if stream.write_all(framed.as_bytes()).is_err() {
            return;
        }
        let _ = stream.flush();
        std::thread::sleep(Duration::from_millis(2));
    }
    let _ = stream.write_all(b"0\r\n\r\n");
    let _ = stream.flush();
}

fn write_json(stream: &mut TcpStream, body: &str) {
    let res = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(res.as_bytes());
    let _ = stream.flush();
}

fn sse(payload: &str) -> String {
    format!("data: {payload}\n\n")
}

fn text_chunk(text: &str) -> String {
    sse(&format!(
        r#"{{"choices":[{{"delta":{{"content":{}}}}}]}}"#,
        serde_json::to_string(text).unwrap()
    ))
}

/// Pedido de ferramenta com os argumentos partidos em dois chunks — é assim
/// que o llama-server entrega de verdade.
fn tool_call_chunks(id: &str, name: &str, args_a: &str, args_b: &str) -> Vec<String> {
    vec![
        sse(&format!(
            r#"{{"choices":[{{"delta":{{"tool_calls":[{{"index":0,"id":"{id}","function":{{"name":"{name}","arguments":{}}}}}]}}}}]}}"#,
            serde_json::to_string(args_a).unwrap()
        )),
        sse(&format!(
            r#"{{"choices":[{{"delta":{{"tool_calls":[{{"index":0,"function":{{"arguments":{}}}}}]}}}}]}}"#,
            serde_json::to_string(args_b).unwrap()
        )),
        sse(r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#),
        done(),
    ]
}

/// Prefixo que transforma a próxima resposta do roteiro num erro HTTP 500.
const HTTP_500: &str = "__HTTP500__";

fn erro_500(corpo: &str) -> Vec<String> {
    vec![format!("{HTTP_500}{corpo}")]
}

fn write_error(stream: &mut TcpStream, body: &str) {
    let head = format!(
        "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
}

fn done() -> String {
    "data: [DONE]\n\n".to_string()
}

// ------------------------------------------------------------- ambiente ---

struct Harness {
    _dir: tempfile::TempDir,
    _data: tempfile::TempDir,
    workspace: std::path::PathBuf,
    host: AgentHost,
    store: Arc<Store>,
    events: Arc<Mutex<Vec<RunEvent>>>,
}

impl Harness {
    fn new() -> Self {
        Self::with_tools(Vec::new())
    }

    fn with_tools(extra: Vec<lr_tools::SharedTool>) -> Self {
        let dir = tempfile::tempdir().expect("workspace");
        let data = tempfile::tempdir().expect("data");
        let workspace = dir.path().to_path_buf();
        let store = Arc::new(Store::open_in_memory().expect("store"));
        let mut reg = lr_tools::builtin_registry();
        for tool in extra {
            reg.register(tool);
        }
        let registry = Arc::new(reg);
        let host = AgentHost::new(
            store.clone(),
            registry,
            AgentConfig::new(data.path().to_path_buf()),
        );
        Self {
            _dir: dir,
            _data: data,
            workspace,
            host,
            store,
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn options(&self, mode: RunMode) -> RunOptions {
        RunOptions {
            chat_id: 0,
            model: "modelo-de-teste".into(),
            mode,
            workspace_dir: Some(self.workspace.to_string_lossy().into_owned()),
            max_steps: 6,
            mcp_servers: Vec::new(),
            temperature: None,
            top_p: None,
            top_k: None,
            max_tokens: None,
            system_prompt: None,
        }
    }

    /// Dispara o run e espera ele terminar (ou estourar o tempo).
    async fn run(&self, prompt: &str, mode: RunMode, endpoint: Endpoint) -> RunStatus {
        let sink = self.events.clone();
        let handle = self
            .host
            .start(
                StartRun {
                    prompt: prompt.into(),
                    history: Vec::new(),
                    memory: Vec::new(),
                    options: self.options(mode),
                    endpoint,
                    work_mode: WorkMode::Agent,
                    plan: None,
                },
                Some(Arc::new(move |ev: RunEvent| {
                    sink.lock().unwrap().push(ev);
                })),
            )
            .expect("run começou");

        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(status) = self.finished_status() {
                return status;
            }
            if Instant::now() > deadline {
                handle.cancel();
                panic!("o run não terminou em 20s");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn finished_status(&self) -> Option<RunStatus> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .find_map(|e| match e.event {
                RunEventKind::RunFinished { status, .. } => Some(status),
                _ => None,
            })
    }

    fn kinds(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.kind_name().to_string())
            .collect()
    }

    fn has(&self, kind: &str) -> bool {
        self.kinds().iter().any(|k| k == kind)
    }
}

// ---------------------------------------------------------------- testes ---

/// O caminho feliz completo: o modelo pede para escrever um arquivo, o modo
/// automático libera, o arquivo aparece no disco e o run termina explicando.
#[tokio::test]
async fn agent_writes_a_file_and_finishes() {
    let h = Harness::new();
    let server = FakeLlama::spawn(vec![
        tool_call_chunks(
            "call_1",
            "fs_write",
            r#"{"path":"notas.md","#,
            r#""content":"linha um\nlinha dois"}"#,
        ),
        vec![text_chunk("Criei o arquivo notas.md."), done()],
    ]);

    let status = h
        .run("crie notas.md", RunMode::Yolo, server.endpoint())
        .await;
    assert_eq!(status, RunStatus::Done, "eventos: {:?}", h.kinds());

    // O arquivo existe com o conteúdo pedido — inclusive com os argumentos
    // tendo chegado partidos em dois chunks.
    let written = std::fs::read_to_string(h.workspace.join("notas.md")).expect("arquivo criado");
    assert_eq!(written, "linha um\nlinha dois");

    // A trilha conta a história inteira, na ordem.
    let kinds = h.kinds();
    let pos = |k: &str| kinds.iter().position(|x| x == k);
    assert!(pos("run.started").is_some());
    assert!(pos("tool.requested") < pos("tool.result"), "{kinds:?}");
    assert!(pos("tool.result") < pos("run.finished"), "{kinds:?}");
    assert!(h.has("tool.approved"));
    assert!(h.has("verification"), "toda escrita é conferida no fim");

    // Antes de mexer no projeto, uma foto para poder voltar atrás.
    assert!(h.has("checkpoint.created"), "faltou o checkpoint");
    let checkpoints = h
        .store
        .list_checkpoints(&h.workspace.to_string_lossy())
        .expect("checkpoints");
    assert_eq!(checkpoints.len(), 1);

    // E a execução fica registrada para consulta depois.
    let calls = h
        .store
        .list_tool_calls(&h.events.lock().unwrap()[0].run_id.clone());
    assert_eq!(calls.expect("chamadas").len(), 1);
}

/// Em modo "pedir sempre", nada acontece sem confirmação: o run fica parado
/// esperando, e a interface tem o evento para desenhar a pergunta.
#[tokio::test]
async fn write_waits_for_confirmation() {
    let h = Harness::new();
    let server = FakeLlama::spawn(vec![tool_call_chunks(
        "call_1",
        "fs_write",
        r#"{"path":"a.txt",""#,
        r#"content":"oi"}"#,
    )]);

    let sink = h.events.clone();
    let handle = h
        .host
        .start(
            StartRun {
                prompt: "escreva a.txt".into(),
                history: Vec::new(),
                memory: Vec::new(),
                options: h.options(RunMode::Approve),
                endpoint: server.endpoint(),
                work_mode: WorkMode::Agent,
                plan: None,
            },
            Some(Arc::new(move |ev: RunEvent| {
                sink.lock().unwrap().push(ev);
            })),
        )
        .expect("run começou");

    // Espera o pedido de confirmação aparecer.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !h.has("run.paused") {
        assert!(
            Instant::now() < deadline,
            "não pediu confirmação: {:?}",
            h.kinds()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Nada foi escrito enquanto a pessoa não respondeu.
    assert!(
        !h.workspace.join("a.txt").exists(),
        "escreveu sem confirmação"
    );
    let requested = h
        .events
        .lock()
        .unwrap()
        .iter()
        .find_map(|e| match &e.event {
            RunEventKind::ToolRequested {
                call_id,
                requires_approval,
                preview,
                ..
            } => Some((call_id.clone(), *requires_approval, preview.is_some())),
            _ => None,
        })
        .expect("evento de pedido");
    assert!(requested.1, "deveria exigir confirmação");
    assert!(requested.2, "a pergunta precisa mostrar o que vai mudar");

    handle.cancel();
}

/// O contador de execuções vivas é o que diz ao app se dá para derrubar o
/// motor. Se ele mentisse, reiniciar o servidor mataria uma execução no meio.
#[tokio::test]
async fn a_finished_run_stops_counting_as_live() {
    let h = Harness::new();
    assert_eq!(h.host.live_count(), 0);

    let server = FakeLlama::spawn(vec![vec![text_chunk("Pronto."), done()]]);
    let status = h.run("diga pronto", RunMode::Yolo, server.endpoint()).await;
    assert_eq!(status, RunStatus::Done);

    // A remoção acontece na task do run, logo depois do laço terminar.
    let limite = Instant::now() + Duration::from_secs(5);
    while h.host.live_count() > 0 {
        assert!(
            Instant::now() < limite,
            "a execução terminou e continua contando como viva"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Negar é uma resposta válida: o agente recebe o motivo e conclui sem
/// tocar no projeto.
#[tokio::test]
async fn denied_tool_does_not_touch_the_project() {
    let h = Harness::new();
    let server = FakeLlama::spawn(vec![
        tool_call_chunks(
            "call_1",
            "fs_write",
            r#"{"path":"x.txt","#,
            r#""content":"nao"}"#,
        ),
        vec![text_chunk("Tudo bem, não vou criar o arquivo."), done()],
    ]);

    let sink = h.events.clone();
    let handle = h
        .host
        .start(
            StartRun {
                prompt: "crie x.txt".into(),
                history: Vec::new(),
                memory: Vec::new(),
                options: h.options(RunMode::Approve),
                endpoint: server.endpoint(),
                work_mode: WorkMode::Agent,
                plan: None,
            },
            Some(Arc::new(move |ev: RunEvent| {
                sink.lock().unwrap().push(ev);
            })),
        )
        .expect("run começou");

    let deadline = Instant::now() + Duration::from_secs(10);
    let call_id = loop {
        let found = h
            .events
            .lock()
            .unwrap()
            .iter()
            .find_map(|e| match &e.event {
                RunEventKind::ToolRequested { call_id, .. } => Some(call_id.clone()),
                _ => None,
            });
        if let Some(id) = found {
            break id;
        }
        assert!(Instant::now() < deadline, "não pediu confirmação");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };

    assert!(handle.resolve(
        &call_id,
        lr_types::agent::ApprovalDecision::Deny {
            reason: Some("não quero esse arquivo".into()),
        },
    ));

    let deadline = Instant::now() + Duration::from_secs(15);
    while h.finished_status().is_none() {
        assert!(
            Instant::now() < deadline,
            "o run não terminou: {:?}",
            h.kinds()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert!(!h.workspace.join("x.txt").exists(), "criou mesmo negado");
    assert!(h.has("tool.denied"));
}

/// Ferramenta que não existe não derruba o run: o modelo recebe o erro e
/// segue (é o caso mais comum com modelo pequeno).
#[tokio::test]
async fn unknown_tool_becomes_feedback_not_a_crash() {
    let h = Harness::new();
    let server = FakeLlama::spawn(vec![
        tool_call_chunks("call_1", "ferramenta_inventada", r#"{"a":"#, r#"1}"#),
        vec![
            text_chunk("Não existe essa ferramenta; respondi direto."),
            done(),
        ],
    ]);

    let status = h.run("faça algo", RunMode::Yolo, server.endpoint()).await;
    assert_eq!(status, RunStatus::Done, "{:?}", h.kinds());

    let failed = h
        .events
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(&e.event, RunEventKind::ToolResult { ok, .. } if !ok));
    assert!(failed, "o resultado da chamada deveria vir como falha");
}

/// Repetir a MESMA chamada é sintoma de laço: o agente para e devolve para a
/// pessoa antes de gastar o teto inteiro de passos.
#[tokio::test]
async fn identical_calls_stop_the_run_early() {
    let h = Harness::new();
    let mut replies = Vec::new();
    for i in 0..12 {
        replies.push(tool_call_chunks(
            &format!("call_{i}"),
            "fs_list",
            r#"{"pa"#,
            r#"th":"."}"#, // sempre o mesmo argumento
        ));
    }
    let server = FakeLlama::spawn(replies);

    let status = h
        .run("liste sem parar", RunMode::Yolo, server.endpoint())
        .await;
    assert_eq!(
        status,
        RunStatus::Escalated,
        "chamada repetida deveria escalar: {:?}",
        h.kinds()
    );
    // Parou antes do teto de 6 passos configurado no Harness.
    assert!(server.calls.load(Ordering::SeqCst) < 6);
}

/// Com chamadas DIFERENTES a cada passo o detector de repetição não dispara,
/// e quem termina o run é o teto — a garantia de que ele sempre acaba.
#[tokio::test]
async fn step_ceiling_ends_a_runaway_loop() {
    let h = Harness::new();
    std::fs::create_dir_all(h.workspace.join("a/b/c/d/e/f/g/h")).expect("pastas");
    let mut replies = Vec::new();
    for i in 0..12 {
        // Caminho diferente a cada volta: nenhuma chamada é igual à anterior.
        let path = "a/b/c/d/e/f/g/h"
            .split('/')
            .take((i % 8) + 1)
            .collect::<Vec<_>>()
            .join("/");
        replies.push(tool_call_chunks(
            &format!("call_{i}"),
            "fs_list",
            r#"{"pa"#,
            &format!(r#"th":"{path}"}}"#),
        ));
    }
    let server = FakeLlama::spawn(replies);

    let status = h
        .run("explore tudo sem parar", RunMode::Yolo, server.endpoint())
        .await;
    assert_eq!(
        status,
        RunStatus::MaxSteps,
        "deveria parar no teto: {:?}",
        h.kinds()
    );
    // 6 passos configurados no Harness — nunca muito além disso.
    assert!(server.calls.load(Ordering::SeqCst) <= 7);
}

/// Cancelar interrompe de verdade, sem deixar o run pendurado.
#[tokio::test]
async fn cancel_stops_the_run() {
    let h = Harness::new();
    let mut replies = Vec::new();
    for i in 0..10 {
        replies.push(tool_call_chunks(
            &format!("c{i}"),
            "fs_list",
            r#"{"pa"#,
            r#"th":"."}"#,
        ));
    }
    let server = FakeLlama::spawn(replies);

    let sink = h.events.clone();
    let handle = h
        .host
        .start(
            StartRun {
                prompt: "liste".into(),
                history: Vec::new(),
                memory: Vec::new(),
                options: h.options(RunMode::Yolo),
                endpoint: server.endpoint(),
                work_mode: WorkMode::Agent,
                plan: None,
            },
            Some(Arc::new(move |ev: RunEvent| {
                sink.lock().unwrap().push(ev);
            })),
        )
        .expect("run começou");

    // Deixa começar e cancela.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !h.has("step.started") {
        assert!(Instant::now() < deadline, "o run nem começou");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    handle.cancel();

    let deadline = Instant::now() + Duration::from_secs(15);
    while h.finished_status().is_none() {
        assert!(Instant::now() < deadline, "não parou ao cancelar");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(h.finished_status(), Some(RunStatus::Cancelled));
}

/// Os eventos são numerados sem buracos — é isso que permite a interface
/// reconstruir a execução depois de recarregar.
#[tokio::test]
async fn events_are_numbered_for_replay() {
    let h = Harness::new();
    let server = FakeLlama::spawn(vec![
        tool_call_chunks("call_1", "fs_list", r#"{"pa"#, r#"th":"."}"#),
        vec![text_chunk("Listei."), done()],
    ]);
    h.run("liste a pasta", RunMode::Yolo, server.endpoint())
        .await;

    let events = h.events.lock().unwrap();
    let seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
    assert_eq!(
        seqs,
        (1..=seqs.len() as u64).collect::<Vec<_>>(),
        "sem buracos"
    );

    // E o que importa foi para o banco (deltas de token não vão).
    let run_id = events[0].run_id.clone();
    drop(events);
    let saved = h.store.list_run_events(&run_id, 0).expect("trilha salva");
    assert!(!saved.is_empty());
    assert!(
        saved.iter().all(|e| e.kind != "assistant.delta"),
        "delta de token não pode ir para o banco"
    );
}

// ------------------------------------------- checkpoint fora da categoria ---

/// Ferramenta de REDE que grava um arquivo do projeto — o formato do
/// `web_download`. A categoria não é `Edit`, mas ela declara o que vai
/// sobrescrever, e é isso que precisa disparar a foto.
struct FakeDownload;

#[async_trait::async_trait]
impl Tool for FakeDownload {
    fn name(&self) -> &str {
        "fake_download"
    }
    fn description(&self) -> &str {
        "Baixa um arquivo (falso) para o projeto."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Network
    }
    fn files_at_risk(&self, args: &serde_json::Value, _ctx: &ToolContext) -> Vec<String> {
        args.get("path")
            .and_then(|v| v.as_str())
            .map(|p| vec![p.to_string()])
            .unwrap_or_default()
    }
    async fn execute(&self, args: serde_json::Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let rel = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let path = ctx.resolve(rel)?;
        std::fs::write(&path, "conteudo baixado")?;
        Ok(ToolOutput::text("baixado").with_changed(vec![rel.to_string()]))
    }
}

/// O arquivo já existia; depois do "download" dá para voltar ao que era.
#[tokio::test]
async fn a_network_tool_that_overwrites_a_file_gets_a_checkpoint() {
    let h = Harness::with_tools(vec![Arc::new(FakeDownload)]);
    std::fs::write(h.workspace.join("dados.json"), "original").expect("arquivo inicial");

    let server = FakeLlama::spawn(vec![
        tool_call_chunks("call_1", "fake_download", r#"{"path":"#, r#""dados.json"}"#),
        vec![text_chunk("Baixei o arquivo."), done()],
    ]);

    let sink = h.events.clone();
    let handle = h
        .host
        .start(
            StartRun {
                prompt: "baixe dados.json".into(),
                history: Vec::new(),
                memory: Vec::new(),
                options: h.options(RunMode::Yolo),
                endpoint: server.endpoint(),
                work_mode: WorkMode::Agent,
                plan: None,
            },
            Some(Arc::new(move |ev: RunEvent| {
                sink.lock().unwrap().push(ev);
            })),
        )
        .expect("run começou");

    // Rede pede confirmação em qualquer modo, inclusive no automático.
    let deadline = Instant::now() + Duration::from_secs(10);
    let call_id = loop {
        let found = h
            .events
            .lock()
            .unwrap()
            .iter()
            .find_map(|e| match &e.event {
                RunEventKind::ToolRequested {
                    call_id,
                    requires_approval,
                    ..
                } if *requires_approval => Some(call_id.clone()),
                _ => None,
            });
        if let Some(id) = found {
            break id;
        }
        assert!(
            Instant::now() < deadline,
            "não pediu confirmação: {:?}",
            h.kinds()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(handle.resolve(&call_id, ApprovalDecision::AllowOnce));

    let deadline = Instant::now() + Duration::from_secs(20);
    while h.finished_status().is_none() {
        assert!(
            Instant::now() < deadline,
            "o run não terminou: {:?}",
            h.kinds()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert_eq!(
        std::fs::read_to_string(h.workspace.join("dados.json")).unwrap(),
        "conteudo baixado"
    );
    assert!(
        h.has("checkpoint.created"),
        "gravar arquivo do projeto exige foto antes, mesmo sem ser categoria Edit: {:?}",
        h.kinds()
    );
    let checkpoints = h
        .store
        .list_checkpoints(&h.workspace.to_string_lossy())
        .expect("checkpoints");
    assert_eq!(checkpoints.len(), 1);
}

// ------------------------------------------------------- cardápio curado ---

/// Ferramenta inócua, só para encher o catálogo. O nome não casa com nada do
/// objetivo, então a curadoria tem que deixá-la de fora.
struct Filler(&'static str);

#[async_trait::async_trait]
impl Tool for Filler {
    fn name(&self) -> &str {
        self.0
    }
    fn description(&self) -> &str {
        "Ferramenta zebra de enchimento, sem relação com o pedido."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Read
    }
    async fn execute(
        &self,
        _args: serde_json::Value,
        _ctx: &ToolContext,
    ) -> ToolResult<ToolOutput> {
        Ok(ToolOutput::text("nada a fazer"))
    }
}

/// Janela de 8k não comporta o catálogo inteiro: o modelo recebe um recorte
/// e a porta de saída, e o que ele pede por ela passa a existir.
#[tokio::test]
async fn a_small_window_gets_a_partial_menu_the_model_can_extend() {
    let extras: Vec<lr_tools::SharedTool> = [
        "zebra_alfa",
        "zebra_beta",
        "zebra_gama",
        "zebra_delta",
        "zebra_epsilon",
        "zebra_zeta",
    ]
    .into_iter()
    .map(|n| Arc::new(Filler(n)) as lr_tools::SharedTool)
    .collect();
    let h = Harness::with_tools(extras);

    let server = FakeLlama::spawn(vec![
        tool_call_chunks("call_1", "tools_find", r#"{"que"#, r#"ry":"zebra"}"#),
        vec![text_chunk("Consegui as ferramentas zebra."), done()],
    ]);

    let status = h
        .run("liste a pasta do projeto", RunMode::Yolo, server.endpoint())
        .await;
    assert_eq!(status, RunStatus::Done, "eventos: {:?}", h.kinds());

    let events = h.events.lock().unwrap().clone();
    let selections: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.event {
            RunEventKind::ToolsSelected {
                available,
                active,
                limit,
                requested,
            } => Some((*available, active.clone(), *limit, *requested)),
            _ => None,
        })
        .collect();
    assert_eq!(selections.len(), 2, "abertura + pedido do modelo");

    // Abertura: o recorte cabe na janela e nenhuma zebra entrou.
    let (available, active, limit, requested) = &selections[0];
    assert!(!requested);
    assert_eq!(*limit, 8, "8k de janela");
    assert!(
        *available >= 14,
        "o catálogo do teste é maior que o recorte"
    );
    // O recorte respeita o teto. As ferramentas DO RUN (a porta de saída e a
    // delegação) ficam fora da conta: são a máquina, não o catálogo. O que
    // não tem relação com o pedido fica de fora — sobra vaga vira
    // enchimento, e tudo bem: o que não pode é o catálogo inteiro entrar.
    let do_catalogo = active
        .iter()
        .filter(|n| *n != "tools_find" && *n != "agent_delegate")
        .count();
    assert!(do_catalogo <= *limit as usize, "{active:?}");
    let zebras = active.iter().filter(|n| n.starts_with("zebra_")).count();
    assert!(zebras <= 1, "só o que sobrou de vaga: {active:?}");
    assert!(
        active.iter().any(|n| n == "tools_find"),
        "cardápio parcial precisa da porta de saída: {active:?}"
    );
    for essencial in ["fs_read", "fs_list", "fs_grep", "terminal_run"] {
        assert!(active.iter().any(|n| n == essencial), "faltou {essencial}");
    }

    // Pedido do modelo: só o que acabou de entrar, e as zebras entraram.
    let (_, pedidas, _, requested) = &selections[1];
    assert!(requested);
    assert!(
        pedidas.iter().all(|n| n.starts_with("zebra_")),
        "o evento do pedido traz só a novidade: {pedidas:?}"
    );
}

// --------------------------------------------------------------- ajudante ---

/// Delegar poupa a janela do agente principal: o ajudante lê o que precisar
/// com contexto próprio e o que sobe é o resumo.
#[tokio::test]
async fn the_agent_delegates_and_gets_back_a_summary() {
    let h = Harness::new();
    let server = FakeLlama::spawn(vec![
        // 1) O agente principal entrega a missão.
        tool_call_chunks(
            "call_1",
            "agent_delegate",
            r#"{"objective":"descubra onde fica a configuração","#,
            r#""role":"explorer"}"#,
        ),
        // 2) O ajudante responde (mesmo servidor, contexto novo).
        vec![
            text_chunk("A configuração fica em config/app.toml, seção [server]."),
            done(),
        ],
        // 3) O principal conclui com o que recebeu.
        vec![
            text_chunk("A configuração está em config/app.toml."),
            done(),
        ],
    ]);

    let status = h
        .run(
            "onde fica a configuração deste projeto?",
            RunMode::Yolo,
            server.endpoint(),
        )
        .await;
    assert_eq!(status, RunStatus::Done, "eventos: {:?}", h.kinds());

    let kinds = h.kinds();
    let pos = |k: &str| kinds.iter().position(|x| x == k);
    assert!(
        pos("subagent.started") < pos("subagent.finished"),
        "{kinds:?}"
    );

    let events = h.events.lock().unwrap().clone();
    let comeco = events.iter().find_map(|e| match &e.event {
        RunEventKind::SubagentStarted {
            call_id, objective, ..
        } => Some((call_id.clone(), objective.clone())),
        _ => None,
    });
    let fim = events.iter().find_map(|e| match &e.event {
        RunEventKind::SubagentFinished {
            call_id,
            status,
            summary,
            ..
        } => Some((call_id.clone(), *status, summary.clone())),
        _ => None,
    });
    let (id_inicio, objetivo) = comeco.expect("o começo do ajudante");
    let (id_fim, status_fim, resumo) = fim.expect("o fim do ajudante");
    assert_eq!(id_inicio, id_fim, "as duas pontas são do mesmo ajudante");
    assert!(objetivo.contains("configuração"));
    assert_eq!(status_fim, RunStatus::Done);
    assert!(resumo.contains("config/app.toml"), "{resumo}");

    // O resumo é o que o agente principal recebeu como resultado.
    let resultado = events.iter().find_map(|e| match &e.event {
        RunEventKind::ToolResult { result_preview, .. }
            if result_preview.contains("config/app.toml") =>
        {
            Some(result_preview.clone())
        }
        _ => None,
    });
    assert!(resultado.is_some(), "eventos: {kinds:?}");
}

/// **O bug que fazia o modo agente ser um chat com outro nome.**
///
/// No modo Router, `GET /props` sem `?model=` é respondido pelo próprio
/// roteador: `role: "router"`, `model_path: "none"`, sem `chat_template_caps`.
/// O harness lia isso como "o modelo não suporta ferramentas" e rodava o run
/// inteiro sem nenhuma — com qualquer modelo, sem dizer nada. Na tela, o
/// resultado era o modelo respondendo "não tenho ferramentas de busca".
///
/// Agora as capacidades são pedidas pelo nome do modelo; e quando ainda assim
/// vier a resposta do roteador, o app **tenta com ferramentas** em vez de
/// desistir por falta de informação.
#[tokio::test]
async fn a_router_answering_about_itself_does_not_disarm_the_agent() {
    let h = Harness::new();
    let server = FakeLlama::spawn_with_props(
        vec![vec![text_chunk("Pronto."), done()]],
        r#"{"role":"router","max_instances":1,"model_path":"none",
            "default_generation_settings":{"params":null,"n_ctx":0}}"#,
    );

    let status = h
        .run(
            "pesquise o resultado de ontem",
            RunMode::Yolo,
            server.endpoint(),
        )
        .await;
    assert_eq!(status, RunStatus::Done, "eventos: {:?}", h.kinds());

    let pedido = server.body(0);
    assert!(
        pedido.contains("\"tools\""),
        "o pedido ao modelo tem de levar as ferramentas: {pedido}"
    );
    assert!(
        !h.has("tools.off"),
        "não houve recusa: nada a avisar — {:?}",
        h.kinds()
    );
}

/// O contrário: quando dá para ler o template E ele não aceita ferramentas,
/// o run roda sem elas — mas **avisa**. Sem esse aviso, a mesma tela de
/// "não tenho acesso a nada" volta a parecer culpa do modelo.
#[tokio::test]
async fn a_model_that_cannot_take_tools_says_so_in_the_trail() {
    let h = Harness::new();
    let server = FakeLlama::spawn_with_props(
        vec![vec![text_chunk("Não consigo pesquisar."), done()]],
        r#"{"model_path":"/m/sem-tools.gguf",
            "chat_template_caps":{"supports_tools":false},
            "default_generation_settings":{"n_ctx":8192}}"#,
    );

    let status = h
        .run(
            "pesquise o resultado de ontem",
            RunMode::Yolo,
            server.endpoint(),
        )
        .await;
    assert_eq!(status, RunStatus::Done, "eventos: {:?}", h.kinds());

    let pedido = server.body(0);
    assert!(
        !pedido.contains("\"tools\""),
        "template que não aceita não recebe: {pedido}"
    );
    assert!(h.has("tools.off"), "faltou dizer por quê: {:?}", h.kinds());
}

/// Parar não apaga o que o agente já disse.
///
/// A trilha guardava o texto, mas ela não sobrevive inteira a um recarregar:
/// só quatro eventos por run vão para o banco. Então, ao sair da conversa e
/// voltar, a resposta de um run cancelado (ou que morreu no limite de passos)
/// simplesmente não estava mais lá. Agora ela é gravada como mensagem, que é
/// onde a pessoa vai procurar por ela.
#[tokio::test]
async fn a_cancelled_run_keeps_what_it_already_said_in_the_chat() {
    let h = Harness::new();
    let chat = h.store.create_chat("conversa", None).expect("conversa");
    // Fala longa de propósito: o cancelamento precisa chegar COM o texto em
    // curso, que é o caso que perdia o que já tinha sido dito.
    let mut fala = vec![text_chunk("Comecei a responder e")];
    fala.extend((0..200).map(|_| text_chunk(" sigo falando")));
    fala.push(done());
    let server = FakeLlama::spawn(vec![fala]);

    let mut opts = h.options(RunMode::Yolo);
    opts.chat_id = chat;
    let sink = h.events.clone();
    let handle = h
        .host
        .start(
            StartRun {
                prompt: "responda".into(),
                history: Vec::new(),
                memory: Vec::new(),
                options: opts,
                endpoint: server.endpoint(),
                work_mode: WorkMode::Agent,
                plan: None,
            },
            Some(Arc::new(move |ev: RunEvent| {
                sink.lock().unwrap().push(ev);
            })),
        )
        .expect("run começou");

    // Puxa o tapete no meio da fala, não depois dela.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !h.has("assistant.delta") {
        assert!(Instant::now() < deadline, "o modelo não começou a falar");
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    handle.cancel();

    let deadline = Instant::now() + Duration::from_secs(15);
    while !h.has("run.finished") {
        assert!(Instant::now() < deadline, "o run não encerrou");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let status = h
        .events
        .lock()
        .unwrap()
        .iter()
        .find_map(|e| match &e.event {
            RunEventKind::RunFinished { status, .. } => Some(*status),
            _ => None,
        });
    assert_eq!(
        status,
        Some(RunStatus::Cancelled),
        "o teste só vale se o run foi mesmo cancelado"
    );

    let mensagens = h.store.list_messages(chat).expect("mensagens");
    let resposta = mensagens
        .iter()
        .find(|m| m.role == "assistant")
        .expect("o que foi dito antes do cancelamento fica na conversa");
    assert!(
        resposta.content.starts_with("Comecei a responder e"),
        "conteúdo: {}",
        resposta.content
    );
}

/// O agente que anuncia e para: o laço insiste uma vez, e aí o trabalho sai.
///
/// Vem de um run real com um 9B: "Vou criar os três arquivos. Começando com o
/// `app.py`:" encerrava o run como CONCLUÍDO com a pasta vazia. Aqui a
/// primeira resposta é o anúncio, a segunda é a ação — e o arquivo precisa
/// existir no fim.
#[tokio::test]
async fn a_model_that_only_announces_gets_pushed_to_actually_do_it() {
    let h = Harness::new();
    let server = FakeLlama::spawn(vec![
        vec![
            text_chunk("Vou criar os arquivos. Começando com o `notas.md`:"),
            done(),
        ],
        tool_call_chunks(
            "call_1",
            "fs_write",
            r#"{"path":"notas.md","#,
            r#""content":"conteúdo"}"#,
        ),
        vec![text_chunk("Criei o notas.md."), done()],
    ]);

    let status = h
        .run("crie notas.md", RunMode::Yolo, server.endpoint())
        .await;
    assert_eq!(status, RunStatus::Done, "eventos: {:?}", h.kinds());
    assert!(
        h.workspace.join("notas.md").exists(),
        "o anúncio virou run encerrado sem trabalho nenhum"
    );

    // A cutucada é uma mensagem de usuário no MESMO passo — não um passo novo.
    let segundo = server.body(1);
    assert!(
        segundo.contains("não chamou ferramenta"),
        "faltou o empurrão no pedido seguinte: {segundo}"
    );
}

/// O modo automático não pode parar para perguntar por causa de um `|`.
///
/// Encontrado rodando o agente de verdade: `ls -la . | head -20` classificava
/// como comando inanalisável, e comando inanalisável pede confirmação em
/// TODOS os modos. Num run automático não há quem clique — o run ficava
/// pendurado até estourar o tempo, sem nada na tela explicando o que esperava.
#[tokio::test]
async fn a_piped_command_does_not_stall_the_automatic_mode() {
    let h = Harness::new();
    let server = FakeLlama::spawn(vec![
        tool_call_chunks(
            "c1",
            "terminal_run",
            r#"{"command":"ls -la"#,
            r#" . | head -20"}"#,
        ),
        vec![text_chunk("Listei."), done()],
    ]);

    let status = h.run("liste", RunMode::Yolo, server.endpoint()).await;
    assert_eq!(status, RunStatus::Done, "eventos: {:?}", h.kinds());
    assert!(
        !h.has("run.paused"),
        "modo automático não pergunta: {:?}",
        h.kinds()
    );
    assert!(h.has("tool.result"), "o comando nem chegou a rodar");
}

/// O modelo que escreve a chamada em vez de fazê-la.
///
/// Visto com o qwen2.5-coder-14b: em vez de emitir a tool call, ele imprime o
/// JSON `{"name": …, "arguments": …}` no texto. Como texto sem tool call
/// significa "terminei", o run encerrava no primeiro passo sem fazer nada.
/// Agora o laço pede duas vezes que ele use o mecanismo e, se ele insistir,
/// executa o que escreveu — pela mesma política de sempre.
#[tokio::test]
async fn a_tool_call_written_as_text_still_gets_executed() {
    let h = Harness::new();
    let escrita = r#"{"name": "fs_write", "arguments": {"path": "notas.md", "content": "oi"}}"#;
    let server = FakeLlama::spawn(vec![
        vec![text_chunk(escrita), done()],
        vec![text_chunk(escrita), done()],
        vec![text_chunk(escrita), done()],
        vec![text_chunk("Criei o notas.md."), done()],
    ]);

    let status = h
        .run("crie notas.md", RunMode::Yolo, server.endpoint())
        .await;
    assert_eq!(status, RunStatus::Done, "eventos: {:?}", h.kinds());
    assert_eq!(
        std::fs::read_to_string(h.workspace.join("notas.md")).ok(),
        Some("oi".to_string()),
        "a chamada escrita como texto tinha de rodar"
    );

    // Antes de executar, o laço INSISTIU: as duas primeiras respostas viraram
    // pedidos para usar o mecanismo de ferramentas.
    assert!(
        server.body(1).contains("mecanismo de ferramentas"),
        "faltou o empurrão: {}",
        server.body(1)
    );
}

/// A porta dos fundos não é atalho: nome que não está no cardápio não roda.
#[tokio::test]
async fn a_written_call_to_an_unknown_tool_is_ignored() {
    let h = Harness::new();
    let escrita = r#"{"name": "formatar_disco", "arguments": {"alvo": "/"}}"#;
    let server = FakeLlama::spawn(vec![
        vec![text_chunk(escrita), done()],
        vec![text_chunk(escrita), done()],
        vec![text_chunk(escrita), done()],
        vec![text_chunk("Desisto."), done()],
    ]);

    let status = h.run("faça algo", RunMode::Yolo, server.endpoint()).await;
    assert_eq!(status, RunStatus::Done);
    assert!(
        !h.has("tool.requested"),
        "ferramenta fora do cardápio não pode rodar: {:?}",
        h.kinds()
    );
    // Mas o modelo é avisado do nome certo em vez de o run morrer calado.
    assert!(
        server.body(3).contains("Use exatamente um destes nomes"),
        "faltou dizer quais existem"
    );
}

/// O 500 que matava a execução.
///
/// Com um arquivo grande, o modelo erra o escape no meio dos argumentos e o
/// llama.cpp recusa a requisição inteira: `Failed to parse tool call arguments
/// as JSON […] missing closing quote`. Isso derrubava o run com "o modelo não
/// respondeu" — e o trabalho ia junto. É erro do modelo, e tem conserto: o
/// laço devolve o problema com a saída (escrever em pedaços) e segue.
#[tokio::test]
async fn a_tool_call_with_broken_json_is_fixable_not_fatal() {
    let h = Harness::new();
    let server = FakeLlama::spawn(vec![
        erro_500(
            r#"{"error":{"code":500,"message":"Failed to parse tool call arguments as JSON: [json.exception.parse_error.101] parse error at line 1, column 7018: syntax error while parsing value - invalid string: missing closing quote"}}"#,
        ),
        tool_call_chunks(
            "call_1",
            "fs_write",
            r#"{"path":"index.html","#,
            r#""content":"<!DOCTYPE html>"}"#,
        ),
        vec![
            text_chunk("Criei a base; agora completo com fs_edit."),
            done(),
        ],
    ]);

    let status = h
        .run("faça a página", RunMode::Yolo, server.endpoint())
        .await;
    assert_eq!(status, RunStatus::Done, "eventos: {:?}", h.kinds());
    assert!(
        h.workspace.join("index.html").exists(),
        "o run morreu no 500 em vez de tentar de novo"
    );

    // E o modelo recebeu a saída, não só a notícia do erro.
    let segundo = server.body(1);
    assert!(
        segundo.contains("fs_edit") && segundo.contains("pedaços"),
        "faltou dizer COMO consertar: {segundo}"
    );
}
