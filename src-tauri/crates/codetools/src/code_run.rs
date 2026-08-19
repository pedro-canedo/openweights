//! `code_run`: roda um trecho curto de Python ou Node.
//!
//! Serve para o que não vale um arquivo: conferir uma conta, ver o formato de
//! um JSON, testar uma expressão antes de colocá-la no código. Sem isto o
//! agente "simula mentalmente" o resultado — e modelo pequeno erra conta,
//! erra fuso horário e erra parsing com uma confiança impressionante.
//!
//! ## Por que um arquivo, e não `python -c`
//!
//! Passar o código na linha de comando parece mais simples e é pior em três
//! pontos: quebra em qualquer trecho com aspas ou quebra de linha, aparece
//! inteiro na lista de processos da máquina, e o erro vem sem número de linha
//! útil (`<string>:1`). Com arquivo o traceback aponta a linha certa, que é
//! justamente o que o agente precisa ler para se corrigir.
//!
//! ## Onde o arquivo mora, e por quanto tempo
//!
//! Em `.openweights/scratch/`, dentro do projeto — não em `/tmp` — para que o
//! trecho enxergue os arquivos do projeto por caminho relativo, que é como o
//! modelo escreve. A limpeza é feita por um guarda com `Drop`: some no fim,
//! **inclusive** quando a execução falha, estoura o tempo ou o interpretador
//! nem existe. Deixar lixo dentro do projeto do usuário não é opção.

use crate::detect;
use crate::exec::{self};
use crate::{budget, timeout_of, workspace};
use async_trait::async_trait;
use lr_tools::{Tool, ToolContext, ToolError, ToolOutput, ToolResult, arg_str};
use lr_types::agent::{ToolCategory, ToolPreview};
use serde_json::{Value, json};
use std::path::PathBuf;

/// Pasta dos arquivos temporários, dentro do projeto.
pub const SCRATCH_DIR: &str = ".openweights/scratch";

/// Tempo padrão de um trecho solto (é para ser rápido).
const DEFAULT_TIMEOUT_SECS: u64 = 60;

/// Teto de tamanho do trecho: acima disso é arquivo, não trecho.
const MAX_CODE_BYTES: usize = 128 * 1024;

/// Quanto do código aparece na tela de confirmação.
const PREVIEW_CHARS: usize = 2_000;

/// Roda um trecho de código em Python ou Node.
pub struct CodeRun {
    python: String,
    node: String,
}

impl Default for CodeRun {
    fn default() -> Self {
        Self {
            python: detect::python_program().to_string(),
            node: "node".to_string(),
        }
    }
}

impl CodeRun {
    /// Versão com outros nomes de interpretador (usada nos testes para
    /// exercitar o caminho "não está instalado" sem desinstalar nada).
    #[cfg(test)]
    pub(crate) fn with_programs(python: &str, node: &str) -> Self {
        Self {
            python: python.to_string(),
            node: node.to_string(),
        }
    }

    fn program_for(&self, language: Language) -> &str {
        match language {
            Language::Python => &self.python,
            Language::Node => &self.node,
        }
    }
}

/// As duas linguagens que aceitamos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Language {
    Python,
    Node,
}

impl Language {
    /// Aceita as grafias que um modelo escreve na prática.
    fn parse(raw: &str) -> ToolResult<Self> {
        match raw.trim().to_lowercase().as_str() {
            "python" | "python3" | "py" => Ok(Language::Python),
            "node" | "nodejs" | "js" | "javascript" => Ok(Language::Node),
            other => Err(ToolError::InvalidArgs(format!(
                "linguagem `{other}` não é suportada. Use \"python\" ou \"node\" — para outras \
                 linguagens, escreva um arquivo com `fs_write` e rode com `terminal_run`."
            ))),
        }
    }
}

