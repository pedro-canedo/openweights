//! Ponte com o `spawner` do `lr_tools`.
//!
//! Todo comando destas ferramentas passa por aqui, e por um motivo: o
//! `spawner` é quem sabe matar a árvore inteira quando o tempo estoura (Job
//! Object no Windows, grupo de processos no Unix). Um `npm test` abandonado
//! continua queimando CPU depois que o agente já seguiu em frente — usar
//! `std::process` direto reintroduziria exatamente esse problema.
//!
//! Duas coisas que este módulo acrescenta ao `spawner`:
//!
//! **Programa que não existe vira instrução.** `Erro: No such file or
//! directory (os error 2)` não ajuda ninguém. O modelo precisa ler "o `go`
//! não está instalado; instale em go.dev/dl **ou** use outra ferramenta" para
//! decidir o próximo passo sozinho.
//!
//! **Plano B de nome.** Em muita máquina Linux só existe `python`, em outra só
//! `python3`; o `npx` some quando o Node foi instalado sem npm. Tentar o
//! irmão antes de desistir evita um erro que não é do projeto, é da máquina.

use crate::detect::Cmd;
use lr_tools::spawner::{self, SpawnOutcome, SpawnRequest};
use lr_tools::{ToolError, ToolResult};
use std::io::ErrorKind;
use std::path::Path;

/// Tempo padrão de um comando de projeto (teste/compilação demoram).
pub const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// Teto: acima disso não é passo de agente, é trabalho de servidor.
pub const MAX_TIMEOUT_SECS: u64 = 1_800;

/// Tempo do modo conferência do formatador chamado dentro da prévia.
pub const PREVIEW_TIMEOUT_SECS: u64 = 60;

/// Quanto capturamos do processo antes de resumir.
///
/// Bem maior que o orçamento do modelo, e de propósito. O spawner guarda o
/// **começo** da saída e descarta o resto — mas quase tudo que interessa num
/// log de teste ou de compilação está no **fim**: `test result: …`, os blocos
/// de falha, o `error: could not compile`. Capturar pouco economizaria memória
/// e jogaria fora justamente a parte que vira o resumo. Dois megabytes cobrem
/// praticamente qualquer suíte e somem da memória assim que a chamada termina.
pub const CAPTURE_BYTES: usize = 2 * 1024 * 1024;

/// Resultado de um comando: o que rodou e o que saiu.
#[derive(Debug, Clone)]
pub struct Run {
    pub cmd: Cmd,
    pub outcome: SpawnOutcome,
}

impl Run {
    pub fn success(&self) -> bool {
        self.outcome.success()
    }

    /// Cabeçalho comum a todos os resumos: comando, código de saída e avisos.
    pub fn header(&self) -> String {
        let mut head = self.cmd.display();
        match self.outcome.exit_code {
            Some(0) => head.push_str(" — ok (código 0)"),
            Some(code) => head.push_str(&format!(" — falhou (código {code})")),
            None => head.push_str(" — encerrado sem código de saída"),
        }
        if self.outcome.timed_out {
            head.push_str(
                "\nINTERROMPIDO: passou do tempo limite e foi morto. O resumo abaixo é \
                           só do que saiu até ali.",
            );
        }
        if self.outcome.truncated {
            // Explica de antemão uma contagem que não aparecer: o rodapé com
            // os totais é a última coisa que o runner escreve.
            head.push_str(
                "\nAVISO: a saída foi longa demais e o FIM dela se perdeu — o rodapé com os \
                 totais pode não estar no resumo. Rode com filtro para diminuir.",
            );
        }
        head
    }
}

