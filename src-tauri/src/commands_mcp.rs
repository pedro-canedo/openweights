//! Comandos da tela de conectores (MCP).
//!
//! A camada é fina de propósito: toda a regra (parse do JSON colado,
//! normalização do Windows, hash das definições, portão de aprovação) vive em
//! `lr_mcp`, onde é testável sem Tauri. Aqui só traduzimos erro para texto e
//! devolvemos as visões que a interface desenha.
//!
//! Ordem esperada na tela: adicionar → **testar conexão** (é o que lê o
//! catálogo e grava o hash) → revisar as ferramentas → aprovar. Enquanto a
//! aprovação não vem, o conector não entrega ferramenta nenhuma ao agente.

use crate::state::AppState;
use lr_mcp::{McpServerView, McpToolView};
use tauri::State;

type CmdResult<T> = Result<T, String>;

/// Todos os conectores configurados, com estado e pendência de revisão.
#[tauri::command]
pub async fn mcp_servers_list(state: State<'_, AppState>) -> CmdResult<Vec<McpServerView>> {
    state.mcp.views().await.map_err(|e| e.to_user_message())
}

/// Adiciona um conector a partir de um bloco `mcpServers` colado (ou de uma
/// entrada única montada pelo formulário).
///
/// `name` só é aplicado quando o bloco define um servidor só — colar um
/// `mcpServers` com vários mantém os nomes de dentro do JSON.
#[tauri::command]
pub async fn mcp_server_add(
    state: State<'_, AppState>,
    name: String,
    config_json: String,
) -> CmdResult<Vec<McpServerView>> {
    // `McpHost` é um punhado de `Arc`: clonar evita segurar um empréstimo do
    // estado do app através dos dois `await` seguidos.
    let host = state.mcp.clone();
    host.add(&name, &config_json)
        .await
        .map_err(|e| e.to_user_message())?;
    // Um conector novo significa um provedor novo no catálogo: refazer agora
    // evita ter de reiniciar o app para o agente enxergar as ferramentas.
    state.rebuild_tools()?;
    host.views().await.map_err(|e| e.to_user_message())
}

/// Remove o conector, derrubando a conexão e o cache de ferramentas.
#[tauri::command]
pub async fn mcp_server_remove(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state
        .mcp
        .remove(&id)
        .await
        .map_err(|e| e.to_user_message())?;
    state.rebuild_tools()
}

/// Liga/desliga um conector. Desligar derruba a conexão na hora.
#[tauri::command]
pub async fn mcp_server_enable(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> CmdResult<()> {
    state
        .mcp
        .set_enabled(&id, enabled)
        .await
        .map_err(|e| e.to_user_message())
}

/// Conecta, lê o catálogo e grava o hash atual.
///
/// É também o passo que destrava a aprovação: sem um hash gravado não há o
/// que aprovar. A lista volta para a tela mostrar os selos de cada
/// ferramenta antes de a pessoa decidir.
#[tauri::command]
pub async fn mcp_server_test(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<Vec<McpToolView>> {
    let tools = state
        .mcp
        .refresh(&id)
        .await
        .map_err(|e| e.to_user_message())?;
    Ok(tools.iter().map(McpToolView::from).collect())
}

/// Ferramentas conhecidas do conector, direto do cache — sem conectar.
///
/// É o que alimenta o banner de re-aprovação: a pessoa precisa ver o que
/// mudou mesmo com o servidor fora do ar.
#[tauri::command]
pub async fn mcp_server_tools(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<Vec<McpToolView>> {
    state
        .mcp
        .tool_views(&id)
        .await
        .map_err(|e| e.to_user_message())
}

/// Registra que a pessoa revisou e aprovou o catálogo atual.
///
/// `hash` é o `toolsHash` que veio na listagem; `lr_mcp` confere se ainda é o
/// atual antes de aprovar, para que uma tela velha não libere um catálogo que
/// já mudou.
#[tauri::command]
pub async fn mcp_server_approve_tools(
    state: State<'_, AppState>,
    id: String,
    hash: String,
) -> CmdResult<()> {
    state
        .mcp
        .approve_tools(&id, &hash)
        .await
        .map_err(|e| e.to_user_message())
}
