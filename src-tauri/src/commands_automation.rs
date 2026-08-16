//! Comandos das automações: listar, salvar, apagar e rodar agora.
//!
//! O relógio que dispara sozinho vive em [`crate::scheduler`]; aqui está só o
//! que a interface chama.

use crate::scheduler::{self, now_ms};
use crate::state::AppState;
use lr_types::automation::{ScheduledTask, ScheduledTaskInput};
use tauri::{AppHandle, State};

type CmdResult<T> = Result<T, String>;

fn err_str<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
pub fn automations_list(state: State<'_, AppState>) -> CmdResult<Vec<ScheduledTask>> {
    state.store.list_scheduled_tasks().map_err(err_str)
}

/// Cria ou edita. Devolve a automação já como ficou gravada.
#[tauri::command]
pub fn automation_save(
    state: State<'_, AppState>,
    task: ScheduledTaskInput,
) -> CmdResult<ScheduledTask> {
    if let Some(problema) = task.problem() {
        return Err(problema);
    }
    let id = state
        .store
        .save_scheduled_task(&task, &lr_agent::new_id("auto"), now_ms())
        .map_err(err_str)?;
    state
        .store
        .scheduled_task(&id)
        .map_err(err_str)?
        .ok_or_else(|| "a automação sumiu logo depois de salva".to_string())
}

#[tauri::command]
pub fn automation_delete(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.store.delete_scheduled_task(&id).map_err(err_str)
}

/// Roda agora, sem esperar o horário. Devolve o id da execução, que a
/// interface usa para abrir a trilha.
#[tauri::command]
pub async fn automation_run_now(app: AppHandle, id: String) -> CmdResult<String> {
    scheduler::run_now(&app, &id).await
}
