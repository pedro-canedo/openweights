//! Ferramenta de terminal: roda um comando na pasta do projeto.
//!
//! É a ferramenta mais poderosa do agente e por isso a mais cuidadosa. Duas
//! decisões explicam o resto do arquivo:
//!
//! **Sem shell por padrão.** A linha é dividida em programa + argumentos e vai
//! direto para o sistema. Assim `git commit -m "mensagem com espaço"` chega
//! como um argumento só, e nada é reinterpretado no caminho.
//!
//! **Com shell quando a linha tem sintaxe de shell.** Pipe, `&&`,
//! redireção e variáveis não existem sem um shell — rodar `ls | grep x` sem
//! ele simplesmente não faz o que a pessoa leu na tela. Como `Opaque` é
//! justamente a classe que a política manda confirmar *sempre* (inclusive em
//! modo automático), a linha só chega aqui depois de alguém ter lido e
//! aprovado exatamente o que vai rodar. Executá-la pelo shell é honrar essa
//! aprovação, não contorná-la.

use crate::spawner::{self, SpawnRequest};
use crate::{Tool, ToolContext, ToolError, ToolOutput, ToolResult, arg_str, arg_str_opt, arg_u64};
use async_trait::async_trait;
use lr_policy::classify;
use lr_types::agent::{ToolCategory, ToolPreview};
use serde_json::{Value, json};
use std::path::PathBuf;

/// Tempo padrão de um comando do agente.
const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Teto de tempo: acima disso é tarefa de servidor, não de agente.
const MAX_TIMEOUT_SECS: u64 = 600;

/// Espaço reservado no resultado para o cabeçalho e os rótulos.
const RESULT_OVERHEAD_BYTES: usize = 512;

/// Roda um comando de terminal na pasta do projeto.
pub struct TerminalRun;

/// Divide a linha em tokens respeitando aspas simples e duplas.
///
/// Não interpreta escapes nem expande nada: é só o suficiente para separar
/// programa e argumentos de uma linha simples. Linha complicada não passa por
/// aqui — vira `Opaque` e vai para o shell.
fn tokenize(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut open = false;

    for ch in line.chars() {
        match (quote, ch) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => current.push(c),
            (None, c @ ('"' | '\'')) => {
                quote = Some(c);
                open = true; // `--msg=""` produz um argumento vazio de verdade.
            }
            (None, c) if c.is_whitespace() => {
                if !current.is_empty() || open {
                    out.push(std::mem::take(&mut current));
                    open = false;
                }
            }
            (None, c) => current.push(c),
        }
    }
    if !current.is_empty() || open {
        out.push(current);
    }
    out
}

/// Pasta onde o comando roda: a do argumento `cwd` ou a raiz do projeto.
fn resolve_cwd(args: &Value, ctx: &ToolContext) -> ToolResult<PathBuf> {
    let dir = match arg_str_opt(args, "cwd") {
        Some(rel) => ctx.resolve(&rel)?,
        None => ctx.resolve(".")?,
    };
    if !dir.is_dir() {
        return Err(ToolError::NotFound(ctx.relativize(&dir)));
    }
    Ok(dir)
}

