//! `memory_save`: como o modelo guarda um fato sozinho.
//!
//! O agente descobre coisas durante o trabalho — "o script de teste é
//! `pnpm test`", "os componentes ficam em `src/components`". Sem esta
//! ferramenta, essa descoberta morre no fim da conversa e a próxima execução
//! recomeça do zero, refazendo as mesmas buscas.
//!
//! É uma ferramenta **Meta**: não lê nem altera arquivo do projeto, então não
//! precisa de confirmação — o custo de um fato errado é uma linha no
//! `MEMORY.md` que a pessoa apaga com um clique. Mesmo assim tudo passa pela
//! curadoria de [`crate::facts`]: sem isso, um modelo pequeno "memoriza" o
//! próprio raciocínio e envenena todas as execuções seguintes.
//!
//! Recusa nunca é erro cru: duplicata volta como resultado normal ("já
//! sabíamos"), porque um erro faria o modelo tentar de novo com outra
//! redação — gastando um passo do run para guardar a mesma coisa.

use crate::facts::FactScope;
use crate::{MemoryError, MemoryStore};
use async_trait::async_trait;
use lr_store::Store;
use lr_tools::{SharedTool, Tool, ToolContext, ToolError, ToolOutput, ToolResult};
use lr_types::agent::ToolCategory;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

/// Guarda um fato durável na memória de longo prazo.
pub struct MemorySave {
    store: Arc<Store>,
    /// Pasta usada quando a chamada não traz uma (o registro de ferramentas
    /// é montado uma vez, na abertura do app; o run é que sabe a pasta atual).
    fallback_workspace: Option<PathBuf>,
}

impl MemorySave {
    pub fn new(store: Arc<Store>, workspace: Option<PathBuf>) -> Self {
        Self {
            store,
            fallback_workspace: workspace,
        }
    }

    /// A pasta da chamada manda; a da construção é só rede de segurança.
    fn memory_for(&self, ctx: &ToolContext) -> MemoryStore {
        let workspace = ctx
            .workspace
            .clone()
            .or_else(|| self.fallback_workspace.clone());
        MemoryStore::new(self.store.clone(), workspace)
    }
}

/// Lê o escopo pedido, aceitando os sinônimos que os modelos usam.
fn scope_arg(args: &Value) -> Option<FactScope> {
    let raw = args.get("scope").and_then(Value::as_str)?;
    match raw.trim().to_lowercase().as_str() {
        "global" | "user" | "usuario" | "usuário" | "always" => Some(FactScope::Global),
        "project" | "projeto" | "workspace" | "repo" | "local" => Some(FactScope::Workspace),
        _ => None,
    }
}

#[async_trait]
impl Tool for MemorySave {
    fn name(&self) -> &str {
        "memory_save"
    }

    fn description(&self) -> &str {
        "Guarda um fato durável para as próximas conversas: como o projeto é construído, \
         como rodar os testes, convenções combinadas ou uma preferência da pessoa. \
         Uma frase curta por chamada. Não use para o que você acabou de fazer nem para \
         algo que só vale agora."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "fact": {
                    "type": "string",
                    "description": "O fato em uma frase curta, ex.: os testes rodam com `pnpm test`."
                },
                "scope": {
                    "type": "string",
                    "enum": ["project", "global"],
                    "description": "`project` (padrão) vale só nesta pasta; `global` vale em qualquer projeto (preferências da pessoa)."
                }
            },
            "required": ["fact"],
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Meta
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let raw = ["fact", "content", "text", "memory"]
            .iter()
            .find_map(|k| args.get(*k).and_then(Value::as_str))
            .unwrap_or_default();
        if raw.trim().is_empty() {
            return Err(ToolError::InvalidArgs(
                "faltou `fact` — mande o fato numa frase curta".into(),
            ));
        }

        let memory = self.memory_for(ctx);
        match memory.save(raw, scope_arg(&args), None) {
            Ok(saved) => {
                let onde = match saved.scope {
                    FactScope::Global => "para todos os projetos".to_string(),
                    FactScope::Workspace => format!("neste projeto (assunto: {})", saved.topic),
                };
                Ok(
                    ToolOutput::text(format!("Memorizado {onde}: {}", saved.content))
                        .truncated_to(ctx.max_output_bytes),
                )
            }
            // Recusa de curadoria é resultado, não erro: o modelo não deve
            // gastar outro passo tentando reescrever o mesmo fato.
            Err(MemoryError::Curation(e)) => Ok(ToolOutput::text(e.to_model_message())),
            Err(e) => Err(ToolError::Other(e.to_string())),
        }
    }
}