/// Roda o comando na pasta indicada.
///
/// Só devolve `Err` quando o programa não pôde nem começar. Programa que roda
/// e falha (teste vermelho, compilação quebrada) é `Ok`: o interessante está
/// na saída, e é isso que o agente precisa ler para se corrigir.
///
/// `on_output` recebe a saída em pedaços enquanto o comando roda — é o que
/// alimenta o terminal da sessão. Tentativa que nem começa (o plano B do
/// `candidates`) nunca emite nada, então o painel não vê nome errado.
pub async fn run(
    cmd: &Cmd,
    cwd: &Path,
    timeout_secs: u64,
    max_output_bytes: usize,
    mut on_output: impl FnMut(&str) + Send,
) -> ToolResult<Run> {
    if cmd.argv.is_empty() {
        return Err(ToolError::Other("comando vazio".into()));
    }
    let timeout = timeout_secs.clamp(1, MAX_TIMEOUT_SECS);
    let mut last_error = None;

    for program in candidates(cmd.program()) {
        let request = SpawnRequest::new(program.clone(), cmd.args().to_vec(), cwd.to_path_buf())
            .with_timeout(timeout)
            .with_max_output(max_output_bytes.max(2_048));

        match spawner::run(request, &mut on_output).await {
            Ok(outcome) => {
                // Se o plano B foi usado, o resumo mostra o nome que rodou de
                // verdade — senão a pessoa procura um erro em outro lugar.
                let mut ran = cmd.clone();
                ran.argv[0] = program;
                return Ok(Run { cmd: ran, outcome });
            }
            Err(e) if e.kind() == ErrorKind::NotFound => last_error = Some(e),
            Err(e) => {
                return Err(ToolError::Other(format!(
                    "não foi possível iniciar `{program}` em {}: {e}. Confira a pasta e as \
                     permissões.",
                    cwd.display()
                )));
            }
        }
    }

    log::debug!("programa ausente: {:?}", last_error);
    Err(ToolError::Other(missing_program_message(cmd.program())))
}

/// Nomes a tentar para um programa, do preferido ao plano B.
///
/// Só entram aqui nomes que aceitam **os mesmos argumentos** — trocar `gofmt`
/// por `go`, por exemplo, exigiria reescrever a linha inteira e por isso fica
/// de fora: melhor um erro claro de "não instalado" do que um comando errado.
fn candidates(program: &str) -> Vec<String> {
    let extra: &[&str] = match program {
        "python3" => &["python", "py"],
        "python" => &["python3", "py"],
        _ => &[],
    };
    let mut names = vec![program.to_string()];
    names.extend(extra.iter().map(|s| s.to_string()));
    names
}

/// Mensagem de "não está instalado" que diz o que fazer em seguida.
pub fn missing_program_message(program: &str) -> String {
    format!(
        "`{program}` não foi encontrado no PATH — {}. Enquanto isso, use `terminal_run` com outro \
         comando ou peça ao usuário para instalar.",
        install_hint(program)
    )
}

/// Onde se instala cada ferramenta (texto curto, direto ao ponto).
fn install_hint(program: &str) -> &'static str {
    match program {
        "cargo" | "rustc" | "rustfmt" | "clippy-driver" => "instale o Rust em https://rustup.rs",
        "node" | "npm" | "npx" => "instale o Node.js em https://nodejs.org",
        "pnpm" => "instale com `npm install -g pnpm`",
        "yarn" => "instale com `npm install -g yarn`",
        "bun" | "bunx" => "instale o Bun em https://bun.sh",
        "python" | "python3" | "py" => "instale o Python em https://python.org/downloads",
        "uv" => "instale o uv em https://docs.astral.sh/uv",
        "poetry" => "instale com `pipx install poetry`",
        "pipenv" => "instale com `pip install pipenv`",
        "go" | "gofmt" => "instale o Go em https://go.dev/dl",
        "golangci-lint" => "instale em https://golangci-lint.run",
        "mvn" => "instale o Maven em https://maven.apache.org",
        "gradle" => "instale o Gradle em https://gradle.org",
        "dotnet" => "instale o .NET SDK em https://dotnet.microsoft.com/download",
        "composer" | "php" => "instale o PHP e o Composer em https://getcomposer.org",
        "bundle" | "ruby" => "instale o Ruby e o Bundler em https://ruby-lang.org",
        _ => "confira se ele está instalado e visível no PATH",
    }
}

