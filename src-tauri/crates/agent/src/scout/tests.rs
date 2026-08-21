//! Testes da Scout Rule contra um **servidor HTTP falso** (std::net, sem
//! dependência nova), no mesmo espírito de `crates/engine/src/client/tests.rs`.
//!
//! O que precisa ficar provado aqui:
//! 1. a decomposição sobrevive ao que um modelo pequeno devolve (JSON limpo,
//!    JSON em cerca de código, lixo, lista vazia, lista gigante);
//! 2. cada etapa roda com contexto NOVO — só o handoff atravessa;
//! 3. uma etapa que falha duas vezes trava sem derrubar o plano.

use super::*;
use crate::events::EventSink;
use crate::plan_tools::PlanTools;
use crate::reliability::{ErrorStreak, ReadLedger, RepeatDetector};
use crate::run::StepEngine;
use crate::{AgentConfig, RunHandle};
use lr_policy::PolicyEngine;
use lr_store::Store;
use lr_tools::ToolRegistry;
use lr_types::agent::{RunEvent, RunMode, RunOptions, ToolCategory, ToolOrigin, ToolTier};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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

    fn client(&self) -> LlamaClient {
        LlamaClient::new(format!("http://{}", self.addr))
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

/// Resposta não-streaming com um texto.
fn completion(text: &str) -> String {
    json!({ "choices": [{ "message": { "content": text }, "finish_reason": "stop" }] }).to_string()
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

fn props_body() -> String {
    json!({
        "chat_template_caps": { "supports_tools": true, "supports_system_role": true },
        "default_generation_settings": { "n_ctx": 8192 }
    })
    .to_string()
}

// ------------------------------------------------------------ decomposição ---

/// Servidor que devolve sempre a mesma resposta de chat.
fn one_shot(reply: String) -> FakeServer {
    FakeServer::spawn(move |r| match r.path.as_str() {
        "/props" => FakeResponse::json(props_body()),
        // A decomposição passou a pedir stream (é o que faz o raciocínio do
        // modelo aparecer enquanto o plano é montado); o corpo continua sendo
        // o mesmo JSON, agora em pedaços.
        "/v1/chat/completions" if r.body.contains("\"stream\":true") => {
            FakeResponse::sse(stream_text(&reply))
        }
        "/v1/chat/completions" => FakeResponse::json(completion(&reply)),
        _ => FakeResponse::error(404, "{}"),
    })
}

fn budget() -> WindowBudget {
    WindowBudget::new(8_192)
}

async fn decompose_with(reply: String) -> (TaskPlan, FakeServer) {
    let server = one_shot(reply);
    let plan = decompose(
        &server.client(),
        "m",
        "arrumar o projeto",
        budget(),
        "",
        MAX_TASKS as u32,
        &mut |_| {},
    )
    .await
    .expect("o servidor respondeu");
    (plan, server)
}

fn plan_json(count: usize) -> String {
    let tasks: Vec<Value> = (0..count)
        .map(|i| {
            json!({
                "title": format!("etapa {i}"),
                "instruction": format!("faça a parte {i}"),
                "done_when": format!("a parte {i} existe")
            })
        })
        .collect();
    json!({ "tasks": tasks }).to_string()
}

async fn triagem(reply: &str) -> bool {
    let server = one_shot(reply.to_string());
    needs_plan(&server.client(), "m", "Bom dia", "", &mut |_| {}).await
}

#[tokio::test]
async fn a_greeting_does_not_become_a_work_plan() {
    let nao = json!({ "precisa_plano": false, "porque": "é uma saudação" }).to_string();
    assert!(!triagem(&nao).await);
}

#[tokio::test]
async fn real_work_still_gets_a_plan() {
    let sim = json!({ "precisa_plano": true, "porque": "pede para criar arquivos" }).to_string();
    assert!(triagem(&sim).await);
}

/// Na dúvida, planeja: recusar plano a um pedido real devolveria a pessoa ao
/// laço solto, sem gate por entrega.
#[tokio::test]
async fn an_unreadable_triage_keeps_planning() {
    assert!(triagem("não sei dizer").await);
    assert!(triagem(&json!({ "porque": "faltou o booleano" }).to_string()).await);
}

#[tokio::test]
async fn a_server_that_does_not_answer_keeps_planning() {
    let server = FakeServer::spawn(|r| match r.path.as_str() {
        "/props" => FakeResponse::json(props_body()),
        _ => FakeResponse::error(500, "{}"),
    });
    assert!(needs_plan(&server.client(), "m", "qualquer coisa", "", &mut |_| {}).await);
}

#[tokio::test]
async fn decompose_reads_a_valid_plan() {
    let (plan, _server) = decompose_with(plan_json(3)).await;
    assert_eq!(plan.goal, "arrumar o projeto");
    assert_eq!(plan.tasks.len(), 3);
    assert_eq!(plan.tasks[0].title, "etapa 0");
    assert_eq!(plan.tasks[1].instruction, "faça a parte 1");
    assert_eq!(plan.tasks[2].done_when, "a parte 2 existe");
    assert!(plan.tasks.iter().all(|t| t.status == TaskStatus::Pending));
    assert_eq!(plan.tasks[0].id, "t1");
    assert!(
        plan.tasks[0].est_tokens > 0,
        "cada etapa carrega o teto de tokens dela"
    );
    assert!(!plan.approved, "plano novo não nasce aprovado");
}

#[tokio::test]
async fn decompose_accepts_json_inside_a_code_fence() {
    let reply = format!(
        "Claro! Aqui vai:\n```json\n{}\n```\nEspero ter ajudado.",
        plan_json(2)
    );
    let (plan, _server) = decompose_with(reply).await;
    assert_eq!(plan.tasks.len(), 2, "a cerca de código não pode atrapalhar");
}

#[tokio::test]
async fn decompose_forces_a_grammar_with_json_schema() {
    let (_plan, server) = decompose_with(plan_json(2)).await;
    let body = server
        .chat_bodies()
        .into_iter()
        .find(|b| b.contains("response_format"))
        .expect("o pedido tem que levar o schema");
    assert!(body.contains("json_schema"));
    assert!(body.contains("done_when"));
    assert!(body.contains("maxItems"), "o teto de 12 vai na gramática");
}

#[tokio::test]
async fn decompose_falls_back_to_a_single_task_when_the_answer_is_garbage() {
    let (plan, _server) = decompose_with("desculpe, não consegui pensar nisso".into()).await;
    assert_eq!(plan.tasks.len(), 1, "degrade gracioso: uma entrega só");
    assert_eq!(plan.tasks[0].instruction, "arrumar o projeto");
    assert_eq!(plan.goal, "arrumar o projeto");
}

#[tokio::test]
async fn decompose_falls_back_when_the_list_comes_empty() {
    let (plan, _server) = decompose_with(json!({ "tasks": [] }).to_string()).await;
    assert_eq!(plan.tasks.len(), 1);
    assert_eq!(plan.tasks[0].title, "arrumar o projeto");
}

#[tokio::test]
async fn decompose_cuts_the_plan_at_the_ceiling() {
    let (plan, _server) = decompose_with(plan_json(20)).await;
    assert_eq!(plan.tasks.len(), MAX_TASKS);
    assert_eq!(plan.tasks.last().unwrap().title, "etapa 11");
}

#[tokio::test]
async fn decompose_drops_dependencies_that_would_deadlock_the_plan() {
    let reply = json!({
        "tasks": [
            // Depende de uma etapa que vem DEPOIS: nunca ficaria pronta.
            {"title": "um", "instruction": "i1", "done_when": "d1", "depends_on": [2]},
            // 0 vale; 99 não existe e 1 é ela mesma.
            {"title": "dois", "instruction": "i2", "done_when": "d2", "depends_on": [0, 99, 1]},
            {"title": "três", "instruction": "i3", "done_when": "d3", "depends_on": [1, 0]}
        ]
    })
    .to_string();
    let (plan, _server) = decompose_with(reply).await;
    assert!(plan.tasks[0].depends_on.is_empty());
    assert_eq!(plan.tasks[1].depends_on, vec![0]);
    assert_eq!(plan.tasks[2].depends_on, vec![0, 1]);
    // O plano continua executável do começo ao fim.
    assert_eq!(plan.next_index(), Some(0));
}

#[tokio::test]
async fn decompose_reports_a_server_that_did_not_answer() {
    let server =
        FakeServer::spawn(|_| FakeResponse::error(500, "{\"error\":{\"message\":\"caiu\"}}"));
    let err = decompose(
        &server.client(),
        "m",
        "arrumar",
        budget(),
        "",
        MAX_TASKS as u32,
        &mut |_| {},
    )
    .await
    .unwrap_err();
    assert!(err.contains("caiu") || err.contains("500"), "{err}");
}

/// Orçamento apertado não vira plano grande: uma automação que pede doze
/// passos não pode planejar oito entregas de oito passos cada.
#[tokio::test]
async fn a_small_step_budget_asks_for_a_small_plan() {
    let server = one_shot(plan_json(2));
    let _ = decompose(
        &server.client(),
        "m",
        "arrumar",
        budget(),
        "",
        1,
        &mut |_| {},
    )
    .await
    .unwrap();

    let corpos = server.chat_bodies();
    let pedido = corpos.last().cloned().unwrap_or_default();
    assert!(
        pedido.contains("cerca de 1 entregas") || pedido.contains("cerca de 1 entrega"),
        "o pedido de divisão precisa carregar o teto: {pedido}"
    );
}

#[tokio::test]
async fn decompose_keeps_the_investigation_notes() {
    let server = one_shot(plan_json(2));
    let plan = decompose(
        &server.client(),
        "m",
        "arrumar",
        budget(),
        "o projeto usa pnpm e vitest",
        MAX_TASKS as u32,
        &mut |_| {},
    )
    .await
    .unwrap();
    assert!(plan.notes.contains("pnpm"));
}

// ------------------------------------------------------------- validação ---

#[test]
fn a_plain_list_of_strings_still_becomes_a_plan() {
    let plan = plan_from_value(
        "objetivo",
        &json!(["ler o README", "escrever o teste"]),
        100,
    )
    .expect("lista de strings é aceita");
    assert_eq!(plan.tasks.len(), 2);
    assert_eq!(plan.tasks[0].title, "ler o README");
    assert_eq!(plan.tasks[0].instruction, "ler o README");
    assert!(!plan.tasks[0].done_when.is_empty(), "sempre há um critério");
}

#[test]
fn a_task_carries_the_files_it_promises_to_touch() {
    // Sem este campo a conferência de entrega não tem o que procurar em
    // disco, e "acabou" volta a ser a palavra do modelo.
    let plan = plan_from_value(
        "objetivo",
        &json!({"tasks": [{
            "title": "HUD",
            "instruction": "criar a HUD",
            "done_when": "existe",
            "files": ["src/ui/hud.js", "src/ui/hud.css"]
        }]}),
        0,
    )
    .unwrap();
    assert_eq!(plan.tasks[0].files, vec!["src/ui/hud.js", "src/ui/hud.css"]);
}

#[test]
fn a_path_that_escapes_the_project_is_not_a_deliverable() {
    // Caminho absoluto ou com `..` nunca seria encontrado sob a pasta do
    // projeto: viraria pendência eterna, e o run cobraria para sempre uma
    // entrega impossível.
    let plan = plan_from_value(
        "objetivo",
        &json!({"tasks": [{
            "title": "t",
            "instruction": "i",
            "done_when": "d",
            "files": ["/etc/passwd", "../fora.js", "C:\\x.js", "  ", "dentro.js"]
        }]}),
        0,
    )
    .unwrap();
    assert_eq!(plan.tasks[0].files, vec!["dentro.js"]);
}

#[test]
fn tasks_without_title_and_without_instruction_are_dropped() {
    let plan = plan_from_value(
        "objetivo",
        &json!({"tasks": [{"done_when": "nada"}, {"title": "vale", "instruction": "faça"}]}),
        0,
    )
    .unwrap();
    assert_eq!(plan.tasks.len(), 1);
    assert_eq!(plan.tasks[0].title, "vale");
    assert!(plan_from_value("o", &json!({"tasks": []}), 0).is_none());
    assert!(plan_from_value("o", &json!("nada disso"), 0).is_none());
}

#[test]
fn the_json_is_found_even_cercado_de_conversa() {
    let v = json_from_text("Segue o plano: {\"tasks\": [{\"title\": \"a\"}]} — pronto!").unwrap();
    assert_eq!(v["tasks"][0]["title"], "a");
    // Chave com chave dentro de string não confunde o balanceamento.
    let v = json_from_text("blá ```{\"nota\": \"um } aqui\", \"n\": 1}``` fim").unwrap();
    assert_eq!(v["n"], 1);
    assert!(json_from_text("sem json nenhum").is_none());
    assert!(json_from_text("   ").is_none());
}

#[test]
fn a_single_task_plan_keeps_the_goal_intact() {
    let plan = single_task_plan("  publicar o site  ", 1_000);
    assert_eq!(plan.goal, "publicar o site");
    assert_eq!(plan.tasks.len(), 1);
    assert_eq!(plan.tasks[0].instruction, "publicar o site");
    assert_eq!(plan.tasks[0].est_tokens, 1_000);
}

#[test]
fn named_phases_in_a_huge_spec_become_many_deliveries() {
    let spec = "\
Execute apenas a etapa atual. Não avance automaticamente.\n\
\n\
## Fase 1 Fundação\n\
Criar o app Next.js com Three.js e SSR modular.\n\
\n\
## Fase 2 Mundo\n\
Cena 3D, chão e câmera.\n\
\n\
## Fase 3 Personagens\n\
Modelos e animações.\n\
\n\
## Fase 4 Combate\n\
Habilidades e HUD.\n\
\n\
## Fase 5 Rede\n\
Matchmaking.\n\
\n\
## Fase 6 Polimento\n\
Áudio e menu.\n";
    let plan = fallback_plan(spec, 8_000);
    assert!(
        plan.tasks.len() >= 6,
        "esperava uma entrega por fase, veio {}",
        plan.tasks.len()
    );
    for t in &plan.tasks {
        assert!(
            t.instruction.chars().count() < spec.chars().count(),
            "instrução não pode ser o spec inteiro"
        );
        assert!(
            t.instruction.contains("Implemente APENAS esta fase"),
            "{}",
            t.instruction
        );
    }
    let brief = task_brief(spec, "", 0, plan.tasks.len(), &plan.tasks[0]);
    assert!(brief.contains("Objetivo geral (recorte)"));
    assert!(
        !brief.contains("Fase 6"),
        "a etapa 1 não pode carregar as fases seguintes: {brief}"
    );
    assert!(brief.contains("Fase 1") || brief.contains("Fundação"));
}

#[test]
fn a_one_task_model_plan_is_replaced_when_the_spec_already_has_phases() {
    let spec = format!(
        "Fase 1 Fundação\ncriar src/app/page.tsx\n\nFase 2 Mundo\ncena three.js\n\n{}",
        "lorem ".repeat(800)
    );
    let gigante = single_task_plan(&spec, 14_746);
    assert_eq!(gigante.tasks.len(), 1);
    let plan = prefer_split_plan(&spec, gigante, 14_746);
    assert!(plan.tasks.len() >= 2, "veio {}", plan.tasks.len());
}

#[test]
fn node_v_is_not_kept_as_acceptance_command() {
    let plan = plan_from_value(
        "objetivo",
        &json!({"tasks": [{
            "title": "app",
            "instruction": "criar o next app",
            "done_when": "existe",
            "files": ["package.json"],
            "check_cmd": "node -v"
        }]}),
        0,
    )
    .unwrap();
    assert!(plan.tasks[0].check_cmd.is_none());
}

#[test]
fn the_brief_carries_the_digest_and_not_the_history() {
    let task = Task {
        instruction: "escreva o teste em tests/novo.rs".into(),
        done_when: "o teste roda".into(),
        ..Task::new("t2", "escrever o teste")
    };
    let brief = task_brief(
        "arrumar o projeto",
        "- ler o README: o projeto usa pnpm",
        1,
        3,
        &task,
    );
    assert!(brief.contains("Objetivo geral (recorte): arrumar o projeto"));
    assert!(brief.contains("o projeto usa pnpm"), "o handoff atravessa");
    assert!(brief.contains("2 de 3"));
    assert!(brief.contains("tests/novo.rs"));
    assert!(brief.contains("Pronto quando: o teste roda"));
    assert!(brief.contains("task_complete"));
    // Curto o bastante para caber numa janela de modelo pequeno.
    assert!(
        brief.len() < 2_200,
        "brief longo demais: {} bytes",
        brief.len()
    );
}

#[test]
fn a_retry_tells_the_model_what_failed_before() {
    let task = Task {
        instruction: "rode a suíte".into(),
        error: Some("a conferência não passou: `cargo test` terminou com código 101".into()),
        ..Task::new("t1", "rodar os testes")
    };
    let brief = task_brief("arrumar", "", 0, 1, &task);
    assert!(brief.contains("tentativa anterior"));
    assert!(brief.contains("código 101"));
}

#[test]
fn plan_mode_only_sees_read_only_tools() {
    let specs = vec![
        spec("fs_read", ToolCategory::Read, true),
        spec("fs_write", ToolCategory::Edit, false),
        spec("terminal_run", ToolCategory::Execute, false),
        spec("todo_update", ToolCategory::Meta, true),
    ];

    let names = |mode| {
        menu_for(mode, specs.clone())
            .into_iter()
            .map(|s| s.name)
            .collect::<Vec<_>>()
    };
    assert_eq!(names(WorkMode::Plan), vec!["fs_read", "todo_update"]);
    assert!(names(WorkMode::Chat).is_empty());
    assert_eq!(names(WorkMode::Agent).len(), 4);
    assert_eq!(names(WorkMode::Loop).len(), 4);
}

fn spec(name: &str, category: ToolCategory, read_only: bool) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        description: String::new(),
        parameters: json!({"type": "object"}),
        category,
        tier: ToolTier::Safe,
        origin: ToolOrigin::Builtin,
        read_only,
    }
}

