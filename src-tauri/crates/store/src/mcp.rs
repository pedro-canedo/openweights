//! Persistência dos conectores MCP: configuração dos servidores e cache das
//! definições de ferramentas.
//!
//! O par `tools_hash` / `tools_approved_hash` implementa a defesa contra
//! "rug pull": se o servidor mudar suas ferramentas depois de aprovado, os
//! hashes divergem e o app volta a pedir revisão do usuário.

use crate::{Store, StoreError};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS mcp_servers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    transport TEXT NOT NULL,
    config_json TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    tools_hash TEXT,
    tools_approved_hash TEXT,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS mcp_tools_cache (
    server_id TEXT NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT,
    schema_json TEXT NOT NULL,
    annotations_json TEXT,
    PRIMARY KEY (server_id, name)
);
"#;

pub(crate) fn init(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(SCHEMA)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerRow {
    pub id: String,
    pub name: String,
    /// `stdio` ou `http`.
    pub transport: String,
    pub config_json: String,
    pub enabled: bool,
    pub tools_hash: Option<String>,
    pub tools_approved_hash: Option<String>,
}

impl McpServerRow {
    /// Verdadeiro quando as ferramentas mudaram desde a última aprovação.
    pub fn needs_approval(&self) -> bool {
        match (&self.tools_hash, &self.tools_approved_hash) {
            (Some(cur), Some(approved)) => cur != approved,
            (Some(_), None) => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolRow {
    pub server_id: String,
    pub name: String,
    pub description: Option<String>,
    pub schema_json: String,
    pub annotations_json: Option<String>,
}

impl Store {
    pub fn add_mcp_server(
        &self,
        id: &str,
        name: &str,
        transport: &str,
        config_json: &str,
    ) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO mcp_servers (id, name, transport, config_json, enabled, created_at)
             VALUES (?1, ?2, ?3, ?4, 1, ?5)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name,
                 transport = excluded.transport, config_json = excluded.config_json",
            params![id, name, transport, config_json, Self::now()],
        )?;
        Ok(())
    }

    pub fn list_mcp_servers(&self) -> Result<Vec<McpServerRow>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, name, transport, config_json, enabled, tools_hash, tools_approved_hash
             FROM mcp_servers ORDER BY name",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(McpServerRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    transport: r.get(2)?,
                    config_json: r.get(3)?,
                    enabled: r.get::<_, i64>(4)? != 0,
                    tools_hash: r.get(5)?,
                    tools_approved_hash: r.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_mcp_server(&self, id: &str) -> Result<Option<McpServerRow>, StoreError> {
        Ok(self.list_mcp_servers()?.into_iter().find(|s| s.id == id))
    }

    pub fn set_mcp_enabled(&self, id: &str, enabled: bool) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "UPDATE mcp_servers SET enabled = ?2 WHERE id = ?1",
            params![id, enabled as i64],
        )?;
        Ok(())
    }

    pub fn remove_mcp_server(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute("DELETE FROM mcp_servers WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Grava o hash atual das definições (sem aprovar).
    pub fn set_mcp_tools_hash(&self, id: &str, hash: &str) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "UPDATE mcp_servers SET tools_hash = ?2 WHERE id = ?1",
            params![id, hash],
        )?;
        Ok(())
    }

    /// Usuário revisou e aprovou as ferramentas atuais.
    pub fn approve_mcp_tools(&self, id: &str, hash: &str) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "UPDATE mcp_servers SET tools_hash = ?2, tools_approved_hash = ?2 WHERE id = ?1",
            params![id, hash],
        )?;
        Ok(())
    }

    /// Substitui o cache de ferramentas de um servidor.
    pub fn replace_mcp_tools(
        &self,
        server_id: &str,
        tools: &[McpToolRow],
    ) -> Result<(), StoreError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM mcp_tools_cache WHERE server_id = ?1",
            [server_id],
        )?;
        for t in tools {
            tx.execute(
                "INSERT INTO mcp_tools_cache (server_id, name, description, schema_json, annotations_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![server_id, t.name, t.description, t.schema_json, t.annotations_json],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list_mcp_tools(&self, server_id: &str) -> Result<Vec<McpToolRow>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT server_id, name, description, schema_json, annotations_json
             FROM mcp_tools_cache WHERE server_id = ?1 ORDER BY name",
        )?;
        let rows = stmt
            .query_map([server_id], |r| {
                Ok(McpToolRow {
                    server_id: r.get(0)?,
                    name: r.get(1)?,
                    description: r.get(2)?,
                    schema_json: r.get(3)?,
                    annotations_json: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Existe algum servidor habilitado aguardando re-aprovação?
    pub fn mcp_servers_needing_approval(&self) -> Result<Vec<McpServerRow>, StoreError> {
        Ok(self
            .list_mcp_servers()?
            .into_iter()
            .filter(|s| s.enabled && s.needs_approval())
            .collect())
    }

    /// Config bruta de um servidor (para o host conectar).
    pub fn mcp_config(&self, id: &str) -> Result<Option<String>, StoreError> {
        let conn = self.conn();
        let cfg = conn
            .query_row(
                "SELECT config_json FROM mcp_servers WHERE id = ?1",
                [id],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(server: &str, name: &str) -> McpToolRow {
        McpToolRow {
            server_id: server.into(),
            name: name.into(),
            description: Some("desc".into()),
            schema_json: r#"{"type":"object"}"#.into(),
            annotations_json: Some(r#"{"readOnlyHint":true}"#.into()),
        }
    }

    #[test]
    fn server_crud_and_tools_cache() {
        let s = Store::open_in_memory().unwrap();
        s.add_mcp_server("gh", "GitHub", "stdio", r#"{"command":"npx"}"#)
            .unwrap();
        assert_eq!(s.list_mcp_servers().unwrap().len(), 1);

        s.replace_mcp_tools("gh", &[tool("gh", "create_issue"), tool("gh", "list_prs")])
            .unwrap();
        assert_eq!(s.list_mcp_tools("gh").unwrap().len(), 2);

        // Substituição troca o conjunto inteiro.
        s.replace_mcp_tools("gh", &[tool("gh", "create_issue")])
            .unwrap();
        assert_eq!(s.list_mcp_tools("gh").unwrap().len(), 1);

        s.set_mcp_enabled("gh", false).unwrap();
        assert!(!s.get_mcp_server("gh").unwrap().unwrap().enabled);

        s.remove_mcp_server("gh").unwrap();
        assert!(s.list_mcp_servers().unwrap().is_empty());
        // Cascade apagou o cache.
        assert!(s.list_mcp_tools("gh").unwrap().is_empty());
    }

    #[test]
    fn tools_hash_gates_reapproval() {
        let s = Store::open_in_memory().unwrap();
        s.add_mcp_server("x", "X", "stdio", "{}").unwrap();

        // Recém-adicionado, sem hash: nada a aprovar ainda.
        assert!(!s.get_mcp_server("x").unwrap().unwrap().needs_approval());

        // Primeira listagem de tools: precisa de aprovação.
        s.set_mcp_tools_hash("x", "h1").unwrap();
        assert!(s.get_mcp_server("x").unwrap().unwrap().needs_approval());
        assert_eq!(s.mcp_servers_needing_approval().unwrap().len(), 1);

        s.approve_mcp_tools("x", "h1").unwrap();
        assert!(!s.get_mcp_server("x").unwrap().unwrap().needs_approval());
        assert!(s.mcp_servers_needing_approval().unwrap().is_empty());

        // Servidor mudou as ferramentas depois de aprovado (rug pull).
        s.set_mcp_tools_hash("x", "h2").unwrap();
        assert!(s.get_mcp_server("x").unwrap().unwrap().needs_approval());

        // Desabilitado não aparece na fila de revisão.
        s.set_mcp_enabled("x", false).unwrap();
        assert!(s.mcp_servers_needing_approval().unwrap().is_empty());
    }
}
