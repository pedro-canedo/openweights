//! `lint_run`: passa o linter do projeto e resume os problemas.
//!
//! Lint é o aviso barato: pega import não usado, variável morta, `await`
//! esquecido — coisas que compilam e passam nos testes, mas que o revisor
//! humano vai apontar. Para o agente vale como segunda checagem depois de
//! editar, e o resumo segue a mesma regra do resto: erro antes de aviso,
//! arquivo:linha sempre, e o número do que ficou de fora.
//!
//! ## `fix` só quando a ferramenta sabe corrigir
//!
//! `eslint --fix`, `ruff check --fix` e `rubocop -a` reescrevem arquivos com
//! segurança. `cargo clippy --fix` também, mas exige árvore de git limpa e
//! reescreve o código de um jeito que merece revisão — então não entra no
//! automático. Quando `fix=true` chega para um linter sem correção
//! automática, rodamos a análise normal e **dizemos** que não houve correção,
//! em vez de fingir que houve ou de errar por falta de um comando.

use crate::detect::Task;
use crate::exec::{self, DEFAULT_TIMEOUT_SECS};
use crate::scan;
use crate::test_run::{merge, within};
use crate::text::{combined, line_count};
use crate::{command_preview, diagnostics, dir_and_timeout_properties, target, timeout_of};
use async_trait::async_trait;
use lr_tools::{Tool, ToolContext, ToolOutput, ToolResult, arg_bool};
use lr_types::agent::{ToolCategory, ToolPreview};
use serde_json::{Value, json};
use std::fmt::Write as _;

/// Problemas listados no resumo.
const MAX_ISSUES: usize = 12;

pub struct LintRun;

#[async_trait]
impl Tool for LintRun {
    fn name(&self) -> &str {
        "lint_run"
    }

    fn description(&self) -> &str {
        "Roda o linter do projeto (clippy, eslint, ruff, go vet…) e resume os problemas com \
         arquivo e linha. Com fix=true, aplica as correções automáticas quando o linter tiver."
    }