// -------------------------------------------------------- laço do plano ---

/// Um run montado à mão, sem `AgentHost`: só o que o laço de plano precisa.
struct Harness {
    store: Arc<Store>,
    handle: Arc<RunHandle>,
    config: Arc<AgentConfig>,
    registry: Arc<ToolRegistry>,
    opts: RunOptions,
    events: Arc<Mutex<Vec<RunEvent>>>,
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
        registry: Arc::new(ToolRegistry::new()),
        opts: serde_json::from_value(json!({
            "chatId": 0, "model": "m", "mode": "yolo", "maxSteps": 40
        }))
        .unwrap(),
        store,
        events,
        _dir: dir,
    }
}

fn tool_runner(h: &Harness, tools: &PlanTools) -> ToolRunner {
    ToolRunner {
        dentro_de_programa: false,
        escalonar_apos_programa: None,
        saida_ao_vivo: None,
        code_menu: None,
        run_id: "r1".into(),
        mode: RunMode::Yolo,
        workspace: None,
        sink: h.handle.sink(),
        registry: h.registry.clone(),
        store: h.store.clone(),
        handle: h.handle.clone(),
        config: h.config.clone(),
        policy: PolicyEngine::new(Vec::new()),
        overrides: Vec::new(),
        snapshotted: Default::default(),
        full_snapshot: false,
        reads: ReadLedger::default(),
        repeats: RepeatDetector::default(),
        errors: ErrorStreak::default(),
        written: Vec::new(),
        task_written0: 0,
        exige_escrita: false,
        reescritas: Default::default(),
        commands: Vec::new(),
        focus_md: None,
        tool_calls: 0,
        local_tools: tools.tools.clone(),
        halt: tools.halt.clone(),
        counters: Default::default(),
    }
}

