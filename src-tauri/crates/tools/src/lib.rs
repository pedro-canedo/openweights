//! Ferramentas do agente.
//!
//! Uma ferramenta é uma função com nome, descrição e um JSON Schema simples
//! — é o vocabulário que o modelo usa para agir no mundo. O registro
//! ([`ToolRegistry`]) junta as ferramentas nativas e as vindas de conectores
//! MCP sob a mesma interface.
//!
//! Duas regras valem para toda ferramenta:
//! 1. **Nada executa sem passar pela política** — quem chama é o loop do
//!    agente, que consulta `lr_policy` antes.
//! 2. **Resultado sempre volta ao modelo**, inclusive erro. Um erro bem
//!    explicado é o que permite o agente se corrigir sozinho.

use async_trait::async_trait;
use lr_types::agent::{ToolCategory, ToolPreview, ToolSpec, ToolTier};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub mod diff;
pub mod fs;
pub mod registry;
pub mod spawner;
pub mod terminal;
pub mod todo;

pub use fs::builtin_registry;
pub use registry::{ToolProvider, ToolRegistry};

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("argumento inválido: {0}")]
    InvalidArgs(String),
    #[error("caminho fora da pasta do projeto: {0}")]
    OutsideWorkspace(String),
    #[error("arquivo não encontrado: {0}")]
    NotFound(String),
    #[error("falha de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("tempo esgotado após {0}s")]
    Timeout(u64),
    #[error("cancelado")]
    Cancelled,
    #[error("{0}")]
    Other(String),
}

impl ToolError {
    /// Mensagem que volta para o modelo. Deve ser acionável: dizer o que deu
    /// errado E o que tentar em seguida.
    pub fn to_model_message(&self) -> String {
        match self {
            ToolError::InvalidArgs(m) => {
                format!("Argumentos inválidos: {m}. Revise o formato e tente de novo.")
            }
            ToolError::OutsideWorkspace(p) => format!(
                "O caminho `{p}` está fora da pasta do projeto. Use caminhos relativos a ela."
            ),
            ToolError::NotFound(p) => {
                format!("`{p}` não existe. Liste a pasta antes para conferir o caminho.")
            }
            ToolError::Timeout(s) => {
                format!("A operação passou de {s}s e foi interrompida. Tente algo mais específico.")
            }
            other => other.to_string(),
        }
    }
}

pub type ToolResult<T> = Result<T, ToolError>;

/// Contexto de execução de uma chamada.
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// Pasta do projeto. `None` = agente sem workspace (só ferramentas meta).
    pub workspace: Option<PathBuf>,
    /// Identificador da chamada (para correlacionar saída em streaming).
    pub call_id: String,
    /// Teto de bytes que o resultado pode devolver ao modelo.
    pub max_output_bytes: usize,
}

impl ToolContext {
    pub fn new(workspace: Option<PathBuf>, call_id: impl Into<String>) -> Self {
        Self {
            workspace,
            call_id: call_id.into(),
            max_output_bytes: 30_000,
        }
    }

    /// Resolve um caminho relativo dentro do projeto, recusando escapes.
    ///
    /// Aceita apenas caminhos relativos e sem `..`; resolve links simbólicos
    /// do que já existe e confirma que o resultado continua sob a raiz.
    pub fn resolve(&self, rel: &str) -> ToolResult<PathBuf> {
        let root = self
            .workspace
            .as_ref()
            .ok_or_else(|| ToolError::Other("nenhuma pasta de projeto foi escolhida".into()))?;
        resolve_under(root, rel)
    }

    /// Caminho relativo (com `/`) de um absoluto dentro do projeto.
    pub fn relativize(&self, path: &Path) -> String {
        match &self.workspace {
            Some(root) => path
                .strip_prefix(root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| path.to_string_lossy().into_owned()),
            None => path.to_string_lossy().into_owned(),
        }
    }
}

