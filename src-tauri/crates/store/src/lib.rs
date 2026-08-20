//! Persistência local em SQLite: conversas, mensagens, presets, settings e o
//! estado do harness agêntico (runs, ferramentas, permissões, MCP, memória).
//!
//! Este arquivo é a fachada: os DAOs do harness moram nos submódulos
//! [`agent`], [`mcp`] e [`memory`], que compartilham a mesma conexão.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

pub mod activity;
pub mod agent;
pub mod automation;
pub mod mcp;
pub mod memory;
pub mod perf;
pub mod tuning;

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
    /// JSON de `ChatParams` desta conversa (NULL = padrões).
    pub params_json: Option<String>,
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
    /// Tokens gerados na resposta (timings do llama-server).
    pub gen_tokens: Option<i64>,
    /// Duração da geração em ms.
    pub gen_ms: Option<i64>,
    /// Execução do agente que produziu esta resposta, quando houve uma.
    /// É o que devolve a trilha de ações à conversa depois de reabri-la:
    /// sem ele a resposta volta sozinha, como se nada tivesse sido feito.
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetRow {
    pub id: i64,
    pub name: String,
    pub json: String,
}

pub struct Store {
    conn: Mutex<Connection>,
}

/// Garante que `table.column` exista, consultando `PRAGMA table_info` e
/// aplicando `ALTER TABLE ... ADD COLUMN` quando o banco veio de uma versão
/// anterior do esquema. No-op se a coluna já existe.
fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    decl: &str,
) -> Result<(), StoreError> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let exists = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .any(|name| name == column);
    if !exists {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"))?;
    }
    Ok(())
}

/// Reata as respostas antigas do agente à execução que as produziu.
///
/// A coluna `messages.run_id` existia desde o começo e nunca foi escrita: as
/// conversas já feitas guardam o texto final e nada do trabalho por trás
/// dele. A janela de uma execução é apertada — a resposta é gravada momentos
/// antes de o run fechar, e a interface não deixa mandar outra coisa
/// enquanto o agente trabalha — então a última resposta dentro dela é a
/// dela. Roda uma vez só; conversas novas já nascem com o ponteiro.
fn backfill_message_runs(conn: &Connection) -> Result<(), StoreError> {
    let feito: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'messages.run_id.backfill'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if feito.is_some() {
        return Ok(());
    }
    conn.execute(
        "UPDATE messages SET run_id = (
             SELECT r.id FROM runs r
              WHERE r.chat_id = messages.chat_id
                AND r.finished_at IS NOT NULL
                AND messages.created_at >= r.created_at
                AND messages.created_at <= r.finished_at + 2
              ORDER BY r.created_at DESC LIMIT 1
         )
         WHERE role = 'assistant' AND run_id IS NULL",
        [],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES ('messages.run_id.backfill', '1')",
        [],
    )?;
    Ok(())
}

