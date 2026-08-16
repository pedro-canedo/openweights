//! A ferramenta `workspace_search`: o índice visto pelo modelo.
//!
//! O agente já tem `grep` e listagem de arquivos. O que falta é a pergunta
//! sem palavra-chave: "onde a sessão expira?" não casa com nenhum literal se o
//! código chama isso de `ttl`. Essa é a lacuna que esta ferramenta cobre.
//!
//! Duas decisões de formato importam:
//! - **Cabeçalho `caminho:linha` antes de cada trecho.** É o que permite ao
//!   modelo citar a origem e ao usuário conferir. Trecho sem endereço vira
//!   alegação que ninguém verifica.
//! - **Índice ausente não é erro.** Devolver `Err` faria o laço tentar de novo
//!   e falhar de novo. Melhor uma resposta que explica a situação e aponta o
//!   caminho alternativo (`grep`), para o agente seguir trabalhando.

use std::sync::Arc;

use async_trait::async_trait;
use lr_tools::{Tool, ToolContext, ToolError, ToolOutput, ToolResult, arg_str, arg_u64};
use lr_types::agent::ToolCategory;
use serde_json::{Value, json};

use crate::{RagHandle, search::preview};

/// Teto de caracteres por trecho na resposta. Oito trechos inteiros de 512
/// tokens encheriam o contexto sozinhos; o suficiente para o modelo decidir se
/// vale abrir o arquivo é bem menos.
const SNIPPET_CHARS: usize = 700;

const DEFAULT_LIMIT: u64 = 6;
const MAX_LIMIT: u64 = 20;

pub struct WorkspaceSearchTool {
    handle: Arc<RagHandle>,
}

impl WorkspaceSearchTool {
    pub fn new(handle: Arc<RagHandle>) -> Self {
        Self { handle }
    }
}

#[async_trait]
impl Tool for WorkspaceSearchTool {
    fn name(&self) -> &str {
        "workspace_search"
    }

    fn description(&self) -> &str {
        "Busca trechos do projeto por SIGNIFICADO (semântica + texto), não por nome de arquivo. \
         Use quando não souber onde algo está ou como foi nomeado — ex.: \"onde a sessão expira\", \
         \"como o upload valida o tamanho\". Cada resultado vem com caminho:linha para você citar. \
         Para casar um literal exato (nome de função, mensagem de erro), prefira a busca por texto."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "O que procurar, em linguagem natural ou por termos."
                },
                "limit": {
                    "type": "integer",
                    "description": "Quantos trechos devolver (1 a 20; padrão 6).",
                    "minimum": 1,
                    "maximum": 20
                }
            },
            "required": ["query"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Read
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let query = arg_str(&args, "query")?;
        if query.trim().is_empty() {
            return Err(ToolError::InvalidArgs("`query` está vazio".into()));
        }
        let limit = arg_u64(&args, "limit", DEFAULT_LIMIT).clamp(1, MAX_LIMIT) as usize;

        let workspace = ctx
            .workspace
            .clone()
            .ok_or_else(|| ToolError::Other("nenhuma pasta de projeto foi escolhida".into()))?;

        let status = self
            .handle
            .status(&workspace)
            .map_err(|e| ToolError::Other(e.to_string()))?;
        if !status.indexed {
            return Ok(ToolOutput::text(
                "O índice deste projeto ainda não foi criado, então a busca por significado não \
                 está disponível. Peça ao usuário para indexar o projeto no painel da pasta, ou \
                 siga com busca por texto e leitura de arquivos.",
            ));
        }

        let hits = self
            .handle
            .search(&workspace, &query, limit)
            .await
            .map_err(|e| ToolError::Other(e.to_string()))?;

        if hits.is_empty() {
            return Ok(ToolOutput::text(format!(
                "Nenhum trecho do projeto casou com \"{query}\". Tente outras palavras, ou use \
                 busca por texto se souber um literal exato."
            )));
        }

        let mut out = String::new();
        let vector_off = status.vectors == 0;
        out.push_str(&format!(
            "{} trecho(s) para \"{}\"{}.\n",
            hits.len(),
            query,
            if vector_off {
                " (só busca textual: não há vetores neste índice)"
            } else {
                ""
            }
        ));
        for (i, hit) in hits.iter().enumerate() {
            out.push_str(&format!("\n{}. {}\n", i + 1, hit.citation()));
            out.push_str("```\n");
            out.push_str(preview(hit.snippet.trim_end(), SNIPPET_CHARS).trim_end());
            out.push_str("\n```\n");
        }
        out.push_str(
            "\nCite sempre `caminho:linha`. Abra o arquivo antes de alterar qualquer coisa.",
        );

        Ok(ToolOutput::text(out).truncated_to(ctx.max_output_bytes))
    }
}