fn engine<'a>(h: &Harness, client: &'a LlamaClient, opts: &'a RunOptions) -> StepEngine<'a> {
    StepEngine {
        client,
        sink: h.handle.sink(),
        handle: h.handle.clone(),
        store: h.store.clone(),
        run_id: "r1".into(),
        opts,
        menu: std::sync::Arc::new(crate::menu::MenuState::new(
            crate::menu::curate(Vec::new(), "", &lr_types::agent::ToolGroup::ALL, None),
            0,
        )),
        groups: lr_types::agent::ToolGroup::ALL.to_vec(),
        tools_on: std::sync::atomic::AtomicBool::new(true),
        recusas_de_tools: std::sync::atomic::AtomicU32::new(0),
        context: crate::reliability::ContextBudget::new(Some(8_192), 0.85),
    }
}

fn task(id: &str, title: &str, instruction: &str) -> Task {
    Task {
        instruction: instruction.into(),
        done_when: "está pronto".into(),
        ..Task::new(id, title)
    }
}

fn build_system(focus: Option<&String>) -> String {
    format!("PROMPT DE SISTEMA\n{}", focus.cloned().unwrap_or_default())
}

/// Servidor do laço: stream para os passos, JSON para o handoff, e 500 para
/// qualquer etapa cuja instrução contenha `falha_em`.
fn loop_server(falha_em: Option<&'static str>) -> FakeServer {
    FakeServer::spawn(move |r| match r.path.as_str() {
        "/props" => FakeResponse::json(props_body()),
        "/v1/chat/completions/input_tokens" => {
            FakeResponse::json(json!({ "prompt_tokens": 120 }).to_string())
        }
        "/v1/chat/completions" => {
            if r.body.contains("\"stream\":true") {
                if falha_em.is_some_and(|marca| r.body.contains(marca)) {
                    return FakeResponse::error(500, "{\"error\":{\"message\":\"sem memória\"}}");
                }
                return FakeResponse::sse(stream_text("terminei o que dava"));
            }
            // Pedido de handoff: o corpo traz o título da etapa.
            let quem = if r.body.contains("alfa") {
                "alfa"
            } else {
                "beta"
            };
            FakeResponse::json(completion(&format!("resumo-{quem}: deixei tudo no lugar")))
        }
        _ => FakeResponse::error(404, "{}"),
    })
}

