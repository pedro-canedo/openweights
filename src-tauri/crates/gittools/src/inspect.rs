//! Leitura do repositório: `git_diff`, `git_log` e `git_branch`.
//!
//! São as três perguntas que o agente faz antes de mexer em qualquer coisa:
//! *o que mudou*, *o que já foi feito aqui* e *onde eu estou*. Todas são
//! somente-leitura e, por isso, rodam sem confirmação dentro da pasta do
//! projeto.
//!
//! Uma decisão de formato atravessa o arquivo: **saída em campos separados por
//! `0x1F`** (`%x1f` do `--pretty`, `%09` do `--format`) em vez de tentar
//! interpretar a saída bonita do git. Assunto de commit tem espaço, nome de
//! autor tem espaço, caminho tem espaço — separar por espaço quebraria. O
//! separador de unidade não aparece em texto normal, então o corte é exato.
//!
//! A outra decisão é sobre repositório recém-criado: `git log` e `git branch`
//! **falham** quando não há nenhum commit. Isso não é erro do agente, é o
//! estado normal de um projeto que acabou de nascer, então devolvemos um
//! resultado explicando a situação em vez de um `Err` que faria o laço tentar
//! de novo e falhar de novo.

use crate::run::{command_failed, git, has_commits, repo_root};
use async_trait::async_trait;
use lr_tools::{Tool, ToolContext, ToolOutput, ToolResult, arg_bool, arg_str_opt, arg_u64};
use lr_types::agent::{ToolCategory, ToolPreview};
use serde_json::{Value, json};

/// Separador de campo usado nos formatos do git.
const UNIT: char = '\u{1f}';

/// Quantos commits o `git_log` traz quando ninguém pede um número.
const DEFAULT_LOG_LIMIT: u64 = 20;

/// Teto de commits por chamada: acima disso o histórico vira ruído.
const MAX_LOG_LIMIT: u64 = 200;

/// Teto de caracteres da prévia mostrada na tela de confirmação.
const PREVIEW_CHARS: usize = 6_000;

/// Larguras das colunas do histórico.
const AUTHOR_WIDTH: usize = 18;
const SUBJECT_WIDTH: usize = 90;

/// Máximo de branches listadas.
const MAX_BRANCHES: usize = 100;

/// Completa (ou corta) um texto até `width` colunas, contando caracteres.
fn pad(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count > width {
        let cut: String = text.chars().take(width.saturating_sub(1)).collect();
        return format!("{cut}…");
    }
    format!("{text}{}", " ".repeat(width - count))
}

/// Confere que um caminho opcional está dentro do projeto e o devolve como
/// veio (o git roda com a raiz do projeto como pasta atual).
fn relative_path(args: &Value, ctx: &ToolContext) -> ToolResult<Option<String>> {
    match arg_str_opt(args, "path") {
        Some(rel) => {
            ctx.resolve(&rel)?;
            Ok(Some(rel))
        }
        None => Ok(None),
    }
}

fn path_is_inside(args: &Value, ctx: &ToolContext) -> bool {
    match arg_str_opt(args, "path") {
        Some(rel) => ctx.resolve(&rel).is_ok(),
        None => true,
    }
}

// ------------------------------------------------------------------ diff ---

/// Ferramenta `git_diff`.
pub struct GitDiff;

impl GitDiff {
    /// Monta a chamada ao git. `--no-ext-diff` ignora um difftool externo
    /// configurado pelo usuário: aqui queremos o formato unificado que o
    /// modelo sabe ler, não o visualizador gráfico dele.
    async fn diff_text(&self, args: &Value, ctx: &ToolContext) -> ToolResult<(String, bool)> {
        let staged = arg_bool(args, "staged", false);
        let rel = relative_path(args, ctx)?;

        let mut cmd: Vec<&str> = vec!["diff", "--no-color", "--no-ext-diff"];
        if staged {
            cmd.push("--staged");
        }
        if let Some(path) = &rel {
            cmd.push("--");
            cmd.push(path);
        }

        let run = git(ctx, &cmd).await?;
        if !run.ok() {
            return Err(command_failed("mostrar as diferenças", &run));
        }
        Ok((run.stdout, run.truncated))
    }
}

#[async_trait]
impl Tool for GitDiff {
    fn name(&self) -> &str {
        "git_diff"
    }

