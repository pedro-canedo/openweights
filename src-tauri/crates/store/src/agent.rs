//! Persistência do harness agêntico: execuções (runs), trilha de eventos,
//! chamadas de ferramenta, auditoria de aprovações, checkpoints e overrides
//! de permissão.
//!
//! A trilha `run_events` é append-only e é o que permite reconstruir a
//! interface depois de recarregar o webview (replay por `seq`).

use crate::{Store, StoreError};
use lr_types::agent::{
    ApprovalSource, PolicyScope, RunEvent, RunEventKind, RunMode, RunStatus, RunSummary,
    ToolPolicy, UsageStats,
};
use lr_types::scout::WorkMode;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS runs (
    id TEXT PRIMARY KEY,
    chat_id INTEGER REFERENCES chats(id) ON DELETE SET NULL,
    model TEXT NOT NULL,
    mode TEXT NOT NULL,
    yolo INTEGER NOT NULL DEFAULT 0,
    workspace_dir TEXT,
    status TEXT NOT NULL,
    prompt TEXT NOT NULL,
    summary TEXT,
    focus_md TEXT,
    usage_json TEXT,
    plan_json TEXT,
    verify_json TEXT,
    work_mode TEXT,
    question_json TEXT,
    answer TEXT,
    resumed_by TEXT,
    created_at INTEGER NOT NULL,
    finished_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_runs_chat ON runs(chat_id);

CREATE TABLE IF NOT EXISTS run_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    kind TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_run_events_seq ON run_events(run_id, seq);

CREATE TABLE IF NOT EXISTS steps (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    idx INTEGER NOT NULL,
    assistant_content TEXT,
    reasoning TEXT,
    prompt_tokens INTEGER,
    completion_tokens INTEGER,
    started_at INTEGER,
    finished_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_steps_run ON steps(run_id);

CREATE TABLE IF NOT EXISTS tool_calls (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    step_id TEXT,
    tool_name TEXT NOT NULL,
    origin TEXT NOT NULL,
    args_json TEXT NOT NULL,
    state TEXT NOT NULL,
    result_json TEXT,
    result_bytes INTEGER,
    error TEXT,
    started_at INTEGER,
    finished_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_tool_calls_run ON tool_calls(run_id);

CREATE TABLE IF NOT EXISTS approvals (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL,
    tool_call_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    decision TEXT NOT NULL,
    source TEXT NOT NULL,
    args_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS checkpoints (
    id TEXT PRIMARY KEY,
    run_id TEXT,
    workspace_dir TEXT NOT NULL,
    backend TEXT NOT NULL,
    ref_id TEXT NOT NULL,
    label TEXT,
    files_json TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_checkpoints_ws ON checkpoints(workspace_dir);

CREATE TABLE IF NOT EXISTS tool_permissions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scope TEXT NOT NULL,
    workspace_dir TEXT NOT NULL DEFAULT '',
    tool_name TEXT NOT NULL,
    policy TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(scope, workspace_dir, tool_name)
);
"#;

pub(crate) fn init(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(SCHEMA)?;
    Ok(())
}

// ------------------------------------------------------------------- linhas ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEventRow {
    pub seq: u64,
    pub kind: String,
    /// Payload completo do `RunEvent` (envelope inclusive) em JSON.
    pub payload_json: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallRow {
    pub id: String,
    pub run_id: String,
    pub step_id: Option<String>,
    pub tool_name: String,
    pub origin: String,
    pub args_json: String,
    pub state: String,
    pub result_json: Option<String>,
    pub error: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointRow {
    pub id: String,
    pub run_id: Option<String>,
    pub workspace_dir: String,
    pub backend: String,
    pub ref_id: String,
    pub label: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPermissionRow {
    pub id: i64,
    pub scope: PolicyScope,
    /// Vazio quando o escopo é global.
    pub workspace_dir: Option<String>,
    pub tool_name: String,
    pub policy: ToolPolicy,
}

/// Serializa um enum serde para a forma textual usada nas colunas.
pub(crate) fn enum_str<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|x| x.as_str().map(str::to_string))
        .unwrap_or_default()
}

pub(crate) fn enum_from<T: serde::de::DeserializeOwned>(s: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
}

impl Store {
    // -------------------------------------------------------------- runs --

    #[allow(clippy::too_many_arguments)]
    pub fn create_run(
        &self,
        id: &str,
        chat_id: Option<i64>,
        model: &str,
        mode: RunMode,
        yolo: bool,
        workspace_dir: Option<&str>,
        prompt: &str,
    ) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO runs (id, chat_id, model, mode, yolo, workspace_dir, status, prompt, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                chat_id,
                model,
                enum_str(&mode),
                yolo as i64,
                workspace_dir,
                enum_str(&RunStatus::Running),
                prompt,
                Self::now()
            ],
        )?;
        Ok(())
    }

    /// Grava o modo de trabalho escolhido: o run reaberto precisa saber como
    /// foi conduzido — antes isso vivia só na memória da interface.
    pub fn set_run_work_mode(&self, run_id: &str, mode: WorkMode) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "UPDATE runs SET work_mode = ?2 WHERE id = ?1",
            params![run_id, enum_str(&mode)],
        )?;
        Ok(())
    }

    pub fn set_run_status(&self, run_id: &str, status: RunStatus) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "UPDATE runs SET status = ?2 WHERE id = ?1",
            params![run_id, enum_str(&status)],
        )?;
        Ok(())
    }

    pub fn finish_run(
        &self,
        run_id: &str,
        status: RunStatus,
        summary: &str,
        usage_json: &str,
    ) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "UPDATE runs SET status = ?2, summary = ?3, usage_json = ?4, finished_at = ?5
             WHERE id = ?1",
            params![run_id, enum_str(&status), summary, usage_json, Self::now()],
        )?;
        Ok(())
    }

    pub fn set_run_focus(&self, run_id: &str, focus_md: &str) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "UPDATE runs SET focus_md = ?2 WHERE id = ?1",
            params![run_id, focus_md],
        )?;
        Ok(())
    }

    /// Grava as perguntas que pausaram o run (persistidas: a pausa sobrevive
    /// a reinício e é respondível pela tela de Atividade).
    pub fn set_run_question(
        &self,
        run_id: &str,
        question: &lr_types::scout::PendingQuestion,
    ) -> Result<(), StoreError> {
        let json = serde_json::to_string(question).unwrap_or_else(|_| "{}".into());
        let conn = self.conn();
        conn.execute(
            "UPDATE runs SET question_json = ?2 WHERE id = ?1",
            params![run_id, json],
        )?;
        Ok(())
    }

    /// Grava a resposta da pessoa e o run que retomou o trabalho — a
    /// pendência sai da fila de "aguardando resposta".
    pub fn set_run_answer(
        &self,
        run_id: &str,
        answer: &str,
        resumed_by: &str,
    ) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "UPDATE runs SET answer = ?2, resumed_by = ?3 WHERE id = ?1",
            params![run_id, answer, resumed_by],
        )?;
        Ok(())
    }

    /// Runs pausados numa pergunta e ainda sem resposta — a fila que a tela
    /// de Atividade mostra em "Aguardando resposta".
    pub fn runs_waiting_answer(&self) -> Result<Vec<RunSummary>, StoreError> {
        Ok(self
            .list_runs(None)?
            .into_iter()
            .filter(|r| {
                r.status == RunStatus::Escalated
                    && r.question.is_some()
                    && r.answer.is_none()
                    && r.resumed_by.is_none()
            })
            .collect())
    }

    /// Guarda o veredito da conferência automática do run.
    pub fn set_run_verification(
        &self,
        run_id: &str,
        verification: &lr_types::agent::RunVerification,
    ) -> Result<(), StoreError> {
        let json = serde_json::to_string(verification).unwrap_or_else(|_| "{}".into());
        let conn = self.conn();
        conn.execute(
            "UPDATE runs SET verify_json = ?2 WHERE id = ?1",
            params![run_id, json],
        )?;
        Ok(())
    }

    /// Guarda o plano de trabalho (Scout Rule) do run.
    pub fn set_run_plan(&self, run_id: &str, plan_json: &str) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "UPDATE runs SET plan_json = ?2 WHERE id = ?1",
            params![run_id, plan_json],
        )?;
        Ok(())
    }

    /// Plano de trabalho do run, se houver.
    pub fn get_run_plan(&self, run_id: &str) -> Result<Option<String>, StoreError> {
        let conn = self.conn();
        let plan = conn
            .query_row("SELECT plan_json FROM runs WHERE id = ?1", [run_id], |r| {
                r.get::<_, Option<String>>(0)
            })
            .optional()?
            .flatten();
        Ok(plan)
    }

    /// Runs de uma conversa (mais recentes primeiro). `None` = todos.
    pub fn list_runs(&self, chat_id: Option<i64>) -> Result<Vec<RunSummary>, StoreError> {
        let conn = self.conn();
        let sql = "SELECT id, chat_id, model, mode, status, prompt, summary, workspace_dir,
                          created_at, finished_at, usage_json, verify_json,
                          plan_json IS NOT NULL AND plan_json != '', work_mode,
                          question_json, answer, resumed_by
                   FROM runs {WHERE} ORDER BY created_at DESC LIMIT 200";
        let map = |r: &rusqlite::Row<'_>| -> rusqlite::Result<RunSummary> {
            Ok(RunSummary {
                id: r.get(0)?,
                chat_id: r.get(1)?,
                model: r.get(2)?,
                mode: enum_from(&r.get::<_, String>(3)?).unwrap_or_default(),
                status: enum_from(&r.get::<_, String>(4)?).unwrap_or(RunStatus::Error),
                prompt: r.get(5)?,
                summary: r.get(6)?,
                workspace_dir: r.get(7)?,
                created_at: r.get(8)?,
                finished_at: r.get(9)?,
                usage: r
                    .get::<_, Option<String>>(10)?
                    .and_then(|j| serde_json::from_str(&j).ok()),
                verification: r
                    .get::<_, Option<String>>(11)?
                    .and_then(|j| serde_json::from_str(&j).ok()),
                has_plan: r.get(12)?,
                work_mode: r
                    .get::<_, Option<String>>(13)?
                    .and_then(|s| enum_from(&s))
                    .unwrap_or_default(),
                question: r
                    .get::<_, Option<String>>(14)?
                    .and_then(|j| serde_json::from_str(&j).ok()),
                answer: r.get(15)?,
                resumed_by: r.get(16)?,
            })
        };
        let rows = match chat_id {
            Some(id) => {
                let mut stmt = conn.prepare(&sql.replace("{WHERE}", "WHERE chat_id = ?1"))?;
                stmt.query_map([id], map)?.collect::<Result<Vec<_>, _>>()?
            }
            None => {
                let mut stmt = conn.prepare(&sql.replace("{WHERE}", ""))?;
                stmt.query_map([], map)?.collect::<Result<Vec<_>, _>>()?
            }
        };
        Ok(rows)
    }

    pub fn get_run(&self, run_id: &str) -> Result<Option<RunSummary>, StoreError> {
        Ok(self.list_runs(None)?.into_iter().find(|r| r.id == run_id))
    }

    /// Marca como `Error` runs deixados em aberto por um fechamento abrupto —
    /// com a causa escrita: cada um ganha `summary` e um `run.finished`
    /// sintético no fim da própria trilha. Sem isso a conversa reaberta
    /// terminava no nada, como se a execução tivesse evaporado.
    pub fn fail_orphan_runs(&self) -> Result<usize, StoreError> {
        const CAUSA: &str = "O aplicativo foi fechado com esta execução em andamento — \
                             ela não chegou ao fim.";
        let conn = self.conn();
        let orfaos: Vec<String> = {
            let mut stmt =
                conn.prepare("SELECT id FROM runs WHERE status IN ('running', 'waitingApproval')")?;
            stmt.query_map([], |r| r.get(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        for id in &orfaos {
            Self::carimbar_orfao(&conn, id, CAUSA)?;
        }
        Ok(orfaos.len())
    }

    /// Carimba UM run que o processo já não conhece mas o banco ainda diz
    /// vivo — é o run que morreu sem rastro (pânico engolido, task perdida).
    /// Devolve `true` quando havia algo a carimbar; um run que já tem
    /// desfecho legítimo NUNCA é sobrescrito.
    pub fn reap_run(&self, run_id: &str, causa: &str) -> Result<bool, StoreError> {
        let conn = self.conn();
        let vivo: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM runs WHERE id = ?1 AND status IN ('running', 'waitingApproval')",
                params![run_id],
                |r| r.get(0),
            )
            .optional()?;
        if vivo.is_none() {
            return Ok(false);
        }
        Self::carimbar_orfao(&conn, run_id, causa)?;
        Ok(true)
    }

    /// `finish_run` que NUNCA sobrescreve um desfecho: só grava se o run
    /// ainda está aberto. Devolve `true` quando gravou. É o que o socorro de
    /// pânico usa — sem inserir evento sintético, porque quem chama tem o
    /// `EventSink` vivo e o emit persiste o `run.finished` uma vez só.
    pub fn finish_run_if_open(
        &self,
        run_id: &str,
        status: RunStatus,
        summary: &str,
    ) -> Result<bool, StoreError> {
        let conn = self.conn();
        let n = conn.execute(
            "UPDATE runs SET status = ?2, summary = ?3, finished_at = ?4
             WHERE id = ?1 AND status IN ('running', 'waitingApproval')",
            params![run_id, enum_str(&status), summary, Self::now()],
        )?;
        Ok(n > 0)
    }

    /// A trilha deste run já tem um `run.finished`?
    pub fn run_has_finish_event(&self, run_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM run_events WHERE run_id = ?1 AND kind = 'run.finished'",
            params![run_id],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// Desfecho de socorro: `status = Error` + `summary` com a causa + um
    /// `run.finished` sintético no fim da trilha, para o replay terminar em
    /// explicação e não no nada.
    fn carimbar_orfao(conn: &Connection, id: &str, causa: &str) -> Result<(), StoreError> {
        conn.execute(
            "UPDATE runs SET status = ?2, summary = ?3, finished_at = ?4 WHERE id = ?1",
            params![id, enum_str(&RunStatus::Error), causa, Self::now()],
        )?;
        let seq: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM run_events WHERE run_id = ?1",
            params![id],
            |r| r.get(0),
        )?;
        let ev = RunEvent {
            run_id: id.to_string(),
            seq: seq as u64,
            ts_ms: Self::now_ms().max(0) as u64,
            event: RunEventKind::RunFinished {
                status: RunStatus::Error,
                summary: causa.to_string(),
                usage: UsageStats::default(),
            },
        };
        let payload = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
        conn.execute(
            "INSERT OR IGNORE INTO run_events (run_id, seq, kind, payload_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, seq, ev.kind_name(), payload, Self::now()],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------ eventos --

    pub fn append_run_event(&self, ev: &RunEvent) -> Result<(), StoreError> {
        let payload = serde_json::to_string(ev).unwrap_or_else(|_| "{}".into());
        let conn = self.conn();
        conn.execute(
            "INSERT OR IGNORE INTO run_events (run_id, seq, kind, payload_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                ev.run_id,
                ev.seq as i64,
                ev.kind_name(),
                payload,
                Self::now()
            ],
        )?;
        Ok(())
    }

    /// Eventos de um run com `seq > after_seq`, em ordem (replay).
    pub fn list_run_events(
        &self,
        run_id: &str,
        after_seq: u64,
    ) -> Result<Vec<RunEventRow>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT seq, kind, payload_json, created_at FROM run_events
             WHERE run_id = ?1 AND seq > ?2 ORDER BY seq",
        )?;
        let rows = stmt
            .query_map(params![run_id, after_seq as i64], |r| {
                Ok(RunEventRow {
                    seq: r.get::<_, i64>(0)? as u64,
                    kind: r.get(1)?,
                    payload_json: r.get(2)?,
                    created_at: r.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Maior `seq` já gravado (para retomar a numeração).
    pub fn last_run_seq(&self, run_id: &str) -> Result<u64, StoreError> {
        let conn = self.conn();
        let seq: Option<i64> = conn
            .query_row(
                "SELECT MAX(seq) FROM run_events WHERE run_id = ?1",
                [run_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        Ok(seq.unwrap_or(0) as u64)
    }

    // -------------------------------------------------------------- steps --

    pub fn create_step(&self, id: &str, run_id: &str, idx: u32) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO steps (id, run_id, idx, started_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, run_id, idx, Self::now_ms()],
        )?;
        Ok(())
    }

    pub fn finish_step(
        &self,
        id: &str,
        content: &str,
        reasoning: &str,
        prompt_tokens: Option<u32>,
        completion_tokens: Option<u32>,
    ) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "UPDATE steps SET assistant_content = ?2, reasoning = ?3, prompt_tokens = ?4,
                    completion_tokens = ?5, finished_at = ?6 WHERE id = ?1",
            params![
                id,
                content,
                reasoning,
                prompt_tokens,
                completion_tokens,
                Self::now_ms()
            ],
        )?;
        Ok(())
    }

    // --------------------------------------------------------- tool calls --

    pub fn create_tool_call(
        &self,
        id: &str,
        run_id: &str,
        step_id: Option<&str>,
        tool_name: &str,
        origin: &str,
        args_json: &str,
    ) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO tool_calls (id, run_id, step_id, tool_name, origin, args_json, state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'requested')",
            params![id, run_id, step_id, tool_name, origin, args_json],
        )?;
        Ok(())
    }

    pub fn set_tool_call_state(&self, id: &str, state: &str) -> Result<(), StoreError> {
        let conn = self.conn();
        let started = if state == "running" {
            Some(Self::now_ms())
        } else {
            None
        };
        conn.execute(
            "UPDATE tool_calls SET state = ?2, started_at = COALESCE(?3, started_at) WHERE id = ?1",
            params![id, state, started],
        )?;
        Ok(())
    }

    pub fn finish_tool_call(
        &self,
        id: &str,
        ok: bool,
        result_json: &str,
        result_bytes: u64,
        error: Option<&str>,
    ) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "UPDATE tool_calls SET state = ?2, result_json = ?3, result_bytes = ?4,
                    error = ?5, finished_at = ?6 WHERE id = ?1",
            params![
                id,
                if ok { "done" } else { "error" },
                result_json,
                result_bytes as i64,
                error,
                Self::now_ms()
            ],
        )?;
        Ok(())
    }

    pub fn list_tool_calls(&self, run_id: &str) -> Result<Vec<ToolCallRow>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, run_id, step_id, tool_name, origin, args_json, state, result_json,
                    error, started_at, finished_at
             FROM tool_calls WHERE run_id = ?1 ORDER BY rowid",
        )?;
        let rows = stmt
            .query_map([run_id], |r| {
                Ok(ToolCallRow {
                    id: r.get(0)?,
                    run_id: r.get(1)?,
                    step_id: r.get(2)?,
                    tool_name: r.get(3)?,
                    origin: r.get(4)?,
                    args_json: r.get(5)?,
                    state: r.get(6)?,
                    result_json: r.get(7)?,
                    error: r.get(8)?,
                    started_at: crate::ms_or_secs(r.get(9)?),
                    finished_at: crate::ms_or_secs(r.get(10)?),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ---------------------------------------------------------- auditoria --

    pub fn log_approval(
        &self,
        run_id: &str,
        tool_call_id: &str,
        tool_name: &str,
        decision: &str,
        source: ApprovalSource,
        args_hash: &str,
    ) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO approvals (run_id, tool_call_id, tool_name, decision, source, args_hash, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                run_id,
                tool_call_id,
                tool_name,
                decision,
                enum_str(&source),
                args_hash,
                Self::now()
            ],
        )?;
        Ok(())
    }

    // --------------------------------------------------------- permissões --

    pub fn set_tool_permission(
        &self,
        scope: PolicyScope,
        workspace_dir: Option<&str>,
        tool_name: &str,
        policy: ToolPolicy,
    ) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO tool_permissions (scope, workspace_dir, tool_name, policy, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(scope, workspace_dir, tool_name)
             DO UPDATE SET policy = excluded.policy, updated_at = excluded.updated_at",
            params![
                enum_str(&scope),
                workspace_dir.unwrap_or(""),
                tool_name,
                enum_str(&policy),
                Self::now()
            ],
        )?;
        Ok(())
    }

    pub fn clear_tool_permission(
        &self,
        scope: PolicyScope,
        workspace_dir: Option<&str>,
        tool_name: &str,
    ) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM tool_permissions WHERE scope = ?1 AND workspace_dir = ?2 AND tool_name = ?3",
            params![enum_str(&scope), workspace_dir.unwrap_or(""), tool_name],
        )?;
        Ok(())
    }

    /// Overrides aplicáveis: globais + os do workspace informado.
    pub fn list_tool_permissions(
        &self,
        workspace_dir: Option<&str>,
    ) -> Result<Vec<ToolPermissionRow>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, scope, workspace_dir, tool_name, policy FROM tool_permissions
             WHERE scope = 'global' OR workspace_dir = ?1
             ORDER BY tool_name",
        )?;
        let rows = stmt
            .query_map([workspace_dir.unwrap_or("")], |r| {
                let ws: String = r.get(2)?;
                Ok(ToolPermissionRow {
                    id: r.get(0)?,
                    scope: enum_from(&r.get::<_, String>(1)?).unwrap_or(PolicyScope::Global),
                    workspace_dir: (!ws.is_empty()).then_some(ws),
                    tool_name: r.get(3)?,
                    policy: enum_from(&r.get::<_, String>(4)?).unwrap_or(ToolPolicy::Ask),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // -------------------------------------------------------- checkpoints --

    #[allow(clippy::too_many_arguments)]
    pub fn add_checkpoint(
        &self,
        id: &str,
        run_id: Option<&str>,
        workspace_dir: &str,
        backend: &str,
        ref_id: &str,
        label: Option<&str>,
        files_json: Option<&str>,
    ) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO checkpoints (id, run_id, workspace_dir, backend, ref_id, label, files_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id,
                run_id,
                workspace_dir,
                backend,
                ref_id,
                label,
                files_json,
                Self::now()
            ],
        )?;
        Ok(())
    }

    pub fn list_checkpoints(&self, workspace_dir: &str) -> Result<Vec<CheckpointRow>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, run_id, workspace_dir, backend, ref_id, label, created_at
             FROM checkpoints WHERE workspace_dir = ?1 ORDER BY created_at DESC LIMIT 100",
        )?;
        let rows = stmt
            .query_map([workspace_dir], |r| {
                Ok(CheckpointRow {
                    id: r.get(0)?,
                    run_id: r.get(1)?,
                    workspace_dir: r.get(2)?,
                    backend: r.get(3)?,
                    ref_id: r.get(4)?,
                    label: r.get(5)?,
                    created_at: r.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_checkpoint(&self, id: &str) -> Result<Option<CheckpointRow>, StoreError> {
        let conn = self.conn();
        let row = conn
            .query_row(
                "SELECT id, run_id, workspace_dir, backend, ref_id, label, created_at
                 FROM checkpoints WHERE id = ?1",
                [id],
                |r| {
                    Ok(CheckpointRow {
                        id: r.get(0)?,
                        run_id: r.get(1)?,
                        workspace_dir: r.get(2)?,
                        backend: r.get(3)?,
                        ref_id: r.get(4)?,
                        label: r.get(5)?,
                        created_at: r.get(6)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_types::agent::{RunEventKind, UsageStats};

    fn ev(run_id: &str, seq: u64, event: RunEventKind) -> RunEvent {
        RunEvent {
            run_id: run_id.into(),
            seq,
            ts_ms: 0,
            event,
        }
    }

    #[test]
    fn run_lifecycle_and_event_replay() {
        let s = Store::open_in_memory().unwrap();
        let chat = s.create_chat("t", Some("m")).unwrap();
        s.create_run(
            "r1",
            Some(chat),
            "m",
            RunMode::Approve,
            false,
            Some("/ws"),
            "faça algo",
        )
        .unwrap();

        for seq in 1..=3u64 {
            s.append_run_event(&ev(
                "r1",
                seq,
                RunEventKind::StepStarted {
                    step_id: format!("s{seq}"),
                    index: seq as u32,
                },
            ))
            .unwrap();
        }
        // Idempotência: mesmo seq não duplica (INSERT OR IGNORE).
        s.append_run_event(&ev(
            "r1",
            3,
            RunEventKind::StepStarted {
                step_id: "s3".into(),
                index: 3,
            },
        ))
        .unwrap();

        assert_eq!(s.last_run_seq("r1").unwrap(), 3);
        assert_eq!(s.list_run_events("r1", 0).unwrap().len(), 3);
        assert_eq!(s.list_run_events("r1", 2).unwrap().len(), 1);

        s.finish_run("r1", RunStatus::Done, "pronto", "{}").unwrap();
        let runs = s.list_runs(Some(chat)).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, RunStatus::Done);
        assert_eq!(runs[0].mode, RunMode::Approve);
        assert_eq!(runs[0].summary.as_deref(), Some("pronto"));
    }

    #[test]
    fn tool_call_flow_is_recorded() {
        let s = Store::open_in_memory().unwrap();
        s.create_run("r", None, "m", RunMode::Yolo, true, None, "p")
            .unwrap();
        s.create_tool_call("c1", "r", None, "fs_write", "builtin", r#"{"path":"a"}"#)
            .unwrap();
        s.set_tool_call_state("c1", "running").unwrap();
        s.finish_tool_call("c1", true, r#"{"ok":true}"#, 12, None)
            .unwrap();
        s.log_approval(
            "r",
            "c1",
            "fs_write",
            "allowOnce",
            ApprovalSource::Yolo,
            "abc",
        )
        .unwrap();

        let calls = s.list_tool_calls("r").unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].state, "done");
        assert!(calls[0].started_at.is_some());
        assert!(calls[0].finished_at.is_some());
    }

    /// O tempo de uma ação é medido em milissegundos: em segundos toda
    /// ferramenta durava "0 ms" e a data caía em 1970 na tela de atividade.
    #[test]
    fn tool_call_timestamps_are_milliseconds() {
        let s = Store::open_in_memory().unwrap();
        s.create_run("r", None, "m", RunMode::Yolo, true, None, "p")
            .unwrap();
        s.create_tool_call("c1", "r", None, "fs_read", "builtin", "{}")
            .unwrap();
        s.set_tool_call_state("c1", "running").unwrap();
        s.finish_tool_call("c1", true, "{}", 0, None).unwrap();

        let call = &s.list_tool_calls("r").unwrap()[0];
        // Setembro de 2001 em milissegundos: um carimbo em segundos não chega
        // lá antes do ano 5138.
        assert!(call.started_at.unwrap() > 1_000_000_000_000);
        assert!(call.finished_at.unwrap() > 1_000_000_000_000);
    }

    /// Banco de antes da migração: os segundos gravados voltam em ms.
    #[test]
    fn seconds_from_older_databases_are_read_as_milliseconds() {
        let s = Store::open_in_memory().unwrap();
        s.create_run("r", None, "m", RunMode::Yolo, true, None, "p")
            .unwrap();
        s.create_tool_call("c1", "r", None, "fs_read", "builtin", "{}")
            .unwrap();
        {
            let conn = s.conn();
            conn.execute(
                "UPDATE tool_calls SET started_at = 1_700_000_000,
                        finished_at = 1_700_000_002 WHERE id = 'c1'",
                [],
            )
            .unwrap();
        }
        let call = &s.list_tool_calls("r").unwrap()[0];
        assert_eq!(call.started_at, Some(1_700_000_000_000));
        assert_eq!(call.finished_at, Some(1_700_000_002_000));
    }

    #[test]
    fn permissions_scope_and_upsert() {
        let s = Store::open_in_memory().unwrap();
        s.set_tool_permission(
            PolicyScope::Global,
            None,
            "fs_read",
            ToolPolicy::AlwaysAllow,
        )
        .unwrap();
        s.set_tool_permission(
            PolicyScope::Workspace,
            Some("/ws"),
            "terminal_run",
            ToolPolicy::Ask,
        )
        .unwrap();
        // Upsert: mesma chave troca a política, não duplica.
        s.set_tool_permission(
            PolicyScope::Workspace,
            Some("/ws"),
            "terminal_run",
            ToolPolicy::Never,
        )
        .unwrap();

        let all = s.list_tool_permissions(Some("/ws")).unwrap();
        assert_eq!(all.len(), 2);
        let term = all.iter().find(|p| p.tool_name == "terminal_run").unwrap();
        assert_eq!(term.policy, ToolPolicy::Never);
        assert_eq!(term.workspace_dir.as_deref(), Some("/ws"));

        // Outro workspace só enxerga o global.
        let other = s.list_tool_permissions(Some("/outro")).unwrap();
        assert_eq!(other.len(), 1);
        assert_eq!(other[0].tool_name, "fs_read");

        s.clear_tool_permission(PolicyScope::Global, None, "fs_read")
            .unwrap();
        assert!(s.list_tool_permissions(Some("/outro")).unwrap().is_empty());
    }

    #[test]
    fn checkpoints_roundtrip() {
        let s = Store::open_in_memory().unwrap();
        s.add_checkpoint("cp1", None, "/ws", "git", "deadbeef", Some("antes"), None)
            .unwrap();
        let list = s.list_checkpoints("/ws").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].ref_id, "deadbeef");
        assert!(s.get_checkpoint("cp1").unwrap().is_some());
        assert!(s.list_checkpoints("/outro").unwrap().is_empty());
    }

    #[test]
    fn orphan_runs_are_failed_on_startup() {
        let s = Store::open_in_memory().unwrap();
        s.create_run("r1", None, "m", RunMode::Approve, false, None, "p")
            .unwrap();
        s.create_run("r2", None, "m", RunMode::Approve, false, None, "p")
            .unwrap();
        s.finish_run("r2", RunStatus::Done, "", "{}").unwrap();

        assert_eq!(s.fail_orphan_runs().unwrap(), 1);
        let runs = s.list_runs(None).unwrap();
        let r1 = runs.iter().find(|r| r.id == "r1").unwrap();
        assert_eq!(r1.status, RunStatus::Error);
        let r2 = runs.iter().find(|r| r.id == "r2").unwrap();
        assert_eq!(r2.status, RunStatus::Done);
    }

    #[test]
    fn usage_json_roundtrips_through_finish() {
        let s = Store::open_in_memory().unwrap();
        s.create_run("r", None, "m", RunMode::Smart, false, None, "p")
            .unwrap();
        let usage = UsageStats {
            steps: 3,
            tool_calls: 5,
            ..Default::default()
        };
        s.finish_run(
            "r",
            RunStatus::Done,
            "ok",
            &serde_json::to_string(&usage).unwrap(),
        )
        .unwrap();
        assert_eq!(s.get_run("r").unwrap().unwrap().status, RunStatus::Done);
    }
}