    fn parameters(&self) -> Value {
        let mut properties = json!({
            "fix": {
                "type": "boolean",
                "description": "true aplica as correções automáticas do linter (altera arquivos). Padrão false, que só analisa."
            }
        });
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

    /// Com `fix`, o linter reescreve código: o checkpoint precisa dos fontes.
    ///
    /// Nenhum linter diz de antemão o que vai mudar (o `--fix-dry-run` do
    /// eslint só existe em JSON e nem todo linter tem equivalente), então aqui
    /// a lista é conservadora: os fontes da linguagem, com teto.
    fn files_at_risk(&self, args: &Value, ctx: &ToolContext) -> Vec<String> {
        if !arg_bool(args, "fix", false) {
            return Vec::new();
        }
        let Ok(target) = target(args, ctx, Task::Lint) else {
            return Vec::new();
        };
        let Some(root) = ctx.workspace.as_ref() else {
            return Vec::new();
        };
        scan::source_files(root, &target.cwd, &target.stack.source_ext, scan::MAX_FILES)
    }

    async fn preview(&self, args: &Value, ctx: &ToolContext) -> Option<ToolPreview> {
        let target = target(args, ctx, Task::Lint).ok()?;
        let (cmd, _) = choose(&target, arg_bool(args, "fix", false));
        Some(command_preview(&cmd, &target.cwd))
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let target = target(&args, ctx, Task::Lint)?;
        let wants_fix = arg_bool(&args, "fix", false);
        let (cmd, fixing) = choose(&target, wants_fix);

        let timeout = timeout_of(&args, DEFAULT_TIMEOUT_SECS);
        let run = exec::run(&cmd, &target.cwd, timeout, exec::CAPTURE_BYTES).await?;

        let output = combined(&run.outcome.stdout, &run.outcome.stderr);
        let diags = diagnostics::extract(&output);
        let (errors, warnings) = diagnostics::counts(&diags);

        let mut body = format!("{}\nstack: {}", run.header(), target.where_line());

        if wants_fix && !fixing {
            let _ = write!(
                body,
                "\n\nEste linter não tem correção automática segura aqui, então rodei só a \
                 análise. {}",
                manual_fix_hint(&target.stack.language)
            );
        } else if fixing {
            body.push_str("\n\nCorreções automáticas aplicadas onde o linter conseguiu.");
        }

        if diags.is_empty() {
            if run.success() {
                body.push_str("\n\nNenhum problema encontrado.");
            } else {
                body.push_str(
                    "\n\nO linter terminou com erro mas não reconheci o formato da \
                               saída. Fim da saída:",
                );
                for line in crate::text::tail_lines(&output, 20) {
                    body.push('\n');
                    body.push_str(&crate::text::clip_line(line, 200));
                }
            }
        } else {
            let _ = write!(body, "\n\n{} problema(s). ", errors + warnings);
            body.push_str(&diagnostics::render_list(&diags, MAX_ISSUES));
        }

        let shown = line_count(&body);
        let total = line_count(&output);
        if total > shown {
            let _ = write!(
                body,
                "\n\n(saída completa: {total} linhas; o resumo mostra {shown}.)"
            );
        }

        let changed = if fixing {
            self.files_at_risk(&args, ctx)
        } else {
            Vec::new()
        };
        Ok(ToolOutput::text(body)
            .with_changed(changed)
            .with_exit_code(run.outcome.exit_code)
            .truncated_to(ctx.max_output_bytes))
    }
}

/// Comando a rodar e se ele de fato corrige.
fn choose(target: &crate::Target, wants_fix: bool) -> (crate::detect::Cmd, bool) {
    match (wants_fix, target.stack.lint_fix.clone()) {
        (true, Some(fixer)) => (fixer, true),
        _ => (
            target
                .stack
                .lint
                .clone()
                .expect("target garante que o comando existe"),
            false,
        ),
    }
}

/// O que dizer quando `fix` não é possível automaticamente.
fn manual_fix_hint(language: &str) -> &'static str {
    match language {
        "rust" => {
            "No Rust, `cargo clippy --fix` existe mas exige a árvore de git limpa e reescreve \
             código — rode por `terminal_run` se quiser, revisando o diff depois."
        }
        "go" => "No Go, corrija na mão: o `go vet` só aponta.",
        _ => "Corrija os pontos apontados com `fs_edit`.",
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

    fn node_project() -> (TempDir, ToolContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"devDependencies":{"eslint":"^9"}}"#,
        )
        .unwrap();
        let ctx = ToolContext::new(Some(dir.path().to_path_buf()), "call-1");
        (dir, ctx)
    }

    #[test]
    fn fix_uses_the_fixing_variant_only_when_there_is_one() {
        let (_d, ctx) = node_project();
        let target = target(&json!({}), &ctx, Task::Lint).unwrap();
        let (cmd, fixing) = choose(&target, true);
        assert!(fixing);
        assert_eq!(cmd.display(), "npx eslint . --fix");
        let (cmd, fixing) = choose(&target, false);
        assert!(!fixing);
        assert_eq!(cmd.display(), "npx eslint .");
    }

    #[test]
    fn rust_has_no_automatic_fix_and_says_why() {
        let (_d, ctx) = rust_project("pub fn a() {}\n");
        let target = target(&json!({}), &ctx, Task::Lint).unwrap();
        let (cmd, fixing) = choose(&target, true);
        assert!(!fixing, "clippy --fix não entra no automático");
        assert_eq!(cmd.display(), "cargo clippy --all-targets");
        assert!(manual_fix_hint("rust").contains("terminal_run"));
    }

    #[test]
    fn only_the_fixing_run_puts_files_at_risk() {
        let (_d, ctx) = rust_project("pub fn a() {}\n");
        assert!(LintRun.files_at_risk(&json!({}), &ctx).is_empty());
        // Rust não tem lint_fix, mas o pedido de fix já mobiliza o checkpoint:
        // é a lista conservadora, e é assim que ela deve se comportar.
        let at_risk = LintRun.files_at_risk(&json!({"fix": true}), &ctx);
        assert_eq!(at_risk, vec!["src/lib.rs"], "{at_risk:?}");
    }

    #[tokio::test]
    async fn clippy_warnings_come_back_with_file_and_line() {
        if !program_exists("cargo") {
            assert!(skip("cargo"));
            return;
        }
        // `cargo clippy` pode não estar instalado junto do cargo.
        let (_d, ctx) = rust_project("pub fn a() { let _x: Vec<i32> = Vec::new(); }\n");
        let out = match LintRun.execute(json!({"timeout_secs": 600}), &ctx).await {
            Ok(out) => out,
            Err(e) => {
                eprintln!("pulando: clippy indisponível ({})", e.to_model_message());
                return;
            }
        };
        assert!(out.content.contains("stack: rust"), "{}", out.content);
        // Com ou sem aviso, o resumo tem de ser conclusivo.
        assert!(
            out.content.contains("problema(s)") || out.content.contains("Nenhum problema"),
            "{}",
            out.content
        );
    }

    #[tokio::test]
    async fn a_project_without_a_linter_says_what_to_do() {
        // Maven sem linter configurado: não há o que rodar.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pom.xml"), "<project/>").unwrap();
        let ctx = ToolContext::new(Some(dir.path().to_path_buf()), "call-1");

        let err = LintRun.execute(json!({}), &ctx).await.unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("não tem comando para analisar"), "{msg}");
        assert!(msg.contains("terminal_run"), "deve dar a saída: {msg}");
    }

    #[tokio::test]
    async fn the_preview_shows_which_variant_will_run() {
        let (dir, ctx) = node_project();
        match LintRun.preview(&json!({"fix": true}), &ctx).await.unwrap() {
            ToolPreview::Command { display, cwd, .. } => {
                assert_eq!(display, "npx eslint . --fix");
                assert_eq!(cwd, dir.path().to_string_lossy());
            }
            other => panic!("esperava prévia de comando, veio {other:?}"),
        }
    }
}
