//! Persistência local em SQLite: conversas, mensagens, presets e settings.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("erro de banco: {0}")]
    Db(#[from] rusqlite::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRow {
    pub id: i64,
    pub title: String,
    pub model_id: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageRow {
    pub id: i64,
    pub chat_id: i64,
    pub role: String,
    pub content: String,
    pub created_at: i64,
    pub tokens_per_sec: Option<f64>,
}

pub struct Store {
    conn: Mutex<Connection>,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS chats (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    model_id TEXT,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    chat_id INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    tokens_per_sec REAL
);
CREATE INDEX IF NOT EXISTS idx_messages_chat ON messages(chat_id);
CREATE TABLE IF NOT EXISTS presets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, StoreError> {
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    // ------------------------------------------------------------- chats --

    pub fn create_chat(&self, title: &str, model_id: Option<&str>) -> Result<i64, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO chats (title, model_id, created_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![title, model_id, Self::now()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_chats(&self) -> Result<Vec<ChatRow>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT id, title, model_id, created_at FROM chats ORDER BY id DESC")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ChatRow {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    model_id: r.get(2)?,
                    created_at: r.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn delete_chat(&self, chat_id: i64) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM chats WHERE id = ?1", [chat_id])?;
        Ok(())
    }

    // ---------------------------------------------------------- messages --

    pub fn add_message(
        &self,
        chat_id: i64,
        role: &str,
        content: &str,
        tokens_per_sec: Option<f64>,
    ) -> Result<i64, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO messages (chat_id, role, content, created_at, tokens_per_sec)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![chat_id, role, content, Self::now(), tokens_per_sec],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_messages(&self, chat_id: i64) -> Result<Vec<MessageRow>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, chat_id, role, content, created_at, tokens_per_sec
             FROM messages WHERE chat_id = ?1 ORDER BY id",
        )?;
        let rows = stmt
            .query_map([chat_id], |r| {
                Ok(MessageRow {
                    id: r.get(0)?,
                    chat_id: r.get(1)?,
                    role: r.get(2)?,
                    content: r.get(3)?,
                    created_at: r.get(4)?,
                    tokens_per_sec: r.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ---------------------------------------------------------- settings --

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query([key])?;
        Ok(match rows.next()? {
            Some(row) => Some(row.get(0)?),
            None => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_and_message_roundtrip() {
        let s = Store::open_in_memory().unwrap();
        let chat = s
            .create_chat("Primeira conversa", Some("qwen3-8b"))
            .unwrap();
        s.add_message(chat, "user", "Olá!", None).unwrap();
        s.add_message(chat, "assistant", "Oi! Como posso ajudar?", Some(42.5))
            .unwrap();

        let chats = s.list_chats().unwrap();
        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0].title, "Primeira conversa");

        let msgs = s.list_messages(chat).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].tokens_per_sec, Some(42.5));

        s.delete_chat(chat).unwrap();
        assert!(s.list_chats().unwrap().is_empty());
        assert!(s.list_messages(chat).unwrap().is_empty());
    }

    #[test]
    fn settings_upsert() {
        let s = Store::open_in_memory().unwrap();
        assert_eq!(s.get_setting("hf_token").unwrap(), None);
        s.set_setting("hf_token", "abc").unwrap();
        s.set_setting("hf_token", "xyz").unwrap();
        assert_eq!(s.get_setting("hf_token").unwrap().as_deref(), Some("xyz"));
    }
}
