//! Registro de ferramentas: junta as nativas e as de conectores externos.
//!
//! Modelos pequenos (7B–30B) erram mais quanto maior o cardápio. Por isso o
//! registro sabe entregar um subconjunto ([`ToolRegistry::specs_for`]) em vez
//! de despejar tudo em toda chamada.

use crate::{SharedTool, ToolContext, ToolError, ToolOutput, ToolResult};
use async_trait::async_trait;
use lr_types::agent::{ToolOrigin, ToolSpec};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Fonte externa de ferramentas (um servidor MCP, por exemplo).
#[async_trait]
pub trait ToolProvider: Send + Sync {
    /// Prefixo dos nomes expostos ao modelo (`github__criar_issue`).
    fn id(&self) -> &str;

    /// Ferramentas disponíveis agora.
    async fn list(&self) -> Vec<ToolSpec>;

    /// Executa pelo nome JÁ SEM o prefixo.
    async fn call(&self, tool: &str, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput>;
}

/// Separador entre o id do provedor e o nome da ferramenta.
pub const PROVIDER_SEP: &str = "__";

#[derive(Default)]
pub struct ToolRegistry {
    builtin: BTreeMap<String, SharedTool>,
    providers: Vec<Arc<dyn ToolProvider>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: SharedTool) -> &mut Self {
        self.builtin.insert(tool.name().to_string(), tool);
        self
    }

    pub fn add_provider(&mut self, provider: Arc<dyn ToolProvider>) -> &mut Self {
        self.providers.push(provider);
        self
    }

    pub fn get(&self, name: &str) -> Option<&SharedTool> {
        self.builtin.get(name)
    }

    /// Nomes nativos registrados.
    pub fn builtin_names(&self) -> Vec<String> {
        self.builtin.keys().cloned().collect()
    }

    /// Catálogo completo (nativas + provedores, já com prefixo).
    pub async fn specs(&self) -> Vec<ToolSpec> {
        let mut out: Vec<ToolSpec> = self.builtin.values().map(|t| t.spec()).collect();
        for p in &self.providers {
            for mut spec in p.list().await {
                spec.name = format!("{}{PROVIDER_SEP}{}", p.id(), spec.name);
                spec.origin = ToolOrigin::Mcp {
                    server_id: p.id().to_string(),
                };
                out.push(spec);
            }
        }
        out
    }

    /// Catálogo filtrado por nome — o loop usa para encolher o cardápio.
    pub async fn specs_for(&self, allowed: &[String]) -> Vec<ToolSpec> {
        self.specs()
            .await
            .into_iter()
            .filter(|s| allowed.iter().any(|a| a == &s.name))
            .collect()
    }

    /// Executa `name`, roteando para a nativa ou para o provedor do prefixo.
    pub async fn execute(
        &self,
        name: &str,
        args: Value,
        ctx: &ToolContext,
    ) -> ToolResult<ToolOutput> {
        if let Some(tool) = self.builtin.get(name) {
            return tool.execute(args, ctx).await;
        }
        if let Some((prefix, rest)) = name.split_once(PROVIDER_SEP)
            && let Some(p) = self.providers.iter().find(|p| p.id() == prefix)
        {
            return p.call(rest, args, ctx).await;
        }
        Err(ToolError::Other(format!(
            "a ferramenta `{name}` não existe. Use apenas as ferramentas listadas."
        )))
    }

    /// Metadados de uma ferramenta pelo nome (nativa ou de provedor).
    pub async fn spec_of(&self, name: &str) -> Option<ToolSpec> {
        if let Some(t) = self.builtin.get(name) {
            return Some(t.spec());
        }
        self.specs().await.into_iter().find(|s| s.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Tool;
    use lr_types::agent::{ToolCategory, ToolTier};

    struct Echo;

    #[async_trait]
    impl Tool for Echo {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "devolve o texto recebido"
        }
        fn parameters(&self) -> Value {
            serde_json::json!({"type":"object","properties":{"t":{"type":"string"}}})
        }
        fn category(&self) -> ToolCategory {
            ToolCategory::Meta
        }
        async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult<ToolOutput> {
            Ok(ToolOutput::text(
                args.get("t").and_then(|v| v.as_str()).unwrap_or(""),
            ))
        }
    }

    struct FakeProvider;

    #[async_trait]
    impl ToolProvider for FakeProvider {
        fn id(&self) -> &str {
            "github"
        }
        async fn list(&self) -> Vec<ToolSpec> {
            vec![ToolSpec {
                name: "criar_issue".into(),
                description: "cria uma issue".into(),
                parameters: serde_json::json!({"type":"object"}),
                category: ToolCategory::Mcp,
                tier: ToolTier::Danger,
                origin: ToolOrigin::Builtin,
                read_only: false,
            }]
        }
        async fn call(&self, tool: &str, _a: Value, _c: &ToolContext) -> ToolResult<ToolOutput> {
            Ok(ToolOutput::text(format!("chamou {tool}")))
        }
    }

    fn ctx() -> ToolContext {
        ToolContext::new(None, "c1")
    }

    #[tokio::test]
    async fn routes_builtin_and_provider() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(Echo))
            .add_provider(Arc::new(FakeProvider));

        let out = reg
            .execute("echo", serde_json::json!({"t": "oi"}), &ctx())
            .await
            .unwrap();
        assert_eq!(out.content, "oi");

        let out = reg
            .execute("github__criar_issue", serde_json::json!({}), &ctx())
            .await
            .unwrap();
        assert_eq!(out.content, "chamou criar_issue");
    }

    #[tokio::test]
    async fn provider_specs_get_prefix_and_origin() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(Echo))
            .add_provider(Arc::new(FakeProvider));
        let specs = reg.specs().await;
        let mcp = specs
            .iter()
            .find(|s| s.name.contains("criar_issue"))
            .unwrap();
        assert_eq!(mcp.name, "github__criar_issue");
        assert!(matches!(&mcp.origin, ToolOrigin::Mcp { server_id } if server_id == "github"));
    }

    #[tokio::test]
    async fn unknown_tool_error_tells_the_model_what_to_do() {
        let reg = ToolRegistry::new();
        let err = reg
            .execute("inventada", serde_json::json!({}), &ctx())
            .await
            .unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("inventada"));
        assert!(msg.contains("listadas"));
    }

    #[tokio::test]
    async fn specs_for_filters_the_menu() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(Echo))
            .add_provider(Arc::new(FakeProvider));
        let subset = reg.specs_for(&["echo".to_string()]).await;
        assert_eq!(subset.len(), 1);
        assert_eq!(subset[0].name, "echo");
    }
}