    fn description(&self) -> &str {
        "Mostra, em formato unificado, o que mudou nos arquivos em relação ao último commit. \
         Sem argumentos traz as alterações ainda não preparadas; com `staged: true` traz o que \
         já foi preparado com `git_add`. Use antes de commitar para conferir o que vai entrar."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Arquivo ou pasta específica, relativo à raiz do projeto (ex.: src/app.rs). Omita para ver tudo."
                },
                "staged": {
                    "type": "boolean",
                    "description": "true mostra o que já está preparado para o commit; false (padrão) mostra o que ainda não foi preparado."
                }
            },
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Read
    }

    fn within_workspace(&self, args: &Value, ctx: &ToolContext) -> bool {
        path_is_inside(args, ctx)
    }

    async fn preview(&self, args: &Value, ctx: &ToolContext) -> Option<ToolPreview> {
        repo_root(ctx).await.ok()?;
        let (text, _) = self.diff_text(args, ctx).await.ok()?;
        let alvo = arg_str_opt(args, "path").unwrap_or_else(|| "(todos os arquivos)".into());
        if text.trim().is_empty() {
            return Some(ToolPreview::Text {
                body: format!("Nenhuma diferença em {alvo}."),
            });
        }
        let unified: String = text.chars().take(PREVIEW_CHARS).collect();
        Some(ToolPreview::Diff {
            path: alvo,
            unified,
            created: false,
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        repo_root(ctx).await?;
        let staged = arg_bool(&args, "staged", false);
        let (text, truncated) = self.diff_text(&args, ctx).await?;

        if text.trim().is_empty() {
            let alvo = arg_str_opt(&args, "path")
                .map(|p| format!("em `{p}`"))
                .unwrap_or_else(|| "no projeto".into());
            let dica = if staged {
                "Não há nada preparado para o commit — use `git_add` primeiro."
            } else {
                "Se você já preparou os arquivos com `git_add`, repita com `staged: true`."
            };
            return Ok(ToolOutput::text(format!(
                "Nenhuma diferença {alvo}. {dica}"
            )));
        }

        let mut body = text;
        if truncated {
            body.push_str(
                "\n[diferença cortada por tamanho: chame de novo com `path` para ver um arquivo \
                 de cada vez]\n",
            );
        }
        Ok(ToolOutput::text(body).truncated_to(ctx.max_output_bytes))
    }
}

// ------------------------------------------------------------------- log ---

/// Ferramenta `git_log`.
pub struct GitLog;

/// Uma linha do histórico já separada em campos.
#[derive(Debug, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub author: String,
    pub when: String,
    pub subject: String,
}

/// Interpreta a saída de `--pretty=format:%h%x1f%an%x1f%ad%x1f%s`.
pub fn parse_log(text: &str) -> Vec<Commit> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| {
            let mut fields = line.split(UNIT);
            Some(Commit {
                hash: fields.next()?.to_string(),
                author: fields.next()?.to_string(),
                when: fields.next()?.to_string(),
                // O assunto pode estar vazio (commit sem título), mas o campo
                // precisa existir — senão a linha não é do formato esperado.
                subject: fields.next()?.to_string(),
            })
        })
        .collect()
}

/// Histórico em colunas alinhadas.
pub fn render_log(commits: &[Commit]) -> String {
    let hash_w = commits
        .iter()
        .map(|c| c.hash.chars().count())
        .max()
        .unwrap_or(7);
    let when_w = commits
        .iter()
        .map(|c| c.when.chars().count())
        .max()
        .unwrap_or(10)
        .min(20);

    let mut out = String::new();
    for c in commits {
        out.push_str(&format!(
            "{}  {}  {}  {}\n",
            pad(&c.hash, hash_w),
            pad(&c.when, when_w),
            pad(&c.author, AUTHOR_WIDTH),
            pad(&c.subject, SUBJECT_WIDTH).trim_end()
        ));
    }
    out
}

#[async_trait]
impl Tool for GitLog {
    fn name(&self) -> &str {
        "git_log"
    }

