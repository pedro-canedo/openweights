//! Provedores de LLM além do llama.cpp local.
//!
//! A fronteira deste crate é "para onde mandar a conversa e o que anexar":
//! ele resolve um nome de modelo num endpoint OpenAI-compatible, e conhece o
//! catálogo do OpenRouter. Quem sobe processo (9router, Node) e quem fala o
//! protocolo de chat (`lr_engine`) são outros — de propósito, para que o
//! provedor remoto não arraste consigo a gestão de processo local.

pub mod config;
pub mod jev;
pub mod model_ref;
pub mod openrouter;

pub use config::{
    EndpointError, NINEROUTER_DEFAULT_PORT, NineRouterConfig, OPENROUTER_BASE_URL,
    OpenRouterConfig, ProvidersConfig, ResolvedEndpoint,
};
pub use jev::{
    ClienteJev, JEV_MODELO_PADRAO, JevError, Pergunta, RespostaJev, RespostasJev, UsoJev,
};
pub use model_ref::{ModelRef, ProviderId};
pub use openrouter::{KeyInfo, OpenRouterModel, ProviderError};
