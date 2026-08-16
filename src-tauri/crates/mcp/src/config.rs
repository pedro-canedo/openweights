//! Leitura da configuração de um conector MCP.
//!
//! A convenção `mcpServers` virou o formato de fato do ecossistema (Claude
//! Desktop, Cursor, VS Code): a pessoa copia um bloco JSON do README do
//! servidor e cola. Por isso este módulo aceita as três formas em que esse
//! bloco costuma aparecer — o objeto inteiro com a chave `mcpServers`, um
//! mapa cru de servidores, ou uma única entrada — em vez de exigir que ela
//! desmonte o JSON à mão.
//!
//! ## Por que a normalização do Windows acontece na execução
//!
//! `npx`, `uvx`, `npm`, `pnpm` e `yarn` são arquivos `.cmd` no Windows, e
//! `CreateProcess` não executa `.cmd` diretamente; além disso um app de
//! janela não herda o `PATH` do shell interativo. A saída é rodar via
//! `cmd /c <programa> <args>`. Isso é feito só na hora de subir o processo
//! ([`McpServerConfig::launch`]) e nunca reescreve o que a pessoa digitou:
//! o JSON guardado continua igual ao do README, então exportar a
//! configuração de volta (ou abrir o mesmo perfil noutro sistema) funciona.

use crate::error::McpError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Programas que no Windows são scripts `.cmd` e precisam do `cmd /c`.
const SHELL_WRAPPED: [&str; 5] = ["npx", "uvx", "npm", "pnpm", "yarn"];

/// Como o app fala com o servidor. A especificação tem só estes dois
/// transportes vivos — o SSE legado foi removido e não é suportado.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    rename_all_fields = "camelCase"
)]
pub enum McpTransport {
    /// Processo filho falando JSON-RPC por stdin/stdout.
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
    },
    /// Servidor remoto por HTTP streamable.
    Http {
        url: String,
        headers: BTreeMap<String, String>,
    },
}

/// Um conector pronto para ser gravado e conectado.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerConfig {
    /// Identificador estável, derivado do nome. Vira o prefixo dos nomes de
    /// ferramenta expostos ao modelo (`github__create_issue`).
    pub id: String,
    pub name: String,
    pub transport: McpTransport,
    /// `"disabled": true` no JSON de origem — respeitado ao gravar.
    pub disabled: bool,
}

/// Entrada crua do bloco `mcpServers`, como aparece nos READMEs.
///
/// Campos desconhecidos são ignorados de propósito: cada cliente inventa
/// extensões próprias (`alwaysAllow`, `timeout`, `autoApprove`) e recusar o
/// JSON por causa delas só faria a pessoa editar à mão o que copiou.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEntry {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default, alias = "serverUrl", alias = "httpUrl")]
    url: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    disabled: bool,
}

impl McpServerConfig {
    /// `"stdio"` ou `"http"` — o que vai para a coluna `transport`.
    pub fn transport_kind(&self) -> &'static str {
        match self.transport {
            McpTransport::Stdio { .. } => "stdio",
            McpTransport::Http { .. } => "http",
        }
    }

    /// JSON canônico da entrada, no mesmo formato de `mcpServers`. É o que
    /// fica no banco: reversível, legível e colável de volta noutro cliente.
    pub fn to_config_json(&self) -> String {
        let value = match &self.transport {
            McpTransport::Stdio { command, args, env } => serde_json::json!({
                "type": "stdio",
                "command": command,
                "args": args,
                "env": env,
                "disabled": self.disabled,
            }),
            McpTransport::Http { url, headers } => serde_json::json!({
                "type": "http",
                "url": url,
                "headers": headers,
                "disabled": self.disabled,
            }),
        };
        value.to_string()
    }

    /// Programa e argumentos já normalizados para o sistema atual.
    /// `None` para conectores HTTP, que não sobem processo.
    pub fn launch(&self) -> Option<(String, Vec<String>)> {
        match &self.transport {
            McpTransport::Stdio { command, args, .. } => {
                Some(normalize_launch(command, args, cfg!(windows)))
            }
            McpTransport::Http { .. } => None,
        }
    }

    /// Variáveis extras para o processo filho (vazio no HTTP).
    pub fn env(&self) -> BTreeMap<String, String> {
        match &self.transport {
            McpTransport::Stdio { env, .. } => env.clone(),
            McpTransport::Http { .. } => BTreeMap::new(),
        }
    }

    /// Troca o nome e recalcula o id. Usado quando a pessoa nomeia no
    /// formulário um servidor colado sem nome.
    pub fn renamed(mut self, name: &str) -> Self {
        let name = name.trim();
        if !name.is_empty() {
            self.id = slug_id(name);
            self.name = name.to_string();
        }
        self
    }

    /// Resumo de uma linha para a lista de conectores.
    pub fn summary(&self) -> String {
        match &self.transport {
            McpTransport::Stdio { command, args, .. } => {
                if args.is_empty() {
                    command.clone()
                } else {
                    format!("{command} {}", args.join(" "))
                }
            }
            McpTransport::Http { url, .. } => url.clone(),
        }
    }
}

