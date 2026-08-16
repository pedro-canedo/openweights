//! Comandos da memória de longo prazo: listar, acrescentar, esquecer,
//! arrumar em ocioso e abrir a pasta legível.
//!
//! A interface trata memória como uma lista curta que a pessoa controla —
//! por isso todos os comandos são síncronos do ponto de vista dela, menos
//! `memory_consolidate_now`, que fala com o modelo.
//!
//! Duas decisões moram aqui, e não no crate:
//!
//! 1. **Arrumar não atropela trabalho.** A consolidação usa o mesmo
//!    llama-server do agente; disparada no meio de um run, roubaria contexto
//!    e faria a pessoa esperar. O crate expõe a parte pura de propósito e
//!    deixa a decisão para o chamador — a checagem de run ativo é este
//!    arquivo.
//! 2. **Qual modelo usar.** A memória não tem modelo próprio: reaproveita o
//!    último que o agente (ou a conversa) usou. Se não houver nenhum, o erro
//!    diz o que fazer em vez de falhar em silêncio.

use crate::state::AppState;
use lr_memory::{ConsolidateReport, MemoryStore, SavedFact};
use lr_store::memory::MemoryFact;
use lr_types::agent::RunStatus;
use std::path::PathBuf;
use tauri::State;

type CmdResult<T> = Result<T, String>;

fn err_str<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Pasta vazia vinda da interface = nenhum projeto aberto (só memória global).
fn workspace_path(workspace_dir: Option<String>) -> Option<PathBuf> {
    workspace_dir
        .filter(|d| !d.trim().is_empty())
        .map(PathBuf::from)
}

fn memory_for(state: &AppState, workspace_dir: Option<String>) -> MemoryStore {
    MemoryStore::new(state.store.clone(), workspace_path(workspace_dir))
}

/// Fatos que valem no projeto informado (os globais vêm junto).
#[tauri::command]
pub async fn memory_facts_list(
    state: State<'_, AppState>,
    workspace_dir: Option<String>,
) -> CmdResult<Vec<MemoryFact>> {
    memory_for(&state, workspace_dir).facts().map_err(err_str)
}

/// Acrescenta um fato escrito pela pessoa. Passa pela mesma curadoria dos
/// fatos que o modelo salva: recusa vazio, corta o longo demais e avisa
/// quando já sabíamos daquilo.
#[tauri::command]
pub async fn memory_fact_add(
    state: State<'_, AppState>,
    workspace_dir: Option<String>,
    content: String,
) -> CmdResult<SavedFact> {
    memory_for(&state, workspace_dir)
        .save(&content, None, None)
        .map_err(err_str)
}

/// Esquece um fato.
///
/// `workspace_dir` é opcional e serve para limpar também o arquivo legível
/// do projeto — sem ele o fato só sai do banco.
#[tauri::command]
pub async fn memory_fact_delete(
    state: State<'_, AppState>,
    id: i64,
    workspace_dir: Option<String>,
) -> CmdResult<()> {
    memory_for(&state, workspace_dir)
        .forget(id)
        .map_err(err_str)
}

/// Um run em andamento impede a arrumação (ver decisão 1 no topo).
fn has_active_run(state: &AppState) -> bool {
    state
        .store
        .list_runs(None)
        .map(|runs| {
            runs.iter()
                .any(|r| matches!(r.status, RunStatus::Running | RunStatus::WaitingApproval))
        })
        .unwrap_or(false)
}

/// Modelo que a memória usa: o do último run, senão o da última conversa,
/// senão o primeiro da biblioteca local.
fn resolve_model(state: &AppState, requested: Option<String>) -> CmdResult<String> {
    if let Some(model) = requested.filter(|m| !m.trim().is_empty()) {
        return Ok(model);
    }
    if let Ok(runs) = state.store.list_runs(None)
        && let Some(run) = runs.into_iter().find(|r| !r.model.is_empty())
    {
        return Ok(run.model);
    }
    if let Ok(chats) = state.store.list_chats()
        && let Some(model) = chats.into_iter().find_map(|c| c.model_id)
    {
        return Ok(model);
    }
    lr_models::scan_local(&state.models_dir)
        .into_iter()
        .next()
        .map(|a| a.name)
        .ok_or_else(|| "nenhum modelo disponível para organizar a memória".to_string())
}

/// Arrumação sob demanda: lê os episódios pendentes, pede ao modelo os fatos
/// duráveis e grava o que passar pela curadoria.
#[tauri::command]
pub async fn memory_consolidate_now(
    state: State<'_, AppState>,
    workspace_dir: Option<String>,
    model: Option<String>,
) -> CmdResult<ConsolidateReport> {
    if has_active_run(&state) {
        return Err("o agente está trabalhando — organize a memória depois".into());
    }
    let model = resolve_model(&state, model)?;
    let endpoint = state.agent_endpoint().await?;
    let client =
        lr_engine::LlamaClient::new(endpoint.base_url).with_optional_api_key(endpoint.api_key);

    memory_for(&state, workspace_dir)
        .consolidate_now(&client, &model)
        .await
        .map_err(err_str)
}

/// Abre `.openweights/memory/` no explorador de arquivos.
///
/// A pasta é criada na hora se ainda não existir: aqui o clique é o pedido
/// explícito da pessoa, o oposto do "não sujar o projeto sem motivo" que vale
/// para o resto do crate.
#[tauri::command]
pub async fn memory_open_folder(workspace_dir: String) -> CmdResult<()> {
    let root = PathBuf::from(&workspace_dir);
    if !root.is_dir() {
        return Err("escolha a pasta do projeto primeiro".into());
    }
    let folder = lr_memory::memory_dir(&root);
    std::fs::create_dir_all(&folder).map_err(err_str)?;
    crate::workspace::reveal(&workspace_dir, Some(lr_memory::MEMORY_SUBDIR)).map_err(err_str)
}
