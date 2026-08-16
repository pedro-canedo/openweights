//! `project_info`: o que este projeto usa e como se roda cada coisa.
//!
//! É a primeira ferramenta que o agente deve chamar num projeto novo, e a
//! razão é simples: sem ela ele *adivinha*. Um `npm test` num projeto Rust
//! gasta um passo, devolve um erro que não fala do problema real e ainda
//! deixa no histórico um exemplo ruim que o modelo tende a repetir.
//!
//! A resposta é escrita para ser lida por um modelo pequeno: uma stack por
//! bloco, comandos prontos para copiar, e no fim a lista de qual ferramenta
//! vai usar qual comando por padrão — para o modelo saber de antemão o que
//! `test_run` fará antes de chamá-lo.
//!
//! Categoria `Read`: só abre manifestos dentro da pasta, não roda nada.

use crate::detect::{self, Project, Task};
use crate::workspace;
use async_trait::async_trait;
use lr_tools::{Tool, ToolContext, ToolOutput, ToolResult};
use lr_types::agent::{ToolCategory, ToolPreview};
use serde_json::{Value, json};
use std::fmt::Write as _;

/// Scripts listados por stack (o resto vira contagem).
const MAX_SCRIPTS: usize = 12;

pub struct ProjectInfo;

#[async_trait]
impl Tool for ProjectInfo {
    fn name(&self) -> &str {
        "project_info"
    }

    fn description(&self) -> &str {
        "Diz o que o projeto usa (linguagens, gerenciador de pacotes, scripts) e quais são os \
         comandos de teste, lint, formatação e build. Chame ANTES de tentar rodar qualquer \
         comando no projeto."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Read
    }

    fn within_workspace(&self, _args: &Value, ctx: &ToolContext) -> bool {
        ctx.workspace.is_some()
    }

    async fn preview(&self, _args: &Value, ctx: &ToolContext) -> Option<ToolPreview> {
        Some(ToolPreview::Text {
            body: format!(
                "Ler os manifestos de {}",
                ctx.workspace.as_ref()?.to_string_lossy()
            ),
        })
    }

    async fn execute(&self, _args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let root = workspace(ctx)?;
        let project = detect::detect(&root);
        Ok(ToolOutput::text(render(&project)).truncated_to(ctx.max_output_bytes))
    }
}

