//! O agente contra um modelo de verdade.
//!
//! Os outros testes de ponta a ponta provam que as peças funcionam juntas com
//! um servidor falso: eventos na ordem, arquivo no disco, guard-rails. O que
//! eles **não** provam é a parte que só o modelo decide — se ele entende o
//! cardápio, se chama a ferramenta certa, se conserta o próprio erro quando o
//! resultado volta ruim, se termina a tarefa em vez de conversar sobre ela.
//!
//! Este teste responde isso rodando contra um llama-server real, com as
//! ferramentas reais, num diretório temporário. Ele é `#[ignore]` de
//! propósito: precisa de um modelo carregado e leva minutos.
//!
//! ```bash
//! # com o llama-server de pé (modo Router serve, e é o caminho do app):
//! OW_LIVE_URL=http://127.0.0.1:8099 \
//! OW_LIVE_MODEL="Qwen3.5-9B-...-Q4_K_M.gguf" \
//! cargo test -p lr_agent --test live_model -- --ignored --nocapture
//! ```
//!
//! A tarefa é deliberadamente do tamanho de um pedido real: uma página, um
//! servidor com as quatro operações e um README. Pequena o bastante para caber
//! em poucos passos, grande o bastante para exigir várias ferramentas e um
//! arquivo que depende do outro.

use lr_agent::{AgentConfig, AgentHost, Endpoint, StartRun};
use lr_store::Store;
use lr_types::agent::{ApprovalDecision, RunEvent, RunEventKind, RunMode, RunOptions, RunStatus};
use lr_types::scout::WorkMode;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const PEDIDO: &str = "\
Crie neste projeto uma landing page de captura de leads com CRUD em SQLite:

1. `index.html` — página com um formulário (nome, e-mail, empresa) e uma lista \
dos leads já cadastrados, com botões de editar e excluir. Sem framework: HTML, \
CSS e JavaScript puro, conversando com a API por fetch.
2. `app.py` — servidor em Python usando SÓ a biblioteca padrão \
(`http.server` e `sqlite3`), servindo o `index.html` e uma API JSON em `/leads` \
com as quatro operações: criar, listar, atualizar e excluir. O banco é um \
arquivo `leads.db` criado na primeira execução.
3. `README.md` — como rodar, em três linhas.

Quando terminar, rode `python3 -m py_compile app.py` para conferir que o \
servidor não tem erro de sintaxe.";

/// Tempo de sobra: um 9B gerando alguns milhares de tokens leva minutos.
const LIMITE: Duration = Duration::from_secs(900);

/// Começo do run, para a narração dizer QUANDO cada coisa aconteceu — sem
/// isso não dá para ver onde o tempo foi embora.
fn relogio() -> f64 {
    static INICIO: OnceLock<Instant> = OnceLock::new();
    INICIO.get_or_init(Instant::now).elapsed().as_secs_f64()
}

