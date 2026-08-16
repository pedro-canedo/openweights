//! Leituras para a tela de atividade: o que o agente andou fazendo.
//!
//! Nada é gravado aqui — as tabelas já existem (`runs`, `tool_calls`,
//! `approvals`). O que falta é a pergunta certa: *o que este agente fez na
//! minha máquina, quando, e quem deixou?* Uma execução por vez não responde
//! isso; responder exige atravessar as execuções.
//!
//! Sobre "custo": modelo local não cobra em dinheiro. O que ele cobra é tempo
//! e tokens, e é isso que estas contas devolvem — inventar um valor em dólar
//! seria mentira confortável.

use crate::{Store, StoreError};
use rusqlite::Row;
use serde::{Deserialize, Serialize};

/// Uma ação do agente, com quem a autorizou.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityCall {
    pub call_id: String,
    pub run_id: String,
    pub tool_name: String,
    /// `builtin` ou `mcp:<servidor>`.
    pub origin: String,
    /// `requested` | `running` | `ok` | `error` | `denied` (o que o laço gravou).
    pub state: String,
    pub error: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    /// `allowOnce`, `allowAlways`, `deny`… quando houve uma decisão gravada.
    pub decision: Option<String>,
    /// Quem decidiu: `user`, `policy` ou `yolo`. Ausente = não passou por
    /// confirmação (leitura dentro do projeto, por exemplo).
    pub source: Option<String>,
}

/// Contas de um período.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityStats {
    pub runs: u32,
    /// Execuções que não terminaram bem (erro, teto de passos, escalonadas).
    pub failed: u32,
    pub tool_calls: u32,
    /// Ações que a pessoa recusou — sinal de que o agente estava indo para o
    /// lado errado.
    pub denied: u32,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Tempo somado das execuções, em milissegundos.
    pub duration_ms: u64,
}

fn call_from(r: &Row<'_>) -> rusqlite::Result<ActivityCall> {
    Ok(ActivityCall {
        call_id: r.get(0)?,
        run_id: r.get(1)?,
        tool_name: r.get(2)?,
        origin: r.get(3)?,
        state: r.get(4)?,
        error: r.get(5)?,
        started_at: r.get(6)?,
        finished_at: r.get(7)?,
        decision: r.get(8)?,
        source: r.get(9)?,
    })
}