/// Ferramentas de memória para o registro do agente.
///
/// `workspace` é só o valor de partida: cada chamada usa a pasta do
/// [`ToolContext`] quando ela existe, o que deixa a ferramenta correta mesmo
/// registrada uma única vez na abertura do app.
pub fn memory_tools(store: Arc<Store>, workspace: Option<PathBuf>) -> Vec<SharedTool> {
    vec![Arc::new(MemorySave::new(store, workspace))]
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Arc<Store>, MemorySave) {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(Store::open_in_memory().unwrap());
        let tool = MemorySave::new(store.clone(), None);
        (dir, store, tool)
    }

    #[tokio::test]
    async fn the_model_saves_a_fact_and_it_shows_up_in_the_next_run() {
        let (dir, store, tool) = setup();
        let ctx = ToolContext::new(Some(dir.path().to_path_buf()), "call-1");

        let out = tool
            .execute(json!({ "fact": "os testes rodam com pnpm test" }), &ctx)
            .await
            .unwrap();
        assert!(
            out.content.starts_with("Memorizado neste projeto"),
            "{}",
            out.content
        );
        assert!(out.content.contains("testes"));

        // É isto que `run_start` lê para montar o prompt da próxima execução.
        let facts = store
            .list_memory_facts(Some(&dir.path().to_string_lossy()))
            .unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].content, "os testes rodam com pnpm test");
        // E a face legível existe.
        assert!(crate::files::topic_path(dir.path(), "testes").exists());
    }

    #[tokio::test]
    async fn the_scope_argument_sends_the_fact_to_every_project() {
        let (dir, store, tool) = setup();
        let ctx = ToolContext::new(Some(dir.path().to_path_buf()), "call-1");

        tool.execute(
            json!({ "fact": "responda sempre em português", "scope": "global" }),
            &ctx,
        )
        .await
        .unwrap();

        // Visível de outra pasta qualquer — é o que "global" significa.
        let facts = store.list_memory_facts(Some("/outro/projeto")).unwrap();
        assert_eq!(facts.len(), 1);
        assert!(!crate::memory_dir(dir.path()).exists());
    }

    #[tokio::test]
    async fn a_repeated_fact_comes_back_as_a_result_not_as_an_error() {
        let (dir, _store, tool) = setup();
        let ctx = ToolContext::new(Some(dir.path().to_path_buf()), "call-1");
        let args = json!({ "fact": "este projeto usa pnpm" });

        tool.execute(args.clone(), &ctx).await.unwrap();
        let out = tool.execute(args, &ctx).await.unwrap();
        assert!(
            out.content.contains("já está na memória"),
            "{}",
            out.content
        );
    }

    #[tokio::test]
    async fn an_empty_fact_is_an_error_that_says_what_to_do() {
        let (dir, _store, tool) = setup();
        let ctx = ToolContext::new(Some(dir.path().to_path_buf()), "call-1");

        let err = tool
            .execute(json!({ "fact": "   " }), &ctx)
            .await
            .unwrap_err();
        assert!(err.to_model_message().contains("fact"));
        let err = tool.execute(json!({}), &ctx).await.unwrap_err();
        assert!(err.to_model_message().contains("frase curta"));
    }

    #[tokio::test]
    async fn it_is_a_meta_tool_and_works_without_a_folder() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let tools = memory_tools(store, None);
        let tool = &tools[0];
        assert_eq!(tool.name(), "memory_save");
        assert_eq!(tool.category(), ToolCategory::Meta);
        assert!(tool.spec().read_only, "não toca em arquivo do projeto");

        // Sem pasta escolhida o fato ainda é guardado — como global.
        let ctx = ToolContext::new(None, "call-1");
        let out = tool
            .execute(json!({ "fact": "este projeto usa pnpm" }), &ctx)
            .await
            .unwrap();
        assert!(
            out.content.contains("para todos os projetos"),
            "{}",
            out.content
        );
    }
}
