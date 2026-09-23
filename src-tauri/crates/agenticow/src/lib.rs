//! O AgenticOw no app: o runtime pré-compilado do fork do DeepSeek Harness
//! (pedro-canedo/agenticow), instalado com verificação e supervisionado pelo
//! protocolo de controle.
//!
//! - [`catalog`]: a seção `llm-pi-ai` que o app entrega ao AgenticOw;
//! - [`pins`]: o runtime que este binário aceita (sha256 e tamanho embutidos);
//! - [`install`]: download, verificação, identidade e instalação atômica;
//! - [`migrate`]: o home da era do DeepSeek Harness copiado na primeira subida;
//! - [`host`]: o processo em execução e o protocolo de controle;
//! - [`protocol`]: as mensagens, os comandos e a redação do token.

pub mod catalog;
pub mod host;
pub mod install;
pub mod migrate;
pub mod pins;
pub mod protocol;

pub use host::{
    AgenticowHost, Config, ErroDoHost, EventoDoHost, OuvinteDoHost, PRAZO_PARA_SAIR,
    PRAZO_PARA_SUBIR,
};
pub use install::{ErroDeInstalacao, EventoDeInstalacao, Layout};
pub use protocol::Comando;

/// `DSH_HOME` do AgenticOw: profiles, sessões e configurações da pessoa.
pub fn home(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("agenticow-home")
}

/// Porta preferida do Host. Estável entre reinícios: o cookie de sessão e o
/// armazenamento do cliente são por origem (`127.0.0.1:<porta>`).
pub const PORTA_PREFERIDA: u16 = 11730;
