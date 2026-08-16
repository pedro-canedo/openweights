//! Ferramentas de código do agente: rodar o que ele acabou de escrever.
//!
//! O harness já sabia ler, escrever e chamar um comando cru. Faltava a metade
//! que fecha o ciclo **editar → testar → corrigir**: descobrir o que o projeto
//! usa, rodar a suíte, o linter, o formatador e a compilação — e devolver o
//! resultado num tamanho que caiba na janela do modelo.
//!
//! São seis ferramentas:
//!
//! | ferramenta     | categoria | para quê |
//! |----------------|-----------|----------|
//! | `project_info` | Read      | o que o projeto usa e quais comandos existem |
//! | `code_run`     | Execute   | rodar um trecho solto de Python ou Node |
//! | `test_run`     | Execute   | a suíte do projeto, resumida |
//! | `lint_run`     | Execute   | o linter do projeto, resumido |
//! | `format_run`   | Execute   | o formatador (conferir ou aplicar) |
//! | `build_run`    | Execute   | a compilação, com os primeiros erros |
//!
//! ## Três princípios valem para todas
//!
//! **Descobrir antes de chutar.** Nenhuma delas tem comando fixo: todas
//! perguntam ao [`detect`] o que este projeto usa. `npm test` num projeto Rust
//! não é só inútil — gasta um passo, polui o histórico e ensina o modelo a
//! desconfiar da ferramenta.
//!
//! **Resultado curto e útil.** A saída de um `cargo test` grande passa de dez
//! mil linhas. Devolver tudo empurra para fora da janela justamente o código
//! que estava sendo editado. Cada ferramenta extrai o que importa (contagem,
//! falhas, primeiros erros com arquivo:linha) e **diz quantas linhas ficaram
//! de fora** — corte silencioso faria o modelo concluir que a saída acabou.
//!
//! **Nada de shell.** Todo comando é um vetor `argv` entregue ao
//! [`lr_tools::spawner`]. O filtro de teste vem do modelo, e um `argv` faz
//! dele sempre *um argumento* — nunca um segundo comando. De quebra, o
//! spawner é quem sabe matar a árvore de processos quando o tempo estoura.

use lr_tools::{SharedTool, ToolContext, ToolError, ToolResult, arg_str_opt};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

pub mod detect;
pub mod diagnostics;
pub mod exec;
pub mod scan;
pub mod summary;
pub mod text;

mod build_run;
mod code_run;
mod format_run;
mod lint_run;
mod project_info;
mod test_run;

#[cfg(test)]
pub(crate) mod testing;

pub use build_run::BuildRun;
pub use code_run::CodeRun;
pub use format_run::FormatRun;
pub use lint_run::LintRun;
pub use project_info::ProjectInfo;
pub use test_run::TestRun;

use detect::{Cmd, Project, Stack, Task};

/// Todas as ferramentas deste crate, prontas para o registro.
pub fn code_tools() -> Vec<SharedTool> {
    vec![
        Arc::new(ProjectInfo),
        Arc::new(CodeRun::default()),
        Arc::new(TestRun),
        Arc::new(LintRun),
        Arc::new(FormatRun::default()),
        Arc::new(BuildRun),
    ]
}

/// Registra as seis ferramentas num registro já existente.
pub fn register_all(registry: &mut lr_tools::ToolRegistry) {
    for tool in code_tools() {
        registry.register(tool);
    }
}

// ------------------------------------------------------------- suporte ---

/// Raiz do projeto, ou o erro que explica por que não dá para rodar nada.
pub(crate) fn workspace(ctx: &ToolContext) -> ToolResult<PathBuf> {
    ctx.workspace.clone().ok_or_else(|| {
        ToolError::Other(
            "nenhuma pasta de projeto está aberta — peça ao usuário para escolher uma antes de \
             rodar comandos."
                .into(),
        )
    })
}

/// A stack escolhida para uma etapa, com a pasta onde rodar.
#[derive(Debug)]
pub(crate) struct Target {
    pub stack: Stack,
    pub cwd: PathBuf,
    pub project: Project,
}

impl Target {
    /// Cabeçalho que situa o modelo: o que foi escolhido e onde.
    pub fn where_line(&self) -> String {
        format!(
            "{} ({}) em {}",
            self.stack.language,
            self.stack.manager,
            self.stack.where_label()
        )
    }

    /// Aviso de que existe outra stack capaz de fazer a mesma etapa.
    ///
    /// Num repositório misto o agente pode ter editado o TypeScript e receber
    /// o resultado do `cargo test` — verde, correto e completamente irrelevante
    /// para o que ele mudou. Dizer que a outra existe (e como chamá-la) custa
    /// uma linha e evita essa conclusão errada.
    pub fn other_stacks_hint(&self, task: Task) -> Option<String> {
        let others: Vec<&Stack> = self
            .project
            .stacks
            .iter()
            .filter(|s| s.dir != self.stack.dir && s.command(task).is_some())
            .collect();
        let first = others.first()?;
        let names: Vec<String> = others
            .iter()
            .map(|s| format!("{} em `{}`", s.language, s.where_label()))
            .collect();
        Some(format!(
            "Este projeto também tem {}. Se o que você editou fica lá, chame `{}` com dir=\"{}\".",
            names.join(", "),
            task.tool_name(),
            if first.dir.is_empty() {
                "."
            } else {
                &first.dir
            }
        ))
    }
}

