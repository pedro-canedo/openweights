//! Como as ferramentas falam com o Git.
//!
//! **Chamamos o `git` do usuário, não uma biblioteca de Git em Rust.** O
//! repositório é dele: tem a configuração dele (`user.name`, `commit.gpgsign`,
//! `core.autocrlf`), os hooks dele e as credenciais dele. Uma reimplementação
//! em Rust ignoraria tudo isso e produziria commits que o próprio usuário não
//! reconheceria — pior, ignoraria um hook que existia justamente para impedir
//! um commit ruim. Rodar o binário dele é a única forma de o resultado ser
//! igual ao que ele teria feito à mão.
//!
//! Três decisões acompanham essa escolha:
//!
//! - **Sempre pelo [`lr_tools::spawner`]**, nunca `std::process`. É o spawner
//!   que garante o Job Object no Windows e o grupo de processos no Unix — um
//!   `git log` num repositório gigante que trave não pode ficar de herança.
//! - **Configuração forçada por chamada** ([`FORCED_CONFIG`]): o `-c` da linha
//!   de comando vence o arquivo de configuração, então desligamos cor e
//!   *quotepath* sem tocar em nada do usuário. Sem isso, quem tem
//!   `color.ui = always` receberia códigos ANSI no meio do resultado e quem
//!   tem acentos no nome do arquivo receberia `\303\247`.
//! - **"Não é um repositório" é detectado pelo código de saída**, não pelo
//!   texto do erro ([`repo_root`]). O Git é traduzido: procurar por
//!   "not a git repository" quebraria numa máquina em português. O código de
//!   saída de `git rev-parse --show-toplevel` não é traduzido.
//!
//! Versão mínima esperada: **Git 2.23** (por causa de `git restore`). As
//! demais chamadas funcionam desde a 2.14.

use lr_tools::spawner::{self, SpawnRequest};
use lr_tools::{ToolContext, ToolError, ToolResult};
use std::path::PathBuf;

/// Nome do programa.
///
/// É constante e não parâmetro porque o agente nunca escolhe outro binário;
/// os testes exercitam o caminho de "git ausente" chamando [`run_in`] com um
/// nome inexistente, que é exatamente o mesmo caminho de código.
pub const GIT: &str = "git";

/// Teto de tempo de qualquer chamada ao git.
///
/// Um minuto é generoso para `status`/`log`/`commit` mesmo em repositório
/// grande, e curto o bastante para o agente não ficar preso se o git parar
/// esperando (credencial, lock de outro processo).
pub const TIMEOUT_SECS: u64 = 60;

/// Espaço que reservamos no orçamento de saída para os rótulos que nós mesmos
/// escrevemos em volta do texto do git.
pub const OVERHEAD_BYTES: usize = 1_024;

/// Piso de saída: abaixo disso o resultado não diria nada de útil.
const MIN_BUDGET_BYTES: usize = 2_048;

/// Configuração aplicada a TODA chamada, antes do subcomando.
///
/// - `color.ui=false`: nada de ANSI no meio do texto que vai para o modelo.
/// - `core.quotepath=false`: acento em nome de arquivo sai como acento.
pub const FORCED_CONFIG: [&str; 4] = ["-c", "color.ui=false", "-c", "core.quotepath=false"];

/// Resultado bruto de uma chamada ao git.
///
/// Código de saída diferente de zero **não** é erro aqui: `git commit` sem
/// nada preparado sai com 1 e isso é uma informação, não uma falha. Quem
/// chama decide como traduzir.
#[derive(Debug, Clone)]
pub struct GitRun {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

impl GitRun {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }

    /// O que o git disse quando algo deu errado, já limpo.
    pub fn error_text(&self) -> String {
        for candidate in [self.stderr.trim(), self.stdout.trim()] {
            if !candidate.is_empty() {
                return candidate.to_string();
            }
        }
        "o git não explicou o motivo".to_string()
    }

    /// Saída combinada (o git escreve avisos úteis no stderr mesmo quando dá
    /// certo, como o "Switched to branch" ou o nome do stash criado).
    pub fn combined(&self) -> String {
        let mut out = String::new();
        for part in [self.stdout.trim_end(), self.stderr.trim_end()] {
            if part.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(part);
        }
        out
    }
}

/// Roda `program` com os argumentos dados na pasta `cwd`.
///
/// Só devolve `Err` quando o programa não pôde nem começar (não instalado,
/// sem permissão) ou quando estourou o tempo.
pub async fn run_in(
    program: &str,
    cwd: PathBuf,
    args: &[&str],
    max_output: usize,
) -> ToolResult<GitRun> {
    let full: Vec<String> = FORCED_CONFIG
        .iter()
        .chain(args.iter())
        .map(|s| (*s).to_string())
        .collect();

    let request = SpawnRequest::new(program, full, cwd)
        .with_timeout(TIMEOUT_SECS)
        .with_max_output(max_output.max(MIN_BUDGET_BYTES));

    let outcome = spawner::run(request, |_| {})
        .await
        .map_err(|e| start_error(program, &e))?;

    if outcome.timed_out {
        return Err(ToolError::Timeout(TIMEOUT_SECS));
    }

    Ok(GitRun {
        code: outcome.exit_code,
        stdout: outcome.stdout,
        stderr: outcome.stderr,
        truncated: outcome.truncated,
    })
}

/// Roda o git na pasta do projeto, com o orçamento de saída do contexto.
pub async fn git(ctx: &ToolContext, args: &[&str]) -> ToolResult<GitRun> {
    let cwd = ctx.resolve(".")?;
    let budget = ctx
        .max_output_bytes
        .saturating_sub(OVERHEAD_BYTES)
        .max(MIN_BUDGET_BYTES);
    run_in(GIT, cwd, args, budget).await
}

