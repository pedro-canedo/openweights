//! Gateway do OwCLI.
//!
//! O OwCLI — dentro do app ou no terminal do sistema — fala com UM endereço
//! (`http://127.0.0.1:<porta>/owcli/v1`) e prova quem é com um token. O
//! gateway olha o prefixo do modelo pedido (`local:…`, `openrouter:…`,
//! `ninerouter:…`), troca pelo id que a fonte conhece, injeta a chave DAQUELA
//! fonte e repassa a resposta em streaming, byte a byte.
//!
//! Por que um gateway, e não o OwCLI falando com cada fonte:
//!
//! - **As chaves nunca saem do app.** A do OpenRouter é paga; ela não vai para
//!   arquivo, argv nem variável de ambiente de nenhum processo. O OwCLI só
//!   conhece o token deste gateway, que só vale na própria máquina.
//! - **Um endereço estável.** O motor pode trocar de porta; o 9router pode
//!   subir depois. Quem já está com uma sessão aberta não percebe.
//! - **Trocar de fonte no meio da sessão.** Um provedor só no OwCLI, com o
//!   prefixo no id: o seletor `/model` mistura local e remoto.
//!
//! O spike mediu o Codex contra o llama-server b10441 sem nenhuma reescrita de
//! corpo além do `model`: o gateway não mexe em mais nada.

mod reescritas;
mod servico;

use std::sync::{Arc, RwLock};
use std::time::Duration;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::Notify;

/// Porta preferida. Ocupada, o gateway usa uma efêmera: quem precisa da URL
/// lê o `openweights.json`, que o app reescreve.
pub const PORTA_PREFERIDA: u16 = 11740;
/// O caminho que o OwCLI usa como base.
pub const PREFIXO_DO_CAMINHO: &str = "/owcli/v1";

/// Uma fonte de modelos atrás do gateway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rota {
    /// `local`, `openrouter`, `ninerouter` — o que vem antes do `:` no modelo.
    pub prefixo: String,
    /// Base OpenAI-compatível da fonte, com `/v1`.
    pub base_url: String,
    /// Chave da fonte; `None` quando ela não autentica.
    pub chave: Option<String>,
    /// Ids que a fonte atende, sem o prefixo (para o `/models`).
    pub modelos: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Configuracao {
    /// O que o OwCLI manda em `Authorization: Bearer`.
    pub token: String,
    pub rotas: Vec<Rota>,
}

pub(crate) struct Ctx {
    pub(crate) config: RwLock<Configuracao>,
    pub(crate) http: reqwest::Client,
}

pub struct Gateway {
    porta: u16,
    ctx: Arc<Ctx>,
    cancelar: Arc<Notify>,
    tarefa: Option<tokio::task::JoinHandle<()>>,
}

impl Gateway {
    /// Sobe o gateway em 127.0.0.1 (nunca na rede).
    pub async fn iniciar(porta_preferida: u16, config: Configuracao) -> std::io::Result<Self> {
        let listener = match TcpListener::bind(("127.0.0.1", porta_preferida)).await {
            Ok(l) => l,
            Err(_) => TcpListener::bind(("127.0.0.1", 0)).await?,
        };
        let porta = listener.local_addr()?.port();
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            // Sem teto total: uma geração longa em streaming pode levar minutos.
            .build()
            .map_err(std::io::Error::other)?;
        let ctx = Arc::new(Ctx {
            config: RwLock::new(config),
            http,
        });
        let cancelar = Arc::new(Notify::new());
        let tarefa = {
            let ctx = ctx.clone();
            let cancelar = cancelar.clone();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = cancelar.notified() => break,
                        aceito = listener.accept() => match aceito {
                            Ok((stream, _)) => {
                                let ctx = ctx.clone();
                                tokio::spawn(async move {
                                    let io = TokioIo::new(stream);
                                    let svc = service_fn(move |req| servico::atender(req, ctx.clone()));
                                    if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
                                        log::debug!("conexão do gateway do OwCLI encerrada: {e}");
                                    }
                                });
                            }
                            Err(e) => {
                                log::warn!("accept do gateway do OwCLI falhou: {e}");
                                tokio::time::sleep(Duration::from_millis(50)).await;
                            }
                        },
                    }
                }
            })
        };
        Ok(Self {
            porta,
            ctx,
            cancelar,
            tarefa: Some(tarefa),
        })
    }

    pub fn porta(&self) -> u16 {
        self.porta
    }

    /// A base que vai no `openweights.json`.
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}{PREFIXO_DO_CAMINHO}", self.porta)
    }

    /// Troca rotas, chaves e token sem derrubar conexões em andamento.
    pub fn atualizar(&self, config: Configuracao) {
        *self.ctx.config.write().unwrap_or_else(|e| e.into_inner()) = config;
    }

    pub async fn parar(mut self) {
        self.cancelar.notify_one();
        if let Some(tarefa) = self.tarefa.take() {
            let _ = tarefa.await;
        }
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        self.cancelar.notify_one();
    }
}

/// Um token novo, de 32 bytes, em hexadecimal (mesma fonte da chave do
/// servidor local: o RNG do sistema).
pub fn novo_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|e| format!("sem fonte de aleatoriedade do sistema: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