#[tokio::test]
async fn each_task_runs_with_a_fresh_context_carrying_only_the_handoff() {
    let server = loop_server(None);
    let h = harness();
    let client = server.client();
    let plan = crate::plan_tools::shared_plan(TaskPlan {
        goal: "arrumar o projeto".into(),
        tasks: vec![
            task("t1", "alfa", "MARCA-ALFA escreva o resumo"),
            task("t2", "beta", "MARCA-BETA revise o resumo"),
        ],
        ..Default::default()
    });
    let tools = PlanTools::for_mode(WorkMode::Loop, plan.clone());
    let mut runner = tool_runner(&h, &tools);
    let mut steps = StepBudget::new(40);
    let eng = engine(&h, &client, &h.opts);

    let ctx = PlanRun {
        engine: &eng,
        build_system: &build_system,
        plan: plan.clone(),
        workspace: None,
        goal: "arrumar o projeto".into(),
        context: String::new(),
        window: budget(),
        max_tasks: MAX_TASKS as u32,
        checar_com_terminal: false,
    };
    let outcome = run_plan(&ctx, &mut runner, &mut steps).await;

    let saved = crate::plan_tools::snapshot(&plan);
    assert_eq!(outcome.status, RunStatus::Done, "{}", outcome.summary);
    assert!(saved.is_complete());
    assert_eq!(saved.progress(), (2, 2));
    assert!(
        saved.tasks[0]
            .handoff
            .as_deref()
            .unwrap()
            .contains("resumo-alfa")
    );
    assert!(outcome.summary.contains("2 de 2"));

    // O contexto da segunda etapa: leva o handoff da primeira e NÃO leva a
    // instrução dela. É a Scout Rule inteira nesta asserção.
    let beta = server
        .chat_bodies()
        .into_iter()
        .find(|b| b.contains("\"stream\":true") && b.contains("MARCA-BETA"))
        .expect("a segunda etapa rodou");
    assert!(beta.contains("resumo-alfa"), "o handoff atravessa");
    assert!(
        !beta.contains("MARCA-ALFA"),
        "o histórico da etapa anterior não pode atravessar"
    );

    // O plano gravado é o que o comando `run_plan_get` vai devolver.
    let json = h.store.get_run_plan("r1").unwrap().unwrap();
    let from_db: TaskPlan = serde_json::from_str(&json).unwrap();
    assert!(from_db.is_complete());
    // E a interface acompanhou a divisão acontecendo.
    let focos = h
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e.event, RunEventKind::FocusUpdated { .. }))
        .count();
    assert!(focos >= 4, "cada mudança do plano vira um evento: {focos}");
}

