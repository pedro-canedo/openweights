//! Comandos do modo agente: iniciar/acompanhar execuções, responder pedidos
//! de confirmação, consultar a trilha e restaurar checkpoints.
//!
//! A interface só observa: o laço vive no `AgentHost` e sobrevive a fechar a
//! tela ou recarregar o webview. Os eventos chegam por um `Channel` do Tauri
//! e são renumerados por `seq`, o que permite reatar sem perder nada.

use crate::state::AppState;
use lr_store::agent::{CheckpointRow, RunEventRow, ToolPermissionRow};
use lr_types::agent::{
    ApprovalDecision, PolicyScope, RunEvent, RunOptions, RunSummary, ToolGroup, ToolPolicy,
};
use lr_types::scout::{TaskPlan, TaskStatus, WorkMode};
use std::sync::Arc;
use tauri::State;
use tauri::ipc::Channel;

type CmdResult<T> = Result<T, String>;

fn err_str<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Adapta o `Channel` do Tauri ao callback de eventos do agente.
fn channel_sink(channel: Channel<RunEvent>) -> lr_agent::EventCallback {
    Arc::new(move |ev: RunEvent| {
        if let Err(e) = channel.send(ev) {
            // Janela fechada no meio do run: o laço continua, só não há para
            // quem entregar. O replay por `seq` recupera quando ela voltar.
            log::debug!("canal de eventos do agente indisponível: {e}");
        }
    })
}

/// Opções de `run_start`.
///
/// `RunOptions` (compartilhado com a interface) traz o nível de autorização;
/// o modo de trabalho (conversa/planejamento/agente/laço) vem junto no mesmo
/// objeto e é lido aqui. Ausente = agente, o comportamento de sempre.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartOptions {
    #[serde(flatten)]
    pub options: RunOptions,
    #[serde(default)]
    pub work_mode: WorkMode,
}

/// Começa uma execução do agente na conversa indicada.
#[tauri::command]
pub async fn run_start(
    state: State<'_, AppState>,
    prompt: String,
    opts: StartOptions,
    on_event: Channel<RunEvent>,
) -> CmdResult<String> {
    let StartOptions { options, work_mode } = opts;
    let endpoint = state.agent_endpoint().await?;

    // Histórico da conversa (o laço trabalha por cima dele).
    let mut history = Vec::new();
    if options.chat_id > 0 {
        let rows = state
            .store
            .list_messages(options.chat_id)
            .map_err(err_str)?;
        history = lr_agent::history_from_messages(&rows, 40);
    }

    // Fatos memorizados entram no prompt do sistema (memória semântica).
    let memory = state
        .store
        .list_memory_facts(options.workspace_dir.as_deref())
        .map(|facts| facts.into_iter().map(|f| f.content).collect::<Vec<_>>())
        .unwrap_or_default();

    let handle = state
        .agent
        .start(
            lr_agent::StartRun {
                prompt,
                history,
                memory,
                options,
                endpoint,
                work_mode,
                // Sem plano pronto: quem planeja é o próprio run.
                plan: None,
            },
            Some(channel_sink(on_event)),
        )
        .map_err(err_str)?;
    Ok(handle.id.clone())
}

/// Reata a interface a um run em andamento, repetindo o que ela perdeu.
#[tauri::command]
pub async fn run_attach(
    state: State<'_, AppState>,
    run_id: String,
    after_seq: u64,
    on_event: Channel<RunEvent>,
) -> CmdResult<bool> {
    match state.agent.get(&run_id) {
        Some(handle) => {
            handle.sink().attach(after_seq, channel_sink(on_event));
            Ok(true)
        }
        // Run já terminado: quem reconstrói é `run_events_list`.
        None => Ok(false),
    }
}

