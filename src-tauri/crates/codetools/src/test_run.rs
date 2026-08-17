//! `test_run`: roda a suíte do projeto e devolve o que interessa.
//!
//! É a ferramenta que fecha o ciclo. O agente edita, roda os testes, lê as
//! falhas e conserta — e a qualidade desse laço depende inteiramente de o
//! resultado ser **curto e específico**. Um log de dez mil linhas devolvido
//! cru empurra o código editado para fora da janela e faz o modelo perder o
//! fio; um "os testes falharam" sem detalhe não dá o que consertar.
//!
//! Por isso aqui não devolvemos a saída: devolvemos a contagem, as falhas com
//! o texto do erro, e quantas linhas ficaram de fora (para o modelo saber que
//! existe mais, e que `terminal_run` pega o log inteiro se precisar).
//!
//! O filtro vem do modelo e nunca vira sintaxe de shell: cada runner tem sua
//! forma (`cargo test nome`, `pytest -k nome`, `go test -run nome`) e em todas
//! ele entra como **um argumento** do `argv`.

use crate::detect::Task;
use crate::exec::{self, DEFAULT_TIMEOUT_SECS};
use crate::summary::{self, Framework};
use crate::text::combined;
use crate::{budget, command_preview, dir_and_timeout_properties, target, timeout_of};
use async_trait::async_trait;
use lr_tools::{Tool, ToolContext, ToolOutput, ToolResult, arg_str_opt};
use lr_types::agent::{ToolCategory, ToolPreview};
use serde_json::{Value, json};

pub struct TestRun;

impl TestRun {
    /// Comando final (com filtro) e onde ele roda.
    fn plan(
        &self,
        args: &Value,
        ctx: &ToolContext,
    ) -> ToolResult<(crate::Target, crate::detect::Cmd)> {
        let target = target(args, ctx, Task::Test)?;
        let base = target
            .stack
            .test
            .clone()
            .expect("target garante que o comando existe");
        let cmd = match arg_str_opt(args, "filter") {
            Some(filter) => target.stack.test_filter.apply(base, &filter),
            None => base,
        };
        Ok((target, cmd))
    }
}

#[async_trait]
impl Tool for TestRun {
    fn name(&self) -> &str {
        "test_run"
    }

    fn description(&self) -> &str {
        "Roda os testes do projeto (cargo test, npm test, pytest, go test…) e devolve um resumo: \
         quantos passaram, quantos falharam e o erro de cada falha. Use depois de editar código."
    }

