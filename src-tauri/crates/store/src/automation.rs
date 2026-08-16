//! Persistência das automações: tarefas que rodam sozinhas.
//!
//! Uma linha por automação, com o estado da última execução junto. Não há
//! histórico próprio: o que aconteceu está em `runs`/`run_events`, e a coluna
//! `last_run_id` é o ponteiro para lá.

use crate::{Store, StoreError};
use lr_types::agent::{RunMode, RunStatus};
use lr_types::automation::{ScheduledTask, ScheduledTaskInput};
use lr_types::scout::WorkMode;
use rusqlite::{Connection, Row, params};

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS scheduled_tasks (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    prompt TEXT NOT NULL,
    workspace_dir TEXT,
    model TEXT,
    mode TEXT NOT NULL DEFAULT 'approve',
    work_mode TEXT NOT NULL DEFAULT 'agent',
    every_minutes INTEGER NOT NULL DEFAULT 0,
    next_run_at INTEGER,
    enabled INTEGER NOT NULL DEFAULT 1,
    last_run_at INTEGER,
    last_run_id TEXT,
    last_status TEXT,
    last_summary TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_scheduled_next ON scheduled_tasks(enabled, next_run_at);
"#;

pub(crate) fn init(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(SCHEMA)?;
    Ok(())
}

const COLUNAS: &str = "id, name, prompt, workspace_dir, model, mode, work_mode, every_minutes, \
                       next_run_at, enabled, last_run_at, last_run_id, last_status, \
                       last_summary, created_at";

fn task_from(r: &Row<'_>) -> rusqlite::Result<ScheduledTask> {
    Ok(ScheduledTask {
        id: r.get(0)?,
        name: r.get(1)?,
        prompt: r.get(2)?,
        workspace_dir: r.get(3)?,
        model: r.get(4)?,
        mode: crate::agent::enum_from(&r.get::<_, String>(5)?).unwrap_or(RunMode::Approve),
        work_mode: crate::agent::enum_from(&r.get::<_, String>(6)?).unwrap_or(WorkMode::Agent),
        every_minutes: r.get::<_, i64>(7)?.max(0) as u32,
        next_run_at: r.get(8)?,
        enabled: r.get::<_, i64>(9)? != 0,
        last_run_at: r.get(10)?,
        last_run_id: r.get(11)?,
        last_status: r
            .get::<_, Option<String>>(12)?
            .and_then(|s| crate::agent::enum_from::<RunStatus>(&s)),
        last_summary: r.get(13)?,
        created_at: r.get(14)?,
    })
}

impl Store {
    pub fn list_scheduled_tasks(&self) -> Result<Vec<ScheduledTask>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUNAS} FROM scheduled_tasks ORDER BY name"
        ))?;
        let rows = stmt
            .query_map([], task_from)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn scheduled_task(&self, id: &str) -> Result<Option<ScheduledTask>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUNAS} FROM scheduled_tasks WHERE id = ?1"
        ))?;
        let mut rows = stmt.query([id])?;
        Ok(match rows.next()? {
            Some(r) => Some(task_from(r)?),
            None => None,
        })
    }

    /// Cria ou atualiza. Devolve o id (gerado quando é criação).
    ///
    /// O estado da última execução não vem no `input` e é preservado: editar
    /// o texto de uma automação não apaga o que ela já fez.
    pub fn save_scheduled_task(
        &self,
        input: &ScheduledTaskInput,
        new_id: &str,
        now_ms: i64,
    ) -> Result<String, StoreError> {
        let id = input.id.clone().unwrap_or_else(|| new_id.to_string());
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO scheduled_tasks
                (id, name, prompt, workspace_dir, model, mode, work_mode, every_minutes,
                 next_run_at, enabled, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                prompt = excluded.prompt,
                workspace_dir = excluded.workspace_dir,
                model = excluded.model,
                mode = excluded.mode,
                work_mode = excluded.work_mode,
                every_minutes = excluded.every_minutes,
                next_run_at = excluded.next_run_at,
                enabled = excluded.enabled",
            params![
                id,
                input.name.trim(),
                input.prompt.trim(),
                input.workspace_dir,
                input.model,
                crate::agent::enum_str(&input.mode),
                crate::agent::enum_str(&input.work_mode),
                input.every_minutes as i64,
                input.next_run_at,
                input.enabled as i64,
                now_ms,
            ],
        )?;
        Ok(id)
    }

    pub fn delete_scheduled_task(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM scheduled_tasks WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Automações ligadas cuja hora chegou.
    pub fn due_scheduled_tasks(&self, now_ms: i64) -> Result<Vec<ScheduledTask>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUNAS} FROM scheduled_tasks
             WHERE enabled = 1 AND every_minutes > 0
               AND next_run_at IS NOT NULL AND next_run_at <= ?1
             ORDER BY next_run_at"
        ))?;
        let rows = stmt
            .query_map([now_ms], task_from)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Marca o começo e já agenda a próxima vez.
    ///
    /// A próxima é contada a partir de AGORA, não do horário previsto: se o
    /// app ficou dias desligado, a automação roda uma vez ao voltar e segue o
    /// ritmo — não dispara as vinte que "deveriam" ter rodado.
    pub fn scheduled_task_started(
        &self,
        id: &str,
        run_id: Option<&str>,
        now_ms: i64,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE scheduled_tasks
                SET last_run_at = ?2,
                    last_run_id = ?3,
                    last_status = NULL,
                    last_summary = NULL,
                    next_run_at = CASE
                        WHEN every_minutes > 0 THEN ?2 + every_minutes * 60000
                        ELSE next_run_at END
              WHERE id = ?1",
            params![id, now_ms, run_id],
        )?;
        Ok(())
    }

    /// Liga a automação à execução que ela acabou de disparar.
    ///
    /// Vem depois de `scheduled_task_started` porque subir o motor pode
    /// demorar, e a marcação de "começou" tem que ser imediata para o relógio
    /// não disparar a mesma tarefa de novo no tique seguinte.
    pub fn set_scheduled_task_run(&self, id: &str, run_id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE scheduled_tasks SET last_run_id = ?2 WHERE id = ?1",
            params![id, run_id],
        )?;
        Ok(())
    }

    /// Como terminou (ou por que nem começou).
    pub fn scheduled_task_finished(
        &self,
        id: &str,
        status: Option<RunStatus>,
        summary: &str,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE scheduled_tasks SET last_status = ?2, last_summary = ?3 WHERE id = ?1",
            params![
                id,
                status.as_ref().map(crate::agent::enum_str),
                summary.trim(),
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str) -> ScheduledTaskInput {
        ScheduledTaskInput {
            id: None,
            name: name.into(),
            prompt: "resuma o que mudou".into(),
            workspace_dir: Some("/projeto".into()),
            model: None,
            mode: RunMode::Approve,
            work_mode: WorkMode::Agent,
            every_minutes: 60,
            next_run_at: Some(1_000),
            enabled: true,
        }
    }

    #[test]
    fn saving_twice_edits_instead_of_duplicating() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .save_scheduled_task(&input("Resumo"), "auto_1", 1)
            .unwrap();

        let mut edit = input("Resumo diário");
        edit.id = Some(id.clone());
        edit.every_minutes = 1_440;
        store.save_scheduled_task(&edit, "auto_2", 2).unwrap();

        let all = store.list_scheduled_tasks().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "Resumo diário");
        assert_eq!(all[0].every_minutes, 1_440);
        assert_eq!(all[0].id, id);
    }

    #[test]
    fn only_enabled_tasks_whose_time_has_come_are_due() {
        let store = Store::open_in_memory().unwrap();

        let mut agora = input("Agora");
        agora.next_run_at = Some(500);
        store.save_scheduled_task(&agora, "auto_1", 1).unwrap();

        let mut depois = input("Depois");
        depois.next_run_at = Some(5_000);
        store.save_scheduled_task(&depois, "auto_2", 1).unwrap();

        let mut desligada = input("Desligada");
        desligada.next_run_at = Some(100);
        desligada.enabled = false;
        store.save_scheduled_task(&desligada, "auto_3", 1).unwrap();

        let mut manual = input("Só na mão");
        manual.every_minutes = 0;
        manual.next_run_at = Some(100);
        store.save_scheduled_task(&manual, "auto_4", 1).unwrap();

        let due = store.due_scheduled_tasks(1_000).unwrap();
        assert_eq!(due.len(), 1, "{due:?}");
        assert_eq!(due[0].name, "Agora");
    }

    /// App desligado por uma semana não pode acordar disparando as execuções
    /// que "faltaram" — a próxima conta a partir de agora.
    #[test]
    fn the_next_run_is_counted_from_now() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .save_scheduled_task(&input("Resumo"), "auto_1", 1)
            .unwrap();

        store
            .scheduled_task_started(&id, Some("run_1"), 10_000)
            .unwrap();
        let t = store.scheduled_task(&id).unwrap().unwrap();
        assert_eq!(t.last_run_at, Some(10_000));
        assert_eq!(t.last_run_id.as_deref(), Some("run_1"));
        assert_eq!(t.next_run_at, Some(10_000 + 60 * 60_000));
        assert!(t.last_status.is_none(), "execução nova, resultado limpo");

        store
            .scheduled_task_finished(&id, Some(RunStatus::Done), " tudo certo ")
            .unwrap();
        let t = store.scheduled_task(&id).unwrap().unwrap();
        assert_eq!(t.last_status, Some(RunStatus::Done));
        assert_eq!(t.last_summary.as_deref(), Some("tudo certo"));
    }

    #[test]
    fn editing_keeps_the_result_of_the_last_run() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .save_scheduled_task(&input("Resumo"), "auto_1", 1)
            .unwrap();
        store
            .scheduled_task_started(&id, Some("run_1"), 10_000)
            .unwrap();
        store
            .scheduled_task_finished(&id, Some(RunStatus::Done), "3 arquivos")
            .unwrap();

        let mut edit = input("Resumo (novo texto)");
        edit.id = Some(id.clone());
        store.save_scheduled_task(&edit, "auto_9", 20_000).unwrap();

        let t = store.scheduled_task(&id).unwrap().unwrap();
        assert_eq!(t.last_summary.as_deref(), Some("3 arquivos"));
        assert_eq!(t.last_run_id.as_deref(), Some("run_1"));
    }

    #[test]
    fn deleting_removes_it() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .save_scheduled_task(&input("Resumo"), "auto_1", 1)
            .unwrap();
        store.delete_scheduled_task(&id).unwrap();
        assert!(store.list_scheduled_tasks().unwrap().is_empty());
        assert!(store.scheduled_task(&id).unwrap().is_none());
    }
}