/// Lê um bloco de configuração e devolve todos os servidores que ele define.
///
/// Aceita, nesta ordem: `{"mcpServers": {...}}` (ou `{"servers": {...}}`,
/// como no VS Code), um mapa cru `{"nome": {...}}`, ou uma entrada única
/// `{"command": ...}` / `{"url": ...}`.
pub fn parse_servers(json: &str) -> Result<Vec<McpServerConfig>, McpError> {
    let text = json.trim();
    if text.is_empty() {
        return Err(McpError::Config("cole a configuração do servidor".into()));
    }
    let root: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| McpError::Config(format!("não é um JSON válido ({e})")))?;
    let obj = root
        .as_object()
        .ok_or_else(|| McpError::Config("esperava um objeto JSON `{ ... }`".into()))?;

    // (1) Envelope conhecido.
    for key in ["mcpServers", "servers"] {
        if let Some(inner) = obj.get(key) {
            let map = inner.as_object().ok_or_else(|| {
                McpError::Config(format!("`{key}` deveria ser um objeto de servidores"))
            })?;
            if map.is_empty() {
                return Err(McpError::Config(format!("`{key}` está vazio")));
            }
            return map
                .iter()
                .map(|(name, entry)| parse_entry(name, entry))
                .collect();
        }
    }

    // (2) Entrada única, sem nome: o nome sai do próprio comando/URL.
    if looks_like_entry(&root) {
        let cfg = parse_entry("", &root)?;
        let fallback = default_name(&cfg.transport);
        return Ok(vec![cfg.renamed(&fallback)]);
    }

    // (3) Mapa cru de servidores, sem envelope.
    if !obj.is_empty() && obj.values().all(looks_like_entry) {
        return obj
            .iter()
            .map(|(name, entry)| parse_entry(name, entry))
            .collect();
    }

    Err(McpError::Config(
        "não encontrei `mcpServers`, nem `command`, nem `url` neste JSON".into(),
    ))
}

/// Um valor parece a entrada de um servidor?
fn looks_like_entry(v: &serde_json::Value) -> bool {
    v.as_object().is_some_and(|o| {
        ["command", "url", "serverUrl", "httpUrl"]
            .iter()
            .any(|k| o.contains_key(*k))
    })
}

fn parse_entry(name: &str, value: &serde_json::Value) -> Result<McpServerConfig, McpError> {
    let raw: RawEntry = serde_json::from_value(value.clone()).map_err(|e| {
        McpError::Config(format!(
            "entrada `{}` inválida: {e}",
            if name.is_empty() { "(sem nome)" } else { name }
        ))
    })?;
    build(name, raw)
}

fn build(name: &str, raw: RawEntry) -> Result<McpServerConfig, McpError> {
    let command = raw
        .command
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty());
    let url = raw
        .url
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty());
    let kind = raw.kind.unwrap_or_default().to_ascii_lowercase();

    // O SSE legado saiu da especificação; dizer isso é mais útil do que
    // falhar com "faltou url" quando a pessoa cola um JSON antigo.
    if kind == "sse" {
        return Err(McpError::Config(
            "o transporte SSE foi removido da especificação — use `url` (HTTP streamable) \
             ou `command` (programa local)"
                .into(),
        ));
    }

    let transport = match (command, url) {
        (Some(_), Some(_)) => {
            return Err(McpError::Config(
                "informe `command` (programa local) OU `url` (servidor remoto), não os dois".into(),
            ));
        }
        (Some(command), None) => {
            if kind == "http" || kind == "streamable-http" || kind == "streamablehttp" {
                return Err(McpError::Config(
                    "`type` diz HTTP mas só veio `command`; informe `url`".into(),
                ));
            }
            McpTransport::Stdio {
                command,
                args: raw.args,
                env: raw.env,
            }
        }
        (None, Some(url)) => {
            if kind == "stdio" {
                return Err(McpError::Config(
                    "`type` diz stdio mas só veio `url`; informe `command`".into(),
                ));
            }
            validate_url(&url)?;
            McpTransport::Http {
                url,
                headers: raw.headers,
            }
        }
        (None, None) => {
            return Err(McpError::Config(
                "faltou `command` (programa local) ou `url` (servidor remoto)".into(),
            ));
        }
    };

    let name = name.trim();
    let name = if name.is_empty() {
        default_name(&transport)
    } else {
        name.to_string()
    };

    Ok(McpServerConfig {
        id: slug_id(&name),
        name,
        transport,
        disabled: raw.disabled,
    })
}

