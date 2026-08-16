//! O relógio das automações.
//!
//! Um laço acorda de meio em meio minuto, pergunta ao banco quais tarefas
//! venceram e dispara cada uma. É de propósito burro: quem sabe que horas são
//! na casa da pessoa é a interface, que grava a hora da próxima vez em epoch;
//! aqui só se compara número com número.
//!
//! Duas escolhas que valem explicação:
//!
//! - **A automação sobe o motor.** Uma tarefa que só roda com o servidor já
//!   ligado não automatiza nada — quem agenda não está por perto para apertar
//!   o play.
//! - **A marcação de "começou" vem antes do trabalho.** Subir o servidor leva
//!   dezenas de segundos, e o laço acorda a cada trinta: sem marcar primeiro,
//!   a mesma tarefa dispararia de novo enquanto a primeira ainda está subindo.

use crate::state::AppState;
use lr_types::agent::{RunEvent, RunEventKind, RunOptions, RunStatus};
use lr_types::automation::ScheduledTask;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// De quanto em quanto tempo o relógio confere o banco.
const TICK: Duration = Duration::from_secs(30);

/// Espera pelo servidor recém-subido antes de desistir.
const SERVER_WAIT: Duration = Duration::from_secs(90);

/// Teto de passos de uma execução automática. Ninguém está olhando: um teto
/// mais curto que o do chat é o que impede uma tarefa perdida de moer a
/// máquina a noite toda.
const MAX_STEPS: u32 = 12;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn spawn_loop(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let vencidas = {
                let state = app.state::<AppState>();
                state
                    .store
                    .due_scheduled_tasks(now_ms())
                    .unwrap_or_default()
            };
            for task in vencidas {
                let nome = task.name.clone();
                if let Err(e) = start(&app, task).await {
                    log::warn!("automação «{nome}» não rodou: {e}");
                }
            }
        }
    });
}

/// Dispara uma automação pelo id, agora. É o que o botão "rodar agora" usa.
pub async fn run_now(app: &AppHandle, task_id: &str) -> Result<String, String> {
    let task = {
        let state = app.state::<AppState>();
        state
            .store
            .scheduled_task(task_id)
            .map_err(|e| e.to_string())?
            .ok_or("automação não encontrada")?
    };
    start(app, task).await
}

/// Roda a automação e devolve o id da execução.
///
/// O resultado da execução chega depois, pelos eventos: é o ouvinte no fim
/// desta função que grava como terminou.
async fn start(app: &AppHandle, task: ScheduledTask) -> Result<String, String> {
    let store = {
        let state = app.state::<AppState>();
        state.store.clone()
    };
    let comeco = now_ms();
    let _ = store.scheduled_task_started(&task.id, None, comeco);

    let resultado = launch(app, &task).await;
    match &resultado {
        Ok(run_id) => {
            let _ = store.set_scheduled_task_run(&task.id, run_id);
        }
        Err(motivo) => {
            let _ = store.scheduled_task_finished(&task.id, Some(RunStatus::Error), motivo);
            notify(app, &task, None);
        }
    }
    resultado
}

async fn launch(app: &AppHandle, task: &ScheduledTask) -> Result<String, String> {
    ensure_server(app).await?;

    let state = app.state::<AppState>();
    let endpoint = state.agent_endpoint().await?;
    let model = match task.model.as_deref().map(str::trim) {
        Some(m) if !m.is_empty() => m.to_string(),
        // Sem modelo escolhido, o primeiro da biblioteca: é o mesmo nome que
        // o roteador do llama-server conhece.
        _ => crate::commands::local_models(state.clone())
            .first()
            .map(|m| m.artifact.name.clone())
            .ok_or("nenhum modelo instalado")?,
    };

    let memory = state
        .store
        .list_memory_facts(task.workspace_dir.as_deref())
        .map(|facts| facts.into_iter().map(|f| f.content).collect::<Vec<_>>())
        .unwrap_or_default();

    // Sem conversa: a automação não pertence a nenhum chat, e o que ela fez
    // se lê na trilha da execução.
    let options = RunOptions {
        chat_id: 0,
        model,
        mode: task.mode,
        workspace_dir: task.workspace_dir.clone(),
        max_steps: MAX_STEPS,
        mcp_servers: Vec::new(),
        temperature: None,
        top_p: None,
        top_k: None,
        max_tokens: None,
        system_prompt: None,
    };

    let store = state.store.clone();
    let id_tarefa = task.id.clone();
    let app_evento = app.clone();
    let tarefa_evento = task.clone();
    let ouvinte: lr_agent::EventCallback = Arc::new(move |ev: RunEvent| {
        // Só o fim interessa: o resto da trilha já está em `run_events`, e a
        // tela da automação abre a execução por ali.
        let (status, resumo) = match &ev.event {
            RunEventKind::RunFinished {
                status, summary, ..
            } => (*status, summary.clone()),
            RunEventKind::RunPaused { .. } => (RunStatus::WaitingApproval, String::new()),
            _ => return,
        };
        let _ = store.scheduled_task_finished(&id_tarefa, Some(status), &resumo);
        if status != RunStatus::WaitingApproval {
            notify(&app_evento, &tarefa_evento, Some(status));
        }
    });

    let handle = state
        .agent
        .start(
            lr_agent::StartRun {
                prompt: task.prompt.clone(),
                history: Vec::new(),
                memory,
                options,
                endpoint,
                work_mode: task.work_mode,
                plan: None,
            },
            Some(ouvinte),
        )
        .map_err(|e| e.to_string())?;
    Ok(handle.id.clone())
}

/// Garante o motor de IA no ar. Quem agenda não está por perto para ligar.
async fn ensure_server(app: &AppHandle) -> Result<(), String> {
    {
        let state = app.state::<AppState>();
        if state.agent_endpoint().await.is_ok() {
            return Ok(());
        }
    }
    crate::commands::server_start(app.clone(), app.state::<AppState>())
        .await
        .map_err(|e| format!("não consegui ligar o motor de IA: {e}"))?;

    // `server_start` volta assim que o processo sobe; o modelo ainda está
    // carregando. Esperar aqui é melhor do que a execução falhar no primeiro
    // passo por "conexão recusada".
    let limite = std::time::Instant::now() + SERVER_WAIT;
    loop {
        {
            let state = app.state::<AppState>();
            if state.agent_endpoint().await.is_ok() {
                return Ok(());
            }
        }
        if std::time::Instant::now() >= limite {
            return Err("o motor de IA não ficou pronto a tempo".into());
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Avisa a interface que uma automação terminou (a tela atualiza a lista).
fn notify(app: &AppHandle, task: &ScheduledTask, status: Option<RunStatus>) {
    let _ = app.emit(
        "automation",
        serde_json::json!({
            "id": task.id,
            "name": task.name,
            "status": status,
        }),
    );
}
