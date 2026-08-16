//! Indexação incremental do projeto.
//!
//! O padrão é o do Continue.dev: um catálogo com `hash` e `mtime` por arquivo.
//! Reindexar um projeto grande do zero leva minutos; a segunda vez tem que
//! levar segundos, porque quase nada mudou. A comparação é em dois níveis:
//!
//! 1. `mtime` + `size` iguais → nem abre o arquivo (o caso comum, e o barato).
//! 2. abriu e o `hash` bate → só atualiza o `mtime` (salvar sem alterar, `git
//!    checkout` de volta, formatador que não mudou nada).
//!
//! Só o que sobrou é picado e reindexado. Arquivo sumido sai do índice.
//!
//! Os vetores vêm numa **segunda passada**: primeiro o texto inteiro entra no
//! FTS5 (rápido, sem rede), depois os embeddings são pedidos em lote ao
//! llama-server. Assim a busca textual já funciona enquanto o vetor engatinha
//! — e se não houver modelo de embedding, a passada simplesmente não acontece.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::chunk::{ChunkOptions, chunk_text};
use crate::{EmbedConfig, RagError, schema, walker, workspace_key};

/// Quantos trechos por requisição de embedding. Lote grande economiza
/// ida e volta, mas estoura o contexto do modelo de embedding; 16 trechos de
/// ~512 tokens cabem com folga nos 8k típicos.
const EMBED_BATCH: usize = 16;

/// Arquivos por transação. Uma transação por arquivo faria um `fsync` a cada
/// um; uma só para o projeto inteiro perderia tudo num cancelamento. 64 é o
/// meio-termo: rápido e com progresso durável.
const FILES_PER_TX: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IndexPhase {
    /// Varrendo a pasta.
    Scanning,
    /// Lendo, picando e gravando trechos.
    Indexing,
    /// Pedindo vetores ao llama-server.
    Embedding,
    Done,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexProgress {
    pub phase: IndexPhase,
    pub done: usize,
    pub total: usize,
    /// Arquivo (ou trecho) da vez, relativo ao projeto.
    pub path: String,
}

/// Callback de progresso. `Arc` porque o mesmo canal serve à interface e ao
/// log, e a indexação pode ser movida para outra tarefa.
pub type ProgressCallback = Arc<dyn Fn(IndexProgress) + Send + Sync>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexReport {
    /// Arquivos vistos na varredura (já sem os excluídos).
    pub scanned: usize,
    /// Arquivos lidos e reindexados nesta rodada.
    pub indexed: usize,
    /// Arquivos inalterados desde a última vez.
    pub skipped: usize,
    /// Arquivos que sumiram do disco e saíram do índice.
    pub removed: usize,
    /// Trechos gravados nesta rodada.
    pub chunks: usize,
    /// Trechos que ganharam vetor nesta rodada.
    pub embedded: usize,
    /// A rodada teve busca vetorial (falso = índice só textual).
    pub vector: bool,
    pub cancelled: bool,
}

pub struct IndexOptions {
    pub embed: EmbedConfig,
    pub cancel: Arc<AtomicBool>,
    pub progress: Option<ProgressCallback>,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            embed: EmbedConfig::default(),
            cancel: Arc::new(AtomicBool::new(false)),
            progress: None,
        }
    }
}