impl Store {
    /// Últimas ações do agente, da mais recente para a mais antiga.
    ///
    /// A decisão vem por `LEFT JOIN`: ação sem linha em `approvals` é ação que
    /// não precisou de confirmação, e isso também é informação.
    pub fn list_recent_calls(&self, limit: u32) -> Result<Vec<ActivityCall>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT c.id, c.run_id, c.tool_name, c.origin, c.state, c.error,
                    c.started_at, c.finished_at, a.decision, a.source
             FROM tool_calls c
             LEFT JOIN approvals a ON a.tool_call_id = c.id
             ORDER BY c.rowid DESC
             LIMIT ?1",
        )?;
        let rows = stmt
            .query_map([limit.max(1)], call_from)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// As mesmas ações, limitadas a uma execução.
    pub fn list_run_calls_with_decision(
        &self,
        run_id: &str,
    ) -> Result<Vec<ActivityCall>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT c.id, c.run_id, c.tool_name, c.origin, c.state, c.error,
                    c.started_at, c.finished_at, a.decision, a.source
             FROM tool_calls c
             LEFT JOIN approvals a ON a.tool_call_id = c.id
             WHERE c.run_id = ?1
             ORDER BY c.rowid",
        )?;
        let rows = stmt
            .query_map([run_id], call_from)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Contas das execuções criadas a partir de `since` (epoch em SEGUNDOS —
    /// é a unidade de `runs.created_at`, não a de `ts_ms` dos eventos).
    pub fn activity_stats(&self, since: i64) -> Result<ActivityStats, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut out = ActivityStats::default();

        // Uma varredura só: `usage_json` é texto e precisa ser lido em Rust.
        let mut stmt =
            conn.prepare("SELECT status, usage_json FROM runs WHERE created_at >= ?1")?;
        let mut rows = stmt.query([since])?;
        while let Some(r) = rows.next()? {
            out.runs += 1;
            let status: String = r.get(0)?;
            if matches!(status.as_str(), "error" | "maxSteps" | "escalated") {
                out.failed += 1;
            }
            if let Some(json) = r.get::<_, Option<String>>(1)?
                && let Ok(u) = serde_json::from_str::<lr_types::agent::UsageStats>(&json)
            {
                out.prompt_tokens += u.prompt_tokens as u64;
                out.completion_tokens += u.completion_tokens as u64;
                out.duration_ms += u.duration_ms;
                out.tool_calls += u.tool_calls;
            }
        }

        // Recusas não estão no `usage`: elas são a conta do que NÃO aconteceu.
        let mut stmt = conn.prepare(
            "SELECT COUNT(*) FROM approvals a
             JOIN runs r ON r.id = a.run_id
             WHERE r.created_at >= ?1 AND a.decision LIKE 'deny%'",
        )?;
        out.denied = stmt.query_row([since], |r| r.get::<_, i64>(0))? as u32;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_types::agent::{ApprovalSource, RunMode, RunStatus, UsageStats};

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn run(s: &Store, id: &str) {
        s.create_run(
            id,
            None,
            "modelo",
            RunMode::Smart,
            false,
            Some("/ws"),
            "faça",
        )
        .unwrap();
    }

    fn usage(prompt: u32, completion: u32, calls: u32, ms: u64) -> String {
        serde_json::to_string(&UsageStats {
            steps: 1,
            prompt_tokens: prompt,
            completion_tokens: completion,
            tool_calls: calls,
            duration_ms: ms,
        })
        .unwrap()
    }

    #[test]
    fn recent_calls_come_newest_first_with_who_decided() {
        let s = store();
        run(&s, "r1");
        s.create_tool_call("c1", "r1", None, "fs_read", "builtin", "{}")
            .unwrap();
        s.create_tool_call("c2", "r1", None, "fs_write", "builtin", "{}")
            .unwrap();
        s.log_approval(
            "r1",
            "c2",
            "fs_write",
            "allowAlways",
            ApprovalSource::User,
            "hash",
        )
        .unwrap();

        let calls = s.list_recent_calls(10).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].call_id, "c2", "a mais recente vem primeiro");
        assert_eq!(calls[0].source.as_deref(), Some("user"));
        assert!(
            calls[1].decision.is_none(),
            "leitura não passou por confirmação, e isso é informação"
        );
    }

    #[test]
    fn the_limit_is_respected() {
        let s = store();
        run(&s, "r1");
        for i in 0..5 {
            s.create_tool_call(&format!("c{i}"), "r1", None, "fs_read", "builtin", "{}")
                .unwrap();
        }
        assert_eq!(s.list_recent_calls(3).unwrap().len(), 3);
        // Zero seria uma lista vazia sem querer: o mínimo é um.
        assert_eq!(s.list_recent_calls(0).unwrap().len(), 1);
    }

    #[test]
    fn stats_add_up_what_the_runs_cost() {
        let s = store();
        run(&s, "r1");
        run(&s, "r2");
        s.finish_run("r1", RunStatus::Done, "ok", &usage(100, 50, 2, 1_000))
            .unwrap();
        s.finish_run(
            "r2",
            RunStatus::MaxSteps,
            "parou",
            &usage(200, 80, 3, 2_500),
        )
        .unwrap();

        let st = s.activity_stats(0).unwrap();
        assert_eq!(st.runs, 2);
        assert_eq!(st.failed, 1, "teto de passos não é sucesso");
        assert_eq!(st.prompt_tokens, 300);
        assert_eq!(st.completion_tokens, 130);
        assert_eq!(st.tool_calls, 5);
        assert_eq!(st.duration_ms, 3_500);
    }

    #[test]
    fn a_refusal_is_counted_even_though_nothing_happened() {
        let s = store();
        run(&s, "r1");
        s.create_tool_call("c1", "r1", None, "terminal_run", "builtin", "{}")
            .unwrap();
        s.log_approval(
            "r1",
            "c1",
            "terminal_run",
            "deny",
            ApprovalSource::User,
            "hash",
        )
        .unwrap();

        assert_eq!(s.activity_stats(0).unwrap().denied, 1);
    }

    /// Execução de ontem não entra na conta de hoje.
    #[test]
    fn the_period_filters_by_when_the_run_started() {
        let s = store();
        run(&s, "r1");
        s.finish_run("r1", RunStatus::Done, "ok", &usage(10, 10, 0, 10))
            .unwrap();

        assert_eq!(s.activity_stats(0).unwrap().runs, 1);
        let futuro = Store::now() + 60;
        assert_eq!(s.activity_stats(futuro).unwrap().runs, 0);
    }

    #[test]
    fn the_calls_of_one_run_come_in_order() {
        let s = store();
        run(&s, "r1");
        run(&s, "r2");
        s.create_tool_call("a1", "r1", None, "fs_read", "builtin", "{}")
            .unwrap();
        s.create_tool_call("b1", "r2", None, "fs_list", "builtin", "{}")
            .unwrap();
        s.create_tool_call("a2", "r1", None, "fs_write", "builtin", "{}")
            .unwrap();

        let calls = s.list_run_calls_with_decision("r1").unwrap();
        assert_eq!(
            calls.iter().map(|c| c.call_id.as_str()).collect::<Vec<_>>(),
            ["a1", "a2"]
        );
    }
}
