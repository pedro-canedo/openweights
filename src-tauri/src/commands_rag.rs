//! Comandos do índice do projeto (RAG): estado, indexação com progresso,
//! cancelamento, busca e limpeza.
//!
//! O índice mora no mesmo arquivo `.db` do `lr_store`, mas em conexão própria
//! (o `RagHandle` abre a sua a cada operação). Indexar um projeto grande leva
//! minutos e não pode segurar o cadeado de quem só quer listar conversas.
//!
//! Dois estados do mundo mudam por baixo e são sincronizados a cada comando:
//! o endereço do llama-server (que sobe e desce) e o modelo de embedding
//! escolhido (que fica gravado em `settings`, para sobreviver ao reinício).
//! Sem nenhum dos dois o índice continua funcionando — só que apenas com FTS5,
//! e a interface avisa com `agent.rag.vectorOff`.

use crate::state::AppState;
use lr_rag::{EmbedEndpoint, IndexProgress, IndexReport, RagStatus, SearchHit};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;
use tauri::ipc::Channel;

type CmdResult<T> = Result<T, String>;

/// Chave em `settings` com o id do modelo de embedding em uso.
pub const EMBED_MODEL_SETTING: &str = "rag_embed_model";

/// Quantos trechos a busca da interface devolve por padrão.
const DEFAULT_SEARCH_LIMIT: u32 = 10;

fn workspace_path(dir: &str) -> CmdResult<PathBuf> {
    let path = PathBuf::from(dir);
    if !path.is_dir() {
        return Err("a pasta do projeto não está acessível".into());
    }
    Ok(path)
}

/// Alinha o handle com o mundo antes de qualquer operação.
///
/// `embed_model`: `Some(id)` grava a escolha; `Some("")` desliga o vetor;
/// `None` mantém o que já estava gravado.
async fn sync_handle(state: &AppState, embed_model: Option<String>) {
    // Servidor fora do ar não é erro aqui: só significa "sem vetor agora".
    let endpoint = state.agent_endpoint().await.ok().map(|ep| EmbedEndpoint {
        base_url: ep.base_url,
        api_key: ep.api_key,
    });
    state.rag.set_endpoint(endpoint);

    let model = match embed_model {
        Some(chosen) => {
            let trimmed = chosen.trim().to_string();
            if let Err(e) = state.store.set_setting(EMBED_MODEL_SETTING, &trimmed) {
                log::warn!("não deu para gravar o modelo de embedding: {e}");
            }
            (!trimmed.is_empty()).then_some(trimmed)
        }
        None => state
            .store
            .get_setting(EMBED_MODEL_SETTING)
            .ok()
            .flatten()
            .filter(|m| !m.is_empty()),
    };
    state.rag.set_embed_model(model);
}

/// Estado do índice deste projeto: contagens, capacidades do SQLite e se há
/// modelo de embedding configurado.
#[tauri::command]
pub async fn rag_status(state: State<'_, AppState>, workspace_dir: String) -> CmdResult<RagStatus> {
    let path = workspace_path(&workspace_dir)?;
    sync_handle(&state, None).await;
    state.rag.status(&path).map_err(|e| e.to_string())
}

/// Indexa (ou atualiza) o projeto, transmitindo o progresso pelo canal.
///
/// A resposta só chega no fim, com o relatório. Quem acompanha é o canal —
/// mesmo padrão do `run_start`.
#[tauri::command]
pub async fn rag_index_start(
    state: State<'_, AppState>,
    workspace_dir: String,
    embed_model: Option<String>,
    on_progress: Channel<IndexProgress>,
) -> CmdResult<IndexReport> {
    let path = workspace_path(&workspace_dir)?;
    sync_handle(&state, embed_model).await;

    let sink: lr_rag::ProgressCallback = Arc::new(move |p: IndexProgress| {
        if let Err(e) = on_progress.send(p) {
            // Janela fechada no meio: a indexação continua, só não há para
            // quem entregar o progresso.
            log::debug!("canal de progresso do índice indisponível: {e}");
        }
    });

    state
        .rag
        .index(&path, Some(sink))
        .await
        .map_err(|e| e.to_string())
}

/// Pede o cancelamento da indexação em curso (vale a partir do próximo lote).
#[tauri::command]
pub async fn rag_index_cancel(state: State<'_, AppState>) -> CmdResult<()> {
    state.rag.cancel();
    Ok(())
}

/// Busca híbrida (FTS5 + vetor) no índice do projeto.
#[tauri::command]
pub async fn rag_search(
    state: State<'_, AppState>,
    workspace_dir: String,
    query: String,
    limit: Option<u32>,
) -> CmdResult<Vec<SearchHit>> {
    let path = workspace_path(&workspace_dir)?;
    sync_handle(&state, None).await;
    let limit = limit.unwrap_or(DEFAULT_SEARCH_LIMIT).clamp(1, 50) as usize;
    state
        .rag
        .search(&path, &query, limit)
        .await
        .map_err(|e| e.to_string())
}

/// Apaga o índice deste projeto (os outros continuam intactos).
#[tauri::command]
pub async fn rag_clear(state: State<'_, AppState>, workspace_dir: String) -> CmdResult<()> {
    let path = workspace_path(&workspace_dir)?;
    state.rag.clear(&path).map_err(|e| e.to_string())
}