/// Monta a resposta que o modelo vai ler.
fn render(project: &Project) -> String {
    if project.is_empty() {
        return "Nenhum manifesto de projeto reconhecido nesta pasta.\n\
                Procurei por: package.json, Cargo.toml, pyproject.toml, requirements.txt, \
                setup.py, Pipfile, go.mod, pom.xml, build.gradle, composer.json, Gemfile e \
                *.csproj — na raiz e nas subpastas de primeiro nível.\n\
                Use `fs_list` para ver o que existe e `terminal_run` para rodar comandos na mão."
            .to_string();
    }

    let mut out = format!(
        "Projeto em {}\nLinguagens: {}",
        project.root.display(),
        project.languages().join(", ")
    );

    for (i, stack) in project.stacks.iter().enumerate() {
        let _ = write!(
            out,
            "\n\n[{}] {} — gerenciador {} — {} — manifesto {}",
            i + 1,
            stack.language,
            stack.manager,
            stack.where_label(),
            stack.manifest
        );

        if !stack.scripts.is_empty() {
            let names: Vec<&str> = stack
                .scripts
                .keys()
                .take(MAX_SCRIPTS)
                .map(String::as_str)
                .collect();
            let _ = write!(out, "\n    scripts: {}", names.join(", "));
            if stack.scripts.len() > MAX_SCRIPTS {
                let _ = write!(out, " (+{})", stack.scripts.len() - MAX_SCRIPTS);
            }
        }

        for (label, cmd) in [
            ("testar", stack.test.as_ref()),
            ("lint", stack.lint.as_ref()),
            ("formatar", stack.format.as_ref()),
            ("compilar", stack.build.as_ref()),
        ] {
            match cmd {
                Some(cmd) => {
                    let _ = write!(out, "\n    {label}: {}", cmd.display());
                }
                None => {
                    let _ = write!(out, "\n    {label}: (não existe neste projeto)");
                }
            }
        }
    }

    out.push_str("\n\nO que cada ferramenta vai usar se você não passar `dir`:");
    for task in [Task::Test, Task::Lint, Task::Format, Task::Build] {
        let line = match project.pick(task, None).and_then(|s| {
            s.command(task).map(|cmd| {
                let onde = s.where_label();
                format!("{} (em {onde})", cmd.display())
            })
        }) {
            Some(text) => text,
            None => "— nenhuma stack tem esse comando; use terminal_run".to_string(),
        };
        let _ = write!(out, "\n    {:<10} → {line}", task.tool_name());
    }

    if project.stacks.len() > 1 {
        let _ = write!(
            out,
            "\n\nEste projeto tem mais de uma stack. Para rodar na outra, passe \
             dir=\"<pasta>\" (pastas: {}).",
            project.dirs().join(", ")
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn project_with(files: &[(&str, &str)]) -> (TempDir, ToolContext) {
        let dir = tempfile::tempdir().unwrap();
        for (path, body) in files {
            let full = dir.path().join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, body).unwrap();
        }
        let ctx = ToolContext::new(Some(dir.path().to_path_buf()), "call-1");
        (dir, ctx)
    }

    async fn run(ctx: &ToolContext) -> String {
        ProjectInfo.execute(json!({}), ctx).await.unwrap().content
    }

    #[tokio::test]
    async fn describes_a_node_project_with_its_scripts() {
        let (_d, ctx) = project_with(&[
            (
                "package.json",
                r#"{"scripts":{"test":"vitest run","build":"vite build"}}"#,
            ),
            ("pnpm-lock.yaml", ""),
        ]);
        let text = run(&ctx).await;
        assert!(text.contains("Linguagens: node"), "{text}");
        assert!(text.contains("gerenciador pnpm"), "{text}");
        assert!(text.contains("scripts: build, test"), "{text}");
        assert!(text.contains("testar: pnpm run test"), "{text}");
        assert!(text.contains("test_run   → pnpm run test"), "{text}");
    }

    #[tokio::test]
    async fn a_mixed_project_lists_both_stacks_and_how_to_choose() {
        let (_d, ctx) = project_with(&[
            ("package.json", r#"{"scripts":{"build":"vite build"}}"#),
            ("src-tauri/Cargo.toml", "[package]\nname=\"app\"\n"),
        ]);
        let text = run(&ctx).await;
        assert!(text.contains("[1] node"), "{text}");
        assert!(text.contains("[2] rust"), "{text}");
        assert!(text.contains("src-tauri"), "{text}");
        // A raiz não tem teste; o padrão do test_run cai no cargo.
        assert!(
            text.contains("test_run   → cargo test (em src-tauri)"),
            "{text}"
        );
        assert!(text.contains("build_run  → npm run build"), "{text}");
        assert!(text.contains("passe dir="), "{text}");
    }

    #[tokio::test]
    async fn missing_commands_are_stated_instead_of_omitted() {
        let (_d, ctx) = project_with(&[("requirements.txt", "requests\n")]);
        let text = run(&ctx).await;
        assert!(
            text.contains("compilar: (não existe neste projeto)"),
            "{text}"
        );
        assert!(text.contains("build_run  → — nenhuma stack"), "{text}");
    }

    #[tokio::test]
    async fn an_empty_folder_says_what_it_looked_for() {
        let (_d, ctx) = project_with(&[("leiame.txt", "oi")]);
        let text = run(&ctx).await;
        assert!(text.contains("Nenhum manifesto"), "{text}");
        assert!(text.contains("Cargo.toml"), "{text}");
        assert!(
            text.contains("fs_list"),
            "deve dizer o próximo passo: {text}"
        );
    }

    #[tokio::test]
    async fn the_answer_stays_inside_the_output_budget() {
        // 200 scripts: a resposta não pode crescer sem limite.
        let scripts: Vec<String> = (0..200)
            .map(|i| format!("\"script{i}\":\"echo {i}\""))
            .collect();
        let package = format!("{{\"scripts\":{{{}}}}}", scripts.join(","));
        let (_d, mut ctx) = project_with(&[("package.json", &package)]);
        ctx.max_output_bytes = 2_000;
        let text = run(&ctx).await;
        assert!(text.len() <= 2_100, "tamanho {}", text.len());
        assert!(
            text.contains("(+188)"),
            "deve dizer quantos ficaram de fora: {text}"
        );
    }

    #[tokio::test]
    async fn it_is_read_only_and_never_needs_a_confirmation_screen() {
        let (_d, ctx) = project_with(&[("Cargo.toml", "[package]\nname=\"x\"\n")]);
        let spec = ProjectInfo.spec();
        assert_eq!(spec.category, ToolCategory::Read);
        assert!(spec.read_only);
        assert!(ProjectInfo.files_at_risk(&json!({}), &ctx).is_empty());
        assert!(ProjectInfo.preview(&json!({}), &ctx).await.is_some());
    }
}
