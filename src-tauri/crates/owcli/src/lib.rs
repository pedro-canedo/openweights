//! O OwCLI no app: o runtime pré-compilado do fork do Codex CLI
//! (pedro-canedo/owcli), instalado com verificação.
//!
//! - [`pins`]: o runtime que este binário aceita (sha256 e tamanho embutidos);
//! - [`install`]: download, verificação, identidade, instalação atômica e poda;
//! - [`noterminal`]: o `owcli` no terminal do sistema (opt-in);
//! - [`uso`]: alguma sessão roda o executável de uma versão?
//!
//! O gateway, o `openweights.json` e as sessões ficam no app
//! (`commands_owcli`, `commands_terminal`); aqui só o pacote.

pub mod install;
pub mod noterminal;
pub mod pins;
pub mod uso;

pub use install::{ErroDeInstalacao, EventoDeInstalacao, Layout};