/// Descobre o projeto e escolhe a stack que atende a etapa pedida.
///
/// Toda a inteligência de "onde rodar" mora aqui para as cinco ferramentas de
/// execução responderem igual: mesma escolha, mesma mensagem de erro, mesmo
/// jeito de pedir outra pasta.
pub(crate) fn target(args: &Value, ctx: &ToolContext, task: Task) -> ToolResult<Target> {
    let root = workspace(ctx)?;
    let dir_arg = arg_str_opt(args, "dir");

    // Valida os limites antes de olhar o disco: `dir` vem do modelo.
    if let Some(dir) = &dir_arg {
        ctx.resolve(dir)?;
    }

    let project = detect::detect(&root);
    if project.is_empty() {
        return Err(ToolError::Other(format!(
            "não reconheci nenhum manifesto de projeto em `{}` (procurei package.json, \
             Cargo.toml, pyproject.toml, requirements.txt, go.mod, pom.xml, build.gradle, \
             composer.json, Gemfile e *.csproj). Use `fs_list` para ver a pasta ou \
             `terminal_run` para rodar o comando na mão.",
            ctx.relativize(&root)
        )));
    }

    let Some(stack) = project.pick(task, dir_arg.as_deref()) else {
        return Err(ToolError::Other(format!(
            "não há projeto conhecido em `{}`. Pastas com manifesto: {}. Chame `project_info` \
             para ver os detalhes.",
            dir_arg.unwrap_or_default(),
            project.dirs().join(", ")
        )));
    };

    if stack.command(task).is_none() {
        return Err(ToolError::Other(missing_command_message(
            &project, stack, task,
        )));
    }

    // `join("")` acrescentaria uma barra no fim, e esse caminho aparece na
    // tela de confirmação — `/projeto/` em vez de `/projeto` é feio e ainda
    // atrapalha comparação de caminho.
    let cwd = if stack.dir.is_empty() {
        root
    } else {
        root.join(&stack.dir)
    };
    Ok(Target {
        stack: stack.clone(),
        cwd,
        project,
    })
}

/// Explica que a stack escolhida não tem esse comando — e o que fazer então.
fn missing_command_message(project: &Project, stack: &Stack, task: Task) -> String {
    let mut message = format!(
        "a stack {} ({}) não tem comando para {}",
        stack.language,
        stack.where_label(),
        task.label()
    );
    if stack.language == "node" && task == Task::Test {
        message.push_str(" (não há script `test` no package.json nem runner instalado)");
    }
    message.push('.');

    let alternatives: Vec<String> = project
        .stacks
        .iter()
        .filter(|s| s.dir != stack.dir && s.command(task).is_some())
        .map(|s| format!("{} em `{}`", s.language, s.where_label()))
        .collect();

    if alternatives.is_empty() {
        message.push_str(&format!(
            " Nenhuma outra pasta do projeto tem. Use `terminal_run` com o comando certo, ou \
             crie o script que falta ({}).",
            task.tool_name()
        ));
    } else {
        message.push_str(&format!(
            " Quem tem: {}. Chame `{}` de novo com dir=\"{}\".",
            alternatives.join(", "),
            task.tool_name(),
            project
                .stacks
                .iter()
                .find(|s| s.dir != stack.dir && s.command(task).is_some())
                .map(|s| if s.dir.is_empty() { "." } else { &s.dir })
                .unwrap_or(".")
        ));
    }
    message
}

/// Orçamento de bytes do corpo, deixando espaço para o cabeçalho.
pub(crate) fn budget(ctx: &ToolContext) -> usize {
    ctx.max_output_bytes.saturating_sub(512).max(1_024)
}

/// Prévia de comando para a tela de confirmação.
pub(crate) fn command_preview(cmd: &Cmd, cwd: &std::path::Path) -> lr_types::agent::ToolPreview {
    let display = cmd.display();
    // A classificação vem do mesmo analisador que o `terminal_run` usa, para
    // o selo na tela significar sempre a mesma coisa.
    let analysis = lr_policy::classify(&display);
    lr_types::agent::ToolPreview::Command {
        program: analysis.program,
        display,
        cwd: cwd.to_string_lossy().into_owned(),
        class: analysis.class,
    }
}

/// `timeout_secs` do modelo, dentro dos limites.
pub(crate) fn timeout_of(args: &Value, default: u64) -> u64 {
    lr_tools::arg_u64(args, "timeout_secs", default).clamp(1, exec::MAX_TIMEOUT_SECS)
}

