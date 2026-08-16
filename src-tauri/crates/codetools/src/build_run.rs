//! `build_run`: compila o projeto e devolve os primeiros erros.
//!
//! Compilar é a checagem mais barata que existe depois de editar: pega erro de
//! tipo, import quebrado e nome trocado antes de a suíte inteira rodar.
//!
//! O resumo prioriza **erro com arquivo:linha**, e nessa ordem, porque é
//! literalmente o que o agente precisa para o próximo passo: abrir o arquivo
//! na linha certa. Um `cargo build` quebrado escreve dezenas de linhas de
//! moldura por erro (a seta, o trecho de código, o `help:` desenhado) — tudo
//! feito para o olho humano e inútil para quem vai reabrir o arquivo mesmo.
//!
//! Projeto sem etapa de compilação (Python, por exemplo) não recebe um comando
//! inventado: recebe a explicação de que não há build e a sugestão de rodar
//! `test_run` ou `lint_run`, que é o equivalente por lá.

use crate::detect::Task;
use crate::exec::{self, DEFAULT_TIMEOUT_SECS};
use crate::test_run::{merge, within};
use crate::text::{combined, line_count};
use crate::{command_preview, diagnostics, dir_and_timeout_properties, target, timeout_of};
use async_trait::async_trait;
use lr_tools::{Tool, ToolContext, ToolOutput, ToolResult};
use lr_types::agent::{ToolCategory, ToolPreview};
use serde_json::{Value, json};
use std::fmt::Write as _;

/// Erros listados no resumo (os primeiros bastam para o próximo passo).
const MAX_ERRORS: usize = 10;

/// Linhas do fim da saída quando a compilação falha sem diagnóstico legível.
const FALLBACK_TAIL: usize = 25;

pub struct BuildRun;

#[async_trait]
impl Tool for BuildRun {
    fn name(&self) -> &str {
        "build_run"
    }

    fn description(&self) -> &str {
        "Compila o projeto (cargo build, npm run build, go build, mvn compile…) e devolve os \
         primeiros erros de compilação com arquivo e linha. Use para conferir se o código que \
         você escreveu compila."
    }

    fn parameters(&self) -> Value {
        let mut properties = json!({});
        merge(
            &mut properties,
            dir_and_timeout_properties(DEFAULT_TIMEOUT_SECS),
        );
        json!({
            "type": "object",
            "properties": properties,
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Execute
    }

    fn within_workspace(&self, args: &Value, ctx: &ToolContext) -> bool {
        within(args, ctx)
    }

    async fn preview(&self, args: &Value, ctx: &ToolContext) -> Option<ToolPreview> {
        let target = target(args, ctx, Task::Build).ok()?;
        let cmd = target.stack.build.clone()?;
        Some(command_preview(&cmd, &target.cwd))
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let target = target(&args, ctx, Task::Build)?;
        let cmd = target
            .stack
            .build
            .clone()
            .expect("target garante que o comando existe");
        let timeout = timeout_of(&args, DEFAULT_TIMEOUT_SECS);
        let run = exec::run(&cmd, &target.cwd, timeout, exec::CAPTURE_BYTES).await?;

        let output = combined(&run.outcome.stdout, &run.outcome.stderr);
        let diags = diagnostics::extract(&output);
        let (errors, warnings) = diagnostics::counts(&diags);

        let mut body = format!("{}\nstack: {}", run.header(), target.where_line());

        if run.success() {
            body.push_str("\n\nCompilou sem erros");
            if warnings > 0 {
                let _ = write!(body, " ({warnings} aviso(s)).");
                body.push('\n');
                body.push_str(&diagnostics::render_list(&diags, MAX_ERRORS.min(5)));
            } else {
                body.push('.');
            }
        } else if diags.is_empty() {
            // Falhou sem nada que a gente saiba ler: mostra o fim da saída,
            // que é onde a ferramenta costuma dizer o motivo.
            body.push_str(
                "\n\nA compilação falhou e não reconheci o formato dos erros. Fim da saída:",
            );
            for line in crate::text::tail_lines(&output, FALLBACK_TAIL) {
                body.push('\n');
                body.push_str(&crate::text::clip_line(line, 200));
            }
        } else {
            let _ = write!(body, "\n\n{errors} erro(s) de compilação. ");
            body.push_str(&diagnostics::render_list(&diags, MAX_ERRORS));
            body.push_str("\n\nAbra os arquivos nas linhas indicadas e corrija antes de seguir.");
        }

        let shown = line_count(&body);
        let total = line_count(&output);
        if total > shown {
            let _ = write!(
                body,
                "\n\n(saída completa: {total} linhas; o resumo mostra {shown}.)"
            );
        }
        if lr_tools::arg_str_opt(&args, "dir").is_none()
            && let Some(hint) = target.other_stacks_hint(Task::Build)
        {
            body.push_str("\n\n");
            body.push_str(&hint);
        }

        Ok(ToolOutput::text(body).truncated_to(ctx.max_output_bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{program_exists, skip};
    use tempfile::TempDir;

    fn rust_project(code: &str) -> (TempDir, ToolContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            format!(
                "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
                crate::testing::unique_package("alvo")
            ),
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), code).unwrap();
        let ctx = ToolContext::new(Some(dir.path().to_path_buf()), "call-1");
        (dir, ctx)
    }

    #[tokio::test]
    async fn a_clean_build_says_so_in_one_line() {
        if !program_exists("cargo") {
            assert!(skip("cargo"));
            return;
        }
        let (_d, ctx) = rust_project("pub fn soma(a: i32, b: i32) -> i32 { a + b }\n");
        let out = BuildRun
            .execute(json!({"timeout_secs": 600}), &ctx)
            .await
            .unwrap();
        assert!(
            out.content.contains("Compilou sem erros"),
            "{}",
            out.content
        );
        assert!(out.content.len() < 600, "resumo verde deve ser curto");
    }

    #[tokio::test]
    async fn a_broken_build_lists_errors_with_file_and_line() {
        if !program_exists("cargo") {
            assert!(skip("cargo"));
            return;
        }
        let (_d, ctx) = rust_project("pub fn soma(a: i32, b: i32) -> u32 { a + b }\n");
        let out = BuildRun
            .execute(json!({"timeout_secs": 600}), &ctx)
            .await
            .unwrap();
        assert!(
            out.content.contains("erro(s) de compilação"),
            "{}",
            out.content
        );
        assert!(out.content.contains("src/lib.rs:1"), "{}", out.content);
        assert!(out.content.contains("E0308"), "{}", out.content);
        assert!(out.content.contains("saída completa:"), "{}", out.content);
    }

    #[tokio::test]
    async fn a_project_without_a_build_step_explains_instead_of_guessing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("requirements.txt"), "requests\n").unwrap();
        let ctx = ToolContext::new(Some(dir.path().to_path_buf()), "call-1");
        let err = BuildRun.execute(json!({}), &ctx).await.unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("não tem comando para compilar"), "{msg}");
        assert!(msg.contains("terminal_run"), "{msg}");
    }

    #[tokio::test]
    async fn the_preview_shows_the_build_command() {
        let (_d, ctx) = rust_project("pub fn a() {}\n");
        match BuildRun.preview(&json!({}), &ctx).await.unwrap() {
            ToolPreview::Command { display, .. } => assert_eq!(display, "cargo build"),
            other => panic!("esperava prévia de comando, veio {other:?}"),
        }
    }
}
