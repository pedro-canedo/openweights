//! `format_run`: confere ou aplica o formatador do projeto.
//!
//! Formatação é a única ferramenta daqui que **reescreve o código do usuário**
//! sem que ele tenha pedido arquivo por arquivo. Isso muda tudo no desenho:
//!
//! **`fix=false` é o padrão.** Sem pedido explícito, só conferimos e dizemos
//! quais arquivos estão fora do padrão. Nenhum byte é escrito.
//!
//! **A tela de confirmação mostra a lista.** Com `fix=true`, a prévia roda o
//! modo conferência (`cargo fmt --check`, `prettier --check`, `gofmt -l`) e
//! mostra exatamente quais arquivos serão reescritos. Aprovar "reformatar 3
//! arquivos, estes aqui" é uma decisão; aprovar "rodar o formatador" é um
//! salto de fé.
//!
//! **O checkpoint recebe a mesma lista.** `files_at_risk` roda *antes* da
//! execução e não pode chamar processo (é síncrono), então reaproveita o que a
//! prévia descobriu — as duas rodam na mesma chamada, com o mesmo `call_id`, e
//! a prévia sempre vem primeiro. Se a prévia não tiver rodado (ou o formatador
//! não tiver modo conferência), caímos na varredura conservadora do
//! [`crate::scan`]: melhor guardar arquivo demais do que perder o único que
//! foi reescrito.
//!
//! Rodar um processo dentro de `preview` merece justificativa, já que prévia
//! não pode ter efeito colateral: o modo conferência **só lê** — é o mesmo
//! comando com a flag que proíbe escrever — e tem tempo limitado. O que ele
//! muda no mundo é nada; o que ele acrescenta na tela é a lista inteira.

use crate::detect::{Cmd, Task};
use crate::exec::{self, DEFAULT_TIMEOUT_SECS, PREVIEW_TIMEOUT_SECS};
use crate::scan;
use crate::test_run::{merge, within};
use crate::text::{combined, strip_ansi};
use crate::{Target, command_preview, dir_and_timeout_properties, target, timeout_of};
use async_trait::async_trait;
use lr_tools::{Tool, ToolContext, ToolOutput, ToolResult, arg_bool};
use lr_types::agent::{ToolCategory, ToolPreview};
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Mutex;

/// Arquivos listados nominalmente antes de virar contagem.
const MAX_LISTED: usize = 30;

/// Chamadas guardadas no cache da prévia (um run tem poucas em voo).
const CACHE_SIZE: usize = 8;

/// Confere ou aplica o formatador do projeto.
#[derive(Default)]
pub struct FormatRun {
    /// `call_id` → arquivos que o modo conferência apontou.
    ///
    /// Existe só para ligar a prévia (assíncrona, pode rodar processo) ao
    /// `files_at_risk` (síncrono, não pode) dentro da mesma chamada.
    at_risk: Mutex<Vec<(String, Vec<String>)>>,
}

impl FormatRun {
    fn remember(&self, call_id: &str, files: Vec<String>) {
        let Ok(mut cache) = self.at_risk.lock() else {
            return;
        };
        cache.retain(|(id, _)| id != call_id);
        cache.push((call_id.to_string(), files));
        if cache.len() > CACHE_SIZE {
            cache.remove(0);
        }
    }

    fn recall(&self, call_id: &str) -> Option<Vec<String>> {
        let cache = self.at_risk.lock().ok()?;
        cache
            .iter()
            .find(|(id, _)| id == call_id)
            .map(|(_, files)| files.clone())
    }

    /// Roda o modo conferência e devolve os arquivos fora do padrão.
    async fn check(
        &self,
        target: &Target,
        root: &Path,
        timeout: u64,
        on_output: impl FnMut(&str) + Send,
    ) -> Option<CheckResult> {
        let cmd = target.stack.format_check.clone()?;
        let run = exec::run(&cmd, &target.cwd, timeout, exec::CAPTURE_BYTES, on_output)
            .await
            .ok()?;
        let output = combined(&run.outcome.stdout, &run.outcome.stderr);
        Some(CheckResult {
            clean: run.success(),
            files: files_from_check(&output, root, &target.cwd),
            cmd,
        })
    }
}