#[tokio::test]
async fn a_task_that_fails_twice_is_blocked_and_the_loop_moves_on() {
    let server = loop_server(Some("MARCA-ALFA"));
    let h = harness();
    let client = server.client();
    let mut gama = task("t3", "gama", "MARCA-GAMA depende da alfa");
    gama.depends_on = vec![0];
    let plan = crate::plan_tools::shared_plan(TaskPlan {
        goal: "arrumar o projeto".into(),
        tasks: vec![
            task("t1", "alfa", "MARCA-ALFA escreva o resumo"),
            task("t2", "beta", "MARCA-BETA revise o resumo"),
            gama,
        ],
        ..Default::default()
    });
    let tools = PlanTools::for_mode(WorkMode::Loop, plan.clone());
    let mut runner = tool_runner(&h, &tools);
    let mut steps = StepBudget::new(40);
    let eng = engine(&h, &client, &h.opts);

    let ctx = PlanRun {
        engine: &eng,
        build_system: &build_system,
        plan: plan.clone(),
        workspace: None,
        goal: "arrumar o projeto".into(),
        context: String::new(),
        window: budget(),
        max_tasks: MAX_TASKS as u32,
        checar_com_terminal: false,
    };
    let outcome = run_plan(&ctx, &mut runner, &mut steps).await;

    let saved = crate::plan_tools::snapshot(&plan);
    assert_eq!(
        saved.tasks[0].status,
        TaskStatus::Blocked,
        "duas falhas travam"
    );
    assert!(saved.tasks[0].error.is_some());
    assert_eq!(
        saved.tasks[1].status,
        TaskStatus::Done,
        "o laço segue para quem não depende da travada"
    );
    assert_eq!(
        saved.tasks[2].status,
        TaskStatus::Pending,
        "quem depende da travada nunca roda"
    );
    assert_eq!(outcome.status, RunStatus::Escalated);
    assert!(outcome.summary.contains("Travou em"), "{}", outcome.summary);

    let tentativas = server
        .chat_bodies()
        .into_iter()
        .filter(|b| b.contains("\"stream\":true") && b.contains("MARCA-ALFA"))
        .count();
    // Duas tentativas da ETAPA, e cada uma paga os retries transitórios do
    // stream (o 500 genérico é indistinguível de um blip de rede — e retry
    // com as ferramentas intactas é o comportamento certo para blip).
    let esperado = 2 * (1 + crate::run::MAX_TENTATIVAS_STREAM as usize);
    assert_eq!(tentativas, esperado, "duas tentativas × retries do stream");
}

