//! Conexão com um servidor MCP: subir, listar, chamar, desligar.
//!
//! Só os dois transportes vivos da especificação: processo filho por stdio e
//! HTTP streamable. O SSE legado saiu do protocolo e não existe aqui.
//!
//! ## O que fazemos com as `ToolAnnotations`
//!
//! A própria especificação diz que as anotações (`readOnlyHint`,
//! `destructiveHint`, `idempotentHint`, `openWorldHint`) são **dicas não
//! confiáveis**: quem as escreve é o servidor, que é justamente a parte que
//! não controlamos. Então elas só podem tornar a política **mais** rígida,
//! nunca menos:
//!
//! * a categoria padrão é [`ToolCategory::Mcp`] (pede confirmação em todos os
//!   modos, exceto no automático) — e `readOnlyHint: true` **não** rebaixa
//!   para [`ToolCategory::Read`], que é auto-aprovada;
//! * `openWorldHint: true` sobe para [`ToolCategory::Network`], que pede
//!   confirmação inclusive no modo automático, porque é por aí que dados
//!   saem da máquina;
//! * o tier e os selos da interface ("só leitura", "destrutiva", "acessa a
//!   internet") vêm das anotações, mas são informativos.
//!
//! Assim, um servidor mentiroso só consegue piorar a própria vida.

use crate::config::{McpServerConfig, McpTransport};
use crate::error::McpError;
use lr_types::agent::{ToolCategory, ToolOrigin, ToolSpec, ToolTier};
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock};
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::time::Duration;

/// Tempo para o servidor aceitar a conexão e responder ao handshake.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Tempo para listar o catálogo de ferramentas.
pub const LIST_TIMEOUT: Duration = Duration::from_secs(20);
/// Tempo de uma chamada de ferramenta. Generoso: conectores fazem rede.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// Anotações do servidor, já achatadas. Todas opcionais: "ausente" e "falso"
/// significam coisas diferentes na hora de decidir o selo da interface.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAnnotations {
    pub read_only: Option<bool>,
    pub destructive: Option<bool>,
    pub idempotent: Option<bool>,
    pub open_world: Option<bool>,
}

impl McpAnnotations {
    fn flag(v: Option<bool>) -> bool {
        v == Some(true)
    }
}

/// Uma ferramenta anunciada por um servidor.
#[derive(Debug, Clone, PartialEq)]
pub struct McpToolDef {
    /// Nome exposto ao modelo, já saneado para `[A-Za-z0-9_-]`.
    pub name: String,
    /// Nome exato como o servidor o chama — é este que vai no `tools/call`.
    pub remote_name: String,
    pub description: String,
    /// JSON Schema dos parâmetros (sempre um objeto utilizável).
    pub schema: Value,
    pub annotations: McpAnnotations,
}

impl McpToolDef {
    /// Categoria usada pela política de permissões.
    ///
    /// Nunca rebaixa: o melhor que uma anotação consegue é manter o padrão
    /// [`ToolCategory::Mcp`]; `openWorldHint` sobe para
    /// [`ToolCategory::Network`], que pergunta mesmo no modo automático.
    pub fn category(&self) -> ToolCategory {
        if McpAnnotations::flag(self.annotations.open_world) {
            ToolCategory::Network
        } else {
            ToolCategory::Mcp
        }
    }

    /// Risco mostrado na interface e gravado no evento do run.
    pub fn tier(&self) -> ToolTier {
        let a = &self.annotations;
        if McpAnnotations::flag(a.destructive) || McpAnnotations::flag(a.open_world) {
            ToolTier::Danger
        } else if McpAnnotations::flag(a.read_only) {
            ToolTier::Safe
        } else {
            // Sem anotação não há promessa nenhuma; assumir o meio-termo é
            // mais honesto do que fingir que é inofensiva.
            ToolTier::Caution
        }
    }

    /// Selo "só leitura". Informativo — não libera auto-aprovação.
    pub fn read_only(&self) -> bool {
        McpAnnotations::flag(self.annotations.read_only)
    }

    /// Contrato exposto ao modelo. O nome vai sem prefixo: quem prefixa com
    /// `<id do servidor>__` é o `ToolRegistry`.
    pub fn spec(&self, server_id: &str) -> ToolSpec {
        ToolSpec {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: self.schema.clone(),
            category: self.category(),
            tier: self.tier(),
            origin: ToolOrigin::Mcp {
                server_id: server_id.to_string(),
            },
            read_only: self.read_only(),
        }
    }
}