/// Responde a um pedido de confirmação de ferramenta.
#[tauri::command]
pub async fn run_approve(
    state: State<'_, AppState>,
    run_id: String,
    call_id: String,
    decision: ApprovalDecision,
) -> CmdResult<()> {
    // "Permitir/negar sempre" vira uma regra guardada, além de liberar a
    // chamada atual.
    let (scope, policy) = match &decision {
        ApprovalDecision::AllowAlways { scope } => (Some(*scope), Some(ToolPolicy::AlwaysAllow)),
        ApprovalDecision::DenyAlways { scope } => (Some(*scope), Some(ToolPolicy::Never)),
        _ => (None, None),
    };
    if let (Some(scope), Some(policy)) = (scope, policy)
        && let Some(call) = state
            .store
            .list_tool_calls(&run_id)
            .ok()
            .and_then(|calls| calls.into_iter().find(|c| c.id == call_id))
    {
        let workspace = state
            .store
            .get_run(&run_id)
            .ok()
            .flatten()
            .and_then(|r| r.workspace_dir);
        let ws = match scope {
            PolicyScope::Workspace => workspace.as_deref(),
            PolicyScope::Global => None,
        };
        state
            .store
            .set_tool_permission(scope, ws, &call.tool_name, policy)
            .map_err(err_str)?;
    }

    if state.agent.resolve(&run_id, &call_id, decision) {
        Ok(())
    } else {
        Err("esta confirmação já não está pendente".into())
    }
}

#[tauri::command]
pub async fn run_cancel(state: State<'_, AppState>, run_id: String) -> CmdResult<()> {
    state.agent.cancel(&run_id);
    Ok(())
}

#[tauri::command]
pub fn runs_list(state: State<'_, AppState>, chat_id: Option<i64>) -> CmdResult<Vec<RunSummary>> {
    state.store.list_runs(chat_id).map_err(err_str)
}

#[tauri::command]
pub fn run_events_list(
    state: State<'_, AppState>,
    run_id: String,
    after_seq: u64,
) -> CmdResult<Vec<RunEventRow>> {
    state
        .store
        .list_run_events(&run_id, after_seq)
        .map_err(err_str)
}

/// Plano de trabalho (Scout Rule) de uma execução.
///
/// A trilha entrega o plano em markdown para leitura rápida; aqui vai a
/// estrutura completa, que é o que o quadro de tarefas desenha.
#[tauri::command]
pub fn run_plan_get(
    state: State<'_, AppState>,
    run_id: String,
) -> CmdResult<Option<lr_types::scout::TaskPlan>> {
    let raw = state.store.get_run_plan(&run_id).map_err(err_str)?;
    Ok(raw.and_then(|json| match serde_json::from_str(&json) {
        Ok(plan) => Some(plan),
        Err(e) => {
            log::warn!("plano do run {run_id} ilegível: {e}");
            None
        }
    }))
}

