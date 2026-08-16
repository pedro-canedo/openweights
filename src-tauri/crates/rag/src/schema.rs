//! Tabelas do índice, criadas na conexão própria do RAG.
//!
//! Tudo é prefixado com `rag_` porque divide arquivo com o `lr_store`. Todas
//! as tabelas carregam a coluna `workspace` (caminho absoluto normalizado):
//! o app abre um projeto de cada vez, mas o banco é único e guarda o índice de
//! todos — trocar de pasta não pode devolver trecho da pasta anterior.
//!
//! Três estruturas trabalham juntas:
//! - `rag_catalog` — um registro por arquivo, com `hash` e `mtime`. É o que
//!   torna a atualização incremental: sem mudança, nem se abre o arquivo.
//! - `rag_chunks` + `rag_chunks_fts` — os trechos e o índice BM25 sobre eles.
//!   O FTS5 é *external content* (`content='rag_chunks'`): o texto vive uma vez
//!   só, o índice guarda apenas os termos. Gatilhos mantêm os dois em sincronia
//!   — assim ninguém precisa lembrar de atualizar o FTS na mão.
//! - `rag_vec` — tabela `vec0` com os embeddings. A dimensão faz parte da
//!   declaração da tabela, então trocar de modelo de embedding obriga a
//!   recriá-la (ver [`ensure_vec_table`]).

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::{RagCapabilities, RagError};

/// Versão do esquema do RAG. Subir isto apaga o índice antigo (é cache: dá
/// para reconstruir do zero, não vale a pena escrever migração).
pub const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS rag_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS rag_catalog (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace  TEXT NOT NULL,
    path       TEXT NOT NULL,
    hash       TEXT NOT NULL,
    mtime      INTEGER NOT NULL,
    size       INTEGER NOT NULL,
    chunks     INTEGER NOT NULL DEFAULT 0,
    indexed_at INTEGER NOT NULL,
    UNIQUE(workspace, path)
);

CREATE TABLE IF NOT EXISTS rag_chunks (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    file_id    INTEGER NOT NULL REFERENCES rag_catalog(id) ON DELETE CASCADE,
    workspace  TEXT NOT NULL,
    path       TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line   INTEGER NOT NULL,
    embedded   INTEGER NOT NULL DEFAULT 0,
    content    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_rag_chunks_file ON rag_chunks(file_id);
CREATE INDEX IF NOT EXISTS idx_rag_chunks_ws   ON rag_chunks(workspace);

CREATE VIRTUAL TABLE IF NOT EXISTS rag_chunks_fts USING fts5(
    content,
    path,
    content='rag_chunks',
    content_rowid='id',
    tokenize="unicode61 remove_diacritics 2"
);

CREATE TRIGGER IF NOT EXISTS rag_chunks_ai AFTER INSERT ON rag_chunks BEGIN
    INSERT INTO rag_chunks_fts(rowid, content, path)
    VALUES (new.id, new.content, new.path);
END;
CREATE TRIGGER IF NOT EXISTS rag_chunks_ad AFTER DELETE ON rag_chunks BEGIN
    INSERT INTO rag_chunks_fts(rag_chunks_fts, rowid, content, path)
    VALUES ('delete', old.id, old.content, old.path);
END;
CREATE TRIGGER IF NOT EXISTS rag_chunks_au AFTER UPDATE ON rag_chunks BEGIN
    INSERT INTO rag_chunks_fts(rag_chunks_fts, rowid, content, path)
    VALUES ('delete', old.id, old.content, old.path);
    INSERT INTO rag_chunks_fts(rowid, content, path)
    VALUES (new.id, new.content, new.path);
END;
"#;

/// Cria (ou confere) as tabelas. Idempotente.
pub fn ensure_schema(conn: &Connection) -> Result<(), RagError> {
    // Banco de versão anterior: descarta e recomeça. O índice é cache.
    let current: Option<i64> = conn
        .query_row(
            "SELECT value FROM rag_meta WHERE key = 'schema_version'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .unwrap_or(None)
        .and_then(|s| s.parse().ok());
    if matches!(current, Some(v) if v != SCHEMA_VERSION) {
        drop_everything(conn)?;
    }

    conn.execute_batch(SCHEMA)?;
    meta_set(conn, "schema_version", &SCHEMA_VERSION.to_string())?;
    Ok(())
}

fn drop_everything(conn: &Connection) -> Result<(), RagError> {
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS rag_chunks_ai;
         DROP TRIGGER IF EXISTS rag_chunks_ad;
         DROP TRIGGER IF EXISTS rag_chunks_au;
         DROP TABLE IF EXISTS rag_chunks_fts;
         DROP TABLE IF EXISTS rag_vec;
         DROP TABLE IF EXISTS rag_chunks;
         DROP TABLE IF EXISTS rag_catalog;
         DELETE FROM rag_meta;",
    )?;
    Ok(())
}

// ------------------------------------------------------------------ meta ---

pub fn meta_get(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM rag_meta WHERE key = ?1", [key], |r| {
        r.get::<_, String>(0)
    })
    .optional()
    .ok()
    .flatten()
}

