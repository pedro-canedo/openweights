//! `git_status`: o que mudou desde o último commit.
//!
//! Lemos `--porcelain=v2`, e não a saída normal do `git status`, por três
//! motivos: o formato v2 é estável entre versões (a saída humana muda e é
//! traduzida), separa explicitamente o que está preparado do que não está, e
//! traz a linha `# branch.ab` com o quanto a branch está à frente/atrás — que
//! é como respondemos "há commits não enviados?" sem tocar na rede.
//!
//! Passamos `--untracked-files=all` de propósito: quem configurou
//! `status.showUntrackedFiles=no` no global não pode fazer o agente achar que
//! um arquivo novo não existe. E paramos em `--untracked-files=all` sem
//! `--ignored`: arquivo ignorado é ruído (uma `node_modules` inteira).

use crate::run::{git, repo_root};
use async_trait::async_trait;
use lr_tools::{Tool, ToolContext, ToolOutput, ToolResult};
use lr_types::agent::ToolCategory;
use serde_json::{Value, json};

/// Máximo de arquivos listados por seção. Acima disso o que importa é o
/// número, não a lista — e a lista comeria o contexto do modelo.
const MAX_PER_SECTION: usize = 60;

/// Tipo de mudança em um arquivo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
}

impl Change {
    /// Rótulo em português, largura fixa para as listas ficarem alinhadas.
    fn label(self) -> &'static str {
        match self {
            Change::Modified => "modificado",
            Change::Added => "novo      ",
            Change::Deleted => "apagado   ",
            Change::Renamed => "renomeado ",
            Change::Copied => "copiado   ",
            Change::TypeChanged => "tipo mudou",
        }
    }

    /// Código do porcelain v2 (`M`, `A`, `D`, `R`, `C`, `T`); `.` = sem
    /// mudança naquele lado.
    fn from_code(c: char) -> Option<Change> {
        match c {
            'M' => Some(Change::Modified),
            'A' => Some(Change::Added),
            'D' => Some(Change::Deleted),
            'R' => Some(Change::Renamed),
            'C' => Some(Change::Copied),
            'T' => Some(Change::TypeChanged),
            _ => None,
        }
    }
}

/// Um arquivo alterado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub kind: Change,
    pub path: String,
    /// Nome anterior, quando houve renomeação/cópia.
    pub from: Option<String>,
}

/// Estado do repositório já interpretado.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StatusReport {
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: i64,
    pub behind: i64,
    /// Repositório sem nenhum commit ainda.
    pub initial: bool,
    pub staged: Vec<Entry>,
    pub unstaged: Vec<Entry>,
    pub untracked: Vec<String>,
    pub conflicts: Vec<String>,
}

impl StatusReport {
    pub fn is_clean(&self) -> bool {
        self.staged.is_empty()
            && self.unstaged.is_empty()
            && self.untracked.is_empty()
            && self.conflicts.is_empty()
    }
}

/// Interpreta a saída de `git status --porcelain=v2 --branch`.
///
/// Formato (documentado em `git-status(1)`):
/// - `# branch.<campo> <valor>` — cabeçalhos;
/// - `1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <caminho>` — mudança simples;
/// - `2 <XY> ... <score> <novo>\t<antigo>` — renomeação/cópia;
/// - `u <XY> ... <caminho>` — conflito de merge;
/// - `? <caminho>` — não rastreado; `! <caminho>` — ignorado.
///
/// `X` é o estado preparado (índice vs. HEAD) e `Y` o não preparado (árvore
/// vs. índice); um arquivo pode aparecer nas duas listas, e isso é correto —
/// é o caso clássico de "editei de novo depois de dar `git add`".
pub fn parse_porcelain_v2(text: &str) -> StatusReport {
    let mut report = StatusReport::default();

    for line in text.lines() {
        if let Some(header) = line.strip_prefix("# ") {
            parse_header(&mut report, header);
            continue;
        }
        let Some((marker, rest)) = line.split_once(' ') else {
            continue;
        };
        match marker {
            "1" => {
                // marcador + 7 campos + caminho (que pode ter espaços).
                if let Some((xy, path)) = split_fields(rest, 7) {
                    push_entry(&mut report, xy, path, None);
                }
            }
            "2" => {
                // Um campo a mais (o score da renomeação) e o caminho vem
                // como `novo\tantigo`.
                if let Some((xy, tail)) = split_fields(rest, 8) {
                    let (path, from) = match tail.split_once('\t') {
                        Some((novo, antigo)) => (novo, Some(antigo.to_string())),
                        None => (tail, None),
                    };
                    push_entry(&mut report, xy, path, from);
                }
            }
            "u" => {
                if let Some((_, path)) = split_fields(rest, 9) {
                    report.conflicts.push(path.to_string());
                }
            }
            "?" => report.untracked.push(rest.to_string()),
            // `!` (ignorado) é ruído deliberado: não pedimos e não mostramos.
            _ => {}
        }
    }

    report
}