impl IndexOptions {
    fn report(&self, phase: IndexPhase, done: usize, total: usize, path: &str) {
        if let Some(cb) = &self.progress {
            cb(IndexProgress {
                phase,
                done,
                total,
                path: path.to_string(),
            });
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

/// Linha do catálogo, do jeito que a comparação incremental precisa.
struct CatalogRow {
    id: i64,
    hash: String,
    mtime: i64,
    size: i64,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Indexa ou atualiza o projeto. Cancelamento não é erro: devolve o relatório
/// parcial com `cancelled = true`, porque o que já entrou continua valendo.
pub async fn index_workspace(
    db_path: &Path,
    workspace: &Path,
    opts: IndexOptions,
) -> Result<IndexReport, RagError> {
    let ws = workspace_key(workspace);
    let mut conn = crate::vec_init::open_rag_connection(db_path)?;
    schema::ensure_schema(&conn)?;

    opts.report(IndexPhase::Scanning, 0, 0, "");
    let files = walker::scan_workspace(workspace)?;

    // Dimensão do vetor: só o servidor sabe. Uma requisição mínima descobre —
    // e de quebra confirma que o modelo de embedding responde mesmo.
    let mut report = IndexReport {
        scanned: files.len(),
        ..Default::default()
    };
    let embedder = probe_embedder(&opts.embed).await;
    if let Some((_, model, dim)) = &embedder {
        schema::ensure_vec_table(&conn, *dim, model)?;
        report.vector = true;
    }

    // Passada 1 — texto. Sem rede, tudo local, transação por lote.
    if index_text(&mut conn, &ws, &files, &opts, &mut report)? {
        report.cancelled = true;
    }

    // Passada 2 — vetores dos trechos ainda sem embedding.
    if let Some((client, model, _)) = embedder
        && !report.cancelled
    {
        embed_pending(&mut conn, &ws, &client, &model, &opts, &mut report).await?;
    }

    opts.report(
        if report.cancelled {
            IndexPhase::Cancelled
        } else {
            IndexPhase::Done
        },
        report.indexed + report.skipped,
        report.scanned,
        "",
    );
    Ok(report)
}

/// Descobre a dimensão do modelo de embedding. `None` = índice só textual.
async fn probe_embedder(cfg: &EmbedConfig) -> Option<(lr_engine::LlamaClient, String, usize)> {
    let (client, model) = cfg.client()?;
    match client.embeddings(&model, &["dimensao".to_string()]).await {
        Ok(v) => match v.first() {
            Some(first) if !first.is_empty() => Some((client, model, first.len())),
            _ => {
                log::warn!("modelo de embedding `{model}` devolveu vetor vazio");
                None
            }
        },
        Err(e) => {
            // Servidor fora do ar ou modelo não registrado no Router: o índice
            // segue com FTS5 e a interface avisa (agent.rag.vectorOff).
            log::warn!("embeddings indisponíveis ({e}); índice ficará só textual");
            None
        }
    }
}

/// Passada de texto. Devolve `true` se foi cancelada no meio.
///
/// Síncrona de propósito: sem `await` aqui, dá para usar `Transaction` do
/// rusqlite (que empresta a conexão) sem tropeçar no `Send` do futuro.
fn index_text(
    conn: &mut Connection,
    ws: &str,
    files: &[walker::FileEntry],
    opts: &IndexOptions,
    report: &mut IndexReport,
) -> Result<bool, RagError> {
    let catalog = load_catalog(conn, ws)?;
    let on_disk: std::collections::HashSet<&str> =
        files.iter().map(|f| f.rel_path.as_str()).collect();

    // Sumiu do disco: sai do índice antes de qualquer coisa, para uma busca
    // no meio da indexação não citar arquivo que não existe mais.
    for (path, row) in &catalog {
        if !on_disk.contains(path.as_str()) {
            schema::delete_file(conn, row.id)?;
            report.removed += 1;
        }
    }

    let total = files.len();
    let opts_chunk = ChunkOptions::default();
    let mut processed = 0usize;

    for batch in files.chunks(FILES_PER_TX) {
        if opts.cancelled() {
            return Ok(true);
        }
        let tx = conn.transaction()?;
        for entry in batch {
            processed += 1;
            let known = catalog.get(&entry.rel_path);

            // Nível 1: metadados iguais — nem abre o arquivo.
            if let Some(row) = known
                && row.mtime == entry.mtime
                && row.size == entry.size as i64
            {
                report.skipped += 1;
                continue;
            }

            let Some(text) = walker::read_text(&entry.abs_path) else {
                // Virou binário (ou sumiu entre a varredura e a leitura).
                if let Some(row) = known {
                    schema::delete_file(&tx, row.id)?;
                    report.removed += 1;
                }
                continue;
            };
            let hash = walker::content_hash(&text);

            // Nível 2: conteúdo igual — só acerta o mtime e segue.
            if let Some(row) = known
                && row.hash == hash
            {
                tx.execute(
                    "UPDATE rag_catalog SET mtime = ?1, size = ?2 WHERE id = ?3",
                    params![entry.mtime, entry.size as i64, row.id],
                )?;
                report.skipped += 1;
                continue;
            }

            let chunks = chunk_text(&text, &opts_chunk);
            if chunks.is_empty() {
                if let Some(row) = known {
                    schema::delete_file(&tx, row.id)?;
                    report.removed += 1;
                }
                continue;
            }

            // Reindexar = apagar os trechos antigos e gravar os novos. Tentar
            // casar trecho a trecho custaria mais que refazer.
            let file_id = upsert_catalog(&tx, ws, entry, &hash, chunks.len())?;
            schema::delete_vectors_of_file(&tx, file_id)?;
            tx.execute("DELETE FROM rag_chunks WHERE file_id = ?1", [file_id])?;

            {
                let mut ins = tx.prepare_cached(
                    "INSERT INTO rag_chunks
                        (file_id, workspace, path, start_line, end_line, embedded, content)
                     VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
                )?;
                for c in &chunks {
                    ins.execute(params![
                        file_id,
                        ws,
                        entry.rel_path,
                        c.start_line,
                        c.end_line,
                        c.content
                    ])?;
                }
            }

            report.indexed += 1;
            report.chunks += chunks.len();
        }
        tx.commit()?;

        let last = batch.last().map(|e| e.rel_path.as_str()).unwrap_or("");
        opts.report(IndexPhase::Indexing, processed, total, last);
    }

    Ok(false)
}

fn load_catalog(conn: &Connection, ws: &str) -> Result<HashMap<String, CatalogRow>, RagError> {
    let mut stmt =
        conn.prepare("SELECT id, path, hash, mtime, size FROM rag_catalog WHERE workspace = ?1")?;
    let rows = stmt.query_map([ws], |r| {
        Ok((
            r.get::<_, String>(1)?,
            CatalogRow {
                id: r.get(0)?,
                hash: r.get(2)?,
                mtime: r.get(3)?,
                size: r.get(4)?,
            },
        ))
    })?;
    let mut map = HashMap::new();
    for row in rows {
        let (path, cat) = row?;
        map.insert(path, cat);
    }
    Ok(map)
}

fn upsert_catalog(
    conn: &Connection,
    ws: &str,
    entry: &walker::FileEntry,
    hash: &str,
    chunks: usize,
) -> Result<i64, RagError> {
    conn.execute(
        "INSERT INTO rag_catalog (workspace, path, hash, mtime, size, chunks, indexed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(workspace, path) DO UPDATE SET
             hash = excluded.hash,
             mtime = excluded.mtime,
             size = excluded.size,
             chunks = excluded.chunks,
             indexed_at = excluded.indexed_at",
        params![
            ws,
            entry.rel_path,
            hash,
            entry.mtime,
            entry.size as i64,
            chunks as i64,
            now()
        ],
    )?;
    let id = conn.query_row(
        "SELECT id FROM rag_catalog WHERE workspace = ?1 AND path = ?2",
        params![ws, entry.rel_path],
        |r| r.get(0),
    )?;
    Ok(id)
}

/// Segunda passada: pede vetor para todo trecho com `embedded = 0`.
///
/// Duas regras mantêm este futuro `Send` (exigência do runtime do Tauri):
/// nenhum `Statement` sobrevive ao `await` — os dados saem do banco para um
/// `Vec` antes da chamada HTTP e voltam depois — e a conexão entra por
/// `&mut`, não por `&`, porque o `Connection` do rusqlite é `Send` mas **não**
/// é `Sync` (só a referência exclusiva atravessa a fronteira de tarefa).
async fn embed_pending(
    conn: &mut Connection,
    ws: &str,
    client: &lr_engine::LlamaClient,
    model: &str,
    opts: &IndexOptions,
    report: &mut IndexReport,
) -> Result<(), RagError> {
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM rag_chunks WHERE workspace = ?1 AND embedded = 0",
        [ws],
        |r| r.get(0),
    )?;
    if total == 0 {
        return Ok(());
    }
    let total = total as usize;
    let mut done = 0usize;

    loop {
        if opts.cancelled() {
            report.cancelled = true;
            return Ok(());
        }
        let batch = pending_batch(conn, ws, EMBED_BATCH)?;
        if batch.is_empty() {
            break;
        }
        let texts: Vec<String> = batch.iter().map(|(_, _, t)| t.clone()).collect();
        let vectors = match client.embeddings(model, &texts).await {
            Ok(v) => v,
            Err(e) => {
                // Servidor caiu no meio: o que já entrou continua valendo e a
                // próxima atualização retoma de onde parou (embedded = 0).
                log::warn!("embeddings falharam no meio da indexação: {e}");
                return Ok(());
            }
        };
        if vectors.len() != batch.len() {
            log::warn!(
                "servidor devolveu {} vetores para {} trechos; parando a passada vetorial",
                vectors.len(),
                batch.len()
            );
            return Ok(());
        }

        store_vectors(conn, ws, &batch, vectors)?;
        done += batch.len();
        report.embedded += batch.len();
        let last = batch.last().map(|(_, p, _)| p.as_str()).unwrap_or("");
        opts.report(IndexPhase::Embedding, done, total, last);
    }
    Ok(())
}

/// Próximo lote de trechos sem vetor: `(id, caminho, conteúdo)`.
fn pending_batch(
    conn: &Connection,
    ws: &str,
    limit: usize,
) -> Result<Vec<(i64, String, String)>, RagError> {
    let mut stmt = conn.prepare(
        "SELECT id, path, content FROM rag_chunks
         WHERE workspace = ?1 AND embedded = 0
         ORDER BY id LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![ws, limit as i64], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn store_vectors(
    conn: &Connection,
    ws: &str,
    batch: &[(i64, String, String)],
    vectors: Vec<Vec<f32>>,
) -> Result<(), RagError> {
    let tx = conn.unchecked_transaction()?;
    {
        let mut ins = tx.prepare_cached(
            "INSERT INTO rag_vec (chunk_id, workspace, embedding) VALUES (?1, ?2, ?3)",
        )?;
        // O `vec0` não aceita UPSERT: reembeddar é apagar e inserir de novo.
        let mut del = tx.prepare_cached("DELETE FROM rag_vec WHERE chunk_id = ?1")?;
        let mut mark = tx.prepare_cached("UPDATE rag_chunks SET embedded = 1 WHERE id = ?1")?;
        for ((id, _, _), mut v) in batch.iter().zip(vectors) {
            // Norma 1: o vec0 ordena por L2, e L2 sobre vetores unitários dá a
            // mesma ordem do cosseno — a métrica correta para texto.
            crate::normalize(&mut v);
            let _ = del.execute([id]);
            ins.execute(params![id, ws, crate::vec_blob(&v)])?;
            mark.execute([id])?;
        }
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_db(dir: &Path) -> std::path::PathBuf {
        dir.join("idx.db")
    }

    async fn index(db: &Path, ws: &Path) -> IndexReport {
        index_workspace(db, ws, IndexOptions::default())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn indexes_then_skips_unchanged_files() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("proj");
        fs::create_dir_all(proj.join("src")).unwrap();
        fs::write(proj.join("src/a.rs"), "fn a() { println!(\"a\"); }").unwrap();
        fs::write(proj.join("src/b.rs"), "fn b() { println!(\"b\"); }").unwrap();
        let db = tmp_db(dir.path());

        let first = index(&db, &proj).await;
        assert_eq!(first.scanned, 2);
        assert_eq!(first.indexed, 2);
        assert_eq!(first.skipped, 0);
        assert!(first.chunks >= 2);
        assert!(!first.vector, "sem modelo de embedding: só FTS5");

        // Nada mudou: a segunda rodada não pode reindexar nada.
        let second = index(&db, &proj).await;
        assert_eq!(second.indexed, 0, "reindexou arquivo que não mudou");
        assert_eq!(second.skipped, 2);
        assert_eq!(second.chunks, 0);
    }

    #[tokio::test]
    async fn reindexes_only_the_changed_file() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("proj");
        fs::create_dir_all(&proj).unwrap();
        fs::write(proj.join("a.rs"), "conteudo a").unwrap();
        fs::write(proj.join("b.rs"), "conteudo b").unwrap();
        let db = tmp_db(dir.path());
        index(&db, &proj).await;

        fs::write(proj.join("a.rs"), "conteudo a mudou completamente aqui").unwrap();

        let r = index(&db, &proj).await;
        assert_eq!(r.indexed, 1, "deveria reindexar só o arquivo alterado");
        assert_eq!(r.skipped, 1);
    }

    /// Segundo nível da comparação: o `mtime` mudou (arquivo salvo de novo,
    /// `git checkout` de volta, formatador que não mexeu em nada) mas o
    /// conteúdo é o mesmo — o hash tem que evitar a reindexação.
    #[tokio::test]
    async fn new_mtime_with_same_content_is_not_reindexed() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("proj");
        fs::create_dir_all(&proj).unwrap();
        fs::write(proj.join("a.rs"), "sempre igual").unwrap();
        let db = tmp_db(dir.path());
        index(&db, &proj).await;

        // Envelhece o mtime no catálogo: força o caminho lento (ler + hash)
        // sem depender da resolução de tempo do sistema de arquivos.
        {
            let conn = crate::vec_init::open_rag_connection(&db).unwrap();
            conn.execute("UPDATE rag_catalog SET mtime = 1", [])
                .unwrap();
        }

        let r = index(&db, &proj).await;
        assert_eq!(r.indexed, 0, "hash igual não deveria reindexar");
        assert_eq!(r.skipped, 1);

        // E o mtime foi acertado, para a próxima rodada usar o caminho rápido.
        let conn = crate::vec_init::open_rag_connection(&db).unwrap();
        let mtime: i64 = conn
            .query_row("SELECT mtime FROM rag_catalog", [], |r| r.get(0))
            .unwrap();
        assert_ne!(mtime, 1, "mtime deveria ter sido atualizado");
    }

    #[tokio::test]
    async fn deleted_files_leave_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("proj");
        fs::create_dir_all(&proj).unwrap();
        fs::write(proj.join("a.rs"), "vai sumir").unwrap();
        fs::write(proj.join("b.rs"), "fica").unwrap();
        let db = tmp_db(dir.path());
        index(&db, &proj).await;

        fs::remove_file(proj.join("a.rs")).unwrap();
        let r = index(&db, &proj).await;
        assert_eq!(r.removed, 1);

        let conn = crate::vec_init::open_rag_connection(&db).unwrap();
        let st = schema::status(&conn, &workspace_key(&proj)).unwrap();
        assert_eq!(st.files, 1);
    }

    #[tokio::test]
    async fn secrets_and_binaries_never_reach_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("proj");
        fs::create_dir_all(&proj).unwrap();
        fs::write(proj.join("app.rs"), "fn app() {}").unwrap();
        fs::write(proj.join(".env"), "API_TOKEN=supersecreto123").unwrap();
        fs::write(proj.join("server.key"), "-----BEGIN PRIVATE KEY-----\nabc").unwrap();
        fs::write(proj.join("logo.png"), [0x89u8, 0x50, 0x4e, 0x47, 0x00]).unwrap();
        // Binário sem extensão delatora: pego pelo conteúdo.
        fs::write(proj.join("blob.data"), [0x00u8, 0x01, 0x02, 0x03, 0x04]).unwrap();
        // Arquivo grande: acima do teto de 5 MB.
        fs::write(
            proj.join("grande.txt"),
            "a".repeat((walker::MAX_FILE_BYTES + 1) as usize),
        )
        .unwrap();
        let db = tmp_db(dir.path());

        index(&db, &proj).await;

        let conn = crate::vec_init::open_rag_connection(&db).unwrap();
        let mut stmt = conn
            .prepare("SELECT path FROM rag_catalog WHERE workspace = ?1")
            .unwrap();
        let paths: Vec<String> = stmt
            .query_map([workspace_key(&proj)], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert_eq!(paths, vec!["app.rs".to_string()]);

        // E nem o texto do segredo pode estar no índice textual.
        let leaked: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM rag_chunks_fts WHERE rag_chunks_fts MATCH 'supersecreto123'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(leaked, 0, "SEGREDO VAZOU PARA O ÍNDICE");
    }

    #[tokio::test]
    async fn cancellation_stops_and_reports_partial_work() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("proj");
        fs::create_dir_all(&proj).unwrap();
        for i in 0..10 {
            fs::write(proj.join(format!("f{i}.rs")), format!("conteudo {i}")).unwrap();
        }
        let db = tmp_db(dir.path());

        let cancel = Arc::new(AtomicBool::new(true)); // já cancelado
        let r = index_workspace(
            &db,
            &proj,
            IndexOptions {
                cancel,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(r.cancelled);
        assert_eq!(r.indexed, 0);
    }

    #[tokio::test]
    async fn progress_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("proj");
        fs::create_dir_all(&proj).unwrap();
        for i in 0..8 {
            fs::write(proj.join(format!("f{i}.rs")), format!("conteudo {i}")).unwrap();
        }
        let db = tmp_db(dir.path());

        let seen: Arc<std::sync::Mutex<Vec<IndexPhase>>> = Arc::default();
        let sink = seen.clone();
        index_workspace(
            &db,
            &proj,
            IndexOptions {
                progress: Some(Arc::new(move |p: IndexProgress| {
                    sink.lock().unwrap().push(p.phase);
                })),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let phases = seen.lock().unwrap().clone();
        assert!(phases.contains(&IndexPhase::Scanning));
        assert!(phases.contains(&IndexPhase::Indexing));
        assert!(phases.contains(&IndexPhase::Done));
    }
}