/// Nome plausível quando a entrada veio sem um: o programa ou o domínio.
fn default_name(transport: &McpTransport) -> String {
    let raw = match transport {
        McpTransport::Stdio { command, args, .. } => {
            // `npx -y @escopo/servidor-github` → "servidor-github" diz mais
            // do que "npx", que seria o nome de metade dos conectores.
            let pkg = args
                .iter()
                .find(|a| !a.starts_with('-') && !a.is_empty())
                .cloned();
            pkg.unwrap_or_else(|| command.clone())
        }
        McpTransport::Http { url, .. } => url
            .split("://")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .unwrap_or(url)
            .to_string(),
    };
    let base = raw
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(&raw)
        .trim_start_matches('@');
    let cleaned = base.trim();
    if cleaned.is_empty() {
        "conector".to_string()
    } else {
        cleaned.to_string()
    }
}

/// Recusa URLs que não são HTTP(S) ou que não têm host.
///
/// Um `url::Url` completo seria mais rigoroso, mas traria a árvore do `idna`
/// só para isto; o que importa aqui é barrar `file://`, `stdio://` e texto
/// solto antes de tentar conectar.
fn validate_url(url: &str) -> Result<(), McpError> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| {
            McpError::Config(format!("`{url}` precisa começar com http:// ou https://"))
        })?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() || host.chars().any(char::is_whitespace) {
        return Err(McpError::Config(format!(
            "`{url}` não tem um endereço válido"
        )));
    }
    if url.chars().any(char::is_whitespace) {
        return Err(McpError::Config("a URL não pode ter espaços".into()));
    }
    Ok(())
}

/// Id estável a partir do nome.
///
/// Só `[a-z0-9_]`, sem `__` repetido: o registro de ferramentas separa
/// provedor e ferramenta por `__` no primeiro par de sublinhados, então um id
/// com `__` quebraria o roteamento da chamada.
pub fn slug_id(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else {
            '_'
        };
        if mapped == '_' && out.ends_with('_') {
            continue;
        }
        out.push(mapped);
    }
    let trimmed = out.trim_matches('_');
    let mut id: String = trimmed.chars().take(40).collect();
    id = id.trim_matches('_').to_string();
    if id.is_empty() { "mcp".to_string() } else { id }
}

/// Aplica a regra do `cmd /c` para os lançadores que são `.cmd` no Windows.
///
/// `windows` é parâmetro (e não `cfg!`) para que o comportamento dos dois
/// sistemas seja testável a partir de qualquer máquina — este é justamente o
/// trecho que não dá para conferir rodando os testes no Linux.
pub fn normalize_launch(command: &str, args: &[String], windows: bool) -> (String, Vec<String>) {
    if windows && needs_shell(command) {
        let mut out = Vec::with_capacity(args.len() + 2);
        out.push("/c".to_string());
        out.push(command.to_string());
        out.extend(args.iter().cloned());
        return ("cmd".to_string(), out);
    }
    (command.to_string(), args.to_vec())
}