/// Aprova o plano proposto e começa a executá-lo.
///
/// O modo planejamento entrega o plano e para. Aprovar abre uma execução
/// nova em modo laço com as MESMAS etapas — sem dividir o objetivo de novo,
/// para não perder o que a pessoa acabou de revisar.
#[tauri::command]
pub async fn run_plan_approve(
    state: State<'_, AppState>,
    run_id: String,
    on_event: Channel<RunEvent>,
) -> CmdResult<String> {
    let run = state
        .store
        .get_run(&run_id)
        .map_err(err_str)?
        .ok_or("execução não encontrada")?;
    let raw = state
        .store
        .get_run_plan(&run_id)
        .map_err(err_str)?
        .ok_or("esta execução não tem plano")?;
    let mut plan: TaskPlan = serde_json::from_str(&raw).map_err(err_str)?;

    plan.approved = true;
    // Etapas que ficaram pela metade voltam para a fila.
    for task in &mut plan.tasks {
        if task.status == TaskStatus::Running {
            task.status = TaskStatus::Pending;
        }
    }
    // Guarda a aprovação no plano antigo (fica no histórico) e leva a cópia
    // para o run novo.
    if let Ok(json) = serde_json::to_string(&plan) {
        let _ = state.store.set_run_plan(&run_id, &json);
    }

    let endpoint = state.agent_endpoint().await?;
    let chat_id = run.chat_id.unwrap_or(0);
    let history = if chat_id > 0 {
        let rows = state.store.list_messages(chat_id).map_err(err_str)?;
        lr_agent::history_from_messages(&rows, 40)
    } else {
        Vec::new()
    };
    let memory = state
        .store
        .list_memory_facts(run.workspace_dir.as_deref())
        .map(|f| f.into_iter().map(|x| x.content).collect())
        .unwrap_or_default();

    let handle = state
        .agent
        .start(
            lr_agent::StartRun {
                prompt: run.prompt.clone(),
                history,
                memory,
                options: RunOptions {
                    chat_id,
                    model: run.model.clone(),
                    mode: run.mode,
                    workspace_dir: run.workspace_dir.clone(),
                    // O teto do laço é recalculado no crate do agente a
                    // partir do tamanho do plano.
                    max_steps: 24,
                    mcp_servers: Vec::new(),
                    temperature: None,
                    top_p: None,
                    top_k: None,
                    max_tokens: None,
                    system_prompt: None,
                },
                endpoint,
                work_mode: WorkMode::Loop,
                plan: Some(plan),
            },
            Some(channel_sink(on_event)),
        )
        .map_err(err_str)?;
    Ok(handle.id.clone())
}

/// Retoma uma execução que parou esperando a pessoa (pergunta do agente,
/// etapa bloqueada, escalada).
///
/// A resposta NÃO começa do zero: quando há plano persistido, as etapas
/// bloqueadas/falhadas voltam para a fila e um run novo continua o MESMO
/// plano em modo laço, com a resposta anexada ao objetivo; sem plano, a
/// continuação vira um run de agente com o histórico da conversa (que já
/// carrega o que o agente disse antes de parar). O run retomado tem OUTRO
/// id de propósito: a trilha antiga fica íntegra no painel — o que
/// atravessa é o plano, com status e handoffs.
#[tauri::command]
pub async fn run_answer(
    state: State<'_, AppState>,
    run_id: String,
    answer: String,
    on_event: Channel<RunEvent>,
) -> CmdResult<String> {
    let answer = answer.trim().to_string();
    if answer.is_empty() {
        return Err("resposta vazia".into());
    }
    let run = state
        .store
        .get_run(&run_id)
        .map_err(err_str)?
        .ok_or("execução não encontrada")?;

    let plano = state
        .store
        .get_run_plan(&run_id)
        .map_err(err_str)?
        .filter(|raw| !raw.trim().is_empty())
        .and_then(|raw| serde_json::from_str::<TaskPlan>(&raw).ok());

    let endpoint = state.agent_endpoint().await?;
    let chat_id = run.chat_id.unwrap_or(0);
    let history = if chat_id > 0 {
        let rows = state.store.list_messages(chat_id).map_err(err_str)?;
        lr_agent::history_from_messages(&rows, 40)
    } else {
        Vec::new()
    };
    let memory = state
        .store
        .list_memory_facts(run.workspace_dir.as_deref())
        .map(|f| f.into_iter().map(|x| x.content).collect())
        .unwrap_or_default();

    let (prompt, work_mode, plan) = match plano {
        Some(mut plan) => {
            plan.approved = true;
            // A resposta destrava: bloqueada e falhada voltam para a fila
            // (com o erro limpo — a pessoa acabou de responder o que
            // faltava); uma etapa que ficou correndo volta como pendente.
            for task in &mut plan.tasks {
                match task.status {
                    TaskStatus::Blocked | TaskStatus::Failed => {
                        task.status = TaskStatus::Pending;
                        task.error = None;
                    }
                    TaskStatus::Running => task.status = TaskStatus::Pending,
                    _ => {}
                }
            }
            if let Ok(json) = serde_json::to_string(&plan) {
                let _ = state.store.set_run_plan(&run_id, &json);
            }
            (
                format!(
                    "{}

Resposta da pessoa: {answer}",
                    run.prompt
                ),
                WorkMode::Loop,
                Some(plan),
            )
        }
        None => (answer.clone(), WorkMode::Agent, None),
    };

    let handle = state
        .agent
        .start(
            lr_agent::StartRun {
                prompt,
                history,
                memory,
                options: RunOptions {
                    chat_id,
                    model: run.model.clone(),
                    mode: run.mode,
                    workspace_dir: run.workspace_dir.clone(),
                    max_steps: 24,
                    mcp_servers: Vec::new(),
                    temperature: None,
                    top_p: None,
                    top_k: None,
                    max_tokens: None,
                    system_prompt: None,
                },
                endpoint,
                work_mode,
                plan,
            },
            Some(channel_sink(on_event)),
        )
        .map_err(err_str)?;
    Ok(handle.id.clone())
}

