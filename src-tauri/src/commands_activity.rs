//! Comandos da tela de atividade: o histórico do que o agente fez.
//!
//! Tudo aqui é leitura. As execuções já ficam gravadas desde o primeiro dia
//! do harness — o que faltava era a pergunta atravessando todas elas.

use crate::state::AppState;
use lr_store::activity::{ActivityCall, ActivityStats};
use lr_types::agent::RunSummary;
use tauri::State;

type CmdResult<T> = Result<T, String>;

fn err_str<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Execuções de todas as conversas e automações, da mais recente para a mais
/// antiga.
#[tauri::command]
pub fn activity_runs(state: State<'_, AppState>) -> CmdResult<Vec<RunSummary>> {
    state.store.list_runs(None).map_err(err_str)
}

/// Últimas ações do agente, com quem autorizou cada uma.
#[tauri::command]
pub fn activity_calls(
    state: State<'_, AppState>,
    limit: Option<u32>,
) -> CmdResult<Vec<ActivityCall>> {
    state
        .store
        .list_recent_calls(limit.unwrap_or(200))
        .map_err(err_str)
}

/// As ações de UMA execução, na ordem em que aconteceram.
#[tauri::command]
pub fn activity_run_calls(
    state: State<'_, AppState>,
    run_id: String,
) -> CmdResult<Vec<ActivityCall>> {
    state
        .store
        .list_run_calls_with_decision(&run_id)
        .map_err(err_str)
}

/// Contas do período. `since` é epoch em SEGUNDOS (a unidade de
/// `runs.created_at`); ausente = desde sempre.
#[tauri::command]
pub fn activity_stats(state: State<'_, AppState>, since: Option<i64>) -> CmdResult<ActivityStats> {
    state
        .store
        .activity_stats(since.unwrap_or(0))
        .map_err(err_str)
}