/// O programa é um dos lançadores que viram `.cmd` no Windows?
fn needs_shell(command: &str) -> bool {
    let base = command
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(command)
        .to_ascii_lowercase();
    let stem = base.split('.').next().unwrap_or(&base);
    SHELL_WRAPPED.contains(&stem)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_the_whole_mcp_servers_block() {
        let json = r#"{
          "mcpServers": {
            "GitHub": {
              "command": "npx",
              "args": ["-y", "@modelcontextprotocol/server-github"],
              "env": {"GITHUB_TOKEN": "abc"}
            },
            "docs": { "url": "https://exemplo.com/mcp", "headers": {"X-Key": "1"} }
          }
        }"#;
        let mut servers = parse_servers(json).unwrap();
        servers.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(servers.len(), 2);

        let docs = &servers[0];
        assert_eq!(docs.id, "docs");
        assert_eq!(docs.transport_kind(), "http");
        assert!(
            matches!(&docs.transport, McpTransport::Http { url, headers }
            if url == "https://exemplo.com/mcp" && headers["X-Key"] == "1")
        );

        let gh = &servers[1];
        assert_eq!(gh.id, "github", "o id é o nome em minúsculas");
        assert_eq!(gh.name, "GitHub", "o nome digitado é preservado");
        assert!(
            matches!(&gh.transport, McpTransport::Stdio { command, args, env }
            if command == "npx" && args.len() == 2 && env["GITHUB_TOKEN"] == "abc")
        );
    }

    #[test]
    fn parses_a_single_entry_without_the_envelope() {
        let json = r#"{"command":"uvx","args":["mcp-server-fetch"]}"#;
        let servers = parse_servers(json).unwrap();
        assert_eq!(servers.len(), 1);
        // Sem nome no JSON, o pacote é um nome melhor do que "uvx".
        assert_eq!(servers[0].id, "mcp_server_fetch");
        assert_eq!(servers[0].transport_kind(), "stdio");
    }

    #[test]
    fn parses_a_bare_map_of_servers() {
        let json = r#"{"fetch": {"command":"uvx","args":["mcp-server-fetch"]}}"#;
        let servers = parse_servers(json).unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "fetch");
    }

    #[test]
    fn accepts_the_vs_code_envelope_and_unknown_fields() {
        let json = r#"{"servers": {"a": {"command":"node","args":["s.js"],
                       "alwaysAllow":["x"],"timeout":60}}}"#;
        let servers = parse_servers(json).unwrap();
        assert_eq!(servers[0].id, "a");
    }

    #[test]
    fn rejects_broken_configs_with_a_readable_reason() {
        let cases = [
            ("", "cole"),
            ("não é json", "JSON válido"),
            (r#"{"mcpServers": {}}"#, "vazio"),
            (r#"{"foo": 1}"#, "não encontrei"),
            (r#"{"command": "   "}"#, "faltou"),
            (r#"{"command":"npx","url":"https://a.com"}"#, "não os dois"),
            (r#"{"url":"ftp://a.com"}"#, "http://"),
            (r#"{"url":"https://"}"#, "endereço válido"),
            (r#"{"type":"sse","url":"https://a.com/sse"}"#, "SSE"),
            (
                r#"{"type":"stdio","url":"https://a.com"}"#,
                "informe `command`",
            ),
        ];
        for (json, needle) in cases {
            let err = parse_servers(json).unwrap_err().to_string();
            assert!(
                err.contains(needle),
                "esperava `{needle}` no erro de `{json}`, veio: {err}"
            );
        }
    }

    #[test]
    fn windows_wraps_only_the_cmd_launchers() {
        for prog in ["npx", "uvx", "npm", "pnpm", "yarn", "NPX", "npx.cmd"] {
            let (cmd, out) = normalize_launch(prog, &args(&["-y", "pacote"]), true);
            assert_eq!(cmd, "cmd", "{prog} deveria passar pelo cmd");
            assert_eq!(out, args(&["/c", prog, "-y", "pacote"]));
        }
        // Binários de verdade sobem direto, mesmo no Windows.
        for prog in ["node", "python", "C:/tools/meu-servidor.exe", "cmd"] {
            let (cmd, out) = normalize_launch(prog, &args(&["a"]), true);
            assert_eq!(cmd, prog);
            assert_eq!(out, args(&["a"]));
        }
    }

    #[test]
    fn other_systems_never_get_the_cmd_wrapper() {
        let (cmd, out) = normalize_launch("npx", &args(&["-y", "pacote"]), false);
        assert_eq!(cmd, "npx");
        assert_eq!(out, args(&["-y", "pacote"]));
    }

    #[test]
    fn normalization_does_not_rewrite_the_stored_config() {
        let cfg = parse_servers(r#"{"mcpServers":{"a":{"command":"npx","args":["-y","p"]}}}"#)
            .unwrap()
            .remove(0);
        let stored = cfg.to_config_json();
        assert!(stored.contains("\"npx\""), "o JSON guardado é o original");
        assert!(
            !stored.contains("cmd"),
            "nada de `cmd /c` no banco: {stored}"
        );

        // E o que é gravado volta a ser lido igual.
        let back = parse_servers(&stored).unwrap().remove(0);
        assert_eq!(back.transport, cfg.transport);
    }

    #[test]
    fn slug_never_produces_a_double_underscore() {
        // `__` é o separador provedor/ferramenta no registro: um id com `__`
        // faria a chamada ser roteada para um provedor que não existe.
        assert_eq!(slug_id("GitHub  ::  Issues"), "github_issues");
        assert_eq!(slug_id("  @escopo/servidor  "), "escopo_servidor");
        assert_eq!(slug_id("!!!"), "mcp");
        assert_eq!(slug_id(""), "mcp");
        for name in ["a  b", "a--b", "a__b", "***x***"] {
            assert!(!slug_id(name).contains("__"), "{name}");
        }
    }

    #[test]
    fn disabled_flag_survives_the_round_trip() {
        let cfg = parse_servers(r#"{"command":"node","args":["s.js"],"disabled":true}"#)
            .unwrap()
            .remove(0);
        assert!(cfg.disabled);
        assert!(parse_servers(&cfg.to_config_json()).unwrap()[0].disabled);
    }

    #[test]
    fn http_config_never_produces_a_launch_command() {
        let cfg = parse_servers(r#"{"url":"https://exemplo.com/mcp"}"#)
            .unwrap()
            .remove(0);
        assert!(cfg.launch().is_none());
        assert_eq!(cfg.name, "exemplo.com");
    }
}