/// Resolve `rel` sob `root` recusando qualquer saída da pasta.
pub fn resolve_under(root: &Path, rel: &str) -> ToolResult<PathBuf> {
    let rel = rel.trim().replace('\\', "/");
    let candidate = Path::new(&rel);

    if candidate.is_absolute() || rel.contains(':') {
        return Err(ToolError::OutsideWorkspace(rel));
    }

    let mut out = root.to_path_buf();
    for comp in candidate.components() {
        use std::path::Component;
        match comp {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => return Err(ToolError::OutsideWorkspace(rel)),
        }
    }

    // Confirma contra a forma canônica do que já existe (pega symlink).
    let base = root
        .canonicalize()
        .map_err(|_| ToolError::Other("pasta do projeto inacessível".into()))?;
    let check = if out.exists() {
        out.canonicalize().unwrap_or_else(|_| out.clone())
    } else {
        // Arquivo novo: valida o ancestral existente mais próximo.
        let mut probe = out.clone();
        while !probe.exists() {
            match probe.parent() {
                Some(p) => probe = p.to_path_buf(),
                None => break,
            }
        }
        probe.canonicalize().unwrap_or(probe)
    };
    if !check.starts_with(&base) {
        return Err(ToolError::OutsideWorkspace(rel));
    }
    Ok(out)
}

/// O que uma ferramenta devolve.
#[derive(Debug, Clone)]
pub struct ToolOutput {
    /// Texto que volta para o modelo (já limitado em tamanho).
    pub content: String,
    /// Tamanho real antes do corte.
    pub bytes_total: u64,
    /// Caminhos relativos alterados (alimenta o checkpoint e a UI).
    pub changed_files: Vec<String>,
    /// Código de saída, quando a ferramenta rodou um processo.
    ///
    /// Existe para a verificação não depender de MARCADOR TEXTUAL: procurar
    /// "exit code N" no texto pegava o "exit code 1" de dentro do log de um
    /// `cargo test` que na verdade saiu com 0 — e derrubava a verificação de
    /// um comando que passou. Quem roda processo DECLARA o código; o marcador
    /// fica só como reserva para ferramentas que não sabem (MCP).
    pub exit_code: Option<i32>,
}

impl ToolOutput {
    pub fn text(content: impl Into<String>) -> Self {
        let content = content.into();
        Self {
            bytes_total: content.len() as u64,
            content,
            changed_files: Vec::new(),
            exit_code: None,
        }
    }

    pub fn with_changed(mut self, files: Vec<String>) -> Self {
        self.changed_files = files;
        self
    }

    /// Declara o código de saída do processo que a ferramenta rodou.
    pub fn with_exit_code(mut self, code: Option<i32>) -> Self {
        self.exit_code = code;
        self
    }

    /// Corta o conteúdo no teto, avisando o modelo do corte.
    pub fn truncated_to(mut self, max: usize) -> Self {
        if self.content.len() > max {
            let mut cut = max.min(self.content.len());
            while cut > 0 && !self.content.is_char_boundary(cut) {
                cut -= 1;
            }
            self.content.truncate(cut);
            self.content
                .push_str("\n[...saída cortada por ser muito longa...]");
        }
        self
    }
}