/// Mensagem para quando o git não pôde ser iniciado.
///
/// Este é o erro mais provável em máquina de usuário comum — Windows costuma
/// vir sem git — então ele precisa dizer o que instalar e o que dá para fazer
/// enquanto isso.
fn start_error(program: &str, e: &std::io::Error) -> ToolError {
    if e.kind() == std::io::ErrorKind::NotFound {
        return ToolError::Other(format!(
            "O Git não está instalado ou não está no PATH deste computador (não encontrei \
             `{program}`). Instale em https://git-scm.com e reabra o aplicativo. Enquanto isso, \
             nenhuma ferramenta `git_*` vai funcionar — use as ferramentas de arquivo para ler e \
             editar o projeto."
        ));
    }
    ToolError::Other(format!(
        "Não consegui iniciar o `{program}` ({e}). Confira se o Git está instalado e se você tem \
         permissão para executá-lo."
    ))
}

/// Raiz do repositório que contém a pasta do projeto.
///
/// Serve de porta de entrada de toda ferramenta: confirma que o git existe e
/// que estamos dentro de um repositório antes de rodar o comando de verdade.
/// O custo é um processo a mais por chamada (~10 ms) e a troca vale: sem isso,
/// "pasta que não é repositório" chegaria ao modelo como o texto cru do git,
/// possivelmente traduzido, e ele tentaria de novo sem entender.
pub async fn repo_root(ctx: &ToolContext) -> ToolResult<PathBuf> {
    let run = git(ctx, &["rev-parse", "--show-toplevel"]).await?;
    let path = run.stdout.trim();
    if !run.ok() || path.is_empty() {
        return Err(not_a_repository());
    }
    Ok(PathBuf::from(path))
}

/// Erro de "aqui não tem repositório", com o próximo passo junto.
pub fn not_a_repository() -> ToolError {
    ToolError::Other(
        "A pasta do projeto não é um repositório Git (nem está dentro de um). Se quiser começar a \
         versionar, rode `git init` pelo terminal e faça o primeiro commit; sem repositório as \
         ferramentas `git_*` não têm o que ler nem o que gravar."
            .into(),
    )
}

/// Já existe algum commit? (`HEAD` aponta para alguma coisa.)
///
/// Repositório recém-criado faz `git log` e `git branch` falharem, e a
/// explicação certa não é "deu erro", é "ainda não há commits".
pub async fn has_commits(ctx: &ToolContext) -> bool {
    matches!(
        git(ctx, &["rev-parse", "--verify", "--quiet", "HEAD"]).await,
        Ok(run) if run.ok()
    )
}

/// Erro genérico de comando do git que falhou, preservando o texto original.
pub fn command_failed(what: &str, run: &GitRun) -> ToolError {
    ToolError::Other(format!(
        "O git recusou {what}: {}. Confira o estado com `git_status` antes de tentar de novo.",
        run.error_text()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        std::env::temp_dir()
    }

    #[test]
    fn missing_program_message_says_what_to_install() {
        let e = std::io::Error::new(std::io::ErrorKind::NotFound, "no such file");
        let msg = start_error("git", &e).to_model_message();
        assert!(msg.contains("não está instalado"), "{msg}");
        assert!(msg.contains("PATH"), "{msg}");
        assert!(msg.contains("git-scm.com"), "{msg}");
    }

    #[test]
    fn other_start_errors_are_not_confused_with_a_missing_git() {
        let e = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "negado");
        let msg = start_error("git", &e).to_model_message();
        assert!(msg.contains("permissão"), "{msg}");
        assert!(!msg.contains("git-scm.com"), "{msg}");
    }

    /// O caminho de "git não instalado" de ponta a ponta: mesmo código, só
    /// com um nome de programa que garantidamente não existe.
    #[tokio::test]
    async fn a_missing_git_binary_produces_the_install_message() {
        let err = run_in("git-que-nao-existe-xyz", tmp(), &["status"], 4_096)
            .await
            .unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("não está instalado"), "{msg}");
        assert!(msg.contains("git-que-nao-existe-xyz"), "{msg}");
    }

    #[test]
    fn not_a_repository_points_to_git_init() {
        let msg = not_a_repository().to_model_message();
        assert!(msg.contains("não é um repositório"), "{msg}");
        assert!(msg.contains("git init"), "{msg}");
    }

    #[test]
    fn error_text_prefers_stderr_but_falls_back() {
        let run = GitRun {
            code: Some(1),
            stdout: "saída".into(),
            stderr: "  motivo  ".into(),
            truncated: false,
        };
        assert_eq!(run.error_text(), "motivo");

        let run = GitRun {
            stderr: "   ".into(),
            ..run
        };
        assert_eq!(run.error_text(), "saída");

        let run = GitRun {
            stdout: String::new(),
            ..run
        };
        assert!(run.error_text().contains("não explicou"));
    }

    #[test]
    fn combined_joins_both_streams_without_blank_lines() {
        let run = GitRun {
            code: Some(0),
            stdout: "um\n".into(),
            stderr: "dois\n".into(),
            truncated: false,
        };
        assert_eq!(run.combined(), "um\ndois");
    }

    #[test]
    fn forced_config_disables_color_and_quotepath() {
        // Se alguém mexer nisso sem querer, o resultado enche de ANSI e de
        // escapes octais — e o modelo passa a ler lixo.
        assert!(FORCED_CONFIG.contains(&"color.ui=false"));
        assert!(FORCED_CONFIG.contains(&"core.quotepath=false"));
    }
}