    fn description(&self) -> &str {
        "Lista os commits mais recentes do projeto em formato compacto: hash curto, quando foi, \
         quem fez e o título. Use para entender o que já foi feito e como as mensagens de commit \
         deste projeto são escritas."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "limit": {
                    "type": "integer",
                    "description": "Quantos commits trazer (1 a 200; padrão 20).",
                    "minimum": 1,
                    "maximum": 200
                },
                "path": {
                    "type": "string",
                    "description": "Só os commits que tocaram este arquivo ou pasta, relativo à raiz do projeto. Omita para ver tudo."
                }
            },
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Read
    }

    fn within_workspace(&self, args: &Value, ctx: &ToolContext) -> bool {
        path_is_inside(args, ctx)
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        repo_root(ctx).await?;
        let limit = arg_u64(&args, "limit", DEFAULT_LOG_LIMIT).clamp(1, MAX_LOG_LIMIT);
        let rel = relative_path(&args, ctx)?;

        let count = format!("--max-count={limit}");
        let mut cmd: Vec<&str> = vec![
            "log",
            "--no-color",
            &count,
            "--date=relative",
            "--pretty=format:%h\u{1f}%an\u{1f}%ad\u{1f}%s",
        ];
        if let Some(path) = &rel {
            cmd.push("--");
            cmd.push(path);
        }

        let run = git(ctx, &cmd).await?;
        if !run.ok() {
            if !has_commits(ctx).await {
                return Ok(ToolOutput::text(
                    "Este repositório ainda não tem nenhum commit. Prepare arquivos com \
                     `git_add` e grave o primeiro com `git_commit`.",
                ));
            }
            return Err(command_failed("ler o histórico", &run));
        }

        let commits = parse_log(&run.stdout);
        if commits.is_empty() {
            let alvo = rel.map(|p| format!(" para `{p}`")).unwrap_or_default();
            return Ok(ToolOutput::text(format!(
                "Nenhum commit encontrado{alvo}. Confira o caminho com `fs_list` — pode ser um \
                 arquivo que nunca foi commitado."
            )));
        }

        let mut body = format!(
            "{} commit(s), do mais recente para o mais antigo:\n",
            commits.len()
        );
        body.push_str(&render_log(&commits));
        Ok(ToolOutput::text(body).truncated_to(ctx.max_output_bytes))
    }
}

// ---------------------------------------------------------------- branch ---

/// Ferramenta `git_branch`.
pub struct GitBranch;

/// Uma branch da lista.
#[derive(Debug, PartialEq, Eq)]
pub struct Branch {
    pub current: bool,
    pub name: String,
    pub upstream: Option<String>,
    pub hash: String,
}

impl Branch {
    /// A entrada que o git cria quando o `HEAD` não está em branch nenhuma.
    pub fn is_detached(&self) -> bool {
        self.name.starts_with("(HEAD")
    }
}

/// Interpreta `--format=%(HEAD)%09%(refname:short)%09%(upstream:short)%09%(objectname:short)`.
pub fn parse_branches(text: &str) -> Vec<Branch> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let head = fields.next()?;
            let name = fields.next()?.to_string();
            let upstream = fields.next().unwrap_or("").to_string();
            let hash = fields.next().unwrap_or("").to_string();
            Some(Branch {
                current: head.trim() == "*",
                name,
                upstream: (!upstream.is_empty()).then_some(upstream),
                hash,
            })
        })
        .collect()
}

/// Lista de branches em texto, com a atual marcada.
pub fn render_branches(branches: &[Branch]) -> String {
    let atual = branches.iter().find(|b| b.current);
    let mut out = match atual {
        Some(b) if b.is_detached() => {
            format!("Branch atual: nenhuma — o HEAD está solto em {}.\n", b.hash)
        }
        Some(b) => format!("Branch atual: {}\n", b.name),
        None => "Branch atual: não identificada.\n".to_string(),
    };

    let width = branches
        .iter()
        .map(|b| b.name.chars().count())
        .max()
        .unwrap_or(10)
        .min(40);

    out.push_str(&format!("\nBranches ({}):\n", branches.len()));
    for b in branches.iter().take(MAX_BRANCHES) {
        let marca = if b.current { "*" } else { " " };
        let upstream = b
            .upstream
            .as_ref()
            .map(|u| format!("  -> {u}"))
            .unwrap_or_default();
        out.push_str(&format!(
            "{marca} {}  {}{upstream}\n",
            pad(&b.name, width),
            b.hash
        ));
    }
    if branches.len() > MAX_BRANCHES {
        out.push_str(&format!(
            "  ... e mais {} branch(es)\n",
            branches.len() - MAX_BRANCHES
        ));
    }
    out
}

#[async_trait]
impl Tool for GitBranch {
    fn name(&self) -> &str {
        "git_branch"
    }

