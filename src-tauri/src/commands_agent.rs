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

/// Teto de passos que comporta um plano já existente: n etapas × passos por
/// etapa + folga de retentativas. A granularidade já foi decidida (e, no
/// planejamento, revisada pela pessoa) — o orçamento acompanha.
fn teto_para(plan: Option<&TaskPlan>) -> u32 {
    match plan {
        Some(p) if !p.tasks.is_empty() => {
            (p.tasks.len() as u32 * lr_agent::MAX_STEPS_PER_TASK + 4).clamp(24, 100)
        }
        _ => 24,
    }
}

/// Começa uma execução do agente na conversa indicada.
#[tauri::command]
pub async fn run_start(
    state: State<'_, AppState>,
    prompt: String,
    opts: StartOptions,
    on_event: Channel<RunEvent>,
) -> CmdResult<String> {
    let StartOptions {
        mut options,
        work_mode,
    } = opts;
    // A referência traz o provedor (`openrouter:x/y`); o endpoint sai dela e
    // o nome que viaja na requisição é o do modelo, sem o prefixo — quem o
    // recebesse não saberia o que fazer com ele.
    let endpoint = state.endpoint_for(&options.model).await?;
    options.model = lr_providers::ModelRef::parse(&options.model).model;

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

/// Carimba um run que a interface acha vivo mas o processo já não conhece:
/// morreu sem rastro (pânico, task perdida). Grava a causa e um
/// `run.finished` sintético na trilha. `false` = não havia o que carimbar
/// (o run está vivo de verdade, ou já tem desfecho no banco).
#[tauri::command]
pub fn run_reap(state: State<'_, AppState>, run_id: String) -> CmdResult<bool> {
    if state.agent.get(&run_id).is_some() {
        // Vivo de verdade: a interface deve reatar, não carimbar.
        return Ok(false);
    }
    state
        .store
        .reap_run(
            &run_id,
            "A execução morreu sem chegar ao fim: o aplicativo perdeu o processo \
             dela. Detalhes podem estar no log.",
        )
        .map_err(err_str)
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
    // Aprovar É a decisão: a pergunta que ficou aberta deixa de segurar o
    // trabalho. Sem isto, o run novo pausaria de novo na mesma pergunta —
    // e aprovar viraria um beco sem saída.
    let pergunta = plan.pending_question.take();
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

    let endpoint = state.endpoint_for(&run.model).await?;
    let modelo = lr_providers::ModelRef::parse(&run.model).model;
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
                    model: modelo.clone(),
                    mode: run.mode,
                    workspace_dir: run.workspace_dir.clone(),
                    max_steps: teto_para(Some(&plan)),
                    mcp_servers: Vec::new(),
                    temperature: None,
                    top_p: None,
                    top_k: None,
                    max_tokens: None,
                    system_prompt: None,
                    // Runs que não vêm da tela (retomada de plano,
                    // automação) herdam o Code Mode da preferência.
                    code_mode: RunOptions::code_mode_from_setting(
                        state
                            .store
                            .get_setting(RunOptions::CODE_MODE_SETTING)
                            .ok()
                            .flatten()
                            .as_deref(),
                    ),
                },
                endpoint,
                work_mode: WorkMode::Loop,
                plan: Some(plan),
            },
            Some(channel_sink(on_event)),
        )
        .map_err(err_str)?;
    // Havia pergunta aberta e a pessoa aprovou assim mesmo: isso É a
    // resposta. Registrar tira o run da fila "aguardando resposta" — senão
    // ele ficaria lá para sempre, pedindo algo que já foi decidido.
    if pergunta.is_some()
        && let Err(e) =
            state
                .store
                .set_run_answer(&run_id, "(plano aprovado sem responder)", &handle.id)
    {
        log::warn!("run {run_id}: não gravou a aprovação como resposta: {e}");
    }
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
    // Uma resposta por pergunta. Sem esta guarda, duas janelas (ou a janela
    // entre responder e a conversa saber disso) abriam DOIS runs retomando o
    // mesmo plano no mesmo projeto, ao mesmo tempo.
    if run.answer.is_some() || run.resumed_by.is_some() {
        return Err("esta pergunta já foi respondida".into());
    }
    if run.status != lr_types::agent::RunStatus::Escalated {
        return Err("esta execução não está esperando resposta".into());
    }

    let plano = state
        .store
        .get_run_plan(&run_id)
        .map_err(err_str)?
        .filter(|raw| !raw.trim().is_empty())
        .and_then(|raw| serde_json::from_str::<TaskPlan>(&raw).ok());

    let endpoint = state.endpoint_for(&run.model).await?;
    let modelo = lr_providers::ModelRef::parse(&run.model).model;
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
            // A resposta LIMPA a pausa: sem isso o run retomado renasceria
            // já pausado na mesma pergunta.
            let pausa_pre_trabalho = plan
                .pending_question
                .take()
                .is_some_and(|q| q.task_index.is_none());
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
            // Pergunta levantada ANTES de trabalhar (pela decomposição): a
            // resposta pode MUDAR o plano — joga as etapas fora para o run
            // novo re-decompor com a resposta à vista. Só quando nada
            // começou: com trabalho feito, o plano vale e segue.
            if pausa_pre_trabalho && plan.tasks.iter().all(|t| t.status == TaskStatus::Pending) {
                plan.tasks.clear();
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
                // O run retomado continua no MODO em que a pessoa
                // trabalhava — responder uma pergunta do planejamento não
                // pode virar execução sem aprovação.
                match run.work_mode {
                    WorkMode::Plan => WorkMode::Plan,
                    _ => WorkMode::Loop,
                },
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
                    model: modelo.clone(),
                    mode: run.mode,
                    workspace_dir: run.workspace_dir.clone(),
                    max_steps: teto_para(plan.as_ref()),
                    mcp_servers: Vec::new(),
                    temperature: None,
                    top_p: None,
                    top_k: None,
                    max_tokens: None,
                    system_prompt: None,
                    // Runs que não vêm da tela (retomada de plano,
                    // automação) herdam o Code Mode da preferência.
                    code_mode: RunOptions::code_mode_from_setting(
                        state
                            .store
                            .get_setting(RunOptions::CODE_MODE_SETTING)
                            .ok()
                            .flatten()
                            .as_deref(),
                    ),
                },
                endpoint,
                work_mode,
                plan,
            },
            Some(channel_sink(on_event)),
        )
        .map_err(err_str)?;
    // A pendência foi respondida: grava a resposta e o run que retomou —
    // é o que tira este run da fila de "aguardando resposta" da Atividade.
    if let Err(e) = state.store.set_run_answer(&run_id, &answer, &handle.id) {
        log::warn!("run {run_id}: não gravou a resposta: {e}");
    }
    Ok(handle.id.clone())
}

/// Runs pausados numa pergunta e ainda sem resposta — a fila que a tela de
/// Atividade mostra em "Aguardando resposta" (inclui automações, que não
/// têm conversa para responder de dentro).
#[tauri::command]
pub fn runs_waiting_answer(state: State<'_, AppState>) -> CmdResult<Vec<RunSummary>> {
    state.store.runs_waiting_answer().map_err(err_str)
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