/// Converte a ferramenta do SDK para o formato do app.
pub(crate) fn tool_from_rmcp(tool: &rmcp::model::Tool) -> McpToolDef {
    let remote_name = tool.name.to_string();
    let annotations = tool
        .annotations
        .as_ref()
        .map(|a| McpAnnotations {
            read_only: a.read_only_hint,
            destructive: a.destructive_hint,
            idempotent: a.idempotent_hint,
            open_world: a.open_world_hint,
        })
        .unwrap_or_default();

    McpToolDef {
        name: sanitize_for_model(&remote_name),
        description: tool
            .description
            .as_ref()
            .map(|d| d.to_string())
            .unwrap_or_default(),
        schema: sanitize_schema(Value::Object((*tool.input_schema).clone())),
        annotations,
        remote_name,
    }
}

/// Nomes de ferramenta viajam para o modelo dentro do `tool_calls` do
/// formato OpenAI, que só aceita `[A-Za-z0-9_-]`. Servidores usam pontos e
/// barras à vontade, então saneamos aqui e guardamos o original em
/// `remote_name` para a chamada de volta.
pub(crate) fn sanitize_for_model(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('_');
    if trimmed.is_empty() {
        "tool".to_string()
    } else {
        trimmed.chars().take(48).collect()
    }
}

/// Garante um schema que o modelo consegue seguir.
///
/// Servidor que devolve `null`, um array ou um objeto sem `type` derruba a
/// serialização do pedido de ferramenta no llama-server; um objeto vazio é
/// sempre aceito e significa "sem parâmetros".
fn sanitize_schema(schema: Value) -> Value {
    match schema {
        Value::Object(mut map) => {
            if !map.contains_key("type") {
                map.insert("type".into(), Value::String("object".into()));
            }
            Value::Object(map)
        }
        _ => serde_json::json!({"type": "object", "properties": {}}),
    }
}

/// Resultado bruto de uma chamada, já achatado em texto.
#[derive(Debug, Clone)]
pub struct McpCallOutput {
    pub text: String,
    /// `isError: true` na resposta — a ferramenta rodou e falhou.
    pub is_error: bool,
}

/// Uma conexão viva com um servidor.
pub struct McpClient {
    server: String,
    service: RunningService<RoleClient, ()>,
}

impl McpClient {
    /// Sobe a conexão e faz o handshake.
    pub async fn connect(cfg: &McpServerConfig) -> Result<Self, McpError> {
        let server = cfg.id.clone();
        let fut = async {
            match &cfg.transport {
                McpTransport::Stdio { .. } => Self::connect_stdio(cfg).await,
                McpTransport::Http { url, headers } => Self::connect_http(cfg, url, headers).await,
            }
        };
        match tokio::time::timeout(CONNECT_TIMEOUT, fut).await {
            Ok(result) => result,
            Err(_) => Err(McpError::Timeout {
                server,
                secs: CONNECT_TIMEOUT.as_secs(),
            }),
        }
    }

    async fn connect_stdio(cfg: &McpServerConfig) -> Result<Self, McpError> {
        let (program, args) = cfg.launch().expect("stdio sempre tem comando");
        let mut command = tokio::process::Command::new(&program);
        command.args(&args);
        for (key, value) in cfg.env() {
            command.env(key, value);
        }
        // Sem isto o Windows abre um console preto a cada conector.
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let transport = TokioChildProcess::new(command).map_err(|e| McpError::Connect {
            server: cfg.id.clone(),
            reason: launch_hint(&program, &e),
        })?;
        let service = ().serve(transport).await.map_err(|e| McpError::Connect {
            server: cfg.id.clone(),
            reason: e.to_string(),
        })?;
        Ok(Self {
            server: cfg.id.clone(),
            service,
        })
    }