/// Como a pasta aparece na tela de confirmação.
fn cwd_for_preview(args: &Value, ctx: &ToolContext) -> String {
    if let Some(rel) = arg_str_opt(args, "cwd") {
        return match ctx.resolve(&rel) {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(_) => rel,
        };
    }
    ctx.workspace
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[async_trait]
impl Tool for TerminalRun {
    fn name(&self) -> &str {
        "terminal_run"
    }

    fn description(&self) -> &str {
        "Roda um comando de terminal na pasta do projeto e devolve o código de saída com a saída \
         do programa. Use para compilar, testar e rodar ferramentas de linha de comando."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Linha de comando completa, ex.: cargo test -p lr_tools. Um comando por chamada."
                },
                "cwd": {
                    "type": "string",
                    "description": "Pasta onde rodar, relativa à raiz do projeto. Omita para rodar na raiz."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Tempo máximo em segundos antes de interromper. Padrão 120, teto 600."
                }
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Execute
    }

    fn within_workspace(&self, args: &Value, ctx: &ToolContext) -> bool {
        match arg_str_opt(args, "cwd") {
            Some(rel) => ctx.resolve(&rel).is_ok(),
            None => ctx.workspace.is_some(),
        }
    }

    async fn preview(&self, args: &Value, ctx: &ToolContext) -> Option<ToolPreview> {
        let command = arg_str(args, "command").ok()?;
        let command = command.trim().to_string();
        let analysis = classify(&command);
        Some(ToolPreview::Command {
            program: analysis.program,
            display: command,
            cwd: cwd_for_preview(args, ctx),
            class: analysis.class,
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let command = arg_str(&args, "command")?;
        let command = command.trim().to_string();
        if command.is_empty() {
            return Err(ToolError::InvalidArgs(
                "`command` está vazio — mande a linha que quer rodar".into(),
            ));
        }

        let cwd = resolve_cwd(&args, ctx)?;
        let timeout =
            arg_u64(&args, "timeout_secs", DEFAULT_TIMEOUT_SECS).clamp(1, MAX_TIMEOUT_SECS);
        let budget = ctx
            .max_output_bytes
            .saturating_sub(RESULT_OVERHEAD_BYTES)
            .max(1_024);

        // Shell é questão de SINTAXE (pipe, redireção, glob), não de
        // classificação: `ls | head` é analisável e ainda assim precisa do
        // shell para o `|` significar alguma coisa.
        let request = if lr_policy::needs_shell(&command) {
            // Ver a nota no topo: só chega aqui com aprovação humana.
            SpawnRequest::shell(&command, cwd)
        } else {
            let tokens = tokenize(&command);
            let (program, rest) = tokens.split_first().ok_or_else(|| {
                ToolError::InvalidArgs(format!(
                    "não consegui identificar o programa em `{command}`"
                ))
            })?;
            SpawnRequest::new(program.clone(), rest.to_vec(), cwd)
        }
        .with_timeout(timeout)
        .with_max_output(budget);

        let program = request.program.clone();
        let outcome = spawner::run(request, ctx.output_fn()).await.map_err(|e| {
            ToolError::Other(format!(
                "não foi possível iniciar `{program}` ({e}). Confira se o programa está instalado \
                 e se o nome está certo."
            ))
        })?;

        if outcome.timed_out {
            return Err(ToolError::Timeout(timeout));
        }

        let mut body = match outcome.exit_code {
            Some(code) => format!("exit code {code}"),
            // Sem código: o processo morreu por sinal. Deixar sem número é de
            // propósito — quem lê o resultado não deve inventar um zero.
            None => "exit code desconhecido (o processo foi encerrado por fora)".to_string(),
        };

        let stdout = outcome.stdout.trim_end();
        let stderr = outcome.stderr.trim_end();
        if !stdout.is_empty() {
            body.push_str("\n\n[stdout]\n");
            body.push_str(stdout);
        }
        if !stderr.is_empty() {
            body.push_str("\n\n[stderr]\n");
            body.push_str(stderr);
        }
        if stdout.is_empty() && stderr.is_empty() {
            body.push_str("\n(o comando não escreveu nada)");
        }
        if outcome.truncated {
            body.push_str("\n\n[...saída cortada no limite; rode algo mais específico...]");
        }

        // O código declarado no campo, e não só no texto: é o que blinda a
        // verificação contra um stdout que por acaso contenha "exit code N".
        Ok(ToolOutput::text(body)
            .with_exit_code(outcome.exit_code)
            .truncated_to(ctx.max_output_bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_types::agent::CommandClass;
    use tempfile::TempDir;

    fn project() -> (TempDir, ToolContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("marca.txt"), "conteudo\n").unwrap();
        let ctx = ToolContext::new(Some(dir.path().to_path_buf()), "call-1");
        (dir, ctx)
    }

    /// Linha equivalente em cada shell, para a suíte valer nas duas
    /// plataformas.
    fn line(unix: &str, windows: &str) -> String {
        if cfg!(windows) {
            windows.to_string()
        } else {
            unix.to_string()
        }
    }

    async fn run(args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        TerminalRun.execute(args, ctx).await
    }

    /// O contrato que o terminal da sessão consome: a saída chega ENQUANTO o
    /// comando roda, não só no resultado. Sem isto o painel fica em branco
    /// durante um build de três minutos e volta a ser um `<pre>` estático.
    #[tokio::test]
    async fn the_session_sink_receives_output_while_the_command_runs() {
        let (dir, base) = project();
        let visto = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let alvo = visto.clone();
        let ctx = base.with_output_sink(std::sync::Arc::new(move |chunk: &str| {
            alvo.lock().unwrap().push_str(chunk);
        }));

        // `echo` não é programa no Windows (é embutido do `cmd`), e prefixar
        // com `cmd /c` deixaria a linha opaca — que pede confirmação até no
        // modo automático. `findstr`/`grep` são programas de verdade nos dois
        // lados, e `marca.txt` é escrito pelo `project()`.
        let cmd = line("grep -n . marca.txt", "findstr /n . marca.txt");
        let out = run(json!({ "command": cmd }), &ctx).await.unwrap();

        let streamed = visto.lock().unwrap().clone();
        assert!(
            streamed.contains("conteudo"),
            "o sink não recebeu nada: {streamed:?}"
        );
        assert!(out.content.contains("conteudo"), "{}", out.content);
        drop(dir);
    }

    #[test]
    fn tokenize_respects_quotes() {
        assert_eq!(tokenize("git status"), vec!["git", "status"]);
        assert_eq!(
            tokenize(r#"git commit -m "duas palavras""#),
            vec!["git", "commit", "-m", "duas palavras"]
        );
        assert_eq!(
            tokenize("echo 'com aspas simples'"),
            vec!["echo", "com aspas simples"]
        );
        // Argumento vazio explícito não pode sumir.
        assert_eq!(tokenize(r#"prog "" fim"#), vec!["prog", "", "fim"]);
        assert!(tokenize("   ").is_empty());
    }

    #[tokio::test]
    async fn runs_a_simple_command_and_reports_success() {
        let (_d, ctx) = project();
        let cmd = line("echo ola-agente", "cmd /c echo ola-agente");
        let out = run(json!({"command": cmd, "timeout_secs": 30}), &ctx)
            .await
            .unwrap();
        assert!(out.content.starts_with("exit code 0"), "{}", out.content);
        assert!(out.content.contains("ola-agente"), "{}", out.content);
    }

    #[tokio::test]
    async fn reports_a_non_zero_exit_code() {
        let (_d, ctx) = project();
        // `exit` é embutido do shell: o `&&` deixa a linha Opaque e ela roda
        // pelo shell, que é o único jeito de o comando existir.
        let out = run(
            json!({"command": "exit 3 && true", "timeout_secs": 30}),
            &ctx,
        )
        .await
        .unwrap();
        assert!(out.content.starts_with("exit code 3"), "{}", out.content);
    }

    #[tokio::test]
    async fn failing_program_keeps_the_code_for_the_model() {
        let (_d, ctx) = project();
        // Comando reconhecido (sem shell) que falha: caminho inexistente.
        let cmd = line("ls pasta-que-nao-existe", "cmd /c dir pasta-que-nao-existe");
        let out = run(json!({"command": cmd, "timeout_secs": 30}), &ctx)
            .await
            .unwrap();
        assert!(out.content.starts_with("exit code "), "{}", out.content);
        assert!(!out.content.starts_with("exit code 0"), "{}", out.content);
    }

    #[tokio::test]
    async fn chained_lines_go_through_the_shell() {
        let (_d, ctx) = project();
        // Sem shell, `&&` viraria um argumento literal e nada encadearia —
        // e isso NÃO depende de a linha ser analisável ou não.
        let cmd = line("echo um && echo dois", "echo um && echo dois");
        assert!(lr_policy::needs_shell(&cmd));
        let out = run(json!({"command": cmd, "timeout_secs": 30}), &ctx)
            .await
            .unwrap();
        assert!(out.content.contains("um"), "{}", out.content);
        assert!(out.content.contains("dois"), "{}", out.content);
    }

    #[tokio::test]
    async fn timeout_becomes_an_actionable_error() {
        let (_d, ctx) = project();
        let cmd = line("sleep 30", "ping -n 30 127.0.0.1");
        let err = run(json!({"command": cmd, "timeout_secs": 1}), &ctx)
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Timeout(1)), "{err:?}");
        assert!(err.to_model_message().contains("1s"));
    }

    #[tokio::test]
    async fn preview_shows_the_command_and_its_class() {
        let (dir, ctx) = project();
        let args = json!({"command": "  git status  "});
        match TerminalRun.preview(&args, &ctx).await.unwrap() {
            ToolPreview::Command {
                program,
                display,
                cwd,
                class,
            } => {
                assert_eq!(program, "git");
                assert_eq!(display, "git status", "a tela mostra a linha limpa");
                assert_eq!(class, CommandClass::ReadOnly);
                assert_eq!(cwd, dir.path().to_string_lossy());
            }
            other => panic!("esperava prévia de comando, veio {other:?}"),
        }
    }

    #[tokio::test]
    async fn preview_shows_the_worst_link_of_a_chain() {
        let (_d, ctx) = project();
        let args = json!({"command": "cargo build && rm -rf alvo"});
        match TerminalRun.preview(&args, &ctx).await.unwrap() {
            // `rm` é a peça que manda; a linha inteira continua analisável.
            ToolPreview::Command { class, .. } => {
                assert_eq!(class, lr_types::agent::CommandClass::Mutating)
            }
            other => panic!("esperava prévia de comando, veio {other:?}"),
        }
    }

    #[tokio::test]
    async fn runs_inside_the_requested_subfolder() {
        let (_d, ctx) = project();
        let cmd = line("pwd", "cmd /c cd");
        let out = run(
            json!({"command": cmd, "cwd": "src", "timeout_secs": 30}),
            &ctx,
        )
        .await
        .unwrap();
        assert!(out.content.contains("src"), "{}", out.content);
    }

    #[tokio::test]
    async fn a_cwd_outside_the_project_is_flagged_and_refused() {
        let (_d, ctx) = project();
        let args = json!({"command": "ls", "cwd": "../fora"});
        assert!(
            !TerminalRun.within_workspace(&args, &ctx),
            "a política precisa exigir confirmação"
        );
        let err = run(args, &ctx).await.unwrap_err();
        assert!(matches!(err, ToolError::OutsideWorkspace(_)), "{err:?}");
    }

    #[tokio::test]
    async fn without_a_workspace_there_is_nowhere_to_run() {
        let ctx = ToolContext::new(None, "call-1");
        let args = json!({"command": "ls"});
        assert!(!TerminalRun.within_workspace(&args, &ctx));
        assert!(run(args, &ctx).await.is_err());
    }

    #[tokio::test]
    async fn an_empty_command_is_rejected_before_spawning() {
        let (_d, ctx) = project();
        let err = run(json!({"command": "   "}), &ctx).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)), "{err:?}");
    }

    #[tokio::test]
    async fn a_missing_program_explains_itself() {
        let (_d, ctx) = project();
        let err = run(
            json!({"command": "programa-que-nao-existe-xyz --versao", "timeout_secs": 20}),
            &ctx,
        )
        .await
        .unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("programa-que-nao-existe-xyz"), "{msg}");
        assert!(msg.contains("instalado"), "{msg}");
    }

    #[tokio::test]
    async fn quoted_arguments_survive_without_a_shell() {
        let (_d, ctx) = project();
        let cmd = line(
            r#"/bin/echo "uma frase inteira""#,
            r#"cmd /c echo "uma frase inteira""#,
        );
        let out = run(json!({"command": cmd, "timeout_secs": 30}), &ctx)
            .await
            .unwrap();
        assert!(out.content.contains("uma frase inteira"), "{}", out.content);
    }

    #[tokio::test]
    async fn long_output_is_truncated_for_the_model() {
        let (_d, ctx) = project();
        let mut small = ToolContext::new(ctx.workspace.clone(), "call-2");
        small.max_output_bytes = 2_000;
        let cmd = line(
            "i=0; while [ $i -lt 5000 ]; do echo linha-$i; i=$((i+1)); done",
            "cmd /c for /L %i in (1,1,5000) do @echo linha-%i",
        );
        let out = run(json!({"command": cmd, "timeout_secs": 60}), &small)
            .await
            .unwrap();
        assert!(out.content.len() <= 2_100, "tamanho {}", out.content.len());
        assert!(out.content.contains("cortada"), "{}", out.content);
    }
}