/// Uma ferramenta executável.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Nome usado pelo modelo (snake_case, sem espaços).
    fn name(&self) -> &str;

    /// Descrição curta: o que faz e quando usar.
    fn description(&self) -> &str;

    /// JSON Schema dos parâmetros — mantenha plano e com enums.
    fn parameters(&self) -> Value;

    fn category(&self) -> ToolCategory;

    /// Esta ferramenta só lê, em QUALQUER chamada?
    ///
    /// É a promessa que o modo planejamento e o ajudante explorador usam para
    /// montar o cardápio deles — quem responde `true` está dizendo "pode me
    /// oferecer para quem não tem permissão de alterar nada". Por isso não
    /// basta olhar a categoria fixa: ferramenta que muda de natureza conforme
    /// o argumento (veja [`Tool::category_for`]) tem que responder `false`,
    /// ou escapa dos dois filtros antes de a política ser consultada.
    fn read_only(&self) -> bool {
        matches!(self.category(), ToolCategory::Read | ToolCategory::Meta)
    }

    /// Categoria desta chamada específica.
    ///
    /// A mesma ferramenta pode ser leitura ou escrita conforme os
    /// argumentos — `sql_query` consulta por padrão e altera quando recebe
    /// permissão explícita. O laço usa esta versão para decidir se pede
    /// confirmação, então é ela que precisa dizer a verdade.
    fn category_for(&self, _args: &Value) -> ToolCategory {
        self.category()
    }

    fn tier(&self) -> ToolTier {
        match self.category() {
            ToolCategory::Read | ToolCategory::Meta => ToolTier::Safe,
            ToolCategory::Edit | ToolCategory::Execute => ToolTier::Caution,
            _ => ToolTier::Danger,
        }
    }

    /// Prévia para a tela de confirmação (diff, comando, texto).
    /// Roda ANTES da execução e não pode ter efeito colateral.
    async fn preview(&self, _args: &Value, _ctx: &ToolContext) -> Option<ToolPreview> {
        None
    }

    /// Todos os caminhos tocados estão dentro do projeto?
    fn within_workspace(&self, _args: &Value, _ctx: &ToolContext) -> bool {
        true
    }

    /// Arquivos que serão alterados (para o checkpoint antes da execução).
    fn files_at_risk(&self, _args: &Value, _ctx: &ToolContext) -> Vec<String> {
        Vec::new()
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput>;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters(),
            category: self.category(),
            tier: self.tier(),
            origin: lr_types::agent::ToolOrigin::Builtin,
            read_only: self.read_only(),
        }
    }
}

pub type SharedTool = Arc<dyn Tool>;

/// Helpers de leitura de argumentos com erro amigável.
pub fn arg_str(args: &Value, key: &str) -> ToolResult<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| ToolError::InvalidArgs(format!("faltou `{key}` (texto)")))
}

pub fn arg_str_opt(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

pub fn arg_u64(args: &Value, key: &str, default: u64) -> u64 {
    args.get(key).and_then(|v| v.as_u64()).unwrap_or(default)
}

pub fn arg_bool(args: &Value, key: &str, default: bool) -> bool {
    args.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_accepts_relative_paths() {
        let dir = tempfile::tempdir().unwrap();
        let p = resolve_under(dir.path(), "src/main.rs").unwrap();
        assert!(p.ends_with("main.rs"));
        assert!(p.starts_with(dir.path().canonicalize().unwrap_or(dir.path().into())));
    }

    #[test]
    fn resolve_rejects_escapes() {
        let dir = tempfile::tempdir().unwrap();
        for bad in [
            "../fora.txt",
            "a/../../fora.txt",
            "/etc/passwd",
            r"C:\Windows",
        ] {
            assert!(
                resolve_under(dir.path(), bad).is_err(),
                "deveria recusar: {bad}"
            );
        }
    }

    #[test]
    fn resolve_normalizes_backslashes_and_curdir() {
        let dir = tempfile::tempdir().unwrap();
        let a = resolve_under(dir.path(), r"src\lib.rs").unwrap();
        let b = resolve_under(dir.path(), "./src/lib.rs").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn output_truncation_keeps_utf8_valid() {
        let out = ToolOutput::text("çãéíõ".repeat(100)).truncated_to(11);
        assert!(out.content.len() <= 11 + 50);
        assert!(out.content.contains("cortada"));
        // Não deve panicar nem gerar UTF-8 inválido.
        assert!(std::str::from_utf8(out.content.as_bytes()).is_ok());
    }

    #[test]
    fn error_messages_guide_the_model() {
        let msg = ToolError::NotFound("a.txt".into()).to_model_message();
        assert!(msg.contains("a.txt"));
        assert!(
            msg.contains("Liste a pasta"),
            "deve sugerir o próximo passo"
        );
    }
}