pub fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<(), RagError> {
    conn.execute(
        "INSERT INTO rag_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}

// ------------------------------------------------------------- vec0 table --

/// Garante a tabela vetorial com a dimensão pedida.
///
/// A dimensão é parte da declaração do `vec0`, então não há como "alterar":
/// trocar de modelo de embedding (ou de dimensão) recria a tabela e zera a
/// marca `embedded` dos trechos, para a próxima indexação refazer os vetores.
/// Devolve `true` quando a tabela foi recriada.
pub fn ensure_vec_table(conn: &Connection, dim: usize, model: &str) -> Result<bool, RagError> {
    let prev_dim = meta_get(conn, "embed_dim").and_then(|s| s.parse::<usize>().ok());
    let prev_model = meta_get(conn, "embed_model");
    let exists = table_exists(conn, "rag_vec")?;
    let compatible =
        exists && prev_dim == Some(dim) && prev_model.as_deref() == Some(model) && dim > 0;

    if compatible {
        return Ok(false);
    }

    conn.execute_batch("DROP TABLE IF EXISTS rag_vec")?;
    // `chunk_id` é o rowid da tabela; `workspace` é PARTITION KEY para o KNN
    // não vazar trecho de outro projeto (e ficar mais rápido).
    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE rag_vec USING vec0(
             chunk_id INTEGER PRIMARY KEY,
             workspace TEXT PARTITION KEY,
             embedding FLOAT[{dim}]
         )"
    ))?;
    conn.execute("UPDATE rag_chunks SET embedded = 0", [])?;
    meta_set(conn, "embed_dim", &dim.to_string())?;
    meta_set(conn, "embed_model", model)?;
    Ok(true)
}

pub fn table_exists(conn: &Connection, name: &str) -> Result<bool, RagError> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE name = ?1",
        [name],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

// ----------------------------------------------------------------- limpeza --

/// Remove um arquivo do índice (trechos, vetores e catálogo).
pub fn delete_file(conn: &Connection, file_id: i64) -> Result<(), RagError> {
    delete_vectors_of_file(conn, file_id)?;
    // ON DELETE CASCADE cuida de `rag_chunks`; os gatilhos limpam o FTS.
    conn.execute("DELETE FROM rag_catalog WHERE id = ?1", [file_id])?;
    Ok(())
}

