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
    /// Contagem devolvida em `/input_tokens`/`/tokenize`. Ajustável porque a
    /// compactação era IMPOSSÍVEL de acionar em teste com o "10" fixo.
    tokens: Arc<AtomicUsize>,
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
        let tokens = Arc::new(AtomicUsize::new(10));
        let contagem = tokens.clone();
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
                            // Pedido SEM stream (resumo de compactação, título):
                            // responde JSON vazio sem consumir o roteiro — o
                            // roteiro é dos passos do agente.
                            if body.contains("\"stream\":false") {
                                write_json(
                                    &mut stream,
                                    r#"{"choices":[{"message":{"content":""}}]}"#,
                                );
                                continue;
                            }
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
                            let n = contagem.load(Ordering::SeqCst);
                            write_json(&mut stream, &format!(r#"{{"count":{n}}}"#));
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
            tokens,
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
            headers: Vec::new(),
            dialect: lr_engine::Dialect::LlamaCpp,
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

/// Marcador de roteiro: dorme N ms naquele ponto do stream — simula um
/// servidor que emudeceu no meio da geração (o caso do relógio de ociosidade).
const SLEEP_MARKER: &str = "__SLEEP__";
/// Marcador de roteiro: fecha a conexão SEM o terminador do chunked encoding —
/// simula queda de rede no meio do stream (erro transitório).
const CLOSE_MARKER: &str = "__CLOSE__";

fn write_sse(stream: &mut TcpStream, chunks: &[String]) {
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    for c in chunks {
        if let Some(ms) = c.strip_prefix(SLEEP_MARKER) {
            std::thread::sleep(Duration::from_millis(ms.parse().unwrap_or(50)));
            continue;
        }
        if c == CLOSE_MARKER {
            // Sem o "0\r\n\r\n" final: o cliente vê corpo incompleto e erra.
            let _ = stream.flush();
            return;
        }
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
        Self::with_tools_and_config(extra, |_| {})
    }

    /// Variante que deixa o teste apertar os relógios do harness — é como o
    /// teste de stream emudecido roda em milissegundos e não em minutos.
    fn with_config(ajusta: impl FnOnce(&mut AgentConfig)) -> Self {
        Self::with_tools_and_config(Vec::new(), ajusta)
    }

    fn with_tools_and_config(
        extra: Vec<lr_tools::SharedTool>,
        ajusta: impl FnOnce(&mut AgentConfig),
    ) -> Self {
        let dir = tempfile::tempdir().expect("workspace");
        let data = tempfile::tempdir().expect("data");
        let workspace = dir.path().to_path_buf();
        let store = Arc::new(Store::open_in_memory().expect("store"));
        let mut reg = lr_tools::builtin_registry();
        for tool in extra {
            reg.register(tool);
        }
        let registry = Arc::new(reg);
        let mut config = AgentConfig::new(data.path().to_path_buf());
        ajusta(&mut config);
        let host = AgentHost::new(store.clone(), registry, config);
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
            code_mode: false,
        }
    }

    /// Dispara um run com o Code Mode ligado.
    async fn run_code_mode(&self, prompt: &str, mode: RunMode, endpoint: Endpoint) -> RunStatus {
        let mut options = self.options(mode);
        options.code_mode = true;
        self.run_with_options(prompt, options, endpoint).await
    }

    /// Dispara o run e espera ele terminar (ou estourar o tempo).
    async fn run(&self, prompt: &str, mode: RunMode, endpoint: Endpoint) -> RunStatus {
        self.run_with_steps(prompt, mode, endpoint, 6).await
    }

    /// Variante com teto de passos próprio — os cenários de estagnação e de
    /// teto de erros precisam de mais fôlego que os 6 passos do padrão.
    async fn run_with_steps(
        &self,
        prompt: &str,
        mode: RunMode,
        endpoint: Endpoint,
        max_steps: u32,
    ) -> RunStatus {
        let mut options = self.options(mode);
        options.max_steps = max_steps;
        self.run_with_options(prompt, options, endpoint).await
    }

    async fn run_with_options(
        &self,
        prompt: &str,
        options: RunOptions,
        endpoint: Endpoint,
    ) -> RunStatus {
        let sink = self.events.clone();
        let handle = self
            .host
            .start(
                StartRun {
                    prompt: prompt.into(),
                    history: Vec::new(),
                    memory: Vec::new(),
                    options,
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
            text_chunk("Criei a base; agora completo com fs_append."),
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
        segundo.contains("fs_append") && segundo.contains("pedaços"),
        "faltou dizer COMO consertar: {segundo}"
    );
}

/// Um blip de rede no meio do stream NÃO pode rebaixar o agente a chatbot.
///
/// Era o que acontecia: qualquer erro refazia o passo sem ferramentas e, se a
/// segunda tentativa passasse, `tools_on` era desligado para o resto do run —
/// o agente respondia "não tenho acesso a arquivos" e terminava `Done`.
/// Agora erro transitório é retry do MESMO pedido, com as MESMAS ferramentas.
#[tokio::test]
async fn a_dropped_connection_retries_with_tools_intact() {
    let h = Harness::new();
    let server = FakeLlama::spawn(vec![
        // Cai no meio da fala: corpo incompleto, erro de rede no cliente.
        vec![text_chunk("começando"), CLOSE_MARKER.to_string()],
        tool_call_chunks(
            "call_1",
            "fs_write",
            r#"{"path":"notas.md","#,
            r#""content":"oi"}"#,
        ),
        vec![text_chunk("Criei o notas.md."), done()],
    ]);

    let status = h
        .run("crie notas.md", RunMode::Yolo, server.endpoint())
        .await;
    assert_eq!(status, RunStatus::Done, "eventos: {:?}", h.kinds());
    assert!(h.workspace.join("notas.md").exists());

    // O retry (2º pedido) ainda oferece as ferramentas, e nada foi desligado.
    assert!(
        server.body(1).contains("\"tools\""),
        "o retry perdeu as ferramentas"
    );
    assert!(!h.has("tools.off"), "blip de rede não desliga ferramenta");
}

/// Stream que emudece no meio da geração estoura o relógio de ociosidade e
/// vira retry — antes, segurava o run PARA SEMPRE.
#[tokio::test]
async fn a_stalled_stream_times_out_and_retries() {
    let h = Harness::with_config(|c| {
        c.idle_timeout = Duration::from_millis(150);
    });
    let server = FakeLlama::spawn(vec![
        vec![
            text_chunk("a"),
            format!("{SLEEP_MARKER}600"),
            text_chunk("b"),
            done(),
        ],
        vec![text_chunk("Pronto."), done()],
    ]);

    let status = h.run("responda", RunMode::Yolo, server.endpoint()).await;
    assert_eq!(status, RunStatus::Done, "eventos: {:?}", h.kinds());
    assert!(
        server.calls.load(Ordering::SeqCst) >= 2,
        "o prazo não disparou o retry"
    );
    assert!(!h.has("tools.off"));
}

/// A recusa do template por causa de `tools` continua caindo para o caminho
/// sem ferramentas — mas UMA recusa não desliga nada: o passo seguinte volta
/// a oferecê-las, e só a SEGUNDA recusa desliga de vez, avisando.
#[tokio::test]
async fn a_template_rejection_only_disarms_after_the_second_strike() {
    let h = Harness::new();
    let recusa = || {
        erro_500(
            r#"{"error":{"code":500,"message":"the jinja template of this model does not support tools"}}"#,
        )
    };
    let server = FakeLlama::spawn(vec![
        recusa(),
        // Sem ferramentas, o modelo só anuncia — a cutucada refaz o passo.
        vec![text_chunk("Vou criar os arquivos agora:"), done()],
        recusa(),
        vec![text_chunk("Não consegui usar ferramentas."), done()],
    ]);

    let status = h
        .run("crie notas.md", RunMode::Yolo, server.endpoint())
        .await;
    assert_eq!(status, RunStatus::Done, "eventos: {:?}", h.kinds());

    // 1º pedido: com tools. 2º: a retomada sem tools. 3º: o passo seguinte
    // OFERECE DE NOVO (é a prova do rearme por passo). 4º: sem tools de novo.
    assert!(server.body(0).contains("\"tools\""));
    assert!(!server.body(1).contains("\"tools\""));
    assert!(
        server.body(2).contains("\"tools\""),
        "uma recusa só não pode desligar o passo seguinte: {}",
        server.body(2)
    );
    assert!(!server.body(3).contains("\"tools\""));
    // Na segunda recusa o desligamento vira definitivo e é anunciado.
    assert!(h.has("tools.off"), "a segunda recusa tinha que avisar");
}

/// O código de saída declarado pela ferramenta vence o marcador textual: um
/// stdout que por acaso contenha "exit code 1" não pode derrubar a
/// verificação de um comando que saiu com 0.
#[tokio::test]
async fn a_log_that_mentions_an_exit_code_does_not_fail_verification() {
    let h = Harness::new();
    // Precisa de um comando que SAIA COM 0 e cuspa o texto "exit code 1".
    // `echo` não serve: no Windows é embutido do cmd, e prefixar com `cmd /c`
    // torna o comando opaco — opaco sempre pede confirmação, até no modo
    // automático, e o run ficaria pendurado. A saída é gravar a frase num
    // arquivo e mandar um programa de verdade lê-la de volta.
    let leitura = if cfg!(windows) {
        r#" exit log.txt"}"#
    } else {
        r#" log.txt"}"#
    };
    let programa = if cfg!(windows) {
        r#"{"command":"findstr"#
    } else {
        r#"{"command":"cat"#
    };
    let server = FakeLlama::spawn(vec![
        tool_call_chunks(
            "c1",
            "fs_write",
            r#"{"path":"log.txt","#,
            r#""content":"exit code 1"}"#,
        ),
        tool_call_chunks("c2", "terminal_run", programa, leitura),
        vec![text_chunk("Pronto."), done()],
    ]);

    let status = h
        .run("rode e grave", RunMode::Yolo, server.endpoint())
        .await;
    assert_eq!(status, RunStatus::Done, "eventos: {:?}", h.kinds());

    let verificacao = h
        .events
        .lock()
        .unwrap()
        .iter()
        .find_map(|e| match &e.event {
            RunEventKind::Verification { passed, notes } => Some((*passed, notes.clone())),
            _ => None,
        });
    let (passed, notes) = verificacao.expect("houve escrita: a verificação roda");
    assert!(
        passed,
        "o eco de 'exit code 1' derrubou a verificação: {notes}"
    );
}

/// Ler → editar → reler para conferir. A releitura DEPOIS da edição devolve o
/// conteúdo novo — antes, devolvia "você já leu no passo N", que vira mentira
/// no instante em que a edição acontece.
#[tokio::test]
async fn rereading_a_file_after_editing_it_returns_fresh_content() {
    let h = Harness::new();
    std::fs::write(h.workspace.join("app.py"), "v1\n").expect("semente");
    let server = FakeLlama::spawn(vec![
        tool_call_chunks("c1", "fs_read", r#"{"path":"#, r#""app.py"}"#),
        tool_call_chunks(
            "c2",
            "fs_write",
            r#"{"path":"app.py","#,
            r#""content":"v2\n"}"#,
        ),
        tool_call_chunks("c3", "fs_read", r#"{"path":"#, r#""app.py"}"#),
        vec![text_chunk("Conferido."), done()],
    ]);

    let status = h
        .run("edite e confira", RunMode::Yolo, server.endpoint())
        .await;
    assert_eq!(status, RunStatus::Done, "eventos: {:?}", h.kinds());

    // O último pedido ao modelo carrega o resultado da releitura: tem que ser
    // o conteúdo NOVO, não o ponteiro "você já leu".
    let ultimo = server.body(3);
    assert!(
        ultimo.contains("v2"),
        "a releitura depois da edição não devolveu o conteúdo novo: {ultimo}"
    );
    assert!(
        !ultimo.contains("já leu"),
        "a releitura de conferência foi bloqueada: {ultimo}"
    );
}

/// Erro–sucesso–erro–sucesso nunca escalava: qualquer sucesso zerava a
/// contagem de "seguidos". O teto TOTAL de erros do run pega o padrão.
#[tokio::test]
async fn interleaved_errors_hit_the_total_cap() {
    let h = Harness::new();
    std::fs::write(h.workspace.join("x.md"), "conteúdo\n").expect("semente");
    // 8 ferramentas inexistentes (nomes distintos, para não cair no detector
    // de repetição) intercaladas com leituras que dão certo.
    let mut roteiro = Vec::new();
    for i in 0..8 {
        roteiro.push(tool_call_chunks(
            &format!("e{i}"),
            &format!("ferramenta_fantasma_{i}"),
            r#"{"x":"#,
            r#"1}"#,
        ));
        // Leitura válida e sempre DIFERENTE (max_lines varia): sucesso que
        // zera a streak sem cair no detector de repetição.
        roteiro.push(tool_call_chunks(
            &format!("r{i}"),
            "fs_read",
            r#"{"path":"x.md","max_lines":"#,
            &format!("{}}}", i + 1),
        ));
    }
    let server = FakeLlama::spawn(roteiro);

    let status = h
        .run_with_steps("faça algo", RunMode::Yolo, server.endpoint(), 30)
        .await;
    assert_eq!(status, RunStatus::Escalated, "eventos: {:?}", h.kinds());
    let resumo = h
        .events
        .lock()
        .unwrap()
        .iter()
        .find_map(|e| match &e.event {
            RunEventKind::RunFinished { summary, .. } => Some(summary.clone()),
            _ => None,
        });
    assert!(
        resumo.unwrap_or_default().contains("8 erros"),
        "o motivo tinha que ser o teto total"
    );
}

/// Passos que rodam ferramenta e NÃO mudam nada (só releitura repetida)
/// ganham a cutucada no terceiro e param no quinto — antes disso o run
/// seguia queimando passos até o teto, calado.
#[tokio::test]
async fn steps_without_progress_get_nudged_then_stopped() {
    let h = Harness::new();
    for f in ["a.md", "b.md", "c.md", "d.md", "e.md"] {
        std::fs::write(h.workspace.join(f), "conteúdo\n").expect("semente");
    }
    let mut roteiro = Vec::new();
    // Cinco leituras frescas (progresso legítimo de exploração)...
    for (i, f) in ["a.md", "b.md", "c.md", "d.md", "e.md"].iter().enumerate() {
        roteiro.push(tool_call_chunks(
            &format!("p{i}"),
            "fs_read",
            r#"{"path":"#,
            &format!("\"{f}\"}}"),
        ));
    }
    // ...e cinco RELEITURAS, que voltam do ledger sem mudar nada.
    for (i, f) in ["a.md", "b.md", "c.md", "d.md", "e.md"].iter().enumerate() {
        roteiro.push(tool_call_chunks(
            &format!("d{i}"),
            "fs_read",
            r#"{"path":"#,
            &format!("\"{f}\"}}"),
        ));
    }
    let server = FakeLlama::spawn(roteiro);

    let status = h
        .run_with_steps("explore", RunMode::Yolo, server.endpoint(), 20)
        .await;
    assert_eq!(status, RunStatus::Escalated, "eventos: {:?}", h.kinds());

    // A cutucada chegou ao modelo antes de o run parar.
    let cutucou = (0..server.calls.load(Ordering::SeqCst))
        .any(|i| server.body(i).contains("não mudaram NADA"));
    assert!(cutucou, "faltou a cutucada de estagnação antes de parar");
}

/// Confirmação que expira NEGA a chamada (o modelo recebe o motivo e pode
/// contornar); só a segunda expiração encerra o run — antes, a primeira já
/// cancelava tudo, inclusive nas automações onde não há ninguém para clicar.
#[tokio::test]
async fn an_expired_approval_denies_once_then_stops() {
    let h = Harness::with_config(|c| {
        c.approval_timeout = Duration::from_millis(200);
    });
    let server = FakeLlama::spawn(vec![
        tool_call_chunks("w1", "fs_write", r#"{"path":"um.md","#, r#""content":"a"}"#),
        tool_call_chunks(
            "w2",
            "fs_write",
            r#"{"path":"dois.md","#,
            r#""content":"b"}"#,
        ),
        vec![text_chunk("Não deveria chegar aqui."), done()],
    ]);

    // Modo Smart: escrita pede confirmação — e ninguém responde.
    let status = h.run("escreva", RunMode::Smart, server.endpoint()).await;
    assert_eq!(status, RunStatus::Escalated, "eventos: {:?}", h.kinds());

    // A primeira expiração virou negação com motivo, entregue ao modelo.
    assert!(h.has("tool.denied"), "eventos: {:?}", h.kinds());
    let negacao = (0..server.calls.load(Ordering::SeqCst))
        .any(|i| server.body(i).contains("ninguém respondeu"));
    assert!(negacao, "o modelo tinha que receber o motivo da negação");
    // E nada foi escrito.
    assert!(!h.workspace.join("um.md").exists());
    assert!(!h.workspace.join("dois.md").exists());
}

/// Verificação reprovada no modo agente ganha UMA rodada de conserto — e o
/// mesmo comando rodado de novo com sucesso supersede a falha histórica.
///
/// Antes, a reprovação só virava um evento que ninguém lia: o run terminava
/// "concluído" com um comando falhado no meio e nada era feito a respeito.
#[tokio::test]
async fn a_failed_verification_gets_one_repair_round() {
    let h = Harness::new();
    // O teste precisa de um comando que FALHE sem o arquivo e PASSE com ele.
    // No Windows não existe `ls` no PATH (e `cmd /c dir` seria opaco, que
    // pede confirmação até no automático): o `findstr` é programa de verdade
    // e sai com 2 quando o arquivo não abre, 0 quando casa. As duas chamadas
    // usam a MESMA string de propósito — é por ela que a verificação casa a
    // re-execução com a primeira.
    let (lista_a, lista_b) = if cfg!(windows) {
        (r#"{"command":"findstr"#, r#" . faltando.txt"}"#)
    } else {
        (r#"{"command":"ls"#, r#" faltando.txt"}"#)
    };
    let server = FakeLlama::spawn(vec![
        // O comando falha (arquivo não existe) e o modelo declara vitória.
        tool_call_chunks("c1", "terminal_run", lista_a, lista_b),
        vec![text_chunk("Pronto, tudo certo."), done()],
        // A rodada de conserto: cria o arquivo e roda de novo o MESMO comando.
        tool_call_chunks(
            "c2",
            "fs_write",
            r#"{"path":"faltando.txt","#,
            r#""content":"agora existe"}"#,
        ),
        tool_call_chunks("c3", "terminal_run", lista_a, lista_b),
        vec![
            text_chunk("Consertei: o arquivo existe e o comando passa."),
            done(),
        ],
    ]);

    let status = h
        .run_with_steps("liste o arquivo", RunMode::Yolo, server.endpoint(), 12)
        .await;
    assert_eq!(status, RunStatus::Done, "eventos: {:?}", h.kinds());

    // O relatório reprovado chegou ao modelo como instrução de conserto…
    let recebeu = (0..server.calls.load(Ordering::SeqCst))
        .any(|i| server.body(i).contains("verificação automática reprovou"));
    assert!(recebeu, "o modelo tinha que receber o relatório");

    // …e a SEGUNDA verificação passou, porque o mesmo comando rodou verde.
    let verificacoes: Vec<bool> = h
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match &e.event {
            RunEventKind::Verification { passed, .. } => Some(*passed),
            _ => None,
        })
        .collect();
    assert_eq!(verificacoes.len(), 2, "reprovada + re-verificada");
    assert!(!verificacoes[0] && verificacoes[1], "{verificacoes:?}");
    assert!(h.workspace.join("faltando.txt").exists());
}

/// Dois runs na MESMA pasta de projeto se atropelam (escrevem no mesmo
/// arquivo, cruzam checkpoints): o segundo é recusado na entrada. O cenário
/// real é o relógio disparando uma automação enquanto a pessoa usa o agente.
#[tokio::test]
async fn a_second_run_in_the_same_workspace_is_refused() {
    let h = Harness::new();
    // Primeiro run fica vivo esperando o stream que nunca anda.
    let server = FakeLlama::spawn(vec![
        vec![format!("{SLEEP_MARKER}2000"), text_chunk("fim"), done()],
        vec![text_chunk("fim"), done()],
    ]);
    let sink = h.events.clone();
    let _vivo = h
        .host
        .start(
            StartRun {
                prompt: "primeiro".into(),
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
        .expect("primeiro começou");

    let recusa = h.host.start(
        StartRun {
            prompt: "segundo".into(),
            history: Vec::new(),
            memory: Vec::new(),
            options: h.options(RunMode::Yolo),
            endpoint: server.endpoint(),
            work_mode: WorkMode::Agent,
            plan: None,
        },
        None,
    );
    let erro = recusa.err().expect("o segundo tinha que ser recusado");
    assert!(
        erro.contains("mesma") || erro.contains("nesta pasta"),
        "{erro}"
    );
}

/// A compactação agora é acionável em teste — e quando o resumo falha, o
/// plano B determinístico corta o miolo com um marcador em vez de deixar o
/// passo seguinte estourar a janela em silêncio.
#[tokio::test]
async fn compaction_fires_and_survives_a_failed_summary() {
    let h = Harness::new();
    for f in ["a.md", "b.md", "c.md", "d.md", "e.md"] {
        std::fs::write(h.workspace.join(f), "x\n".repeat(40)).expect("semente");
    }
    // Janela minúscula anunciada pelo servidor: o orçamento fica apertado
    // desde o começo. A contagem "exata" também vem alta.
    let server = FakeLlama::spawn_with_props(
        vec![
            tool_call_chunks("c1", "fs_read", r#"{"path":"#, r#""a.md"}"#),
            tool_call_chunks("c2", "fs_read", r#"{"path":"#, r#""b.md"}"#),
            tool_call_chunks("c3", "fs_read", r#"{"path":"#, r#""c.md"}"#),
            tool_call_chunks("c4", "fs_read", r#"{"path":"#, r#""d.md"}"#),
            tool_call_chunks("c5", "fs_read", r#"{"path":"#, r#""e.md"}"#),
            vec![text_chunk("Terminei a leitura."), done()],
        ],
        r#"{"model_path":"/m/a.gguf",
            "chat_template_caps":{"supports_tools":true},
            "default_generation_settings":{"n_ctx":512}}"#,
    );
    server.tokens.store(9_999, Ordering::SeqCst);

    let status = h
        .run_with_steps(
            "leia os cinco arquivos",
            RunMode::Yolo,
            server.endpoint(),
            10,
        )
        .await;
    assert_eq!(status, RunStatus::Done, "eventos: {:?}", h.kinds());
    assert!(
        h.has("context.compacted"),
        "a janela minúscula tinha que compactar: {:?}",
        h.kinds()
    );
    // O resumo veio vazio (o fake responde conteúdo vazio sem stream), então
    // o marcador do plano B tem que estar no histórico enviado depois.
    let marcado = (0..server.calls.load(Ordering::SeqCst))
        .any(|i| server.body(i).contains("mensagens antigas foram removidas"));
    assert!(marcado, "faltou o marcador do plano B no histórico");
}

/// O circuito completo do arquivo grande: o JSON quebra DENTRO do servidor
/// (nenhum fragmento chega ao cliente — não há o que "recuperar"), o aviso
/// endurece, e na segunda recaída o modelo entrega o arquivo em TEXTO puro —
/// que o harness grava com o escape certo, pela política de sempre.
#[tokio::test]
async fn a_file_that_breaks_json_twice_arrives_as_plain_text() {
    let h = Harness::new();
    let quebra = || {
        erro_500(
            r#"{"error":{"code":500,"message":"Failed to parse tool call arguments as JSON: syntax error while parsing value - invalid string: missing closing quote"}}"#,
        )
    };
    let pagina = "<!DOCTYPE html>\n<html lang=\"pt-BR\">\n<body onload=\"init()\">\n<canvas id=\"jogo\"></canvas>\n</html>";
    let entrega = format!("Vou gravar como texto.\n\nARQUIVO: jogo.html\n```html\n{pagina}\n```");
    let server = FakeLlama::spawn(vec![
        quebra(),
        quebra(),
        vec![text_chunk(&entrega), done()],
        vec![text_chunk("Gravado."), done()],
    ]);

    let status = h
        .run_with_steps("crie o jogo", RunMode::Yolo, server.endpoint(), 10)
        .await;
    assert_eq!(status, RunStatus::Done, "eventos: {:?}", h.kinds());

    // O arquivo existe INTEIRO, com aspas e atributos intactos.
    let gravado = std::fs::read_to_string(h.workspace.join("jogo.html")).expect("arquivo gravado");
    assert!(gravado.contains("lang=\"pt-BR\""), "{gravado}");
    assert!(gravado.contains("onload=\"init()\""), "{gravado}");

    // E o segundo aviso pediu exatamente o formato de texto.
    let pediu_texto = (0..server.calls.load(Ordering::SeqCst))
        .any(|i| server.body(i).contains("ARQUIVO: caminho"));
    assert!(pediu_texto, "faltou a instrução do formato de texto");
}

// ------------------------------------------------------------- code mode ---

/// O Node é necessário para o programa rodar de verdade. Sem ele o teste não
/// falha: ele diz por que não rodou. O CI dos três sistemas tem Node.
fn tem_node() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Pedido de `run_code` com um programa — o caminho normal do Code Mode.
fn programa(id: &str, code: &str) -> Vec<String> {
    let args = serde_json::json!({ "code": code }).to_string();
    tool_call_chunks(id, "run_code", &args, "")
}

/// A promessa do Code Mode, medida: três leituras em UM passo do modelo.
///
/// No modo nativo isto seriam três idas e voltas, com os três conteúdos
/// empilhados na janela. Aqui o modelo gasta um passo, o harness executa as
/// três chamadas (cada uma registrada na trilha, com política) e o que volta
/// é só o que o programa imprimiu.
#[tokio::test]
async fn um_passo_do_modelo_vira_tres_chamadas_de_ferramenta() {
    if !tem_node() {
        eprintln!("pulando: `node` não está instalado");
        return;
    }
    let h = Harness::new();
    for (nome, texto) in [("a.txt", "um"), ("b.txt", "dois"), ("c.txt", "três")] {
        std::fs::write(h.workspace.join(nome), texto).unwrap();
    }

    let server = FakeLlama::spawn(vec![
        programa(
            "call_1",
            "const partes = [];\n\
             for (const nome of [\"a.txt\", \"b.txt\", \"c.txt\"]) {\n\
               partes.push((await fs_read({ path: nome })).trim());\n\
             }\n\
             say(\"juntos:\", partes.join(\"|\"));\n",
        ),
        vec![text_chunk("Li os três arquivos."), done()],
    ]);

    let status = h
        .run_code_mode("leia os três arquivos", RunMode::Yolo, server.endpoint())
        .await;
    assert_eq!(status, RunStatus::Done, "eventos: {:?}", h.kinds());

    let run_id = h.events.lock().unwrap()[0].run_id.clone();
    let chamadas = h.store.list_tool_calls(&run_id).expect("chamadas");
    let nomes: Vec<&str> = chamadas.iter().map(|c| c.tool_name.as_str()).collect();
    assert_eq!(
        nomes.iter().filter(|n| **n == "fs_read").count(),
        3,
        "as três leituras precisam estar na trilha: {nomes:?}"
    );
    assert_eq!(nomes.iter().filter(|n| **n == "run_code").count(), 1);

    // O modelo foi chamado duas vezes: o programa e a resposta final. No modo
    // nativo seriam cinco (três leituras + o pedido inicial + o fecho).
    assert_eq!(
        server.calls.load(Ordering::SeqCst),
        2,
        "um passo para o programa, um para o fecho"
    );

    // O resultado do programa — e só ele — voltou para a conversa.
    let saida = h
        .store
        .list_tool_calls(&run_id)
        .unwrap()
        .into_iter()
        .find(|c| c.tool_name == "run_code")
        .and_then(|c| c.result_json)
        .unwrap_or_default();
    assert!(saida.contains("juntos:"), "{saida}");
    // O programa recebe de cada ferramenta o MESMO texto que o modelo
    // receberia — com cabeçalho e número de linha, no caso do `fs_read`.
    for pedaco in ["um", "dois", "três"] {
        assert!(saida.contains(pedaco), "faltou {pedaco} em {saida}");
    }
    assert!(
        saida.contains("3 chamadas de ferramenta"),
        "o rodapé conta o que o modelo não viu: {saida}"
    );
}

/// Modelo que não emite tool call: o programa vem num bloco de código e ainda
/// assim é executado. É o caso do `qwen2.5-coder:14b` nesta máquina.
#[tokio::test]
async fn programa_entregue_em_texto_puro_e_executado() {
    if !tem_node() {
        eprintln!("pulando: `node` não está instalado");
        return;
    }
    let h = Harness::new();
    std::fs::write(h.workspace.join("a.txt"), "conteúdo").unwrap();

    let server = FakeLlama::spawn(vec![
        vec![
            text_chunk("Claro, vou fazer assim:\n\n```js\n"),
            text_chunk("const t = await fs_read({ path: \"a.txt\" });\nsay(t.trim());\n"),
            text_chunk("```\n"),
            done(),
        ],
        vec![text_chunk("Pronto: o arquivo tem \"conteúdo\"."), done()],
    ]);

    let status = h
        .run_code_mode("leia a.txt", RunMode::Yolo, server.endpoint())
        .await;
    assert_eq!(status, RunStatus::Done, "eventos: {:?}", h.kinds());

    let run_id = h.events.lock().unwrap()[0].run_id.clone();
    let chamadas = h.store.list_tool_calls(&run_id).expect("chamadas");
    let nomes: Vec<&str> = chamadas.iter().map(|c| c.tool_name.as_str()).collect();
    assert!(
        nomes.contains(&"run_code") && nomes.contains(&"fs_read"),
        "o bloco de código virou execução: {nomes:?}"
    );
}

/// O programa não é uma porta dos fundos: escrever fora do projeto continua
/// pedindo confirmação — em modo automático, inclusive — e a recusa volta
/// para dentro do programa como exceção, sem derrubá-lo.
///
/// Este é o teste que separa o Code Mode de um `eval` com outro nome. A
/// pessoa autorizou UM programa; o que ele faz continua passando pela mesma
/// política, uma chamada de cada vez.
#[tokio::test]
async fn escrita_fora_do_projeto_dentro_do_programa_ainda_pede_confirmacao() {
    if !tem_node() {
        eprintln!("pulando: `node` não está instalado");
        return;
    }
    let h = Harness::new();
    let server = FakeLlama::spawn(vec![
        programa(
            "call_1",
            "try {\n\
               await fs_write({ path: \"../fora.txt\", content: \"x\" });\n\
               say(\"escreveu\");\n\
             } catch (e) {\n\
               say(\"recusado:\", e.message);\n\
             }\n",
        ),
        vec![text_chunk("Não deu para escrever fora do projeto."), done()],
    ]);

    let sink = h.events.clone();
    let mut options = h.options(RunMode::Yolo);
    options.code_mode = true;
    let handle = h
        .host
        .start(
            StartRun {
                prompt: "escreva fora".into(),
                history: Vec::new(),
                memory: Vec::new(),
                options,
                endpoint: server.endpoint(),
                work_mode: WorkMode::Agent,
                plan: None,
            },
            Some(Arc::new(move |ev: RunEvent| {
                sink.lock().unwrap().push(ev);
            })),
        )
        .expect("run começou");

    // A segunda confirmação pedida é a do `fs_write` (a primeira é a do
    // próprio programa, que o modo automático libera sozinho).
    let deadline = Instant::now() + Duration::from_secs(20);
    let call_id = loop {
        let pedido = h
            .events
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(|e| match &e.event {
                RunEventKind::ToolRequested {
                    call_id,
                    tool,
                    requires_approval: true,
                    ..
                } if tool == "fs_write" => Some(call_id.clone()),
                _ => None,
            });
        if let Some(id) = pedido {
            break id;
        }
        assert!(
            Instant::now() < deadline,
            "escrever fora do projeto tinha que pedir confirmação: {:?}",
            h.kinds()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };

    assert!(handle.resolve(
        &call_id,
        lr_types::agent::ApprovalDecision::Deny {
            reason: Some("fora do projeto, não".into()),
        },
    ));

    let deadline = Instant::now() + Duration::from_secs(20);
    while h.finished_status().is_none() {
        assert!(
            Instant::now() < deadline,
            "o run não terminou: {:?}",
            h.kinds()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert!(
        !h.workspace.parent().unwrap().join("fora.txt").exists(),
        "o arquivo não pode ter sido criado fora do projeto"
    );
    assert!(h.has("tool.denied"));

    // E o programa seguiu vivo depois da recusa: quem imprimiu foi o `catch`.
    let run_id = h.events.lock().unwrap()[0].run_id.clone();
    let saida = h
        .store
        .list_tool_calls(&run_id)
        .unwrap()
        .into_iter()
        .find(|c| c.tool_name == "run_code")
        .and_then(|c| c.result_json)
        .unwrap_or_default();
    assert!(saida.contains("recusado:"), "{saida}");
}

/// A peça que o agente escreve na conversa vira função do programa seguinte.
///
/// É o "modo de criação" do DeepSeek Harness na versão que cabe num harness
/// compilado: a peça não vira ferramenta nativa (isso exigiria recompilar o
/// app), vira uma função dentro do programa — e roda no mesmo cerco de
/// permissões, sem acesso a arquivo por fora da ponte.
#[tokio::test]
async fn uma_peca_escrita_na_conversa_e_usada_no_programa_seguinte() {
    if !tem_node() {
        eprintln!("pulando: `node` não está instalado");
        return;
    }
    let h = Harness::new();
    std::fs::write(
        h.workspace.join("a.txt"),
        "linha um\nlinha dois\nlinha três\n",
    )
    .unwrap();

    let peca = "// @tool {\"name\":\"conta_linhas\",\"description\":\"Conta as linhas de um arquivo.\",\
                \"parameters\":{\"type\":\"object\",\"properties\":{\"path\":{\"type\":\"string\"}},\
                \"required\":[\"path\"]}}\n\
                export default async function ({ path }) {\n\
                  const texto = await fs_read({ path });\n\
                  return texto.trim().split(\"\\n\").length;\n\
                }\n";

    let server = FakeLlama::spawn(vec![
        // Passo 1: o agente escreve a peça com a ferramenta que já tem.
        tool_call_chunks(
            "call_1",
            "fs_write",
            &serde_json::json!({
                "path": ".openweights/plugins/conta_linhas.mjs",
                "content": peca,
            })
            .to_string(),
            "",
        ),
        // Passo 2: o programa usa a peça pelo nome, sem importar nada.
        programa(
            "call_2",
            "say(\"linhas:\", await plugin_conta_linhas({ path: \"a.txt\" }));",
        ),
        vec![text_chunk("Criei a peça e usei."), done()],
    ]);

    let status = h
        .run_code_mode("crie uma peça e use", RunMode::Yolo, server.endpoint())
        .await;
    assert_eq!(status, RunStatus::Done, "eventos: {:?}", h.kinds());

    let run_id = h.events.lock().unwrap()[0].run_id.clone();
    let saida = h
        .store
        .list_tool_calls(&run_id)
        .unwrap()
        .into_iter()
        .find(|c| c.tool_name == "run_code")
        .and_then(|c| c.result_json)
        .unwrap_or_default();
    // A peça chamou a ferramenta e contou as três linhas do arquivo.
    assert!(saida.contains("linhas: 3"), "{saida}");
}

/// O que o programa recebe de uma ferramenta é DADO, não a frase que o modelo
/// leria. Sem isto, `for (const arquivo of await fs_glob(...))` itera a frase
/// caractere por caractere — foi o que aconteceu na primeira medição com o
/// qwen2.5-coder:14b, e o programa acabou pedindo `fs_read({path: "1"})`.
#[tokio::test]
async fn o_programa_recebe_lista_e_texto_cru_das_ferramentas() {
    if !tem_node() {
        eprintln!("pulando: `node` não está instalado");
        return;
    }
    let h = Harness::new();
    std::fs::create_dir_all(h.workspace.join("logs")).unwrap();
    for (nome, texto) in [("a.log", "um\ndois\n"), ("b.log", "três\n")] {
        std::fs::write(h.workspace.join("logs").join(nome), texto).unwrap();
    }

    let server = FakeLlama::spawn(vec![
        programa(
            "call_1",
            "const arquivos = await fs_glob({ pattern: \"logs/*.log\" });\n\
             say(\"tipo:\", Array.isArray(arquivos) ? \"array\" : typeof arquivos);\n\
             let linhas = 0;\n\
             for (const arquivo of arquivos) {\n\
               const texto = await fs_read({ path: arquivo });\n\
               linhas += texto.trim().split(\"\\n\").length;\n\
             }\n\
             say(\"arquivos:\", arquivos.length, \"linhas:\", linhas);\n",
        ),
        vec![text_chunk("Contei as linhas."), done()],
    ]);

    let status = h
        .run_code_mode("conte as linhas", RunMode::Yolo, server.endpoint())
        .await;
    assert_eq!(status, RunStatus::Done, "eventos: {:?}", h.kinds());

    let run_id = h.events.lock().unwrap()[0].run_id.clone();
    let saida = h
        .store
        .list_tool_calls(&run_id)
        .unwrap()
        .into_iter()
        .find(|c| c.tool_name == "run_code")
        .and_then(|c| c.result_json)
        .unwrap_or_default();
    assert!(saida.contains("tipo: array"), "{saida}");
    // Três linhas ao todo, contadas do texto CRU (com cabeçalho e numeração
    // daria outro número).
    assert!(saida.contains("arquivos: 2 linhas: 3"), "{saida}");
}