    async fn connect_http(
        cfg: &McpServerConfig,
        url: &str,
        headers: &std::collections::BTreeMap<String, String>,
    ) -> Result<Self, McpError> {
        let mut config = StreamableHttpClientTransportConfig::with_uri(url.to_string());
        for (name, value) in headers {
            let Ok(header) = http::HeaderName::try_from(name.as_str()) else {
                return Err(McpError::Config(format!("cabeçalho inválido: `{name}`")));
            };
            let Ok(header_value) = http::HeaderValue::try_from(value.as_str()) else {
                return Err(McpError::Config(format!(
                    "valor inválido para o cabeçalho `{name}`"
                )));
            };
            config.custom_headers.insert(header, header_value);
        }

        let transport = StreamableHttpClientTransport::from_config(config);
        let service = ().serve(transport).await.map_err(|e| McpError::Connect {
            server: cfg.id.clone(),
            reason: e.to_string(),
        })?;
        Ok(Self {
            server: cfg.id.clone(),
            service,
        })
    }

    /// Catálogo completo (o SDK pagina sozinho).
    pub async fn list_tools(&self) -> Result<Vec<McpToolDef>, McpError> {
        let listing = tokio::time::timeout(LIST_TIMEOUT, self.service.list_all_tools())
            .await
            .map_err(|_| McpError::Timeout {
                server: self.server.clone(),
                secs: LIST_TIMEOUT.as_secs(),
            })?
            .map_err(|e| McpError::Protocol {
                server: self.server.clone(),
                reason: e.to_string(),
            })?;
        Ok(listing.iter().map(tool_from_rmcp).collect())
    }

    /// Executa uma ferramenta pelo nome remoto.
    pub async fn call(&self, remote_name: &str, args: Value) -> Result<McpCallOutput, McpError> {
        let arguments = match args {
            Value::Object(map) => Some(map),
            Value::Null => None,
            other => {
                // Modelo pequeno às vezes manda um valor solto; embrulhar é
                // melhor do que recusar, e o servidor dirá o que falta.
                let mut map = Map::new();
                map.insert("value".into(), other);
                Some(map)
            }
        };
        let mut params = CallToolRequestParams::new(remote_name.to_string());
        params.arguments = arguments;

        let result = tokio::time::timeout(CALL_TIMEOUT, self.service.call_tool(params))
            .await
            .map_err(|_| McpError::Timeout {
                server: self.server.clone(),
                secs: CALL_TIMEOUT.as_secs(),
            })?
            .map_err(|e| McpError::Protocol {
                server: self.server.clone(),
                reason: e.to_string(),
            })?;

        Ok(render(&result))
    }

    /// Encerra a conexão (e o processo filho, no stdio).
    pub async fn disconnect(self) {
        if let Err(e) = self.service.cancel().await {
            log::debug!("conector `{}` não encerrou limpo: {e}", self.server);
        }
    }
}

/// Falha de spawn no Windows quase sempre é o `.cmd`/PATH. Dizer isso poupa
/// a pessoa de descobrir sozinha por que "funciona no terminal".
fn launch_hint(program: &str, err: &std::io::Error) -> String {
    if err.kind() == std::io::ErrorKind::NotFound {
        format!("`{program}` não foi encontrado no sistema ({err})")
    } else {
        format!("{err}")
    }
}