/// Carimbo que pode ter sido gravado em segundos (bancos anteriores à
/// migração para milissegundos) devolvido sempre em milissegundos.
///
/// O corte é 2001 em milissegundos: qualquer instante real posterior a ele
/// passa do limite quando lido como segundos, e nenhum instante gravado em
/// segundos o alcança antes do ano 5138.
pub(crate) fn ms_or_secs(v: Option<i64>) -> Option<i64> {
    v.map(|n| {
        if n > 0 && n < 1_000_000_000_000 {
            n * 1000
        } else {
            n
        }
    })
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS chats (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    model_id TEXT,
    created_at INTEGER NOT NULL,
    params_json TEXT
);
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    chat_id INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    tokens_per_sec REAL,
    gen_tokens INTEGER,
    gen_ms INTEGER
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
        // Migrações aditivas: bancos criados antes destas colunas ganham-nas
        // aqui via ALTER TABLE; em bancos novos o SCHEMA já as traz (no-op).
        ensure_column(&conn, "chats", "params_json", "TEXT")?;
        ensure_column(&conn, "messages", "gen_tokens", "INTEGER")?;
        ensure_column(&conn, "messages", "gen_ms", "INTEGER")?;
        // Harness agêntico: a resposta final de um run vira mensagem normal,
        // com ponteiro para o trace completo.
        ensure_column(&conn, "messages", "run_id", "TEXT")?;
        // Com que modelo e com que configuração esta resposta foi gerada.
        // É a medição passiva de desempenho: os tokens/s já eram gravados,
        // faltava saber a que configuração eles pertencem.
        ensure_column(&conn, "messages", "model_id", "TEXT")?;
        ensure_column(&conn, "messages", "profile_key", "TEXT")?;
        agent::init(&conn)?;
        // Plano de trabalho (Scout Rule) de execuções já existentes.
        ensure_column(&conn, "runs", "plan_json", "TEXT")?;
        // Veredito da conferência automática — o cartão do run na conversa
        // mostra passou/reprovou sem recarregar a trilha inteira.
        ensure_column(&conn, "runs", "verify_json", "TEXT")?;
        // Modo de trabalho (Scout Rule) do run — o painel reabre com o certo.
        ensure_column(&conn, "runs", "work_mode", "TEXT")?;
        // Pausa por pergunta: a pergunta estruturada, a resposta da pessoa e
        // o run que retomou. É o que torna a pausa durável (sobrevive a
        // reinício) e respondível de fora do chat (tela de Atividade).
        ensure_column(&conn, "runs", "question_json", "TEXT")?;
        ensure_column(&conn, "runs", "answer", "TEXT")?;
        ensure_column(&conn, "runs", "resumed_by", "TEXT")?;
        mcp::init(&conn)?;
        memory::init(&conn)?;
        automation::init(&conn)?;
        // Depois das outras: a migração da janela por modelo lê `settings`.
        tuning::init(&conn)?;
        perf::init(&conn)?;
        // Respostas gravadas antes de a coluna passar a ser preenchida:
        // reata cada uma à execução que estava rodando quando ela nasceu.
        backfill_message_runs(&conn)?;
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

    /// Instante em MILISSEGUNDOS.
    ///
    /// Duas unidades convivem no banco de propósito: uma execução é datada em
    /// segundos (o que a lista de histórico precisa), mas uma ação dura menos
    /// de um segundo — em segundos toda ferramenta virava "0 ms" e a trilha
    /// mentia sobre o próprio tempo. Quem lê normaliza com [`ms_or_secs`],
    /// porque bancos antigos guardaram segundos nestas colunas.
    pub(crate) fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    /// Conexão compartilhada, tolerante a envenenamento: se outra thread
    /// entrou em pânico segurando o lock, recupera o guard em vez de
    /// propagar o pânico. Cada método daqui faz transações curtas e
    /// completas, então não há invariante quebrada a herdar — e sem isso um
    /// único pânico condenaria todo acesso a banco do processo, inclusive o
    /// registro do fim do run que explicaria o pânico à pessoa.
    pub(crate) fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ------------------------------------------------------------- chats --

    pub fn create_chat(&self, title: &str, model_id: Option<&str>) -> Result<i64, StoreError> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO chats (title, model_id, created_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![title, model_id, Self::now()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_chats(&self) -> Result<Vec<ChatRow>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, title, model_id, created_at, params_json FROM chats ORDER BY id DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ChatRow {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    model_id: r.get(2)?,
                    created_at: r.get(3)?,
                    params_json: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn delete_chat(&self, chat_id: i64) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute("DELETE FROM chats WHERE id = ?1", [chat_id])?;
        Ok(())
    }

    pub fn rename_chat(&self, chat_id: i64, title: &str) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "UPDATE chats SET title = ?1 WHERE id = ?2",
            rusqlite::params![title, chat_id],
        )?;
        Ok(())
    }

    /// Guarda o modelo em uso pela conversa (o seletor pode trocar no meio).
    pub fn set_chat_model(&self, chat_id: i64, model_id: &str) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "UPDATE chats SET model_id = ?1 WHERE id = ?2",
            rusqlite::params![model_id, chat_id],
        )?;
        Ok(())
    }

    pub fn set_chat_params(&self, chat_id: i64, params_json: &str) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "UPDATE chats SET params_json = ?1 WHERE id = ?2",
            rusqlite::params![params_json, chat_id],
        )?;
        Ok(())
    }

    // ---------------------------------------------------------- messages --

    #[allow(clippy::too_many_arguments)]
    pub fn add_message(
        &self,
        chat_id: i64,
        role: &str,
        content: &str,
        tokens_per_sec: Option<f64>,
        gen_tokens: Option<i64>,
        gen_ms: Option<i64>,
        model_id: Option<&str>,
        profile_key: Option<&str>,
        // Execução do agente que gerou a resposta (`None` no chat comum).
        run_id: Option<&str>,
    ) -> Result<i64, StoreError> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO messages (chat_id, role, content, created_at, tokens_per_sec, gen_tokens, gen_ms, model_id, profile_key, run_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                chat_id,
                role,
                content,
                Self::now(),
                tokens_per_sec,
                gen_tokens,
                gen_ms,
                model_id,
                profile_key,
                run_id
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_messages(&self, chat_id: i64) -> Result<Vec<MessageRow>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, chat_id, role, content, created_at, tokens_per_sec, gen_tokens, gen_ms, run_id
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
                    gen_tokens: r.get(6)?,
                    gen_ms: r.get(7)?,
                    run_id: r.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn delete_message(&self, message_id: i64) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute("DELETE FROM messages WHERE id = ?1", [message_id])?;
        Ok(())
    }

    pub fn update_message_content(&self, message_id: i64, content: &str) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "UPDATE messages SET content = ?1 WHERE id = ?2",
            rusqlite::params![content, message_id],
        )?;
        Ok(())
    }

    // ----------------------------------------------------------- presets --

    pub fn list_presets(&self) -> Result<Vec<PresetRow>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id, name, json FROM presets ORDER BY name")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(PresetRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    json: r.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Insere ou atualiza (UPSERT por `name`) e devolve o id da linha.
    pub fn save_preset(&self, name: &str, json: &str) -> Result<i64, StoreError> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO presets (name, json) VALUES (?1, ?2)
             ON CONFLICT(name) DO UPDATE SET json = excluded.json",
            [name, json],
        )?;
        // last_insert_rowid não é confiável no caminho de UPDATE do UPSERT;
        // busca explícita pelo nome (UNIQUE) garante o id correto.
        let id = conn.query_row("SELECT id FROM presets WHERE name = ?1", [name], |r| {
            r.get(0)
        })?;
        Ok(id)
    }

    pub fn delete_preset(&self, id: i64) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute("DELETE FROM presets WHERE id = ?1", [id])?;
        Ok(())
    }

    // ---------------------------------------------------------- settings --

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        let conn = self.conn();
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

    /// Um pânico segurando a conexão não pode condenar o banco: o processo
    /// precisa continuar gravando — é assim que o fim de um run vira registro
    /// mesmo quando algo explodiu no meio dele.
    #[test]
    fn panico_com_o_lock_nao_condena_o_banco() {
        let s = Store::open_in_memory().unwrap();
        std::thread::scope(|scope| {
            let apavorada = scope.spawn(|| {
                let _guard = s.conn();
                panic!("pânico proposital segurando a conexão");
            });
            assert!(apavorada.join().is_err(), "a thread deveria ter panicado");
        });
        let chat = s.create_chat("depois do pânico", None).unwrap();
        assert!(s.list_chats().unwrap().iter().any(|c| c.id == chat));
    }

    #[test]
    fn chat_and_message_roundtrip() {
        let s = Store::open_in_memory().unwrap();
        let chat = s
            .create_chat("Primeira conversa", Some("qwen3-8b"))
            .unwrap();
        s.add_message(chat, "user", "Olá!", None, None, None, None, None, None)
            .unwrap();
        s.add_message(
            chat,
            "assistant",
            "Oi! Como posso ajudar?",
            Some(42.5),
            Some(128),
            Some(3012),
            Some("Qwen3-8B-Q4_K_M.gguf"),
            Some("abc123"),
            None,
        )
        .unwrap();

        let chats = s.list_chats().unwrap();
        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0].title, "Primeira conversa");
        assert_eq!(chats[0].params_json, None);

        let msgs = s.list_messages(chat).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].gen_tokens, None);
        assert_eq!(msgs[1].tokens_per_sec, Some(42.5));
        assert_eq!(msgs[1].gen_tokens, Some(128));
        assert_eq!(msgs[1].gen_ms, Some(3012));

        s.delete_chat(chat).unwrap();
        assert!(s.list_chats().unwrap().is_empty());
        assert!(s.list_messages(chat).unwrap().is_empty());
    }

    #[test]
    fn rename_chat_and_set_params() {
        let s = Store::open_in_memory().unwrap();
        let chat = s.create_chat("Sem título", None).unwrap();

        s.rename_chat(chat, "Título novo").unwrap();
        s.set_chat_params(chat, r#"{"temperature":0.5}"#).unwrap();

        let chats = s.list_chats().unwrap();
        assert_eq!(chats[0].title, "Título novo");
        assert_eq!(
            chats[0].params_json.as_deref(),
            Some(r#"{"temperature":0.5}"#)
        );
    }

    /// A resposta de uma execução guarda QUAL execução a produziu: é o
    /// ponteiro que devolve a trilha de ações à conversa depois de reabri-la.
    #[test]
    fn an_agent_answer_keeps_the_run_it_came_from() {
        let s = Store::open_in_memory().unwrap();
        let chat = s.create_chat("Conversa", None).unwrap();
        s.add_message(
            chat,
            "user",
            "faça algo",
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        s.add_message(
            chat,
            "assistant",
            "pronto",
            None,
            Some(12),
            Some(3000),
            Some("modelo"),
            None,
            Some("run-42"),
        )
        .unwrap();

        let msgs = s.list_messages(chat).unwrap();
        assert_eq!(msgs[0].run_id, None);
        assert_eq!(msgs[1].run_id.as_deref(), Some("run-42"));
    }

    /// Conversa feita antes desta coluna existir volta com a trilha no lugar.
    #[test]
    fn old_answers_are_tied_back_to_their_run() {
        use lr_types::agent::{RunMode, RunStatus};

        let s = Store::open_in_memory().unwrap();
        let chat = s.create_chat("Conversa", None).unwrap();
        s.create_run("r1", Some(chat), "m", RunMode::Yolo, true, None, "faça")
            .unwrap();
        let msg = s
            .add_message(
                chat,
                "assistant",
                "pronto",
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        s.finish_run("r1", RunStatus::Done, "ok", "{}").unwrap();

        {
            let conn = s.conn();
            // O `init` já rodou o backfill neste banco novo: desfaz a marca
            // para exercitar a migração como ela roda no banco de alguém.
            conn.execute(
                "DELETE FROM settings WHERE key = 'messages.run_id.backfill'",
                [],
            )
            .unwrap();
            backfill_message_runs(&conn).unwrap();
        }

        let msgs = s.list_messages(chat).unwrap();
        assert_eq!(msgs[0].id, msg);
        assert_eq!(msgs[0].run_id.as_deref(), Some("r1"));
    }

    #[test]
    fn update_and_delete_message() {
        let s = Store::open_in_memory().unwrap();
        let chat = s.create_chat("Conversa", None).unwrap();
        let m1 = s
            .add_message(chat, "user", "rascunho", None, None, None, None, None, None)
            .unwrap();
        let m2 = s
            .add_message(
                chat,
                "assistant",
                "resposta",
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();

        s.update_message_content(m1, "versão final").unwrap();
        s.delete_message(m2).unwrap();

        let msgs = s.list_messages(chat).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].id, m1);
        assert_eq!(msgs[0].content, "versão final");
    }

    #[test]
    fn presets_roundtrip_and_upsert() {
        let s = Store::open_in_memory().unwrap();
        assert!(s.list_presets().unwrap().is_empty());

        let id = s.save_preset("Criativo", r#"{"temperature":1.2}"#).unwrap();
        // Mesmo nome: atualiza o json e mantém o mesmo id (UPSERT).
        let id2 = s.save_preset("Criativo", r#"{"temperature":1.5}"#).unwrap();
        assert_eq!(id, id2);

        let other = s.save_preset("Preciso", r#"{"temperature":0.2}"#).unwrap();
        assert_ne!(id, other);

        let presets = s.list_presets().unwrap();
        assert_eq!(presets.len(), 2);
        let criativo = presets.iter().find(|p| p.name == "Criativo").unwrap();
        assert_eq!(criativo.id, id);
        assert_eq!(criativo.json, r#"{"temperature":1.5}"#);

        s.delete_preset(id).unwrap();
        let presets = s.list_presets().unwrap();
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].name, "Preciso");
    }

    #[test]
    fn migration_adds_missing_columns() {
        // Esquema ANTIGO à mão, sem params_json/gen_tokens/gen_ms — simula um
        // banco criado por uma versão anterior do app.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE chats (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                model_id TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chat_id INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                tokens_per_sec REAL
            );
            CREATE INDEX idx_messages_chat ON messages(chat_id);
            CREATE TABLE presets (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                json TEXT NOT NULL
            );
            CREATE TABLE settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            "#,
        )
        .unwrap();
        // Dado pré-existente, como num banco real em uso.
        conn.execute(
            "INSERT INTO chats (title, created_at) VALUES ('antiga', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (chat_id, role, content, created_at) VALUES (1, 'user', 'oi', 1)",
            [],
        )
        .unwrap();

        // Rotina de init/migração sobre o banco antigo.
        let s = Store::init(conn).unwrap();

        // Linhas antigas continuam legíveis, com as colunas novas em NULL.
        let chats = s.list_chats().unwrap();
        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0].title, "antiga");
        assert_eq!(chats[0].params_json, None);
        let msgs = s.list_messages(chats[0].id).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].gen_tokens, None);
        assert_eq!(msgs[0].gen_ms, None);

        // Insert/select usando as colunas novas funciona após a migração.
        s.set_chat_params(chats[0].id, r#"{"topK":40}"#).unwrap();
        s.add_message(
            chats[0].id,
            "assistant",
            "olá!",
            Some(10.0),
            Some(7),
            Some(700),
            None,
            None,
            None,
        )
        .unwrap();
        let chats = s.list_chats().unwrap();
        assert_eq!(chats[0].params_json.as_deref(), Some(r#"{"topK":40}"#));
        let msgs = s.list_messages(chats[0].id).unwrap();
        assert_eq!(msgs[1].gen_tokens, Some(7));
        assert_eq!(msgs[1].gen_ms, Some(700));

        // Reaplicar a migração num banco já migrado é no-op seguro.
        {
            let conn = s.conn();
            ensure_column(&conn, "chats", "params_json", "TEXT").unwrap();
        }
        assert_eq!(s.list_chats().unwrap().len(), 1);
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