/// Reconhece "o módulo não está instalado" e devolve o conserto.
///
/// `python -m pytest` numa máquina sem pytest sai com código 1 e uma linha só:
/// `No module named pytest`. Sem tradução, o modelo lê isso como "os testes
/// falharam" e vai caçar bug onde não tem.
pub fn missing_module_note(text: &str) -> Option<String> {
    let marker = "No module named ";
    let start = text.find(marker)? + marker.len();
    let name: String = text[start..]
        .trim_start_matches(['\'', '"'])
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
        .collect();
    if name.is_empty() {
        return None;
    }
    let base = name.split('.').next().unwrap_or(&name).to_string();
    Some(format!(
        "O módulo Python `{base}` não está instalado neste ambiente — instale com \
         `pip install {base}` (ou `uv add --dev {base}`) e rode de novo. Nada foi testado."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::Cmd;

    fn tmp() -> std::path::PathBuf {
        std::env::temp_dir()
    }

    #[test]
    fn missing_program_message_says_where_to_get_it() {
        let msg = missing_program_message("cargo");
        assert!(msg.contains("cargo"), "{msg}");
        assert!(msg.contains("rustup.rs"), "{msg}");
        assert!(
            msg.contains("terminal_run"),
            "deve sugerir um próximo passo"
        );

        let unknown = missing_program_message("ferramenta-exotica");
        assert!(unknown.contains("PATH"), "{unknown}");
    }

    #[test]
    fn the_header_warns_when_the_end_of_the_output_was_lost() {
        let run = Run {
            cmd: Cmd::new(["cargo", "test"]),
            outcome: SpawnOutcome {
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
                truncated: true,
                timed_out: false,
            },
        };
        let head = run.header();
        assert!(head.starts_with("cargo test — ok"), "{head}");
        // Sem este aviso, uma contagem ausente pareceria "nenhum teste".
        assert!(head.contains("FIM dela se perdeu"), "{head}");
    }

    #[test]
    fn python_has_a_fallback_name_but_gofmt_does_not() {
        assert_eq!(candidates("python3"), vec!["python3", "python", "py"]);
        assert_eq!(candidates("gofmt"), vec!["gofmt"]);
        assert_eq!(candidates("cargo"), vec!["cargo"]);
    }

    #[test]
    fn missing_module_note_extracts_the_package_name() {
        let out = "/usr/bin/python3: No module named pytest\n";
        let note = missing_module_note(out).unwrap();
        assert!(note.contains("pip install pytest"), "{note}");
        assert!(note.contains("Nada foi testado"), "{note}");

        // Submódulo: sugerimos instalar o pacote de cima.
        let sub = missing_module_note("No module named 'ruff.__main__'").unwrap();
        assert!(sub.contains("pip install ruff"), "{sub}");

        assert!(missing_module_note("tudo certo").is_none());
    }

    #[tokio::test]
    async fn a_program_that_does_not_exist_becomes_an_actionable_error() {
        let cmd = Cmd::new(["programa-inexistente-xyz", "--versao"]);
        let err = run(&cmd, &tmp(), 20, 4_096, |_| {}).await.unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("programa-inexistente-xyz"), "{msg}");
        assert!(msg.contains("PATH"), "{msg}");
    }

    #[tokio::test]
    async fn a_failing_command_is_ok_with_the_exit_code_in_the_header() {
        // `cargo --versao` (com typo) existe como programa e falha como comando.
        if !crate::testing::program_exists("cargo") {
            eprintln!("pulando: cargo não está instalado");
            return;
        }
        let run = run(
            &Cmd::new(["cargo", "--flag-que-nao-existe"]),
            &tmp(),
            60,
            8_192,
            |_| {},
        )
        .await
        .unwrap();
        assert!(!run.success());
        assert!(run.header().contains("falhou"), "{}", run.header());
    }

    #[tokio::test]
    async fn timeout_is_reported_in_the_header_instead_of_erroring() {
        if !crate::testing::program_exists("node") {
            eprintln!("pulando: node não está instalado");
            return;
        }
        let cmd = Cmd::new(["node", "-e", "setTimeout(()=>{}, 60000)"]);
        let run = run(&cmd, &tmp(), 1, 4_096, |_| {}).await.unwrap();
        assert!(run.outcome.timed_out);
        assert!(run.header().contains("INTERROMPIDO"), "{}", run.header());
    }
}