/// Achata o resultado num texto para o modelo.
///
/// Imagens e áudios viram um resumo: mandar base64 para um modelo de texto só
/// queima contexto. Conteúdo estruturado entra quando não há texto nenhum.
fn render(result: &CallToolResult) -> McpCallOutput {
    let mut parts: Vec<String> = Vec::new();
    for block in &result.content {
        match block {
            ContentBlock::Text(text) => parts.push(text.text.clone()),
            ContentBlock::Image(img) => parts.push(format!(
                "[imagem {} — {} bytes em base64, não exibível aqui]",
                img.mime_type,
                img.data.len()
            )),
            other => {
                // Recursos e links: o JSON compacto preserva `uri` e `text`
                // sem precisar acompanhar cada variante nova do protocolo.
                let json = serde_json::to_string(other).unwrap_or_else(|_| "[conteúdo]".into());
                parts.push(json);
            }
        }
    }
    if parts.is_empty()
        && let Some(structured) = &result.structured_content
    {
        parts.push(
            serde_json::to_string_pretty(structured).unwrap_or_else(|_| structured.to_string()),
        );
    }
    let text = if parts.is_empty() {
        "(o conector respondeu sem conteúdo)".to_string()
    } else {
        parts.join("\n")
    };
    McpCallOutput {
        text,
        is_error: result.is_error.unwrap_or(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::{Tool, ToolAnnotations};
    use serde_json::json;
    use std::sync::Arc;

    fn schema() -> Arc<rmcp::model::JsonObject> {
        Arc::new(
            json!({"type":"object","properties":{"q":{"type":"string"}}})
                .as_object()
                .unwrap()
                .clone(),
        )
    }

    fn rmcp_tool(name: &str, annotations: Option<ToolAnnotations>) -> Tool {
        let tool = Tool::new(name.to_string(), "descrição".to_string(), schema());
        match annotations {
            Some(a) => tool.with_annotations(a),
            None => tool,
        }
    }

    #[test]
    fn read_only_hint_never_downgrades_the_category() {
        // O ponto central: `readOnlyHint` é um selo, não uma permissão.
        // Rebaixar para `Read` faria a política auto-aprovar por causa de um
        // campo que o próprio servidor escreve.
        let def = tool_from_rmcp(&rmcp_tool(
            "search",
            Some(ToolAnnotations::new().read_only(true)),
        ));
        assert!(def.read_only());
        assert_eq!(def.tier(), ToolTier::Safe);
        assert_eq!(def.category(), ToolCategory::Mcp);
        assert_ne!(def.category(), ToolCategory::Read);
    }

    #[test]
    fn open_world_hint_upgrades_to_network() {
        let def = tool_from_rmcp(&rmcp_tool(
            "fetch",
            Some(ToolAnnotations::new().open_world(true)),
        ));
        // `Network` pede confirmação inclusive no modo automático.
        assert_eq!(def.category(), ToolCategory::Network);
        assert_eq!(def.tier(), ToolTier::Danger);
    }

    #[test]
    fn destructive_hint_marks_danger() {
        let def = tool_from_rmcp(&rmcp_tool(
            "delete_repo",
            Some(ToolAnnotations::new().destructive(true)),
        ));
        assert_eq!(def.tier(), ToolTier::Danger);
        assert!(!def.read_only());
        assert_eq!(def.category(), ToolCategory::Mcp);
    }

    #[test]
    fn no_annotations_means_caution_and_mcp() {
        let def = tool_from_rmcp(&rmcp_tool("qualquer", None));
        assert_eq!(def.tier(), ToolTier::Caution);
        assert_eq!(def.category(), ToolCategory::Mcp);
        assert!(!def.read_only());
        assert_eq!(def.annotations, McpAnnotations::default());
    }

    #[test]
    fn spec_carries_the_server_origin_without_the_prefix() {
        let def = tool_from_rmcp(&rmcp_tool("search", None));
        let spec = def.spec("github");
        assert_eq!(spec.name, "search", "o prefixo é papel do registro");
        assert!(matches!(&spec.origin, ToolOrigin::Mcp { server_id } if server_id == "github"));
        assert_eq!(spec.parameters["type"], "object");
    }

    #[test]
    fn tool_names_are_sanitized_but_the_remote_name_is_kept() {
        let def = tool_from_rmcp(&rmcp_tool("github.create issue!", None));
        assert_eq!(def.name, "github_create_issue");
        assert_eq!(def.remote_name, "github.create issue!");
        assert!(
            def.name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        );
    }

    #[test]
    fn schema_without_type_is_repaired() {
        let tool = Tool::new(
            "t".to_string(),
            "d".to_string(),
            Arc::new(json!({"properties":{}}).as_object().unwrap().clone()),
        );
        assert_eq!(tool_from_rmcp(&tool).schema["type"], "object");
    }

    #[test]
    fn render_joins_text_and_flags_errors() {
        let ok = CallToolResult::success(vec![
            ContentBlock::text("linha 1"),
            ContentBlock::text("linha 2"),
        ]);
        let out = render(&ok);
        assert_eq!(out.text, "linha 1\nlinha 2");
        assert!(!out.is_error);

        let bad = CallToolResult::error(vec![ContentBlock::text("token expirado")]);
        let out = render(&bad);
        assert!(out.is_error);
        assert!(out.text.contains("token expirado"));
    }

    #[test]
    fn render_summarizes_binary_and_empty_answers() {
        let img = CallToolResult::success(vec![ContentBlock::image("AAAA", "image/png")]);
        let out = render(&img);
        assert!(out.text.contains("image/png"), "{}", out.text);
        assert!(!out.text.contains("AAAA"), "base64 não vai para o modelo");

        let empty = CallToolResult::success(vec![]);
        assert!(render(&empty).text.contains("sem conteúdo"));
    }
}
