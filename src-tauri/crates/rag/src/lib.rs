//! Índice do projeto: o agente acha trechos por SIGNIFICADO, não só por nome.
//!
//! Motivação: `grep` só acha o que o usuário soube escrever. Perguntas do tipo
//! "onde a gente valida o token de sessão?" não têm palavra-chave óbvia — quem
//! responde é a busca semântica. Mas embedding sozinho erra em nome próprio
//! (`RagHandle`, `AGENT_MAX_STEPS`), onde o casamento literal é imbatível. Por
//! isso a busca é **híbrida**: FTS5 (BM25) e vetor rodam em paralelo e o
//! resultado é fundido por RRF.
//!
//! Decisões que valem explicar:
//! - **Conexão própria.** As tabelas moram no mesmo arquivo `.db` do
//!   `lr_store`, mas o RAG abre a **própria** conexão. Indexar leva minutos e
//!   não pode segurar o cadeado de quem só quer listar conversas; o WAL
//!   permite leitor e escritor ao mesmo tempo.
//! - **Sem download e sem serviço externo.** O vetor sai do próprio
//!   llama-server (Router mode) via `/v1/embeddings`. Sem modelo de embedding
//!   configurado, o índice roda **só com FTS5** — pior, mas funcionando.
//! - **Incremental.** Um catálogo com hash e mtime por arquivo evita reler o
//!   projeto inteiro a cada atualização (mesma ideia do Continue.dev).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub mod chunk;
pub mod index;
pub mod schema;
pub mod search;
pub mod tool;
pub mod vec_init;
pub mod walker;

pub use chunk::{Chunk, ChunkOptions, chunk_text};
pub use index::{IndexOptions, IndexPhase, IndexProgress, IndexReport, ProgressCallback};
pub use schema::{RagStatus, ensure_schema};
pub use search::{HitSource, SearchHit, rrf_fuse};
pub use tool::rag_tools;
pub use vec_init::{
    RagCapabilities, open_rag_connection, open_rag_memory, register_vector_extension, vec_version,
};
pub use walker::{FileEntry, is_sensitive_path, scan_workspace};

#[derive(Debug, thiserror::Error)]
pub enum RagError {
    #[error("erro de banco: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("falha de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("erro no servidor de modelos: {0}")]
    Engine(String),
    #[error("nenhuma pasta de projeto foi escolhida")]
    NoWorkspace,
    #[error("já existe uma indexação em andamento")]
    Busy,
    #[error("{0}")]
    Other(String),
}

// Nota: não existe variante `Cancelled`. Cancelar não é falha — a indexação
// devolve `Ok(IndexReport { cancelled: true, .. })` com o que já entrou, para
// a interface poder mostrar o progresso parcial em vez de uma mensagem de erro.

impl From<lr_engine::EngineError> for RagError {
    fn from(e: lr_engine::EngineError) -> Self {
        RagError::Engine(e.to_string())
    }
}