    fn parameters(&self) -> Value {
        let mut properties = json!({
            "filter": {
                "type": "string",
                "description": "Roda só os testes cujo nome casa com este texto. Omita para rodar tudo. Prefira filtrar quando a suíte é grande."
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

    async fn preview(&self, args: &Value, ctx: &ToolContext) -> Option<ToolPreview> {
        let (target, cmd) = self.plan(args, ctx).ok()?;
        Some(command_preview(&cmd, &target.cwd))
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let (target, cmd) = self.plan(&args, ctx)?;
        let timeout = timeout_of(&args, DEFAULT_TIMEOUT_SECS);
        let run = exec::run(&cmd, &target.cwd, timeout, exec::CAPTURE_BYTES).await?;

        let output = combined(&run.outcome.stdout, &run.outcome.stderr);
        let mut body = format!("{}\nstack: {}", run.header(), target.where_line());

        // Módulo ausente não é teste vermelho: é ambiente incompleto.
        if !run.success()
            && let Some(note) = exec::missing_module_note(&output)
        {
            body.push_str("\n\n");
            body.push_str(&note);
            return Ok(ToolOutput::text(body)
                .with_exit_code(run.outcome.exit_code)
                .truncated_to(ctx.max_output_bytes));
        }

        let framework = Framework::from_command(&cmd.argv);
        let summary = summary::summarize(framework, &output);
        body.push_str("\n\n");
        body.push_str(&summary::render(&summary, budget(ctx)));

        // Runner em modo vigia trava até o tempo estourar; o conserto é o
        // usuário ajustar o script, então dizemos isso em vez de deixar o
        // modelo tentar de novo igual.
        if run.outcome.timed_out && framework == Framework::JsRunner {
            body.push_str(
                "\n\nDica: se o script de teste abre em modo \"watch\", ele nunca termina \
                 sozinho. Use um filtro, aumente `timeout_secs` ou peça um script que rode uma \
                 vez só (vitest run / jest --ci).",
            );
        }
        if summary.all_green() && run.success() {
            body.push_str("\n\nSuíte verde.");
        }
        if arg_str_opt(&args, "dir").is_none()
            && let Some(hint) = target.other_stacks_hint(Task::Test)
        {
            body.push_str("\n\n");
            body.push_str(&hint);
        }

        Ok(ToolOutput::text(body)
            .with_exit_code(run.outcome.exit_code)
            .truncated_to(ctx.max_output_bytes))
    }
}

/// `dir` (quando vem) precisa estar dentro do projeto.
pub(crate) fn within(args: &Value, ctx: &ToolContext) -> bool {
    if ctx.workspace.is_none() {
        return false;
    }
    match arg_str_opt(args, "dir") {
        Some(dir) => ctx.resolve(&dir).is_ok(),
        None => true,
    }
}

/// Junta as propriedades comuns às específicas de cada ferramenta.
pub(crate) fn merge(base: &mut Value, extra: Value) {
    if let (Some(base), Some(extra)) = (base.as_object_mut(), extra.as_object()) {
        for (k, v) in extra {
            base.insert(k.clone(), v.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{program_exists, skip};
    use tempfile::TempDir;

    /// Um crate Rust de verdade, com um teste verde e um vermelho.
    fn rust_project(failing: bool) -> (TempDir, ToolContext) {
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
        let esperado = if failing { "5" } else { "4" };
        std::fs::write(
            dir.path().join("src/lib.rs"),
            format!(
                "pub fn soma(a: i32, b: i32) -> i32 {{ a + b }}\n\
                 #[cfg(test)]\nmod tests {{\n    use super::*;\n\
                 #[test]\n    fn soma_ok() {{ assert_eq!(soma(2, 2), 4); }}\n\
                 #[test]\n    fn soma_alvo() {{ assert_eq!(soma(2, 2), {esperado}); }}\n}}\n"
            ),
        )
        .unwrap();
        let ctx = ToolContext::new(Some(dir.path().to_path_buf()), "call-1");
        (dir, ctx)
    }

    #[tokio::test]
    async fn a_green_suite_reports_the_count_and_says_it_is_green() {
        if !program_exists("cargo") {
            assert!(skip("cargo"));
            return;
        }
        let (_d, ctx) = rust_project(false);
        let out = TestRun
            .execute(json!({"timeout_secs": 600}), &ctx)
            .await
            .unwrap();
        assert!(out.content.contains("2 passaram"), "{}", out.content);
        assert!(out.content.contains("Suíte verde"), "{}", out.content);
        assert!(
            out.content.contains("stack: rust (cargo)"),
            "{}",
            out.content
        );
    }

    #[tokio::test]
    async fn a_red_suite_brings_the_failing_test_and_its_assertion() {
        if !program_exists("cargo") {
            assert!(skip("cargo"));
            return;
        }
        let (_d, ctx) = rust_project(true);
        let out = TestRun
            .execute(json!({"timeout_secs": 600}), &ctx)
            .await
            .unwrap();
        assert!(out.content.contains("1 passaram"), "{}", out.content);
        assert!(out.content.contains("1 falharam"), "{}", out.content);
        assert!(out.content.contains("tests::soma_alvo"), "{}", out.content);
        assert!(
            out.content.contains("left: 4") || out.content.contains("assertion"),
            "precisa trazer o motivo: {}",
            out.content
        );
        // O log inteiro tem muito mais que isso.
        assert!(out.content.len() < 3_000, "resumo grande demais");
    }

    #[tokio::test]
    async fn the_filter_reaches_the_runner_as_a_single_argument() {
        let (_d, ctx) = rust_project(true);
        let args = json!({"filter": "soma_ok"});
        let (_target, cmd) = TestRun.plan(&args, &ctx).unwrap();
        assert_eq!(cmd.argv, vec!["cargo", "test", "soma_ok"]);

        // Filtro hostil continua sendo um argumento só.
        let args = json!({"filter": "a && rm -rf ."});
        let (_target, cmd) = TestRun.plan(&args, &ctx).unwrap();
        assert_eq!(cmd.argv.len(), 3);
        assert_eq!(cmd.argv[2], "a && rm -rf .");
    }

    #[tokio::test]
    async fn the_preview_shows_the_command_that_will_run() {
        let (dir, ctx) = rust_project(false);
        match TestRun.preview(&json!({}), &ctx).await.unwrap() {
            ToolPreview::Command {
                program,
                display,
                cwd,
                ..
            } => {
                assert_eq!(program, "cargo");
                assert_eq!(display, "cargo test");
                assert_eq!(cwd, dir.path().to_string_lossy());
            }
            other => panic!("esperava prévia de comando, veio {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_project_that_does_not_compile_says_the_tests_never_ran() {
        if !program_exists("cargo") {
            assert!(skip("cargo"));
            return;
        }
        let (dir, ctx) = rust_project(false);
        std::fs::write(dir.path().join("src/lib.rs"), "isto não é rust válido\n").unwrap();
        let out = TestRun
            .execute(json!({"timeout_secs": 600}), &ctx)
            .await
            .unwrap();
        assert!(
            out.content.contains("NÃO chegaram a rodar"),
            "{}",
            out.content
        );
        // O caminho vem do rustc, que usa o separador do sistema: no Windows
        // a mesma linha sai como `src\lib.rs`.
        let visto = out.content.replace('\\', "/");
        assert!(visto.contains("src/lib.rs"), "{}", out.content);
    }

    #[tokio::test]
    async fn tests_never_put_source_files_at_risk() {
        let (_d, ctx) = rust_project(false);
        assert!(TestRun.files_at_risk(&json!({}), &ctx).is_empty());
        assert!(TestRun.within_workspace(&json!({}), &ctx));
        assert!(!TestRun.within_workspace(&json!({"dir": "../fora"}), &ctx));
    }
}