#[tokio::test]
async fn the_loop_stops_when_the_model_asks_the_person() {
    // Servidor que sempre pede `ask_user` no primeiro passo da etapa.
    let server = FakeServer::spawn(|r| match r.path.as_str() {
        "/props" => FakeResponse::json(props_body()),
        "/v1/chat/completions/input_tokens" => {
            FakeResponse::json(json!({ "prompt_tokens": 100 }).to_string())
        }
        "/v1/chat/completions" if r.body.contains("\"stream\":true") => FakeResponse::sse(vec![
            sse(&json!({"choices":[{"delta":{"tool_calls":[{
                "index": 0, "id": "c1", "type": "function",
                "function": {"name": "ask_user", "arguments": "{\"question\":\"apago o antigo?\"}"}
            }]}}]})
            .to_string()),
            sse(&json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}).to_string()),
            sse("[DONE]"),
        ]),
        "/v1/chat/completions" => FakeResponse::json(completion("resumo")),
        _ => FakeResponse::error(404, "{}"),
    });

    let h = harness();
    let client = server.client();
    let plan = crate::plan_tools::shared_plan(TaskPlan {
        goal: "arrumar".into(),
        tasks: vec![
            task("t1", "alfa", "MARCA-ALFA faça"),
            task("t2", "beta", "MARCA-BETA faça"),
        ],
        ..Default::default()
    });
    let tools = PlanTools::for_mode(WorkMode::Loop, plan.clone());
    let mut runner = tool_runner(&h, &tools);
    let mut steps = StepBudget::new(40);
    let eng = engine(&h, &client, &h.opts);
    let ctx = PlanRun {
        engine: &eng,
        build_system: &build_system,
        plan: plan.clone(),
        workspace: None,
        goal: "arrumar".into(),
        context: String::new(),
        window: budget(),
        max_tasks: MAX_TASKS as u32,
        checar_com_terminal: false,
    };
    let outcome = run_plan(&ctx, &mut runner, &mut steps).await;

    let saved = crate::plan_tools::snapshot(&plan);
    assert_eq!(saved.tasks[0].status, TaskStatus::Blocked);
    assert_eq!(
        saved.tasks[1].status,
        TaskStatus::Pending,
        "a pergunta para o plano inteiro, não só a etapa"
    );
    assert_eq!(outcome.status, RunStatus::Escalated);
    assert_eq!(outcome.question.as_deref(), Some("apago o antigo?"));
    assert!(outcome.summary.contains("apago o antigo?"));
}