/// Extensão do arquivo temporário.
///
/// No Node a extensão decide o sistema de módulos, e escolher errado quebra o
/// trecho antes de ele rodar: `.cjs` não aceita `import`, `.mjs` não aceita
/// `require`. Em vez de impor um dos dois, olhamos o que o código usa.
fn extension_for(language: Language, code: &str) -> &'static str {
    match language {
        Language::Python => "py",
        Language::Node if uses_esm(code) => "mjs",
        Language::Node => "cjs",
    }
}

fn uses_esm(code: &str) -> bool {
    if code.contains("require(") {
        return false;
    }
    code.lines().map(str::trim).any(|line| {
        line.starts_with("export ")
            || line.starts_with("export{")
            || (line.starts_with("import ") && line.contains(" from "))
            || line.starts_with("import{")
            || line.starts_with("import \"")
            || line.starts_with("import '")
    })
}

/// Arquivo temporário que se apaga sozinho.
struct Scratch {
    file: PathBuf,
    dir: PathBuf,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.file) {
            log::warn!("não consegui apagar {}: {e}", self.file.display());
        }
        // `remove_dir` só apaga pasta vazia — exatamente o que queremos: se
        // outra chamada estiver rodando ao mesmo tempo, a dela continua lá.
        let _ = std::fs::remove_dir(&self.dir);
    }
}

#[async_trait]
impl Tool for CodeRun {
    fn name(&self) -> &str {
        "code_run"
    }