/// Serializa um vetor para o BLOB que o `vec0` espera (f32 little-endian).
pub fn vec_blob(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

/// Normaliza um vetor para norma 1. O `vec0` mede distância L2; com vetores
/// normalizados a ordem por L2 coincide com a ordem por cosseno, que é a
/// métrica certa para embeddings de texto.
pub fn normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Endereço do llama-server para pedir embeddings.
#[derive(Debug, Clone, Default)]
pub struct EmbedEndpoint {
    pub base_url: String,
    pub api_key: Option<String>,
}

/// Configuração de embedding em vigor. `model: None` = índice só textual.
#[derive(Debug, Clone, Default)]
pub struct EmbedConfig {
    pub model: Option<String>,
    pub endpoint: Option<EmbedEndpoint>,
}

impl EmbedConfig {
    /// Dá para pedir vetores agora?
    pub fn is_ready(&self) -> bool {
        self.model.as_ref().is_some_and(|m| !m.is_empty()) && self.endpoint.is_some()
    }

    pub(crate) fn client(&self) -> Option<(lr_engine::LlamaClient, String)> {
        let model = self.model.clone().filter(|m| !m.is_empty())?;
        let ep = self.endpoint.as_ref()?;
        let client = lr_engine::LlamaClient::new(ep.base_url.clone())
            .with_optional_api_key(ep.api_key.clone());
        Some((client, model))
    }
}

/// Ponto único de acesso ao índice: a interface, os comandos e a ferramenta do
/// agente falam com o mesmo `RagHandle`.
///
/// Não guarda conexão aberta de propósito — cada operação abre a sua e fecha.
/// Conexão aberta e parada seria um `Statement` vivo segurando página do WAL
/// à toa, e o `Connection` do rusqlite não é `Sync` (não atravessaria `await`).
pub struct RagHandle {
    db_path: PathBuf,
    embed: std::sync::Mutex<EmbedConfig>,
    cancel: Arc<AtomicBool>,
    indexing: AtomicBool,
}

impl RagHandle {
    pub fn new(db_path: impl Into<PathBuf>) -> Self {
        Self {
            db_path: db_path.into(),
            embed: std::sync::Mutex::new(EmbedConfig::default()),
            cancel: Arc::new(AtomicBool::new(false)),
            indexing: AtomicBool::new(false),
        }
    }

    pub fn db_path(&self) -> &std::path::Path {
        &self.db_path
    }

    /// Informa onde o llama-server está ouvindo (muda quando o servidor sobe).
    pub fn set_endpoint(&self, endpoint: Option<EmbedEndpoint>) {
        if let Ok(mut cfg) = self.embed.lock() {
            cfg.endpoint = endpoint;
        }
    }

    /// Define (ou tira) o modelo de embedding em uso.
    pub fn set_embed_model(&self, model: Option<String>) {
        if let Ok(mut cfg) = self.embed.lock() {
            cfg.model = model.filter(|m| !m.is_empty());
        }
    }

    pub fn embed_config(&self) -> EmbedConfig {
        self.embed.lock().map(|c| c.clone()).unwrap_or_default()
    }

    pub fn is_indexing(&self) -> bool {
        self.indexing.load(Ordering::SeqCst)
    }

    /// Abre uma conexão nova, já com esquema garantido.
    pub fn connect(&self) -> Result<rusqlite::Connection, RagError> {
        let conn = vec_init::open_rag_connection(&self.db_path)?;
        schema::ensure_schema(&conn)?;
        Ok(conn)
    }

    /// Estado do índice deste projeto (para a interface).
    pub fn status(&self, workspace: &std::path::Path) -> Result<RagStatus, RagError> {
        let conn = self.connect()?;
        let mut st = schema::status(&conn, &workspace_key(workspace))?;
        st.indexing = self.is_indexing();
        st.embed_model_configured = self.embed_config().is_ready();
        Ok(st)
    }

    /// Pede o cancelamento da indexação em curso (efeito no próximo arquivo).
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    /// Apaga tudo que foi indexado deste projeto.
    pub fn clear(&self, workspace: &std::path::Path) -> Result<(), RagError> {
        let conn = self.connect()?;
        schema::clear_workspace(&conn, &workspace_key(workspace))
    }

    /// Indexa (ou atualiza) o projeto. Só um de cada vez.
    ///
    /// Um pedido de cancelamento anterior é zerado aqui: sinal velho não pode
    /// derrubar a rodada nova.
    pub async fn index(
        &self,
        workspace: &std::path::Path,
        progress: Option<ProgressCallback>,
    ) -> Result<IndexReport, RagError> {
        if self
            .indexing
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(RagError::Busy);
        }
        // A trava sai no `Drop`, não no fim da função: se a interface recarregar
        // no meio, o Tauri descarta este futuro — e sem o guarda o índice
        // ficaria "indexando" para sempre, sem jeito de tentar de novo.
        let _guard = IndexingGuard(&self.indexing);
        self.cancel.store(false, Ordering::SeqCst);
        let opts = IndexOptions {
            embed: self.embed_config(),
            cancel: self.cancel.clone(),
            progress,
        };
        index::index_workspace(&self.db_path, workspace, opts).await
    }

    /// Busca híbrida no projeto.
    pub async fn search(
        &self,
        workspace: &std::path::Path,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchHit>, RagError> {
        search::search(&self.db_path, workspace, query, limit, self.embed_config()).await
    }
}

/// Solta a trava de "uma indexação por vez" mesmo se o futuro for descartado.
struct IndexingGuard<'a>(&'a AtomicBool);

impl Drop for IndexingGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Chave de projeto usada nas tabelas: caminho absoluto normalizado.
///
/// Sem canonicalizar de propósito — o projeto pode ter sido movido/desmontado
/// e a chave precisa continuar batendo com o que foi gravado.
pub fn workspace_key(dir: &std::path::Path) -> String {
    let s = dir.to_string_lossy().replace('\\', "/");
    let trimmed = s.trim_end_matches('/');
    if trimmed.is_empty() {
        s.to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vec_blob_is_little_endian_f32() {
        let b = vec_blob(&[1.0f32, -2.0]);
        assert_eq!(b.len(), 8);
        assert_eq!(&b[0..4], &1.0f32.to_le_bytes());
        assert_eq!(&b[4..8], &(-2.0f32).to_le_bytes());
    }

    #[test]
    fn normalize_makes_unit_vector() {
        let mut v = [3.0f32, 4.0];
        normalize(&mut v);
        let n = (v[0] * v[0] + v[1] * v[1]).sqrt();
        assert!((n - 1.0).abs() < 1e-6);
        // Vetor zero não pode virar NaN.
        let mut z = [0.0f32, 0.0];
        normalize(&mut z);
        assert!(z.iter().all(|x| x.is_finite()));
    }

    /// Os comandos do Tauri rodam num runtime multithread: um futuro que não
    /// seja `Send` nem compila do outro lado. Como o `Connection` do rusqlite
    /// é `Send` mas não `Sync`, basta um `&Connection` vivo sobre um `await`
    /// para quebrar isso — e o erro apareceria lá, no crate do integrador.
    /// Este teste faz a verificação AQUI, onde dá para consertar.
    #[test]
    fn public_futures_are_send() {
        fn assert_send<T: Send>(_: &T) {}
        let handle = RagHandle::new("/tmp/lr-rag-send-check.db");
        let ws = std::path::Path::new("/tmp");
        assert_send(&handle.index(ws, None));
        assert_send(&handle.search(ws, "consulta", 3));
        assert_send(&index::index_workspace(
            std::path::Path::new("/tmp/x.db"),
            ws,
            index::IndexOptions::default(),
        ));
        // O próprio handle vai dentro de um `State` compartilhado.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RagHandle>();
    }

    #[test]
    fn workspace_key_normalizes_separators_and_trailing_slash() {
        let a = workspace_key(std::path::Path::new("/home/user/proj/"));
        let b = workspace_key(std::path::Path::new("/home/user/proj"));
        assert_eq!(a, b);
    }
}