/// O que o modo conferência descobriu.
struct CheckResult {
    /// O formatador terminou com código 0 (nada a mudar).
    clean: bool,
    files: Vec<String>,
    cmd: Cmd,
}

#[async_trait]
impl Tool for FormatRun {
    fn name(&self) -> &str {
        "format_run"
    }

    fn description(&self) -> &str {
        "Confere a formatação do projeto (cargo fmt, prettier, black, gofmt…) e, com fix=true, \
         aplica. Sem fix, apenas lista os arquivos fora do padrão, sem alterar nada."
    }

    fn parameters(&self) -> Value {
        let mut properties = json!({
            "fix": {
                "type": "boolean",
                "description": "true reescreve os arquivos fora do padrão. Padrão false, que só confere e lista."
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

    fn files_at_risk(&self, args: &Value, ctx: &ToolContext) -> Vec<String> {
        if !arg_bool(args, "fix", false) {
            return Vec::new();
        }
        if let Some(files) = self.recall(&ctx.call_id) {
            return files;
        }
        // Sem a resposta da prévia: assume o pior caso, com teto.
        let (Ok(target), Some(root)) = (target(args, ctx, Task::Format), ctx.workspace.as_ref())
        else {
            return Vec::new();
        };
        scan::source_files(root, &target.cwd, &target.stack.source_ext, scan::MAX_FILES)
    }

    async fn preview(&self, args: &Value, ctx: &ToolContext) -> Option<ToolPreview> {
        let target = target(args, ctx, Task::Format).ok()?;
        let root = ctx.workspace.clone()?;

        if !arg_bool(args, "fix", false) {
            let cmd = target
                .stack
                .format_check
                .clone()
                .or_else(|| target.stack.format.clone())?;
            return Some(command_preview(&cmd, &target.cwd));
        }

        let apply = target.stack.format.clone()?;
        let Some(check) = self
            .check(&target, &root, PREVIEW_TIMEOUT_SECS, |_| {})
            .await
        else {
            return Some(command_preview(&apply, &target.cwd));
        };
        self.remember(&ctx.call_id, check.files.clone());

        let mut body = format!("{}\n", apply.display());
        if check.clean && check.files.is_empty() {
            body.push_str("Nada a reformatar: o projeto já está no padrão.");
        } else if check.files.is_empty() {
            body.push_str(
                "Há arquivos fora do padrão (o formatador não disse quais). Todos os fontes \
                 podem ser reescritos.",
            );
        } else {
            let _ = write!(body, "Vai reformatar {} arquivo(s):", check.files.len());
            for file in check.files.iter().take(MAX_LISTED) {
                let _ = write!(body, "\n  {file}");
            }
            if check.files.len() > MAX_LISTED {
                let _ = write!(body, "\n  … e mais {}", check.files.len() - MAX_LISTED);
            }
        }
        Some(ToolPreview::Text { body })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let target = target(&args, ctx, Task::Format)?;
        let root = crate::workspace(ctx)?;
        let fix = arg_bool(&args, "fix", false);
        let timeout = timeout_of(&args, DEFAULT_TIMEOUT_SECS);

        // Conferir primeiro serve para os dois modos: sem `fix` é a resposta
        // inteira; com `fix` é o que permite dizer o que mudou de verdade.
        let check = self.check(&target, &root, timeout, ctx.output_fn()).await;

        if !fix {
            return Ok(no_fix_answer(&target, check, ctx).truncated_to(ctx.max_output_bytes));
        }

        let apply = target
            .stack
            .format
            .clone()
            .expect("target garante que o comando existe");
        let run = exec::run(
            &apply,
            &target.cwd,
            timeout,
            exec::CAPTURE_BYTES,
            ctx.output_fn(),
        )
        .await?;

        let mut body = format!("{}\nstack: {}", run.header(), target.where_line());
        let changed = check.as_ref().map(|c| c.files.clone()).unwrap_or_default();

        if !run.success() {
            let output = combined(&run.outcome.stdout, &run.outcome.stderr);
            body.push_str("\n\nO formatador falhou. Fim da saída:");
            for line in crate::text::tail_lines(&output, 15) {
                body.push('\n');
                body.push_str(&crate::text::clip_line(line, 200));
            }
            return Ok(ToolOutput::text(body)
                .with_exit_code(run.outcome.exit_code)
                .truncated_to(ctx.max_output_bytes));
        }

        match (check.as_ref().map(|c| c.clean), changed.len()) {
            (Some(true), _) => body.push_str("\n\nNada a fazer: já estava tudo formatado."),
            (_, 0) => body.push_str(
                "\n\nFormatador aplicado. Não consegui listar quais arquivos mudaram — use \
                 `terminal_run` com `git status` se precisar saber.",
            ),
            (_, n) => {
                let _ = write!(body, "\n\n{n} arquivo(s) reformatado(s):");
                for file in changed.iter().take(MAX_LISTED) {
                    let _ = write!(body, "\n  {file}");
                }
                if n > MAX_LISTED {
                    let _ = write!(body, "\n  … e mais {}", n - MAX_LISTED);
                }
            }
        }

        Ok(ToolOutput::text(body)
            .with_changed(changed)
            .with_exit_code(run.outcome.exit_code)
            .truncated_to(ctx.max_output_bytes))
    }
}

/// Resposta do modo conferência (nada foi escrito).
fn no_fix_answer(target: &Target, check: Option<CheckResult>, ctx: &ToolContext) -> ToolOutput {
    let Some(check) = check else {
        // O formatador deste projeto só sabe escrever: não fazemos isso sem
        // pedido explícito.
        let apply = target
            .stack
            .format
            .as_ref()
            .map(|c| c.display())
            .unwrap_or_default();
        return ToolOutput::text(format!(
            "O formatador deste projeto (`{apply}`) não tem modo de conferência, então nada foi \
             verificado — e nada foi alterado. Chame de novo com fix=true para aplicá-lo (isso \
             reescreve arquivos).\nstack: {}",
            target.where_line()
        ));
    };

    let mut body = format!(
        "{} — conferência (nenhum arquivo foi alterado)\nstack: {}",
        check.cmd.display(),
        target.where_line()
    );
    if check.clean {
        body.push_str("\n\nTudo formatado: nada a mudar.");
    } else if check.files.is_empty() {
        body.push_str(
            "\n\nHá formatação fora do padrão, mas o formatador não listou os arquivos. Rode com \
             fix=true para aplicar.",
        );
    } else {
        let _ = write!(body, "\n\n{} arquivo(s) fora do padrão:", check.files.len());
        for file in check.files.iter().take(MAX_LISTED) {
            let _ = write!(body, "\n  {file}");
        }
        if check.files.len() > MAX_LISTED {
            let _ = write!(body, "\n  … e mais {}", check.files.len() - MAX_LISTED);
        }
        body.push_str("\n\nChame de novo com fix=true para corrigir.");
    }
    let _ = ctx;
    ToolOutput::text(body)
}

/// Arquivos que o modo conferência apontou, em caminho relativo à raiz.
///
/// Cada formatador anuncia de um jeito; todos anunciam. As cinco formas
/// abaixo cobrem rustfmt, prettier, gofmt, black e ruff — e uma linha que não
/// case simplesmente não vira arquivo, nunca vira arquivo errado.
fn files_from_check(output: &str, root: &Path, cwd: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();

    for line in strip_ansi(output).lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let candidate = if let Some(rest) = line.strip_prefix("Diff in ") {
            // rustfmt novo: `Diff in /abs/src/lib.rs:1:`
            // rustfmt antigo: `Diff in /abs/src/lib.rs at line 3:`
            Some(trim_line_suffix(
                rest.split(" at line ").next().unwrap_or(rest),
            ))
        } else if let Some(rest) = line.strip_prefix("[warn] ") {
            // prettier: `[warn] src/a.ts` (e um rodapé em prosa)
            (!rest.contains(' ')).then(|| rest.to_string())
        } else if let Some(rest) = line.strip_prefix("would reformat ") {
            // black
            Some(rest.to_string())
        } else if let Some(rest) = line.strip_prefix("Would reformat: ") {
            // ruff format --check
            Some(rest.to_string())
        } else if looks_like_bare_path(line) {
            // gofmt -l: só o caminho, um por linha
            Some(line.to_string())
        } else {
            None
        };

        let Some(candidate) = candidate else { continue };
        let candidate = candidate.trim().trim_matches('"');
        if candidate.is_empty() {
            continue;
        }

        let absolute = Path::new(candidate);
        let absolute = if absolute.is_absolute() {
            absolute.to_path_buf()
        } else {
            cwd.join(candidate)
        };
        if let Some(rel) = scan::relativize(root, &absolute)
            && !out.contains(&rel)
        {
            out.push(rel);
        }
    }
    out.sort();
    out
}

/// Tira o `:linha:` (ou `:linha:coluna:`) que o rustfmt cola no caminho.
fn trim_line_suffix(text: &str) -> String {
    let mut rest = text.trim_end_matches(':');
    // No máximo dois campos numéricos no fim; o resto é caminho.
    for _ in 0..2 {
        match rest.rsplit_once(':') {
            Some((head, tail)) if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) => {
                rest = head;
            }
            _ => break,
        }
    }
    rest.to_string()
}

/// Uma linha que é só um caminho de arquivo com extensão.
fn looks_like_bare_path(line: &str) -> bool {
    // Linha de diff (`+ …`, `- …`) não é nome de arquivo.
    if line.starts_with('+') || line.starts_with('-') {
        return false;
    }
    if line.contains(' ') || line.contains(':') && !line.contains('\\') {
        return false;
    }
    Path::new(line)
        .extension()
        .and_then(|e| e.to_str())
        .map(|ext| !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or(false)
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

    /// Código propositalmente mal formatado.
    const TORTO: &str = "pub   fn soma(a:i32,b:i32)->i32{\n a+b\n}\n";

    #[test]
    fn every_formatter_dialect_of_check_output_is_understood() {
        let root = Path::new("/projeto");
        let cwd = Path::new("/projeto/sub");

        let rustfmt = "Diff in /projeto/src/lib.rs at line 3:\n\
                       Diff in /projeto/src/lib.rs at line 9:\n\
                       Diff in /projeto/src/main.rs at line 1:";
        assert_eq!(
            files_from_check(rustfmt, root, cwd),
            vec!["src/lib.rs", "src/main.rs"],
            "repetição vira um arquivo só"
        );

        let prettier = "Checking formatting...\n[warn] src/a.ts\n[warn] src/b.css\n\
                        [warn] Code style issues found in 2 files. Run Prettier to fix.";
        assert_eq!(
            files_from_check(prettier, root, root),
            vec!["src/a.ts", "src/b.css"],
            "o rodapé em prosa não é arquivo"
        );

        let gofmt = "cmd/main.go\ninternal/soma.go\n";
        assert_eq!(
            files_from_check(gofmt, root, root),
            vec!["cmd/main.go", "internal/soma.go"]
        );

        let black = "would reformat /projeto/app/main.py\n1 file would be reformatted.";
        assert_eq!(files_from_check(black, root, root), vec!["app/main.py"]);

        let ruff = "Would reformat: app/api.py\n1 file would be reformatted";
        assert_eq!(files_from_check(ruff, root, root), vec!["app/api.py"]);
    }

    #[test]
    fn paths_are_relative_to_the_project_and_never_escape_it() {
        let root = Path::new("/projeto");
        let cwd = Path::new("/projeto/src-tauri");
        // Caminho relativo é resolvido a partir da pasta da stack.
        assert_eq!(
            files_from_check("src/lib.rs\n", root, cwd),
            vec!["src-tauri/src/lib.rs"]
        );
        // Fora do projeto não entra na lista.
        assert!(files_from_check("Diff in /outro/x.rs at line 1:\n", root, cwd).is_empty());
    }

    #[tokio::test]
    async fn without_fix_nothing_is_written_and_the_files_are_listed() {
        if !program_exists("cargo") {
            assert!(skip("cargo"));
            return;
        }
        let (dir, ctx) = rust_project(TORTO);
        let out = FormatRun::default()
            .execute(json!({"timeout_secs": 300}), &ctx)
            .await
            .unwrap();
        let depois = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
        assert_eq!(depois, TORTO, "conferência não pode reescrever nada");
        assert!(
            out.content.contains("nenhum arquivo foi alterado"),
            "{}",
            out.content
        );
        assert!(out.content.contains("src/lib.rs"), "{}", out.content);
        assert!(out.content.contains("fix=true"), "{}", out.content);
        assert!(out.changed_files.is_empty());
    }

    #[tokio::test]
    async fn with_fix_the_file_is_formatted_and_reported_as_changed() {
        if !program_exists("cargo") {
            assert!(skip("cargo"));
            return;
        }
        let (dir, ctx) = rust_project(TORTO);
        let out = FormatRun::default()
            .execute(json!({"fix": true, "timeout_secs": 300}), &ctx)
            .await
            .unwrap();
        let depois = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
        assert_ne!(depois, TORTO, "com fix o arquivo tem de mudar");
        assert!(
            depois.contains("pub fn soma(a: i32, b: i32) -> i32 {"),
            "{depois}"
        );
        assert_eq!(
            out.changed_files,
            vec!["src/lib.rs"],
            "{:?}",
            out.changed_files
        );
        assert!(
            out.content.contains("1 arquivo(s) reformatado(s)"),
            "{}",
            out.content
        );
    }

    #[tokio::test]
    async fn an_already_formatted_project_says_there_is_nothing_to_do() {
        if !program_exists("cargo") {
            assert!(skip("cargo"));
            return;
        }
        let (_d, ctx) = rust_project("pub fn soma(a: i32, b: i32) -> i32 {\n    a + b\n}\n");
        let out = FormatRun::default()
            .execute(json!({"timeout_secs": 300}), &ctx)
            .await
            .unwrap();
        assert!(out.content.contains("Tudo formatado"), "{}", out.content);
    }

    #[tokio::test]
    async fn the_preview_lists_what_will_be_rewritten_and_feeds_the_checkpoint() {
        if !program_exists("cargo") {
            assert!(skip("cargo"));
            return;
        }
        let (_d, ctx) = rust_project(TORTO);
        let tool = FormatRun::default();
        let args = json!({"fix": true});

        match tool.preview(&args, &ctx).await.unwrap() {
            ToolPreview::Text { body } => {
                assert!(body.contains("cargo fmt"), "{body}");
                assert!(body.contains("Vai reformatar 1 arquivo(s)"), "{body}");
                assert!(body.contains("src/lib.rs"), "{body}");
            }
            other => panic!("esperava prévia de texto, veio {other:?}"),
        }
        // O checkpoint aproveita o que a prévia descobriu, sem rodar de novo.
        assert_eq!(tool.files_at_risk(&args, &ctx), vec!["src/lib.rs"]);
    }

    #[test]
    fn without_a_preview_the_checkpoint_falls_back_to_all_sources() {
        let (_d, ctx) = rust_project(TORTO);
        let tool = FormatRun::default();
        // Sem `fix`, nada é reescrito: nada em risco.
        assert!(tool.files_at_risk(&json!({}), &ctx).is_empty());
        // Com `fix` e sem prévia: lista conservadora dos fontes.
        let at_risk = tool.files_at_risk(&json!({"fix": true}), &ctx);
        assert_eq!(at_risk, vec!["src/lib.rs"], "{at_risk:?}");
    }

    #[test]
    fn the_preview_cache_is_bounded_and_keyed_by_call() {
        let tool = FormatRun::default();
        for i in 0..CACHE_SIZE + 4 {
            tool.remember(&format!("call-{i}"), vec![format!("a{i}.rs")]);
        }
        assert_eq!(tool.at_risk.lock().unwrap().len(), CACHE_SIZE);
        assert!(tool.recall("call-0").is_none(), "o mais antigo saiu");
        assert_eq!(
            tool.recall(&format!("call-{}", CACHE_SIZE + 3)),
            Some(vec![format!("a{}.rs", CACHE_SIZE + 3)])
        );
    }
}