/// Separa `count` campos simples e devolve `(primeiro_campo, resto)`.
///
/// O resto é o caminho — que pode conter espaços, então não pode ser
/// dividido junto.
fn split_fields(rest: &str, count: usize) -> Option<(&str, &str)> {
    let parts: Vec<&str> = rest.splitn(count + 1, ' ').collect();
    // Linha curta demais é linha que não entendemos: melhor ignorar do que
    // inventar um caminho a partir de um campo qualquer.
    if parts.len() != count + 1 || parts[count].is_empty() {
        return None;
    }
    Some((parts[0], parts[count]))
}

fn parse_header(report: &mut StatusReport, header: &str) {
    let (field, value) = match header.split_once(' ') {
        Some(pair) => pair,
        None => (header, ""),
    };
    match field {
        "branch.oid" => report.initial = value == "(initial)",
        "branch.head" => {
            if value != "(detached)" && !value.is_empty() {
                report.branch = Some(value.to_string());
            }
        }
        "branch.upstream" if !value.is_empty() => report.upstream = Some(value.to_string()),
        "branch.ab" => {
            for token in value.split_whitespace() {
                match token.chars().next() {
                    Some('+') => report.ahead = token[1..].parse().unwrap_or(0),
                    Some('-') => report.behind = token[1..].parse().unwrap_or(0),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn push_entry(report: &mut StatusReport, xy: &str, path: &str, from: Option<String>) {
    let mut chars = xy.chars();
    let staged = chars.next().and_then(Change::from_code);
    let unstaged = chars.next().and_then(Change::from_code);

    if let Some(kind) = staged {
        report.staged.push(Entry {
            kind,
            path: path.to_string(),
            from: from.clone(),
        });
    }
    if let Some(kind) = unstaged {
        report.unstaged.push(Entry {
            kind,
            path: path.to_string(),
            // A renomeação é sempre um fato do índice; do lado da árvore o
            // que existe é a edição posterior, e ela não tem "nome antigo".
            from: None,
        });
    }
}

/// Texto final que o modelo lê.
pub fn render(report: &StatusReport) -> String {
    let mut out = String::new();

    out.push_str("Branch: ");
    out.push_str(
        report
            .branch
            .as_deref()
            .unwrap_or("(HEAD solta, sem branch)"),
    );
    out.push('\n');

    out.push_str("Envio: ");
    out.push_str(&sync_line(report));
    out.push('\n');

    section(&mut out, "Preparado para o commit", &report.staged);
    section(&mut out, "Alterado e ainda não preparado", &report.unstaged);
    plain_section(&mut out, "Não rastreado (arquivo novo)", &report.untracked);
    plain_section(
        &mut out,
        "EM CONFLITO (resolva antes de commitar)",
        &report.conflicts,
    );

    if report.is_clean() {
        out.push_str("\nNada para commitar: a árvore de trabalho está limpa.\n");
    }
    out
}

/// A frase sobre commits não enviados.
fn sync_line(report: &StatusReport) -> String {
    if report.initial {
        return "o repositório ainda não tem nenhum commit".into();
    }
    let Some(upstream) = &report.upstream else {
        return "esta branch não tem upstream configurado — nada foi enviado para um servidor"
            .into();
    };
    match (report.ahead, report.behind) {
        (0, 0) => format!("em dia com {upstream}"),
        (a, 0) => format!("{a} commit(s) SEU(S) ainda não enviado(s) para {upstream}"),
        (0, b) => format!("{b} commit(s) para receber de {upstream}"),
        (a, b) => format!("{a} commit(s) a enviar e {b} a receber de {upstream} (divergiu)"),
    }
}

fn section(out: &mut String, title: &str, entries: &[Entry]) {
    if entries.is_empty() {
        return;
    }
    out.push_str(&format!("\n{title} ({}):\n", entries.len()));
    for entry in entries.iter().take(MAX_PER_SECTION) {
        match &entry.from {
            Some(from) => out.push_str(&format!(
                "  {} {} (antes: {from})\n",
                entry.kind.label(),
                entry.path
            )),
            None => out.push_str(&format!("  {} {}\n", entry.kind.label(), entry.path)),
        }
    }
    omitted(out, entries.len());
}

fn plain_section(out: &mut String, title: &str, paths: &[String]) {
    if paths.is_empty() {
        return;
    }
    out.push_str(&format!("\n{title} ({}):\n", paths.len()));
    for path in paths.iter().take(MAX_PER_SECTION) {
        out.push_str(&format!("  {path}\n"));
    }
    omitted(out, paths.len());
}

fn omitted(out: &mut String, total: usize) {
    if total > MAX_PER_SECTION {
        out.push_str(&format!(
            "  ... e mais {} arquivo(s) não listado(s)\n",
            total - MAX_PER_SECTION
        ));
    }
}

/// Ferramenta `git_status`.
pub struct GitStatus;

#[async_trait]
impl Tool for GitStatus {
    fn name(&self) -> &str {
        "git_status"
    }

    fn description(&self) -> &str {
        "Mostra o estado do repositório Git do projeto: arquivos modificados, novos e apagados, \
         o que já está preparado para o commit, a branch atual e se há commits ainda não \
         enviados. Use antes de editar (para saber o que já estava mexido) e antes de commitar."
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
                "status",
                "--porcelain=v2",
                "--branch",
                "--untracked-files=all",
            ],
        )
        .await?;
        if !run.ok() {
            return Err(crate::run::command_failed("ler o estado", &run));
        }

        let report = parse_porcelain_v2(&run.stdout);
        let mut body = render(&report);
        if run.truncated {
            body.push_str("\n[a lista veio cortada: há mudanças demais para caber no resultado]\n");
        }
        Ok(ToolOutput::text(body).truncated_to(ctx.max_output_bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::repo_or_skip;

    /// Amostra real de `git status --porcelain=v2 --branch --untracked-files=all`.
    const SAMPLE: &str = "\
# branch.oid 1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d
# branch.head main
# branch.upstream origin/main
# branch.ab +2 -1
1 M. N... 100644 100644 100644 aaaa bbbb src/preparado.rs
1 .M N... 100644 100644 100644 cccc dddd src/editado.rs
1 A. N... 000000 100644 100644 0000 eeee src/novo.rs
1 .D N... 100644 100644 000000 ffff ffff src/apagado.rs
1 MM N... 100644 100644 100644 1111 2222 src/os dois lados.rs
2 R. N... 100644 100644 100644 3333 3333 R100 src/depois.rs\tsrc/antes.rs
u UU N... 100644 100644 100644 100644 4444 5555 6666 src/conflito.rs
? docs/rascunho.md
! alvo/ignorado.bin
";

    #[test]
    fn parses_branch_and_ahead_behind() {
        let r = parse_porcelain_v2(SAMPLE);
        assert_eq!(r.branch.as_deref(), Some("main"));
        assert_eq!(r.upstream.as_deref(), Some("origin/main"));
        assert_eq!(r.ahead, 2);
        assert_eq!(r.behind, 1);
        assert!(!r.initial);
    }

    #[test]
    fn splits_staged_from_unstaged() {
        let r = parse_porcelain_v2(SAMPLE);
        let staged: Vec<_> = r.staged.iter().map(|e| e.path.as_str()).collect();
        assert!(staged.contains(&"src/preparado.rs"));
        assert!(staged.contains(&"src/novo.rs"));
        assert!(!staged.contains(&"src/editado.rs"));

        let unstaged: Vec<_> = r.unstaged.iter().map(|e| e.path.as_str()).collect();
        assert!(unstaged.contains(&"src/editado.rs"));
        assert!(unstaged.contains(&"src/apagado.rs"));

        // Preparado E editado de novo: precisa aparecer dos dois lados.
        assert!(staged.contains(&"src/os dois lados.rs"));
        assert!(unstaged.contains(&"src/os dois lados.rs"));
    }

    #[test]
    fn understands_renames_deletions_and_untracked() {
        let r = parse_porcelain_v2(SAMPLE);
        let renamed = r
            .staged
            .iter()
            .find(|e| e.kind == Change::Renamed)
            .expect("renomeação");
        assert_eq!(renamed.path, "src/depois.rs");
        assert_eq!(renamed.from.as_deref(), Some("src/antes.rs"));

        assert!(r.unstaged.iter().any(|e| e.kind == Change::Deleted));
        assert_eq!(r.untracked, vec!["docs/rascunho.md"]);
        assert_eq!(r.conflicts, vec!["src/conflito.rs"]);
        // Arquivo ignorado (`!`) não entra em nenhuma lista.
        let todos: Vec<&str> = r
            .staged
            .iter()
            .chain(&r.unstaged)
            .map(|e| e.path.as_str())
            .chain(r.untracked.iter().map(String::as_str))
            .collect();
        assert!(!todos.iter().any(|p| p.contains("ignorado")), "{todos:?}");
    }

    #[test]
    fn malformed_lines_are_ignored_instead_of_breaking_the_parser() {
        let r = parse_porcelain_v2("1 M.\n2 R.\nu UU\n# branch.head main\nlixo\n");
        assert!(r.is_clean());
        assert_eq!(r.branch.as_deref(), Some("main"));
    }

    #[test]
    fn paths_with_spaces_survive() {
        let r = parse_porcelain_v2(SAMPLE);
        assert!(r.staged.iter().any(|e| e.path == "src/os dois lados.rs"));
    }

    #[test]
    fn a_fresh_repository_is_marked_as_initial() {
        let r = parse_porcelain_v2("# branch.oid (initial)\n# branch.head main\n");
        assert!(r.initial);
        assert!(r.is_clean());
        assert!(render(&r).contains("ainda não tem nenhum commit"));
    }

    #[test]
    fn render_reports_unpushed_commits() {
        let r = parse_porcelain_v2(SAMPLE);
        let text = render(&r);
        assert!(text.contains("Branch: main"), "{text}");
        // À frente e atrás ao mesmo tempo: a frase precisa dizer os dois lados.
        assert!(text.contains("2 commit(s) a enviar"), "{text}");
        assert!(text.contains("1 a receber de origin/main"), "{text}");
    }

    #[test]
    fn render_reports_only_unpushed_when_there_is_nothing_to_receive() {
        let r = parse_porcelain_v2(
            "# branch.oid abc\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +3 -0\n",
        );
        let text = render(&r);
        assert!(
            text.contains("3 commit(s) SEU(S) ainda não enviado(s)"),
            "{text}"
        );
    }

    #[test]
    fn render_says_when_there_is_no_upstream() {
        let r = parse_porcelain_v2("# branch.oid abc\n# branch.head main\n");
        assert!(render(&r).contains("não tem upstream"));
    }

    #[test]
    fn long_lists_are_cut_with_the_count() {
        let r = StatusReport {
            untracked: (0..MAX_PER_SECTION + 5)
                .map(|i| format!("arquivo{i}.txt"))
                .collect(),
            ..Default::default()
        };
        let text = render(&r);
        assert!(text.contains("e mais 5 arquivo(s)"), "{text}");
    }

    #[tokio::test]
    async fn a_clean_repository_says_so() {
        let repo = repo_or_skip!();
        let out = GitStatus.execute(json!({}), &repo.ctx).await.unwrap();
        assert!(
            out.content.contains("árvore de trabalho está limpa"),
            "{}",
            out.content
        );
        let branch = repo.current_branch().await;
        assert!(out.content.contains(&branch), "{}", out.content);
    }

    #[tokio::test]
    async fn reports_new_modified_and_deleted_files() {
        let repo = repo_or_skip!();
        repo.write("README.md", "# Projeto\n\nlinha nova\n");
        repo.write("docs/novo.md", "novo\n");
        repo.remove("src/app.txt");

        let out = GitStatus.execute(json!({}), &repo.ctx).await.unwrap();
        let text = &out.content;
        assert!(text.contains("modificado README.md"), "{text}");
        assert!(text.contains("apagado    src/app.txt"), "{text}");
        assert!(text.contains("docs/novo.md"), "{text}");
        assert!(text.contains("Não rastreado"), "{text}");
    }

    #[tokio::test]
    async fn staged_files_appear_in_their_own_section() {
        let repo = repo_or_skip!();
        repo.write("novo.txt", "conteudo\n");
        repo.git(&["add", "novo.txt"]).await;

        let out = GitStatus.execute(json!({}), &repo.ctx).await.unwrap();
        assert!(
            out.content.contains("Preparado para o commit"),
            "{}",
            out.content
        );
        assert!(out.content.contains("novo.txt"), "{}", out.content);
    }

    #[tokio::test]
    async fn outside_a_repository_the_error_explains_itself() {
        if !crate::fixture::git_available().await {
            eprintln!("AVISO: git não encontrado nesta máquina — teste pulado.");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolContext::new(Some(dir.path().to_path_buf()), "c1");
        let err = GitStatus.execute(json!({}), &ctx).await.unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("não é um repositório"), "{msg}");
        assert!(msg.contains("git init"), "{msg}");
    }

    #[tokio::test]
    async fn without_a_project_folder_there_is_nothing_to_inspect() {
        let ctx = ToolContext::new(None, "c1");
        assert!(GitStatus.execute(json!({}), &ctx).await.is_err());
    }
}
