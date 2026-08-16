//! Ferramentas de Git do agente.
//!
//! O agente trabalha em projeto versionado. Sem Git ele não sabe o que já
//! estava mexido antes de ele chegar, não consegue mostrar o que fez e não tem
//! como desfazer com segurança — fica reduzido a ler e escrever arquivos no
//! escuro. Estas oito ferramentas fecham esse buraco:
//!
//! | Ferramenta    | Categoria | Para quê |
//! |---------------|-----------|----------|
//! | `git_status`  | Read      | o que mudou, em que branch, o que falta enviar |
//! | `git_diff`    | Read      | as diferenças, em formato unificado |
//! | `git_log`     | Read      | o histórico recente, compacto |
//! | `git_branch`  | Read      | onde estou e o que mais existe |
//! | `git_add`     | Edit      | preparar arquivos |
//! | `git_commit`  | Edit      | gravar no histórico |
//! | `git_restore` | Edit      | desfazer um arquivo |
//! | `git_stash`   | Edit      | guardar/recuperar trabalho |
//!
//! **Sempre o `git` do usuário**, chamado pelo [`lr_tools::spawner`] — o
//! porquê está em [`run`]. **Nada de rede nesta fase**: `push`, `pull` e
//! `fetch` ficaram de fora de propósito. São as únicas operações que saem da
//! máquina; merecem [`ToolCategory::Network`], prévia dizendo para qual
//! servidor, e uma conversa sobre credenciais que não cabe aqui.
//!
//! [`ToolCategory::Network`]: lr_types::agent::ToolCategory::Network

pub mod change;
#[cfg(test)]
mod fixture;
pub mod inspect;
pub mod run;
pub mod status;

pub use change::{GitAdd, GitCommit, GitRestore, GitStash};
pub use inspect::{GitBranch, GitDiff, GitLog};
pub use status::GitStatus;

use lr_tools::SharedTool;
use std::sync::Arc;

/// Todas as ferramentas de Git, prontas para o registro do agente.
pub fn git_tools() -> Vec<SharedTool> {
    vec![
        Arc::new(GitStatus),
        Arc::new(GitDiff),
        Arc::new(GitLog),
        Arc::new(GitBranch),
        Arc::new(GitAdd),
        Arc::new(GitCommit),
        Arc::new(GitRestore),
        Arc::new(GitStash),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_types::agent::{ToolCategory, ToolTier};

    #[test]
    fn the_catalog_has_the_eight_tools() {
        let mut nomes: Vec<String> = git_tools().iter().map(|t| t.name().to_string()).collect();
        nomes.sort();
        assert_eq!(
            nomes,
            vec![
                "git_add",
                "git_branch",
                "git_commit",
                "git_diff",
                "git_log",
                "git_restore",
                "git_stash",
                "git_status",
            ]
        );
    }

    /// A categoria é o que decide se a pessoa é interrompida ou não. Ler é
    /// livre dentro do projeto; gravar sempre passa pela confirmação.
    #[test]
    fn reading_is_free_and_writing_asks() {
        for tool in git_tools() {
            let esperada = match tool.name() {
                "git_status" | "git_diff" | "git_log" | "git_branch" => ToolCategory::Read,
                _ => ToolCategory::Edit,
            };
            assert_eq!(tool.category(), esperada, "categoria de {}", tool.name());

            let spec = tool.spec();
            assert_eq!(
                spec.read_only,
                esperada == ToolCategory::Read,
                "{}",
                tool.name()
            );
            assert_eq!(
                spec.tier,
                if esperada == ToolCategory::Read {
                    ToolTier::Safe
                } else {
                    ToolTier::Caution
                },
                "tier de {}",
                tool.name()
            );
        }
    }

    /// Nenhuma ferramenta desta fase pode ser de rede: `push`/`pull` ficaram
    /// de fora justamente por isso.
    #[test]
    fn nothing_here_touches_the_network() {
        for tool in git_tools() {
            assert_ne!(tool.category(), ToolCategory::Network, "{}", tool.name());
        }
    }

    /// O modelo escolhe a ferramenta lendo isto; schema sem descrição em cada
    /// campo é schema que ele preenche errado.
    #[test]
    fn every_parameter_is_described_in_portuguese() {
        for tool in git_tools() {
            let params = tool.parameters();
            assert_eq!(params["type"], "object", "{}", tool.name());
            let props = params["properties"].as_object().expect("properties");
            for (nome, campo) in props {
                let desc = campo["description"].as_str().unwrap_or("");
                assert!(
                    desc.len() > 20,
                    "`{}` de `{}` sem descrição útil",
                    nome,
                    tool.name()
                );
            }
            assert!(
                tool.description().len() > 60,
                "descrição curta demais em {}",
                tool.name()
            );
        }
    }
}
