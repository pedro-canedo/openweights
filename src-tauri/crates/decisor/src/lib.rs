//! Proxy OpenAI-compatível na frente do llama-server que aplica a decisão do
//! Jev às requisições dos harnesses.
//!
//! **Por que existe.** O laço do DeepSeek Harness (e do aider, opencode,
//! claude-code) vive fora deste repositório: a única forma de decidir, por
//! requisição, se o modelo local deve pensar é ficar no caminho HTTP. O
//! Traefik do gateway não lê corpo. Então este crate escuta em
//! `127.0.0.1:11712`, repassa tudo ao motor sem tocar — inclusive o
//! streaming, sem buffer — e, só no `POST /v1/chat/completions`, pergunta ao
//! Jev quanto raciocínio a conversa pede e reescreve o corpo antes de
//! encaminhar (ver [`reescrita`]).
//!
//! **O que ele NÃO faz.** Não autentica (repassa o `Authorization` que
//! veio), não junta catálogos, não escuta na rede — é loopback, para os
//! harnesses que o próprio app abre. Cliente de LAN do motor não passa por
//! aqui, e a documentação diz isso.
//!
//! **Fail-open.** Jev fora do ar, chave ausente, prazo estourado, corpo
//! ilegível: a requisição segue como veio. O cabeçalho de resposta
//! `x-openweights-jev` conta o que aconteceu (`alto;0.87;local`,
//! `alto;0.87;jev`, `nenhum;cache`, `default;<motivo>`), para o `curl` e os
//! testes verem sem adivinhar.
//!
//! **Decisores.** A decisão vem de uma cadeia ([`Decisores`]): o decisor
//! local (o `/v1/decision` do llama-server do fork, na máquina) primeiro, o
//! Jev remoto como reserva. O proxy também REPASSA `POST /v1/decision` ao
//! decisor local, para que harnesses, o gateway e o playground usem o mesmo
//! endpoint sem descobrir a porta dele.

mod proxy;
pub mod reescrita;

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify, RwLock};

use lr_providers::{CapacidadeModelo, Contadores, Decisores, NivelRaciocinio};

/// Porta preferida: o motor está em 11711, e "a seguinte" é fácil de
/// lembrar quando alguém cola a URL num harness à mão.
pub const PORTA_PADRAO: u16 = 11712;

/// Prazo para a decisão. Menor que o timeout do cliente Jev (3 s): num laço
/// de agente cada requisição paga isto no pior caso, e o padrão da pessoa
/// está sempre ali como resposta.
pub const TEMPO_DECISAO_PADRAO: Duration = Duration::from_millis(2_500);

/// Teto do corpo de um `chat/completions` que o proxy se dispõe a ler para
/// decidir. Acima disto (imagens em base64, contexto gigante) a requisição
/// passa sem decisão — ler tudo em memória só para o Jev não vale.
pub const TETO_CORPO: usize = 8 * 1024 * 1024;

/// Quantas conversas recentes lembram a decisão (ver
/// [`reescrita::chave_da_conversa`]).
const TAMANHO_CACHE: usize = 64;

/// Quem diz o que um modelo sabe fazer com raciocínio. É o app que sabe
/// onde os GGUF estão; o proxy só pergunta.
pub type ResolverCapacidade = Arc<dyn Fn(&str) -> CapacidadeModelo + Send + Sync>;

pub struct ConfigShim {
    /// Raiz do motor, sem `/v1` (`http://127.0.0.1:11711`).
    pub upstream: String,
    pub porta_preferida: u16,
    /// Vazia = sem decisão: o proxy vira passagem direta (e `/v1/decision`
    /// responde 503).
    pub decisores: Decisores,
    pub min_confianca: f32,
    pub capacidade: ResolverCapacidade,
    pub contadores: Arc<Contadores>,
    pub tempo_decisao: Duration,
}

pub(crate) struct Politica {
    pub decisores: Decisores,
    pub min_confianca: f32,
}

pub(crate) type ClienteUpstream = hyper_util::client::legacy::Client<
    hyper_util::client::legacy::connect::HttpConnector,
    proxy::Corpo,
>;

/// O que cada conexão enxerga.
pub(crate) struct Ctx {
    pub upstream: String,
    pub politica: RwLock<Politica>,
    pub capacidade: ResolverCapacidade,
    pub contadores: Arc<Contadores>,
    pub tempo_decisao: Duration,
    pub cache: Mutex<VecDeque<(u64, NivelRaciocinio)>>,
    pub http: ClienteUpstream,
}

impl Ctx {
    pub(crate) async fn lembrar(&self, chave: u64) -> Option<NivelRaciocinio> {
        self.cache
            .lock()
            .await
            .iter()
            .find(|(k, _)| *k == chave)
            .map(|(_, n)| *n)
    }

    pub(crate) async fn guardar(&self, chave: u64, nivel: NivelRaciocinio) {
        let mut cache = self.cache.lock().await;
        cache.retain(|(k, _)| *k != chave);
        cache.push_back((chave, nivel));
        while cache.len() > TAMANHO_CACHE {
            cache.pop_front();
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ErroShim {
    #[error("não foi possível escutar em 127.0.0.1: {0}")]
    Bind(#[from] std::io::Error),
}

/// O proxy no ar. Soltar o handle NÃO para o servidor: chame [`Shim::parar`].
pub struct Shim {
    porta: u16,
    ctx: Arc<Ctx>,
    cancelar: Arc<Notify>,
    tarefa: tokio::task::JoinHandle<()>,
}

impl Shim {
    pub async fn iniciar(cfg: ConfigShim) -> Result<Self, ErroShim> {
        let listener = match TcpListener::bind(("127.0.0.1", cfg.porta_preferida)).await {
            Ok(l) => l,
            // Porta ocupada (outra instância, outro programa): uma efêmera
            // serve, porque quem precisa da URL pergunta ao app.
            Err(_) => TcpListener::bind(("127.0.0.1", 0)).await?,
        };
        let porta = listener.local_addr()?.port();

        let ctx = Arc::new(Ctx {
            upstream: cfg.upstream.trim_end_matches('/').to_string(),
            politica: RwLock::new(Politica {
                decisores: cfg.decisores,
                min_confianca: cfg.min_confianca,
            }),
            capacidade: cfg.capacidade,
            contadores: cfg.contadores,
            tempo_decisao: cfg.tempo_decisao,
            cache: Mutex::new(VecDeque::new()),
            http: hyper_util::client::legacy::Client::builder(TokioExecutor::new()).build_http(),
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
                                    let servico = service_fn(move |req| proxy::atender(req, ctx.clone()));
                                    if let Err(e) = http1::Builder::new()
                                        .serve_connection(io, servico)
                                        .await
                                    {
                                        log::debug!("conexão do proxy encerrada: {e}");
                                    }
                                });
                            }
                            Err(e) => {
                                log::warn!("accept do proxy falhou: {e}");
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
            tarefa,
        })
    }

    pub fn porta(&self) -> u16 {
        self.porta
    }

    /// Raiz do proxy, sem `/v1` — o mesmo contrato da URL do motor.
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.porta)
    }

    /// Troca decisores e limiar sem derrubar conexões em curso.
    pub async fn atualizar(&self, decisores: Decisores, min_confianca: f32) {
        let mut p = self.ctx.politica.write().await;
        p.decisores = decisores;
        p.min_confianca = min_confianca;
    }

    pub async fn parar(self) {
        self.cancelar.notify_one();
        self.tarefa.abort();
        let _ = self.tarefa.await;
    }
}

#[cfg(test)]
mod testes;