#[tokio::test]
#[ignore = "precisa de um llama-server real (OW_LIVE_URL/OW_LIVE_MODEL)"]
async fn the_agent_builds_a_small_crud_app_with_a_real_model() {
    let Ok(base_url) = std::env::var("OW_LIVE_URL") else {
        panic!("defina OW_LIVE_URL");
    };
    let model = std::env::var("OW_LIVE_MODEL").expect("defina OW_LIVE_MODEL");

    let dir = tempfile::tempdir().expect("workspace");
    let data = tempfile::tempdir().expect("dados");
    let workspace = dir.path().to_path_buf();
    let store = Arc::new(Store::open_in_memory().expect("store"));
    let host = AgentHost::new(
        store.clone(),
        Arc::new(lr_tools::builtin_registry()),
        AgentConfig::new(data.path().to_path_buf()),
    );

    relogio();
    let eventos: Arc<Mutex<Vec<RunEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = eventos.clone();
    // Quem clica. Mesmo no modo automático o harness pede confirmação para
    // comando que não consegue analisar (heredoc, `eval`) — é de propósito, e
    // na tela é a pessoa que responde. Aqui é o teste.
    let pendentes: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let fila = pendentes.clone();
    let handle = host
        .start(
            StartRun {
                prompt: PEDIDO.into(),
                history: Vec::new(),
                memory: Vec::new(),
                options: RunOptions {
                    chat_id: 0,
                    model: model.clone(),
                    // Sem confirmação: o teste não tem quem clique.
                    mode: RunMode::Yolo,
                    workspace_dir: Some(workspace.to_string_lossy().into_owned()),
                    max_steps: 30,
                    mcp_servers: Vec::new(),
                    temperature: None,
                    top_p: None,
                    top_k: None,
                    max_tokens: None,
                    system_prompt: None,
                },
                endpoint: Endpoint {
                    base_url,
                    api_key: None,
                },
                work_mode: WorkMode::Agent,
                plan: None,
            },
            Some(Arc::new(move |ev: RunEvent| {
                narra(&ev);
                if let RunEventKind::ToolRequested {
                    call_id,
                    requires_approval: true,
                    ..
                } = &ev.event
                {
                    fila.lock().unwrap().push(call_id.clone());
                }
                sink.lock().unwrap().push(ev);
            })),
        )
        .expect("run começou");

    let prazo = Instant::now() + LIMITE;
    loop {
        let terminou = eventos
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e.event, RunEventKind::RunFinished { .. }));
        if terminou {
            break;
        }
        for call_id in pendentes.lock().unwrap().drain(..) {
            if handle.resolve(&call_id, ApprovalDecision::AllowOnce) {
                println!("[{:6.1}s]   ✓ confirmado", relogio());
            }
        }
        assert!(Instant::now() < prazo, "o run não terminou em {LIMITE:?}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let vistos = eventos.lock().unwrap().clone();

    // 1. O modelo recebeu ferramentas. É a regressão que motivou o teste: no
    //    modo Router, o `/props` respondia pelo roteador e o run subia com
    //    `tools: []` — agente no nome, chat na prática.
    let ferramentas = vistos.iter().find_map(|e| match &e.event {
        RunEventKind::RunStarted { tools, .. } => Some(tools.clone()),
        _ => None,
    });
    let ferramentas = ferramentas.expect("run.started");
    assert!(
        !ferramentas.is_empty(),
        "o agente subiu sem ferramenta nenhuma"
    );
    assert!(
        !vistos
            .iter()
            .any(|e| matches!(e.event, RunEventKind::ToolsOff { .. })),
        "as ferramentas foram desligadas no meio do caminho"
    );

    // 2. Ele USOU as ferramentas — oferecer não basta.
    let escritas = vistos
        .iter()
        .filter(|e| matches!(&e.event, RunEventKind::ToolResult { ok: true, .. }))
        .count();
    assert!(escritas > 0, "nenhuma ferramenta rodou com sucesso");

    // 3. E o trabalho existe em disco.
    for arquivo in ["index.html", "app.py", "README.md"] {
        let caminho = workspace.join(arquivo);
        assert!(
            caminho.exists(),
            "faltou {arquivo}; ficaram: {:?}",
            listar(&workspace)
        );
        let conteudo = std::fs::read_to_string(&caminho).expect("ler");
        assert!(conteudo.len() > 120, "{arquivo} saiu vazio demais");
    }

    // 4. O CRUD é o pedido: a página precisa falar com a API, e o servidor
    //    precisa saber responder às quatro operações.
    let servidor = std::fs::read_to_string(workspace.join("app.py")).expect("app.py");
    assert!(servidor.contains("sqlite3"), "o servidor não usa SQLite");
    for verbo in ["GET", "POST"] {
        assert!(
            servidor.contains(verbo),
            "o servidor não trata {verbo}:\n{servidor}"
        );
    }

    let status = vistos.iter().find_map(|e| match &e.event {
        RunEventKind::RunFinished { status, .. } => Some(*status),
        _ => None,
    });
    println!(
        "\n--- terminou como {status:?}, arquivos: {:?}",
        listar(&workspace)
    );
    assert_eq!(status, Some(RunStatus::Done), "o run não chegou ao fim");
}

fn listar(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|it| {
            it.filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// Narra o run no terminal — é o que torna a falha legível quando ela vem do
/// modelo, e não do código.
fn narra(ev: &RunEvent) {
    let t = relogio();
    match &ev.event {
        RunEventKind::RunStarted { tools, model, .. } => {
            println!("[{t:6.1}s] · run com {} ferramentas ({model})", tools.len());
            println!("          cardápio: {}", tools.join(", "));
        }
        RunEventKind::ToolsOff { reason } => println!("[{t:6.1}s] · SEM FERRAMENTAS: {reason:?}"),
        RunEventKind::StepStarted { index, .. } => println!("\n[{t:6.1}s] — passo {index}"),
        RunEventKind::ToolRequested {
            tool, args_json, ..
        } => {
            println!("[{t:6.1}s]   → {tool} {}", recorte(args_json, 400));
        }
        RunEventKind::ToolStarted { .. } => println!("[{t:6.1}s]   ▶ rodando"),
        RunEventKind::CheckpointCreated { backend, .. } => {
            println!("[{t:6.1}s]   ⎘ checkpoint ({backend})")
        }
        RunEventKind::ToolResult {
            ok,
            result_preview,
            duration_ms,
            ..
        } => println!(
            "[{t:6.1}s]   ← {} em {:.1}s {}",
            if *ok { "ok" } else { "ERRO" },
            *duration_ms as f64 / 1000.0,
            recorte(result_preview, 200)
        ),
        RunEventKind::AssistantMessage { content, .. } if !content.trim().is_empty() => {
            println!("[{t:6.1}s]   “{}”", recorte(content, 300));
        }
        RunEventKind::RunError { message, .. } => println!("[{t:6.1}s]   !! {message}"),
        RunEventKind::RunFinished { status, usage, .. } => {
            println!(
                "\n[{t:6.1}s] · fim: {status:?} — {} passos, {} chamadas, {:.0}s",
                usage.steps,
                usage.tool_calls,
                usage.duration_ms as f64 / 1000.0
            );
        }
        _ => {}
    }
}

fn recorte(s: &str, n: usize) -> String {
    let limpo = s.replace('\n', " ");
    match limpo.char_indices().nth(n) {
        Some((i, _)) => format!("{}…", &limpo[..i]),
        None => limpo,
    }
}
