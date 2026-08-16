//! Carregamento da extensão `sqlite-vec` e detecção do que o SQLite oferece.
//!
//! Por que este módulo existe separado: o jeito "documentado" de carregar o
//! sqlite-vec (`sqlite3_auto_extension(Some(transmute(...)))` com o tipo
//! inferido) quebra em rusqlite >= 0.34, porque a assinatura do callback
//! deixou de ser `fn()` e virou `fn(*mut sqlite3, *mut *mut c_char,
//! *const sqlite3_api_routines) -> c_int`. Aqui a conversão é **explícita**,
//! usando o alias [`RawAutoExtension`] do próprio rusqlite — se a assinatura
//! mudar de novo, o compilador acusa em vez de o programa falhar em runtime.
//!
//! A extensão é global ao processo (auto-extension vale para conexões abertas
//! DEPOIS do registro), então o registro acontece uma única vez e toda conexão
//! do RAG passa por [`open_rag_connection`].
//!
//! Nada aqui é fatal: se o vetor não carregar, o índice funciona só com FTS5
//! (degrade gracioso — ver [`RagCapabilities`]).

use rusqlite::Connection;
use rusqlite::auto_extension::{RawAutoExtension, register_auto_extension};
use std::path::Path;
use std::sync::OnceLock;

use crate::RagError;

/// O que este SQLite consegue fazer. A interface usa isso para avisar o
/// usuário quando a busca vetorial não está disponível.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RagCapabilities {
    /// `vec0` disponível (busca por significado).
    pub vector: bool,
    /// FTS5 disponível (busca textual BM25).
    pub fts: bool,
}

/// Registra o sqlite-vec como auto-extension, uma vez por processo.
///
/// Devolve `Err` só se o registro em si falhar; a confirmação de que a
/// extensão realmente funciona vem de [`vec_version`].
fn register_once() -> &'static Result<(), String> {
    static REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();
    REGISTERED.get_or_init(|| {
        // SAFETY: `sqlite3_vec_init` é o ponto de entrada da extensão, ligado
        // estaticamente pelo crate `sqlite-vec` (compilado com SQLITE_CORE).
        // O crate a declara como `extern "C" fn()` por conveniência; a forma
        // real é a de `RawAutoExtension`, então a conversão é feita aqui de
        // maneira explícita — nada de inferência silenciosa.
        let entry: RawAutoExtension =
            unsafe { std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ()) };
        unsafe { register_auto_extension(entry) }.map_err(|e| e.to_string())
    })
}

/// Registra a extensão vetorial no processo. Chame **cedo**, antes de abrir
/// qualquer conexão para o mesmo arquivo `.db`.
///
/// A auto-extension só vale para conexões abertas DEPOIS do registro. Uma
/// conexão aberta antes (a do `lr_store`, por exemplo) não conhece o módulo
/// `vec0` — e, se algum dia precisar reler o esquema completo do banco (um
/// `VACUUM`, um `PRAGMA integrity_check`), tropeça na tabela `rag_vec` com
/// "no such module". Chamar isto no início do app elimina esse buraco.
///
/// Devolve `false` se o registro falhou (o índice ainda funciona, só sem
/// vetor). Chamadas repetidas são no-op.
pub fn register_vector_extension() -> bool {
    register_once().is_ok()
}

/// Versão da extensão vetorial, ou `None` quando ela não carregou.
pub fn vec_version(conn: &Connection) -> Option<String> {
    conn.query_row("SELECT vec_version()", [], |r| r.get::<_, String>(0))
        .ok()
}