    fn description(&self) -> &str {
        "Executa um trecho curto de código Python ou Node e devolve o que ele imprimir. Use para \
         conferir uma conta, testar uma expressão ou inspecionar dados — não para criar arquivos \
         do projeto (para isso use fs_write)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "language": {
                    "type": "string",
                    "enum": ["python", "node"],
                    "description": "Linguagem do trecho: \"python\" ou \"node\"."
                },
                "code": {
                    "type": "string",
                    "description": "O código a rodar, completo. Imprima o resultado (print/console.log) — só o que for impresso volta para você."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Tempo máximo em segundos antes de interromper. Padrão 60."
                }
            },
            "required": ["language", "code"],
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Execute
    }

    fn within_workspace(&self, _args: &Value, ctx: &ToolContext) -> bool {
        ctx.workspace.is_some()
    }

    async fn preview(&self, args: &Value, _ctx: &ToolContext) -> Option<ToolPreview> {
        let language = arg_str(args, "language").ok()?;
        let code = arg_str(args, "code").ok()?;
        let shown: String = code.chars().take(PREVIEW_CHARS).collect();
        let cortado = if shown.len() < code.len() {
            "\n[...trecho cortado na prévia...]"
        } else {
            ""
        };
        // A pessoa aprova o que vai rodar: o código aparece inteiro (ou quase).
        Some(ToolPreview::Text {
            body: format!("Rodar este trecho em {language}:\n\n{shown}{cortado}"),
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let root = workspace(ctx)?;
        let language = Language::parse(&arg_str(&args, "language")?)?;
        let code = arg_str(&args, "code")?;
        if code.trim().is_empty() {
            return Err(ToolError::InvalidArgs(
                "`code` está vazio — mande o trecho que quer rodar".into(),
            ));
        }
        if code.len() > MAX_CODE_BYTES {
            return Err(ToolError::InvalidArgs(format!(
                "o trecho tem {} bytes, acima do limite de {MAX_CODE_BYTES}. Grave num arquivo \
                 com `fs_write` e rode com `terminal_run`.",
                code.len()
            )));
        }

        let dir = ctx.resolve(SCRATCH_DIR)?;
        std::fs::create_dir_all(&dir)?;
        let file = dir.join(format!(
            "trecho_{}.{}",
            unique_suffix(&ctx.call_id),
            extension_for(language, &code)
        ));
        std::fs::write(&file, code.as_bytes())?;
        // A partir daqui, qualquer saída da função apaga o arquivo.
        let _scratch = Scratch {
            file: file.clone(),
            dir: dir.clone(),
        };

        let program = self.program_for(language);
        let cmd = detect::Cmd::new([program.to_string(), file.to_string_lossy().into_owned()]);
        let timeout = timeout_of(&args, DEFAULT_TIMEOUT_SECS);
        // Roda na raiz do projeto: caminho relativo dentro do trecho aponta
        // para o projeto, que é como o modelo escreve.
        let run = exec::run(&cmd, &root, timeout, budget(ctx), ctx.output_fn()).await?;

        let mut body = format!("{} — trecho em {program}", run.header());
        let stdout = run.outcome.stdout.trim_end();
        let stderr = run.outcome.stderr.trim_end();
        if !stdout.is_empty() {
            body.push_str("\n\n[saída]\n");
            body.push_str(stdout);
        }
        if !stderr.is_empty() {
            body.push_str("\n\n[erros]\n");
            body.push_str(stderr);
        }
        if stdout.is_empty() && stderr.is_empty() {
            body.push_str(
                "\n(o trecho não imprimiu nada — lembre de usar print()/console.log() para o \
                 resultado voltar para você)",
            );
        }
        if run.outcome.truncated {
            body.push_str("\n\n[...saída cortada no limite; imprima menos coisa...]");
        }

        Ok(ToolOutput::text(body)
            .with_exit_code(run.outcome.exit_code)
            .truncated_to(ctx.max_output_bytes))
    }
}

/// Sufixo único e seguro para nome de arquivo.
///
/// O `call_id` entra para facilitar a correlação com a trilha, mas passa por
/// filtro: ele vem de fora e não pode virar `../` nem caractere proibido.
fn unique_suffix(call_id: &str) -> String {
    let clean: String = call_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(24)
        .collect();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or_default();
    if clean.is_empty() {
        format!("anon_{nanos}")
    } else {
        format!("{clean}_{nanos}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{program_exists, skip};
    use tempfile::TempDir;

    fn project() -> (TempDir, ToolContext) {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolContext::new(Some(dir.path().to_path_buf()), "call-abc");
        (dir, ctx)
    }

    fn scratch_of(dir: &TempDir) -> PathBuf {
        dir.path().join(".openweights").join("scratch")
    }

    #[test]
    fn language_accepts_the_spellings_a_model_writes() {
        for good in ["python", "Python", "py", "python3"] {
            assert_eq!(Language::parse(good).unwrap(), Language::Python);
        }
        for good in ["node", "js", "javascript", "NodeJS"] {
            assert_eq!(Language::parse(good).unwrap(), Language::Node);
        }
        let err = Language::parse("ruby").unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("python"), "{msg}");
        assert!(msg.contains("fs_write"), "deve dizer o plano B: {msg}");
    }

    #[test]
    fn node_extension_follows_the_module_system_of_the_snippet() {
        assert_eq!(
            extension_for(Language::Node, "import fs from 'node:fs'\nconsole.log(1)"),
            "mjs"
        );
        assert_eq!(
            extension_for(Language::Node, "const fs = require('fs')"),
            "cjs"
        );
        assert_eq!(extension_for(Language::Node, "console.log(1)"), "cjs");
        assert_eq!(extension_for(Language::Python, "print(1)"), "py");
    }

    #[test]
    fn the_scratch_file_name_cannot_escape_the_folder() {
        let name = unique_suffix("../../etc/passwd");
        assert!(!name.contains('/'), "{name}");
        assert!(!name.contains('.'), "{name}");
        assert!(!unique_suffix("").is_empty());
    }

    #[tokio::test]
    async fn runs_python_and_brings_back_what_was_printed() {
        if !program_exists("python3") && !program_exists("python") {
            assert!(skip("python3"));
            return;
        }
        let (_d, ctx) = project();
        let out = CodeRun::default()
            .execute(json!({"language": "python", "code": "print(6 * 7)"}), &ctx)
            .await
            .unwrap();
        assert!(out.content.contains("42"), "{}", out.content);
        assert!(out.content.contains("ok (código 0)"), "{}", out.content);
    }

    #[tokio::test]
    async fn runs_node_and_reports_a_failure_with_its_message() {
        if !program_exists("node") {
            assert!(skip("node"));
            return;
        }
        let (_d, ctx) = project();
        let out = CodeRun::default()
            .execute(
                json!({"language": "node", "code": "console.log('oi'); throw new Error('quebrou de proposito')"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.content.contains("oi"), "{}", out.content);
        assert!(
            out.content.contains("quebrou de proposito"),
            "{}",
            out.content
        );
        assert!(out.content.contains("falhou"), "{}", out.content);
    }

    #[tokio::test]
    async fn the_temporary_file_is_created_inside_the_project_and_removed_after() {
        if !program_exists("python3") && !program_exists("python") {
            assert!(skip("python3"));
            return;
        }
        let (dir, ctx) = project();
        // O próprio trecho conta onde está: prova que o arquivo existiu.
        let out = CodeRun::default()
            .execute(
                json!({"language": "python", "code": "print(__file__)"}),
                &ctx,
            )
            .await
            .unwrap();
        let printed = out
            .content
            .lines()
            .find(|l| l.contains("trecho_"))
            .unwrap_or_default()
            .trim()
            .to_string();
        assert!(
            printed.contains(".openweights"),
            "o arquivo devia estar em .openweights/scratch: {}",
            out.content
        );
        assert!(
            !PathBuf::from(&printed).exists(),
            "o arquivo temporário sobreviveu: {printed}"
        );
        assert!(
            !scratch_of(&dir).exists(),
            "a pasta de rascunho devia ter sumido junto"
        );
    }

    #[tokio::test]
    async fn a_missing_interpreter_explains_how_to_install_it() {
        let (dir, ctx) = project();
        let tool = CodeRun::with_programs("python-que-nao-existe-xyz", "node-que-nao-existe-xyz");
        let err = tool
            .execute(json!({"language": "python", "code": "print(1)"}), &ctx)
            .await
            .unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("python-que-nao-existe-xyz"), "{msg}");
        assert!(msg.contains("PATH"), "{msg}");
        // Mesmo falhando antes de rodar, nada de lixo no projeto.
        assert!(
            !scratch_of(&dir).exists(),
            "a pasta de rascunho ficou para trás depois do erro"
        );
    }

    #[tokio::test]
    async fn an_infinite_loop_is_killed_and_reported() {
        if !program_exists("node") {
            assert!(skip("node"));
            return;
        }
        let (dir, ctx) = project();
        let out = CodeRun::default()
            .execute(
                json!({"language": "node", "code": "while (true) {}", "timeout_secs": 1}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.content.contains("INTERROMPIDO"), "{}", out.content);
        assert!(!scratch_of(&dir).exists(), "limpou mesmo depois do timeout");
    }

    #[tokio::test]
    async fn empty_or_giant_snippets_are_refused_before_writing_anything() {
        let (dir, ctx) = project();
        let tool = CodeRun::default();

        let err = tool
            .execute(json!({"language": "python", "code": "   "}), &ctx)
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)), "{err:?}");

        let huge = "x".repeat(MAX_CODE_BYTES + 1);
        let err = tool
            .execute(json!({"language": "python", "code": huge}), &ctx)
            .await
            .unwrap_err();
        assert!(err.to_model_message().contains("fs_write"), "{err:?}");
        assert!(!scratch_of(&dir).exists());
    }

    #[tokio::test]
    async fn the_preview_shows_the_code_that_will_run() {
        let (_d, ctx) = project();
        let args = json!({"language": "python", "code": "print('oi')"});
        match CodeRun::default().preview(&args, &ctx).await.unwrap() {
            ToolPreview::Text { body } => {
                assert!(body.contains("print('oi')"), "{body}");
                assert!(body.contains("python"), "{body}");
            }
            other => panic!("esperava prévia de texto, veio {other:?}"),
        }
    }
}