/// Trecho de schema repetido pelas quatro ferramentas de projeto.
pub(crate) fn dir_and_timeout_properties(default_timeout: u64) -> serde_json::Value {
    serde_json::json!({
        "dir": {
            "type": "string",
            "description": "Subpasta do projeto onde rodar, relativa à raiz (ex.: src-tauri). \
                            Só é preciso em repositório com mais de uma linguagem; omita para \
                            usar a principal."
        },
        "timeout_secs": {
            "type": "integer",
            "description": format!(
                "Tempo máximo em segundos antes de interromper. Padrão {default_timeout}, teto {}.",
                exec::MAX_TIMEOUT_SECS
            )
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_types::agent::{ToolCategory, ToolTier};
    use tempfile::TempDir;

    fn empty_project() -> (TempDir, ToolContext) {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolContext::new(Some(dir.path().to_path_buf()), "call-1");
        (dir, ctx)
    }

    #[test]
    fn the_six_tools_are_exposed_with_the_right_categories() {
        let tools = code_tools();
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert_eq!(
            names,
            vec![
                "project_info",
                "code_run",
                "test_run",
                "lint_run",
                "format_run",
                "build_run"
            ]
        );

        for tool in &tools {
            let spec = tool.spec();
            if spec.name == "project_info" {
                assert_eq!(spec.category, ToolCategory::Read, "só lê manifesto");
                assert!(spec.read_only);
                assert_eq!(spec.tier, ToolTier::Safe);
            } else {
                assert_eq!(
                    spec.category,
                    ToolCategory::Execute,
                    "{} roda processo",
                    spec.name
                );
                assert!(!spec.read_only, "{}", spec.name);
            }
            // O modelo lê o schema para decidir: precisa ser objeto com
            // descrição em cada propriedade.
            let params = tool.parameters();
            assert_eq!(params["type"], "object", "{}", spec.name);
            for (key, value) in params["properties"].as_object().unwrap() {
                assert!(
                    value["description"].is_string(),
                    "{}.{key} sem descrição",
                    spec.name
                );
            }
        }
    }

    #[test]
    fn register_all_puts_them_in_a_registry() {
        let mut registry = lr_tools::ToolRegistry::new();
        register_all(&mut registry);
        let names = registry.builtin_names();
        assert!(names.contains(&"test_run".to_string()), "{names:?}");
        assert_eq!(names.len(), 6);
    }

    #[tokio::test]
    async fn without_a_workspace_every_tool_explains_itself() {
        let ctx = ToolContext::new(None, "call-1");
        for tool in code_tools() {
            let args = serde_json::json!({"language": "python", "code": "print(1)"});
            let err = tool.execute(args, &ctx).await.unwrap_err();
            let msg = err.to_model_message();
            assert!(
                msg.contains("pasta de projeto"),
                "{}: mensagem ruim: {msg}",
                tool.name()
            );
            assert!(
                !tool.within_workspace(&Value::Null, &ctx),
                "{}",
                tool.name()
            );
        }
    }

    #[test]
    fn an_unrecognized_folder_gets_an_actionable_error() {
        let (_dir, ctx) = empty_project();
        let err = target(&Value::Null, &ctx, Task::Test).unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("package.json"), "{msg}");
        assert!(msg.contains("terminal_run"), "deve sugerir a saída: {msg}");
    }

    #[test]
    fn a_dir_outside_the_workspace_is_refused_before_touching_disk() {
        let (_dir, ctx) = empty_project();
        let args = serde_json::json!({"dir": "../fora"});
        let err = target(&args, &ctx, Task::Test).unwrap_err();
        assert!(matches!(err, ToolError::OutsideWorkspace(_)), "{err:?}");
    }

    #[test]
    fn missing_command_points_at_the_stack_that_has_one() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"build":"vite build"}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src-tauri")).unwrap();
        std::fs::write(
            dir.path().join("src-tauri/Cargo.toml"),
            "[package]\nname=\"app\"\n",
        )
        .unwrap();
        let ctx = ToolContext::new(Some(dir.path().to_path_buf()), "call-1");

        // Pedindo explicitamente a raiz, que não tem teste.
        let args = serde_json::json!({"dir": "."});
        let err = target(&args, &ctx, Task::Test).unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("não tem comando para testar"), "{msg}");
        assert!(msg.contains("src-tauri"), "deve indicar quem tem: {msg}");
        assert!(msg.contains("dir=\"src-tauri\""), "{msg}");
    }

    #[test]
    fn timeout_is_clamped_to_the_ceiling() {
        let args = serde_json::json!({"timeout_secs": 999_999});
        assert_eq!(timeout_of(&args, 300), exec::MAX_TIMEOUT_SECS);
        assert_eq!(timeout_of(&serde_json::json!({}), 300), 300);
        assert_eq!(timeout_of(&serde_json::json!({"timeout_secs": 0}), 300), 1);
    }
}