/// Descarta o plano proposto para que a próxima execução divida de novo.
#[tauri::command]
pub fn run_plan_replan(state: State<'_, AppState>, run_id: String) -> CmdResult<()> {
    state.store.set_run_plan(&run_id, "").map_err(err_str)
}

// ------------------------------------------------------------ permissões ---

#[tauri::command]
pub fn tool_permissions_list(
    state: State<'_, AppState>,
    workspace_dir: Option<String>,
) -> CmdResult<Vec<ToolPermissionRow>> {
    state
        .store
        .list_tool_permissions(workspace_dir.as_deref())
        .map_err(err_str)
}

#[tauri::command]
pub fn tool_permission_set(
    state: State<'_, AppState>,
    scope: PolicyScope,
    workspace_dir: Option<String>,
    tool_name: String,
    policy: Option<ToolPolicy>,
) -> CmdResult<()> {
    match policy {
        // `None` volta ao padrão (remove o override).
        None => state
            .store
            .clear_tool_permission(scope, workspace_dir.as_deref(), &tool_name),
        Some(p) => state
            .store
            .set_tool_permission(scope, workspace_dir.as_deref(), &tool_name, p),
    }
    .map_err(err_str)
}

/// Catálogo de ferramentas disponíveis (para a tela de permissões).
#[tauri::command]
pub async fn tools_list(state: State<'_, AppState>) -> CmdResult<Vec<lr_types::agent::ToolSpec>> {
    Ok(state.tools().specs().await)
}

/// Quantas ferramentas cada família tem no catálogo de agora.
///
/// A tela precisa do número para a pessoa dimensionar o que está desligando.
/// Vem do catálogo vivo, e não de uma lista escrita à mão na interface, que
/// mentiria na primeira ferramenta nova. Conectores MCP entram no mesmo
/// caminho, sem contagem à parte.
#[tauri::command]
pub async fn tool_group_counts(
    state: State<'_, AppState>,
) -> CmdResult<std::collections::HashMap<String, u32>> {
    let mut counts: std::collections::HashMap<String, u32> = ToolGroup::ALL
        .iter()
        .map(|g| (g.key().to_string(), 0))
        .collect();
    for spec in state.tools().specs().await {
        *counts
            .entry(lr_agent::group_of(&spec).key().to_string())
            .or_insert(0) += 1;
    }
    Ok(counts)
}

/// Famílias de ferramentas habilitadas. Nunca configurado = todas.
#[tauri::command]
pub fn tool_groups_get(state: State<'_, AppState>) -> CmdResult<Vec<String>> {
    let raw = state
        .store
        .get_setting(ToolGroup::SETTING)
        .map_err(err_str)?;
    Ok(ToolGroup::enabled_from_setting(raw.as_deref())
        .iter()
        .map(|g| g.key().to_string())
        .collect())
}