#[tokio::test]
async fn task_complete_closes_the_step_with_the_handoff_the_model_wrote() {
    // O modelo chama `task_complete` em vez de responder em texto: o laço
    // aproveita o resumo dele e não gasta uma chamada extra.
    let server = FakeServer::spawn(|r| {
        match r.path.as_str() {
        "/props" => FakeResponse::json(props_body()),
        "/v1/chat/completions/input_tokens" => {
            FakeResponse::json(json!({ "prompt_tokens": 100 }).to_string())
        }
        "/v1/chat/completions" if r.body.contains("\"stream\":true") => FakeResponse::sse(vec![
            sse(&json!({"choices":[{"delta":{"tool_calls":[{
                "index": 0, "id": "c1", "type": "function",
                "function": {"name": "task_complete", "arguments": "{\"handoff\":\"criei notas.md com o resumo\"}"}
            }]}}]})
            .to_string()),
            sse(&json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}).to_string()),
            sse("[DONE]"),
        ]),
        _ => FakeResponse::error(500, "{\"error\":{\"message\":\"não era para pedir handoff\"}}"),
    }
    });

    let h = harness();
    let client = server.client();
    let plan = crate::plan_tools::shared_plan(TaskPlan {
        goal: "escrever as notas".into(),
        tasks: vec![task("t1", "alfa", "MARCA-ALFA escreva")],
        ..Default::default()
    });
    let tools = PlanTools::for_mode(WorkMode::Loop, plan.clone());
    let mut runner = tool_runner(&h, &tools);
    let mut steps = StepBudget::new(10);
    let eng = engine(&h, &client, &h.opts);
    let ctx = PlanRun {
        engine: &eng,
        build_system: &build_system,
        plan: plan.clone(),
        workspace: None,
        goal: "escrever as notas".into(),
        context: String::new(),
        window: budget(),
        max_tasks: MAX_TASKS as u32,
        checar_com_terminal: false,
    };
    let outcome = run_plan(&ctx, &mut runner, &mut steps).await;

    let saved = crate::plan_tools::snapshot(&plan);
    assert_eq!(saved.tasks[0].status, TaskStatus::Done);
    assert_eq!(
        saved.tasks[0].handoff.as_deref(),
        Some("criei notas.md com o resumo")
    );
    assert_eq!(outcome.status, RunStatus::Done);
    // Um passo só: a ferramenta encerrou a etapa em vez de gastar outro.
    assert_eq!(outcome.steps, 1);
}

#[tokio::test]
async fn an_empty_plan_is_decomposed_before_the_loop_starts() {
    let server = FakeServer::spawn(|r| {
        match r.path.as_str() {
        "/props" => FakeResponse::json(props_body()),
        "/v1/chat/completions/input_tokens" => {
            FakeResponse::json(json!({ "prompt_tokens": 100 }).to_string())
        }
        // A decomposição também é stream agora; o que a distingue do passo
        // normal é o `response_format` com o esquema do plano.
        "/v1/chat/completions" if r.body.contains("response_format") => FakeResponse::sse(
            stream_text(&json!({"tasks": [{"title": "única", "instruction": "faça tudo", "done_when": "pronto"}]}).to_string()),
        ),
        "/v1/chat/completions" if r.body.contains("\"stream\":true") => {
            FakeResponse::sse(stream_text("feito"))
        }
        "/v1/chat/completions" => FakeResponse::json(completion("resumo curto")),
        _ => FakeResponse::error(404, "{}"),
    }
    });

    let h = harness();
    let client = server.client();
    let plan = crate::plan_tools::shared_plan(TaskPlan::default());
    let tools = PlanTools::for_mode(WorkMode::Loop, plan.clone());
    let mut runner = tool_runner(&h, &tools);
    let mut steps = StepBudget::new(10);
    let eng = engine(&h, &client, &h.opts);
    let ctx = PlanRun {
        engine: &eng,
        build_system: &build_system,
        plan: plan.clone(),
        workspace: None,
        goal: "fazer tudo".into(),
        context: String::new(),
        window: budget(),
        max_tasks: MAX_TASKS as u32,
        checar_com_terminal: false,
    };
    let outcome = run_plan(&ctx, &mut runner, &mut steps).await;

    let saved = crate::plan_tools::snapshot(&plan);
    assert_eq!(saved.tasks.len(), 1);
    assert_eq!(saved.tasks[0].title, "única");
    assert_eq!(outcome.status, RunStatus::Done);
}