/// FTS5 compilado neste SQLite?
pub fn has_fts5(conn: &Connection) -> bool {
    // `pragma_compile_options` é mais barato que tentar criar uma tabela.
    conn.query_row(
        "SELECT COUNT(*) FROM pragma_compile_options WHERE compile_options = 'ENABLE_FTS5'",
        [],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .unwrap_or(false)
}

/// O que esta conexão oferece.
pub fn capabilities(conn: &Connection) -> RagCapabilities {
    RagCapabilities {
        vector: vec_version(conn).is_some(),
        fts: has_fts5(conn),
    }
}

/// Ajustes aplicados a toda conexão do RAG.
///
/// `WAL` porque o banco é compartilhado com o `lr_store`: a indexação escreve
/// por minutos e não pode segurar o cadeado de quem só quer ler uma conversa.
/// `busy_timeout` cobre a janela em que o outro lado está no checkpoint do WAL.
fn tune(conn: &Connection) -> Result<(), RagError> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA busy_timeout=10000;
         PRAGMA synchronous=NORMAL;
         PRAGMA foreign_keys=ON;",
    )?;
    Ok(())
}

/// Abre uma conexão do RAG para o arquivo indicado, já com o sqlite-vec
/// registrado. Use sempre esta função — abrir com `Connection::open` direto
/// deixa a conexão sem `vec0`.
pub fn open_rag_connection(db_path: &Path) -> Result<Connection, RagError> {
    // Falha no registro não impede o índice textual: só derruba o vetor.
    if let Err(e) = register_once() {
        log::warn!("sqlite-vec não pôde ser registrado: {e}");
    }
    let conn = Connection::open(db_path)?;
    tune(&conn)?;
    Ok(conn)
}

/// Conexão em memória (testes e diagnóstico), com as mesmas capacidades.
pub fn open_rag_memory() -> Result<Connection, RagError> {
    if let Err(e) = register_once() {
        log::warn!("sqlite-vec não pôde ser registrado: {e}");
    }
    let conn = Connection::open_in_memory()?;
    // Em memória não há WAL; só o resto dos ajustes.
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TESTE OBRIGATÓRIO: se a extensão não carregar, tudo que depende de
    /// vetor cai silenciosamente para FTS5 — e a gente não perceberia. Este
    /// teste é o alarme.
    #[test]
    fn vec_extension_loads() {
        let conn = open_rag_memory().unwrap();
        let v = vec_version(&conn).expect("vec_version() deve responder: sqlite-vec não carregou");
        assert!(v.starts_with('v'), "versão inesperada: {v}");
    }

    #[test]
    fn fts5_is_available() {
        let conn = open_rag_memory().unwrap();
        assert!(has_fts5(&conn), "FTS5 deveria vir no rusqlite bundled");
        // Prova prática: criar uma tabela FTS5 de verdade.
        conn.execute_batch("CREATE VIRTUAL TABLE t USING fts5(body)")
            .unwrap();
    }

    #[test]
    fn early_registration_is_idempotent() {
        assert!(register_vector_extension());
        assert!(register_vector_extension(), "chamar de novo é no-op");
        // E uma conexão aberta depois enxerga a extensão.
        let conn = Connection::open_in_memory().unwrap();
        assert!(vec_version(&conn).is_some());
    }

    #[test]
    fn capabilities_report_both() {
        let conn = open_rag_memory().unwrap();
        let caps = capabilities(&conn);
        assert!(caps.vector);
        assert!(caps.fts);
    }

    #[test]
    fn vec0_table_accepts_and_ranks_vectors() {
        let conn = open_rag_memory().unwrap();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE v USING vec0(rowid INTEGER PRIMARY KEY, embedding float[3])",
        )
        .unwrap();
        let rows: [(i64, [f32; 3]); 2] = [(1, [1.0, 0.0, 0.0]), (2, [0.0, 1.0, 0.0])];
        for (id, vec) in rows {
            conn.execute(
                "INSERT INTO v (rowid, embedding) VALUES (?1, ?2)",
                rusqlite::params![id, crate::vec_blob(&vec)],
            )
            .unwrap();
        }
        let best: i64 = conn
            .query_row(
                "SELECT rowid FROM v WHERE embedding MATCH ?1 AND k = 1 ORDER BY distance",
                rusqlite::params![crate::vec_blob(&[0.9f32, 0.1, 0.0])],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(best, 1);
    }
}