/// Guarda as famílias habilitadas. Chaves desconhecidas são descartadas, e
/// desabilitar tudo não é possível: sem nenhuma família o agente ficaria sem
/// ferramenta nenhuma sem dizer o motivo — nesse caso volta ao padrão.
#[tauri::command]
pub fn tool_groups_set(state: State<'_, AppState>, groups: Vec<String>) -> CmdResult<Vec<String>> {
    let valid: Vec<String> = groups
        .iter()
        .filter_map(|k| ToolGroup::from_key(k))
        .map(|g| g.key().to_string())
        .collect();
    let json = serde_json::to_string(&valid).map_err(err_str)?;
    state
        .store
        .set_setting(ToolGroup::SETTING, &json)
        .map_err(err_str)?;
    Ok(valid)
}

// ----------------------------------------------------------- checkpoints ---

#[tauri::command]
pub fn checkpoints_list(
    state: State<'_, AppState>,
    workspace_dir: String,
) -> CmdResult<Vec<CheckpointRow>> {
    state
        .store
        .list_checkpoints(&workspace_dir)
        .map_err(err_str)
}

/// Devolve os arquivos do projeto ao estado de um checkpoint.
#[tauri::command]
pub async fn checkpoint_restore(
    state: State<'_, AppState>,
    checkpoint_id: String,
) -> CmdResult<Vec<String>> {
    let row = state
        .store
        .get_checkpoint(&checkpoint_id)
        .map_err(err_str)?
        .ok_or("checkpoint não encontrado")?;

    let store_dir = state.data_dir.clone();
    let workspace = std::path::PathBuf::from(&row.workspace_dir);

    // E/S de disco fora do runtime assíncrono. `checkpoint_from_row`
    // reconstrói a lista de arquivos a partir do manifesto — sem ela a
    // restauração por cópia não teria o que devolver.
    tokio::task::spawn_blocking(move || {
        let cp =
            lr_agent::checkpoint_from_row(&row.id, &row.backend, &row.ref_id, row.label.as_deref());
        lr_agent::restore_blocking(&workspace, &store_dir, &cp).map_err(err_str)
    })
    .await
    .map_err(err_str)?
}

/// Lembra qual modelo a conversa estava usando (o seletor troca no meio).
#[tauri::command]
pub fn chat_set_model(state: State<'_, AppState>, chat_id: i64, model_id: String) -> CmdResult<()> {
    state
        .store
        .set_chat_model(chat_id, &model_id)
        .map_err(err_str)
}

// ------------------------------------------------------- busca na web ---

/// JSON cru da configuração de internet (setting `web.config`), ou `None`
/// quando nunca foi gravada.
///
/// Devolve o texto como está no banco de propósito: quem interpreta é a tela
/// (com parse defensivo), e o formato é o mesmo `WebConfig` que o registry
/// lê na montagem — uma tradução aqui só criaria um terceiro dialeto.
#[tauri::command]
pub fn web_config_get(state: State<'_, AppState>) -> CmdResult<Option<String>> {
    state.store.get_setting("web.config").map_err(err_str)
}

/// Grava a configuração de internet e refaz o catálogo de ferramentas.
///
/// Valida ANTES de gravar: o registry ignora JSON estragado em silêncio
/// (cai nos padrões), então aceitar qualquer texto aqui gravaria uma
/// configuração que nunca teria efeito — e a pessoa acharia que salvou.
/// O `rebuild_tools` no fim é o que aplica sem reiniciar o app, o mesmo
/// mecanismo dos conectores MCP.
#[tauri::command]
pub fn web_config_set(state: State<'_, AppState>, json: String) -> CmdResult<()> {
    serde_json::from_str::<lr_webtools::WebConfig>(&json)
        .map_err(|e| format!("configuração de internet inválida: {e}"))?;
    state
        .store
        .set_setting("web.config", &json)
        .map_err(err_str)?;
    state.rebuild_tools()
}