/// Ferramentas de RAG para registrar no `ToolRegistry`.
pub fn rag_tools(handle: Arc<RagHandle>) -> Vec<Arc<dyn Tool>> {
    vec![Arc::new(WorkspaceSearchTool::new(handle))]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{IndexOptions, index_workspace};
    use lr_types::agent::ToolTier;
    use std::fs;

    fn ctx(workspace: Option<std::path::PathBuf>) -> ToolContext {
        ToolContext::new(workspace, "call-1")
    }

    #[test]
    fn spec_is_read_only_and_safe() {
        let dir = tempfile::tempdir().unwrap();
        let tools = rag_tools(Arc::new(RagHandle::new(dir.path().join("x.db"))));
        assert_eq!(tools.len(), 1);
        let spec = tools[0].spec();
        assert_eq!(spec.name, "workspace_search");
        assert!(spec.read_only, "buscar não altera nada");
        assert_eq!(spec.category, ToolCategory::Read);
        assert_eq!(spec.tier, ToolTier::Safe);
        // O schema tem que anunciar `query` como obrigatório.
        assert_eq!(spec.parameters["required"][0], "query");
    }

    #[tokio::test]
    async fn returns_citable_snippets() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("proj");
        fs::create_dir_all(proj.join("src")).unwrap();
        fs::write(
            proj.join("src/sessao.rs"),
            "// expira a sessao depois do tempo limite\npub const TTL: u64 = 3600;\n",
        )
        .unwrap();
        let db = dir.path().join("idx.db");
        index_workspace(&db, &proj, IndexOptions::default())
            .await
            .unwrap();

        let handle = Arc::new(RagHandle::new(&db));
        let tool = WorkspaceSearchTool::new(handle);
        let out = tool
            .execute(json!({ "query": "expira sessao" }), &ctx(Some(proj)))
            .await
            .unwrap();

        assert!(out.content.contains("src/sessao.rs:1"), "{}", out.content);
        assert!(out.content.contains("TTL"), "{}", out.content);
        assert!(out.content.contains("caminho:linha"));
        assert!(out.changed_files.is_empty(), "busca não altera arquivo");
    }

    #[tokio::test]
    async fn explains_instead_of_failing_when_not_indexed() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("proj");
        fs::create_dir_all(&proj).unwrap();
        let tool = WorkspaceSearchTool::new(Arc::new(RagHandle::new(dir.path().join("idx.db"))));

        let out = tool
            .execute(json!({ "query": "qualquer" }), &ctx(Some(proj)))
            .await
            .unwrap();
        assert!(out.content.contains("índice"), "{}", out.content);
        assert!(out.content.contains("indexar"), "{}", out.content);
    }

    #[tokio::test]
    async fn refuses_without_workspace_and_with_empty_query() {
        let dir = tempfile::tempdir().unwrap();
        let tool = WorkspaceSearchTool::new(Arc::new(RagHandle::new(dir.path().join("idx.db"))));

        let err = tool
            .execute(json!({ "query": "algo" }), &ctx(None))
            .await
            .unwrap_err();
        assert!(err.to_model_message().contains("pasta"));

        let err = tool
            .execute(json!({ "query": "  " }), &ctx(Some(dir.path().into())))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }
}