    fn description(&self) -> &str {
        "Lista as branches locais do projeto, marcando qual é a atual e para onde cada uma envia \
         (upstream). Use para saber onde você está antes de commitar."
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

    async fn execute(&self, _args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        repo_root(ctx).await?;
        let run = git(
            ctx,
            &[
                "branch",
                "--no-color",
                "--format=%(HEAD)%09%(refname:short)%09%(upstream:short)%09%(objectname:short)",
            ],
        )
        .await?;
        if !run.ok() {
            return Err(command_failed("listar as branches", &run));
        }

        let branches = parse_branches(&run.stdout);
        if branches.is_empty() {
            // Repositório novo: a branch existe como referência simbólica mas
            // ainda não aponta para commit nenhum, então não aparece na lista.
            let futura = git(ctx, &["symbolic-ref", "--short", "HEAD"]).await?;
            let nome = futura.stdout.trim();
            let onde = if futura.ok() && !nome.is_empty() {
                format!(" A branch `{nome}` passa a existir no primeiro commit.")
            } else {
                String::new()
            };
            return Ok(ToolOutput::text(format!(
                "Ainda não há branches porque o repositório não tem commits.{onde}"
            )));
        }

        Ok(ToolOutput::text(render_branches(&branches)).truncated_to(ctx.max_output_bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::repo_or_skip;

    #[test]
    fn pad_fits_and_truncates_by_characters() {
        assert_eq!(pad("abc", 5), "abc  ");
        assert_eq!(pad("abcdef", 4), "abc…");
        // Acento é um caractere, não dois: a coluna não pode desalinhar.
        assert_eq!(pad("ção", 4), "ção ");
    }

    #[test]
    fn parse_log_splits_fields_with_spaces_inside() {
        let text = "abc1234\u{1f}Maria da Silva\u{1f}2 dias atrás\u{1f}corrige o login: parte 2\n";
        let commits = parse_log(text);
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].hash, "abc1234");
        assert_eq!(commits[0].author, "Maria da Silva");
        assert_eq!(commits[0].when, "2 dias atrás");
        assert_eq!(commits[0].subject, "corrige o login: parte 2");
    }

    #[test]
    fn render_log_aligns_the_columns() {
        let commits = parse_log(
            "abc1234\u{1f}Ana\u{1f}1 hora atrás\u{1f}primeiro\n\
             de\u{1f}Bernardo\u{1f}3 semanas atrás\u{1f}segundo\n",
        );
        let text = render_log(&commits);
        let linhas: Vec<&str> = text.lines().collect();
        assert_eq!(linhas.len(), 2);
        // Hashes de tamanhos diferentes, mas a data começa na mesma coluna.
        assert_eq!(
            linhas[0].find("1 hora").unwrap(),
            linhas[1].find("3 semanas").unwrap()
        );
    }

    #[test]
    fn parse_branches_marks_the_current_one() {
        let text = "*\tmain\torigin/main\tabc1234\n \toutra\t\tdef5678\n";
        let branches = parse_branches(text);
        assert_eq!(branches.len(), 2);
        assert!(branches[0].current);
        assert_eq!(branches[0].upstream.as_deref(), Some("origin/main"));
        assert!(!branches[1].current);
        assert!(branches[1].upstream.is_none());
        assert!(render_branches(&branches).contains("Branch atual: main"));
    }

    #[test]
    fn a_detached_head_is_named_as_such() {
        let text = "*\t(HEAD detached at abc1234)\t\tabc1234\n \tmain\t\tabc1234\n";
        let branches = parse_branches(text);
        assert!(branches[0].is_detached());
        assert!(render_branches(&branches).contains("HEAD está solto"));
    }

    #[tokio::test]
    async fn diff_shows_the_changed_lines() {
        let repo = repo_or_skip!();
        repo.write("src/app.txt", "alfa\nBETA\ngama\n");
        let out = GitDiff.execute(json!({}), &repo.ctx).await.unwrap();
        assert!(out.content.contains("-beta"), "{}", out.content);
        assert!(out.content.contains("+BETA"), "{}", out.content);
        assert!(out.content.contains("src/app.txt"), "{}", out.content);
    }

    #[tokio::test]
    async fn diff_without_changes_suggests_the_next_step() {
        let repo = repo_or_skip!();
        let out = GitDiff.execute(json!({}), &repo.ctx).await.unwrap();
        assert!(out.content.contains("Nenhuma diferença"), "{}", out.content);
        assert!(out.content.contains("staged"), "{}", out.content);
    }

    #[tokio::test]
    async fn diff_staged_shows_only_what_is_prepared() {
        let repo = repo_or_skip!();
        repo.write("src/app.txt", "alfa\nBETA\ngama\n");
        repo.write("README.md", "# Projeto\n\noutra linha\n");
        repo.git(&["add", "src/app.txt"]).await;

        let staged = GitDiff
            .execute(json!({"staged": true}), &repo.ctx)
            .await
            .unwrap();
        assert!(staged.content.contains("src/app.txt"), "{}", staged.content);
        assert!(!staged.content.contains("README.md"), "{}", staged.content);
    }

    #[tokio::test]
    async fn diff_can_be_limited_to_one_path_and_refuses_escapes() {
        let repo = repo_or_skip!();
        repo.write("src/app.txt", "alfa\nBETA\ngama\n");
        repo.write("README.md", "# Projeto\n\noutra linha\n");

        let out = GitDiff
            .execute(json!({"path": "src/app.txt"}), &repo.ctx)
            .await
            .unwrap();
        assert!(out.content.contains("src/app.txt"), "{}", out.content);
        assert!(!out.content.contains("README.md"), "{}", out.content);

        let args = json!({"path": "../fora.txt"});
        assert!(!GitDiff.within_workspace(&args, &repo.ctx));
        let err = GitDiff.execute(args, &repo.ctx).await.unwrap_err();
        assert!(err.to_model_message().contains("fora da pasta"), "{err:?}");
    }

    #[tokio::test]
    async fn diff_preview_carries_the_unified_text() {
        let repo = repo_or_skip!();
        repo.write("src/app.txt", "alfa\nBETA\ngama\n");
        let preview = GitDiff
            .preview(&json!({"path": "src/app.txt"}), &repo.ctx)
            .await
            .expect("prévia");
        match preview {
            ToolPreview::Diff { path, unified, .. } => {
                assert_eq!(path, "src/app.txt");
                assert!(unified.contains("+BETA"), "{unified}");
            }
            other => panic!("esperava um diff, veio {other:?}"),
        }
    }

    #[tokio::test]
    async fn log_lists_commits_newest_first_and_respects_the_limit() {
        let repo = repo_or_skip!();
        repo.write("b.txt", "b\n");
        repo.git(&["add", "b.txt"]).await;
        repo.git(&["commit", "-m", "segundo commit"]).await;

        let out = GitLog.execute(json!({}), &repo.ctx).await.unwrap();
        let corpo = &out.content;
        assert!(corpo.contains("segundo commit"), "{corpo}");
        assert!(corpo.contains("primeiro commit"), "{corpo}");
        let pos_segundo = corpo.find("segundo commit").unwrap();
        let pos_primeiro = corpo.find("primeiro commit").unwrap();
        assert!(pos_segundo < pos_primeiro, "o mais novo vem antes");

        let um = GitLog
            .execute(json!({"limit": 1}), &repo.ctx)
            .await
            .unwrap();
        assert!(um.content.contains("segundo commit"), "{}", um.content);
        assert!(!um.content.contains("primeiro commit"), "{}", um.content);
    }

    #[tokio::test]
    async fn log_can_follow_a_single_path() {
        let repo = repo_or_skip!();
        repo.write("b.txt", "b\n");
        repo.git(&["add", "b.txt"]).await;
        repo.git(&["commit", "-m", "so o b"]).await;

        let out = GitLog
            .execute(json!({"path": "b.txt"}), &repo.ctx)
            .await
            .unwrap();
        assert!(out.content.contains("so o b"), "{}", out.content);
        assert!(!out.content.contains("primeiro commit"), "{}", out.content);
    }

    #[tokio::test]
    async fn log_on_a_repository_without_commits_explains_instead_of_failing() {
        if !crate::fixture::git_available().await {
            eprintln!("AVISO: git não encontrado nesta máquina — teste pulado.");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolContext::new(Some(dir.path().to_path_buf()), "c1");
        crate::run::run_in(crate::run::GIT, dir.path().to_path_buf(), &["init"], 8_192)
            .await
            .unwrap();

        let out = GitLog.execute(json!({}), &ctx).await.unwrap();
        assert!(
            out.content.contains("não tem nenhum commit"),
            "{}",
            out.content
        );
        assert!(out.content.contains("git_commit"), "{}", out.content);
    }

    #[tokio::test]
    async fn branch_lists_and_marks_the_current_one() {
        let repo = repo_or_skip!();
        repo.git(&["branch", "experimento"]).await;
        let atual = repo.current_branch().await;

        let out = GitBranch.execute(json!({}), &repo.ctx).await.unwrap();
        assert!(
            out.content.contains(&format!("Branch atual: {atual}")),
            "{}",
            out.content
        );
        assert!(out.content.contains("experimento"), "{}", out.content);
        assert!(out.content.contains("Branches (2)"), "{}", out.content);
    }
}