/// Apaga só os vetores dos trechos de um arquivo (o `vec0` não participa de
/// chave estrangeira, então a limpeza é explícita).
pub fn delete_vectors_of_file(conn: &Connection, file_id: i64) -> Result<(), RagError> {
    if !table_exists(conn, "rag_vec")? {
        return Ok(());
    }
    let ids: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM rag_chunks WHERE file_id = ?1")?;
        let rows = stmt.query_map([file_id], |r| r.get::<_, i64>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let mut del = conn.prepare("DELETE FROM rag_vec WHERE chunk_id = ?1")?;
    for id in ids {
        // Trecho sem vetor: o delete não acha nada e segue em frente.
        let _ = del.execute([id]);
    }
    Ok(())
}

/// Apaga tudo que foi indexado de um projeto.
pub fn clear_workspace(conn: &Connection, workspace: &str) -> Result<(), RagError> {
    if table_exists(conn, "rag_vec")? {
        let ids: Vec<i64> = {
            let mut stmt = conn.prepare("SELECT id FROM rag_chunks WHERE workspace = ?1")?;
            let rows = stmt.query_map([workspace], |r| r.get::<_, i64>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let mut del = conn.prepare("DELETE FROM rag_vec WHERE chunk_id = ?1")?;
        for id in ids {
            let _ = del.execute([id]);
        }
    }
    conn.execute("DELETE FROM rag_catalog WHERE workspace = ?1", [workspace])?;
    // Órfãos não deveriam existir (CASCADE), mas bancos antigos podem ter.
    conn.execute("DELETE FROM rag_chunks WHERE workspace = ?1", [workspace])?;
    Ok(())
}

// ------------------------------------------------------------------ status --

/// Estado do índice de um projeto, como a interface mostra.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RagStatus {
    /// O que este SQLite oferece (vetor e/ou texto).
    pub capabilities: RagCapabilities,
    /// Já existe índice para este projeto?
    pub indexed: bool,
    pub files: i64,
    pub chunks: i64,
    /// Trechos que já têm vetor (0 = busca só textual).
    pub vectors: i64,
    /// Modelo usado nos vetores gravados.
    pub embed_model: Option<String>,
    pub embed_dim: Option<i64>,
    /// Há um modelo de embedding configurado agora?
    pub embed_model_configured: bool,
    /// Momento da última atualização (epoch em segundos).
    pub updated_at: Option<i64>,
    /// Indexação em andamento?
    pub indexing: bool,
}

pub fn status(conn: &Connection, workspace: &str) -> Result<RagStatus, RagError> {
    let (files, updated_at): (i64, Option<i64>) = conn.query_row(
        "SELECT COUNT(*), MAX(indexed_at) FROM rag_catalog WHERE workspace = ?1",
        [workspace],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let (chunks, vectors): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(embedded), 0) FROM rag_chunks WHERE workspace = ?1",
        [workspace],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(RagStatus {
        capabilities: crate::vec_init::capabilities(conn),
        indexed: files > 0,
        files,
        chunks,
        vectors,
        embed_model: meta_get(conn, "embed_model"),
        embed_dim: meta_get(conn, "embed_dim").and_then(|s| s.parse().ok()),
        embed_model_configured: false,
        updated_at,
        indexing: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::open_rag_memory;
    use rusqlite::params;

    fn insert_chunk(conn: &Connection, ws: &str, path: &str, body: &str) -> i64 {
        conn.execute(
            "INSERT INTO rag_catalog (workspace, path, hash, mtime, size, chunks, indexed_at)
             VALUES (?1, ?2, 'h', 1, 1, 1, 1)
             ON CONFLICT(workspace, path) DO UPDATE SET hash = 'h'",
            params![ws, path],
        )
        .unwrap();
        let file_id: i64 = conn
            .query_row(
                "SELECT id FROM rag_catalog WHERE workspace = ?1 AND path = ?2",
                params![ws, path],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO rag_chunks (file_id, workspace, path, start_line, end_line, content)
             VALUES (?1, ?2, ?3, 1, 5, ?4)",
            params![file_id, ws, path, body],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn schema_is_idempotent() {
        let conn = open_rag_memory().unwrap();
        ensure_schema(&conn).unwrap();
        ensure_schema(&conn).unwrap();
        assert!(table_exists(&conn, "rag_catalog").unwrap());
        assert!(table_exists(&conn, "rag_chunks_fts").unwrap());
    }

    #[test]
    fn fts_index_follows_chunk_writes() {
        let conn = open_rag_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let id = insert_chunk(
            &conn,
            "/p",
            "src/a.rs",
            "valida o token de sessao do usuario",
        );

        let hits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM rag_chunks_fts WHERE rag_chunks_fts MATCH 'token'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1);

        // Gatilho de DELETE tem que tirar do índice também.
        conn.execute("DELETE FROM rag_chunks WHERE id = ?1", [id])
            .unwrap();
        let hits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM rag_chunks_fts WHERE rag_chunks_fts MATCH 'token'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 0);
    }

    #[test]
    fn vec_table_is_recreated_when_dimension_changes() {
        let conn = open_rag_memory().unwrap();
        ensure_schema(&conn).unwrap();
        assert!(ensure_vec_table(&conn, 4, "m1").unwrap());
        // Mesma dimensão e modelo: não mexe.
        assert!(!ensure_vec_table(&conn, 4, "m1").unwrap());
        // Modelo diferente: recria.
        assert!(ensure_vec_table(&conn, 4, "m2").unwrap());
        // Dimensão diferente: recria.
        assert!(ensure_vec_table(&conn, 8, "m2").unwrap());
        assert_eq!(meta_get(&conn, "embed_dim").as_deref(), Some("8"));
    }

    #[test]
    fn vec_partition_keeps_workspaces_apart() {
        let conn = open_rag_memory().unwrap();
        ensure_schema(&conn).unwrap();
        ensure_vec_table(&conn, 3, "m").unwrap();

        let a = insert_chunk(&conn, "/pa", "a.rs", "alpha");
        let b = insert_chunk(&conn, "/pb", "b.rs", "beta");
        for (id, ws, v) in [
            (a, "/pa", [1.0f32, 0.0, 0.0]),
            (b, "/pb", [1.0f32, 0.0, 0.0]),
        ] {
            conn.execute(
                "INSERT INTO rag_vec (chunk_id, workspace, embedding) VALUES (?1, ?2, ?3)",
                params![id, ws, crate::vec_blob(&v)],
            )
            .unwrap();
        }

        let mut stmt = conn
            .prepare(
                "SELECT chunk_id FROM rag_vec
                 WHERE embedding MATCH ?1 AND workspace = ?2 AND k = 5",
            )
            .unwrap();
        let found: Vec<i64> = stmt
            .query_map(params![crate::vec_blob(&[1.0f32, 0.0, 0.0]), "/pa"], |r| {
                r.get(0)
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(found, vec![a], "KNN não pode vazar trecho de outro projeto");
    }

    #[test]
    fn clear_workspace_removes_only_that_project() {
        let conn = open_rag_memory().unwrap();
        ensure_schema(&conn).unwrap();
        ensure_vec_table(&conn, 3, "m").unwrap();
        let a = insert_chunk(&conn, "/pa", "a.rs", "alpha");
        insert_chunk(&conn, "/pb", "b.rs", "beta");
        conn.execute(
            "INSERT INTO rag_vec (chunk_id, workspace, embedding) VALUES (?1, '/pa', ?2)",
            params![a, crate::vec_blob(&[1.0f32, 0.0, 0.0])],
        )
        .unwrap();

        clear_workspace(&conn, "/pa").unwrap();

        assert_eq!(status(&conn, "/pa").unwrap().files, 0);
        assert_eq!(status(&conn, "/pb").unwrap().files, 1);
        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM rag_vec", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn status_reports_counts() {
        let conn = open_rag_memory().unwrap();
        ensure_schema(&conn).unwrap();
        insert_chunk(&conn, "/p", "a.rs", "um");
        insert_chunk(&conn, "/p", "b.rs", "dois");
        let st = status(&conn, "/p").unwrap();
        assert!(st.indexed);
        assert_eq!(st.files, 2);
        assert_eq!(st.chunks, 2);
        assert_eq!(st.vectors, 0);
        assert!(st.capabilities.fts);
    }
}