/// O teto de passos é o que a pessoa (ou automação) escolheu — o plano nunca
/// o estoura. Uma automação de madrugada com 12 passos não pode virar 100
/// porque o modelo resolveu devolver mais entregas.
#[tokio::test]
async fn the_global_step_ceiling_still_wins() {
    let server = loop_server(None);
    let h = harness();
    let client = server.client();
    let plan = crate::plan_tools::shared_plan(TaskPlan {
        goal: "muita coisa".into(),
        tasks: vec![
            task("t1", "alfa", "MARCA-ALFA faça"),
            task("t2", "beta", "MARCA-BETA faça"),
        ],
        ..Default::default()
    });
    let tools = PlanTools::for_mode(WorkMode::Loop, plan.clone());
    let mut runner = tool_runner(&h, &tools);
    // Um passo só para o run inteiro: a primeira etapa consome tudo.
    let mut steps = StepBudget::new(1);
    let eng = engine(&h, &client, &h.opts);
    let ctx = PlanRun {
        engine: &eng,
        build_system: &build_system,
        plan: plan.clone(),
        workspace: None,
        goal: "muita coisa".into(),
        context: String::new(),
        window: budget(),
        max_tasks: MAX_TASKS as u32,
        checar_com_terminal: false,
    };
    let outcome = run_plan(&ctx, &mut runner, &mut steps).await;

    assert_eq!(outcome.status, RunStatus::MaxSteps);
    let saved = crate::plan_tools::snapshot(&plan);
    assert_eq!(saved.tasks[1].status, TaskStatus::Pending);
    assert!(outcome.summary.contains("Faltou"), "{}", outcome.summary);
}

/// Perguntas da decomposição chegam estruturadas ao plano — com `task_index`
/// vazio (a pausa é ANTES da primeira etapa).
#[test]
fn plan_from_value_keeps_pre_work_questions() {
    let v = json!({
        "tasks": [{
            "title": "a", "instruction": "faça", "done_when": "x",
            "files": [], "check_cmd": ""
        }],
        "questions": ["Next ou Vite?", "  ", "Onde salvar?"]
    });
    let plan = plan_from_value("obj", &v, 100).expect("plano");
    let q = plan.pending_question.expect("perguntas presentes");
    assert_eq!(q.items.len(), 2, "vazio não vira pergunta");
    assert_eq!(q.items[0].text, "Next ou Vite?");
    assert_eq!(q.task_index, None);
}

/// Com perguntas pendentes, NADA executa: o laço para antes da etapa 1, o
/// desfecho é Escalated e a pausa estruturada sai no resultado — é a régua
/// de "perguntas sempre pausam", inclusive nas automações.
#[tokio::test]
async fn a_pending_question_pauses_before_the_first_task() {
    let server = loop_server(None);
    let h = harness();
    let client = server.client();
    let plan = crate::plan_tools::shared_plan(TaskPlan {
        goal: "algo ambíguo".into(),
        tasks: vec![task("t1", "alfa", "faça")],
        pending_question: Some(PendingQuestion {
            items: vec![QuestionItem {
                text: "Next ou Vite?".into(),
                options: vec!["Next".into(), "Vite".into()],
            }],
            task_index: None,
            asked_at_ms: 0,
        }),
        ..Default::default()
    });
    let tools = PlanTools::for_mode(WorkMode::Loop, plan.clone());
    let mut runner = tool_runner(&h, &tools);
    let mut steps = StepBudget::new(24);
    let eng = engine(&h, &client, &h.opts);
    let ctx = PlanRun {
        engine: &eng,
        build_system: &build_system,
        plan: plan.clone(),
        workspace: None,
        goal: "algo ambíguo".into(),
        context: String::new(),
        window: budget(),
        max_tasks: MAX_TASKS as u32,
        checar_com_terminal: false,
    };
    let outcome = run_plan(&ctx, &mut runner, &mut steps).await;

    assert_eq!(outcome.status, RunStatus::Escalated);
    let pendente = outcome
        .pending
        .expect("a pausa estruturada viaja no resultado");
    assert_eq!(pendente.items[0].options, vec!["Next", "Vite"]);
    assert!(
        server.chat_bodies().is_empty(),
        "nenhuma chamada ao modelo antes da resposta"
    );
    assert_eq!(
        crate::plan_tools::snapshot(&plan).tasks[0].status,
        TaskStatus::Pending,
        "a etapa não pode ter começado"
    );
}
