//! Persistência da memória do agente: fatos (semântica, curada, injetada no
//! prompt) e episódios (histórico do que foi feito, consolidado em idle).
//!
//! Decisão de projeto: memória NÃO é banco vetorial. Fatos são poucos, curtos
//! e inspecionáveis; o arquivo `MEMORY.md` do workspace é a face legível deles.

use crate::{Store, StoreError};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS memory_facts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace_dir TEXT NOT NULL DEFAULT '',
    content TEXT NOT NULL,
    source_run_id TEXT,
    archived INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_memory_facts_ws ON memory_facts(workspace_dir, archived);

CREATE TABLE IF NOT EXISTS memory_episodes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace_dir TEXT NOT NULL DEFAULT '',
    chat_id INTEGER,
    run_id TEXT,
    summary TEXT NOT NULL,
    consolidated INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_memory_episodes_ws ON memory_episodes(workspace_dir, consolidated);
"#;

pub(crate) fn init(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(SCHEMA)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryFact {
    pub id: i64,
    /// `None` = fato global (vale em qualquer projeto).
    pub workspace_dir: Option<String>,
    pub content: String,
    pub source_run_id: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEpisode {
    pub id: i64,
    pub workspace_dir: Option<String>,
    pub chat_id: Option<i64>,
    pub run_id: Option<String>,
    pub summary: String,
    pub created_at: i64,
}

/// Normaliza um fato para comparação de duplicatas (caixa, espaços, pontuação
/// final). Usada tanto na inserção quanto na consolidação.
pub fn fact_key(content: &str) -> String {
    content
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_ascii_punctuation())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

impl Store {
    /// Insere um fato, ignorando duplicata equivalente no mesmo escopo.
    /// Retorna o id (novo ou existente).
    pub fn add_memory_fact(
        &self,
        workspace_dir: Option<&str>,
        content: &str,
        source_run_id: Option<&str>,
    ) -> Result<i64, StoreError> {
        let ws = workspace_dir.unwrap_or("");
        let key = fact_key(content);
        let existing = self
            .list_memory_facts(workspace_dir)?
            .into_iter()
            .find(|f| fact_key(&f.content) == key);
        if let Some(f) = existing {
            return Ok(f.id);
        }
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO memory_facts (workspace_dir, content, source_run_id, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![ws, content.trim(), source_run_id, Self::now()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Fatos aplicáveis: globais + os do workspace informado.
    pub fn list_memory_facts(
        &self,
        workspace_dir: Option<&str>,
    ) -> Result<Vec<MemoryFact>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, workspace_dir, content, source_run_id, created_at
             FROM memory_facts
             WHERE archived = 0 AND (workspace_dir = '' OR workspace_dir = ?1)
             ORDER BY created_at",
        )?;
        let rows = stmt
            .query_map([workspace_dir.unwrap_or("")], |r| {
                let ws: String = r.get(1)?;
                Ok(MemoryFact {
                    id: r.get(0)?,
                    workspace_dir: (!ws.is_empty()).then_some(ws),
                    content: r.get(2)?,
                    source_run_id: r.get(3)?,
                    created_at: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn delete_memory_fact(&self, id: i64) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM memory_facts WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Arquiva em vez de apagar (a consolidação prefere arquivar).
    pub fn archive_memory_fact(&self, id: i64) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE memory_facts SET archived = 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn add_memory_episode(
        &self,
        workspace_dir: Option<&str>,
        chat_id: Option<i64>,
        run_id: Option<&str>,
        summary: &str,
    ) -> Result<i64, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO memory_episodes (workspace_dir, chat_id, run_id, summary, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                workspace_dir.unwrap_or(""),
                chat_id,
                run_id,
                summary,
                Self::now()
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Episódios ainda não consolidados (entrada do job de idle).
    pub fn pending_episodes(
        &self,
        workspace_dir: Option<&str>,
        limit: u32,
    ) -> Result<Vec<MemoryEpisode>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, workspace_dir, chat_id, run_id, summary, created_at
             FROM memory_episodes
             WHERE consolidated = 0 AND (workspace_dir = '' OR workspace_dir = ?1)
             ORDER BY created_at LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![workspace_dir.unwrap_or(""), limit], |r| {
                let ws: String = r.get(1)?;
                Ok(MemoryEpisode {
                    id: r.get(0)?,
                    workspace_dir: (!ws.is_empty()).then_some(ws),
                    chat_id: r.get(2)?,
                    run_id: r.get(3)?,
                    summary: r.get(4)?,
                    created_at: r.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn mark_episodes_consolidated(&self, ids: &[i64]) -> Result<(), StoreError> {
        if ids.is_empty() {
            return Ok(());
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        for id in ids {
            tx.execute(
                "UPDATE memory_episodes SET consolidated = 1 WHERE id = ?1",
                [id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Últimos episódios (para exibir e para contexto do run).
    pub fn recent_episodes(
        &self,
        workspace_dir: Option<&str>,
        limit: u32,
    ) -> Result<Vec<MemoryEpisode>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, workspace_dir, chat_id, run_id, summary, created_at
             FROM memory_episodes
             WHERE workspace_dir = '' OR workspace_dir = ?1
             ORDER BY created_at DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![workspace_dir.unwrap_or(""), limit], |r| {
                let ws: String = r.get(1)?;
                Ok(MemoryEpisode {
                    id: r.get(0)?,
                    workspace_dir: (!ws.is_empty()).then_some(ws),
                    chat_id: r.get(2)?,
                    run_id: r.get(3)?,
                    summary: r.get(4)?,
                    created_at: r.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facts_dedup_and_scope() {
        let s = Store::open_in_memory().unwrap();
        let a = s
            .add_memory_fact(Some("/ws"), "Usa pnpm, não npm.", None)
            .unwrap();
        // Duplicata equivalente (caixa/pontuação/espaços) não cria linha nova.
        let b = s
            .add_memory_fact(Some("/ws"), "  usa   pnpm  nao npm  ", None)
            .unwrap();
        assert_ne!(a, b, "chave normalizada difere por acento — esperado");

        let c = s
            .add_memory_fact(Some("/ws"), "USA PNPM, NÃO NPM!", None)
            .unwrap();
        assert_eq!(
            a, c,
            "mesma frase com outra caixa/pontuação deve deduplicar"
        );

        s.add_memory_fact(None, "Prefere respostas curtas.", None)
            .unwrap();
        // Workspace enxerga o próprio + global.
        assert_eq!(s.list_memory_facts(Some("/ws")).unwrap().len(), 3);
        // Outro workspace só o global.
        assert_eq!(s.list_memory_facts(Some("/outro")).unwrap().len(), 1);
    }

    #[test]
    fn facts_delete_and_archive() {
        let s = Store::open_in_memory().unwrap();
        let id = s.add_memory_fact(Some("/ws"), "algo", None).unwrap();
        s.archive_memory_fact(id).unwrap();
        assert!(s.list_memory_facts(Some("/ws")).unwrap().is_empty());

        let id2 = s.add_memory_fact(Some("/ws"), "outro", None).unwrap();
        s.delete_memory_fact(id2).unwrap();
        assert!(s.list_memory_facts(Some("/ws")).unwrap().is_empty());
    }

    #[test]
    fn episodes_pending_then_consolidated() {
        let s = Store::open_in_memory().unwrap();
        let e1 = s
            .add_memory_episode(Some("/ws"), Some(1), Some("r1"), "corrigiu build")
            .unwrap();
        s.add_memory_episode(Some("/ws"), None, Some("r2"), "rodou testes")
            .unwrap();

        assert_eq!(s.pending_episodes(Some("/ws"), 10).unwrap().len(), 2);
        s.mark_episodes_consolidated(&[e1]).unwrap();
        assert_eq!(s.pending_episodes(Some("/ws"), 10).unwrap().len(), 1);
        // Recentes ignora o flag de consolidação.
        assert_eq!(s.recent_episodes(Some("/ws"), 10).unwrap().len(), 2);
    }

    #[test]
    fn fact_key_normalizes() {
        assert_eq!(fact_key("Usa  PNPM!"), fact_key("usa pnpm"));
        assert_ne!(fact_key("usa pnpm"), fact_key("usa npm"));
    }
}
