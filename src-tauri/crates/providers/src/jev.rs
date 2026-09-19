//! Cliente do Jev (TypeSafe) através do OpenRouter.
//!
//! O Jev **não é um modelo de linguagem**: ele não gera texto. Recebe um
//! `state` (texto ou JSON) e um dicionário de perguntas tipadas — `noul`
//! (sim/não, devolve probabilidade), `choice` (uma entre N opções, com
//! probabilidades e confiança) e `score` (nível numa escala) — e devolve as
//! respostas tipadas em 70–500 ms, cobrando só tokens de entrada
//! (US$ 0,042 por milhão). É um "if inteligente", não um pensador.
//!
//! Pelo OpenRouter o endpoint é outro: `POST /api/alpha/decisions`. Mandar o
//! modelo para `/chat/completions` responde "is a decisions model and cannot
//! be used with the chat/completions endpoint" — por isso este módulo não
//! reaproveita o cliente de chat de `lr_engine`.
//!
//! Este arquivo é só o transporte. As perguntas que o app faz e os limiares
//! que transformam probabilidade em decisão vivem em [`crate::jev_esforco`].

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::OPENROUTER_API_ROOT;

/// Modelo padrão. Versão fixada de propósito: o alias `~typesafe/jev-latest`
/// "anda" quando a TypeSafe publica versão nova, e as respostas por trás dele
/// mudam sem ninguém decidir nada por aqui.
pub const JEV_MODELO_PADRAO: &str = "typesafe/jev-1.13";

/// Caminho do endpoint de decisões, relativo à raiz da API do OpenRouter.
pub const JEV_DECISIONS_PATH: &str = "/alpha/decisions";

/// Prazo por chamada. O Jev responde em menos de meio segundo; três
/// segundos já é sinal de que algo está errado, e quem chama tem um fallback
/// (o esforço que a pessoa configurou) — esperar mais só atrasaria a resposta.
const TIMEOUT: Duration = Duration::from_secs(3);

/// Pausa antes da única repetição em 429/529.
const ESPERA_REPETICAO: Duration = Duration::from_millis(400);

// --- perguntas ------------------------------------------------------------

/// Uma pergunta tipada, no formato do wire (`{"type": "...", ...}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Pergunta {
    /// Sim/não. `criteria` é opcional e explica o que cada lado significa.
    Noul {
        instructions: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        criteria: Option<CriterioNoul>,
    },
    /// Uma opção entre N. As chaves de `criteria` são as opções; os valores,
    /// a descrição de cada uma. `BTreeMap` para o JSON sair sempre na mesma
    /// ordem (o Jev não se importa, mas os testes e o log sim).
    Choice {
        instructions: String,
        criteria: BTreeMap<String, String>,
    },
    /// Um nível numa escala ordenada; `criteria` descreve cada degrau.
    Score {
        instructions: String,
        criteria: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CriterioNoul {
    #[serde(rename = "true")]
    pub sim: String,
    #[serde(rename = "false")]
    pub nao: String,
}

impl Pergunta {
    pub fn noul(instructions: impl Into<String>) -> Self {
        Self::Noul {
            instructions: instructions.into(),
            criteria: None,
        }
    }

    pub fn choice<K, V>(
        instructions: impl Into<String>,
        opcoes: impl IntoIterator<Item = (K, V)>,
    ) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        Self::Choice {
            instructions: instructions.into(),
            criteria: opcoes
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }
}

// --- respostas ------------------------------------------------------------

/// A resposta a uma pergunta. Campos que o wire traz além destes (`legend`,
/// `probabilities` do score) são ignorados: só o que o app consome entra.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum RespostaJev {
    Noul {
        noul: f64,
    },
    Choice {
        choice: String,
        #[serde(default)]
        probabilities: BTreeMap<String, f64>,
        #[serde(default)]
        confidence: f64,
    },
    Score {
        score: f64,
        #[serde(default)]
        confidence: f64,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsoJev {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    /// Em dólares. Só o OpenRouter informa; a API direta da TypeSafe não.
    #[serde(default)]
    pub cost: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RespostasJev {
    /// O modelo que respondeu de fato (`typesafe/jev-1.13-20260917`), útil
    /// para saber qual versão está por trás do alias pedido.
    #[serde(default)]
    pub model: String,
    pub answers: BTreeMap<String, RespostaJev>,
    #[serde(default)]
    pub usage: UsoJev,
}

impl RespostasJev {
    /// A escolha e a confiança de uma pergunta `choice`.
    pub fn escolha(&self, id: &str) -> Option<(&str, f64)> {
        match self.answers.get(id)? {
            RespostaJev::Choice {
                choice, confidence, ..
            } => Some((choice.as_str(), *confidence)),
            _ => None,
        }
    }

    /// A probabilidade de uma pergunta `noul`.
    pub fn noul(&self, id: &str) -> Option<f64> {
        match self.answers.get(id)? {
            RespostaJev::Noul { noul } => Some(*noul),
            _ => None,
        }
    }
}

// --- erros ----------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum JevError {
    #[error("chave de API recusada pelo OpenRouter")]
    Unauthorized,
    #[error("requisição recusada pelo Jev: {0}")]
    Invalido(String),
    #[error("o Jev não respondeu a tempo")]
    Tempo,
    #[error("o Jev está sobrecarregado (HTTP {0})")]
    Sobrecarga(u16),
    #[error("falha de rede: {0}")]
    Rede(reqwest::Error),
    #[error("resposta inesperada do Jev: {0}")]
    Formato(String),
}

impl From<reqwest::Error> for JevError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            Self::Tempo
        } else {
            Self::Rede(e)
        }
    }
}

// --- cliente --------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ClienteJev {
    base_url: String,
    api_key: String,
    modelo: String,
    timeout: Duration,
    http: reqwest::Client,
}

impl ClienteJev {
    /// Cliente apontado para o OpenRouter, com a chave da pessoa.
    pub fn openrouter(api_key: impl Into<String>) -> Self {
        Self::novo(OPENROUTER_API_ROOT, api_key, TIMEOUT)
    }

    fn novo(base_url: &str, api_key: impl Into<String>, timeout: Duration) -> Self {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .user_agent(concat!("OpenWeights/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            modelo: JEV_MODELO_PADRAO.to_string(),
            timeout,
            http,
        }
    }

    /// Outra raiz de API — para os testes falarem com um servidor falso.
    pub fn with_base_url(self, base_url: &str) -> Self {
        Self::novo(base_url, self.api_key, self.timeout).with_modelo(self.modelo)
    }

    pub fn with_modelo(mut self, modelo: impl Into<String>) -> Self {
        let m = modelo.into();
        if !m.trim().is_empty() {
            self.modelo = m;
        }
        self
    }

    pub fn with_timeout(self, timeout: Duration) -> Self {
        Self::novo(&self.base_url, self.api_key, timeout).with_modelo(self.modelo)
    }

    pub fn modelo(&self) -> &str {
        &self.modelo
    }

    pub fn url(&self) -> String {
        format!("{}{JEV_DECISIONS_PATH}", self.base_url)
    }

    /// O corpo exatamente como vai no wire. Pura, para o teste travar o
    /// formato documentado sem precisar de rede.
    pub fn corpo(&self, state: &Value, perguntas: &BTreeMap<String, Pergunta>) -> Value {
        serde_json::json!({
            "model": self.modelo,
            "state": state,
            "questions": perguntas,
        })
    }

    /// Uma rodada de decisões. Repete UMA vez em 429/529, depois de uma
    /// pausa curta; qualquer outro erro volta na hora — quem chama tem um
    /// padrão para cair, e insistir só atrasaria a resposta ao usuário.
    pub async fn decidir(
        &self,
        state: &Value,
        perguntas: &BTreeMap<String, Pergunta>,
    ) -> Result<RespostasJev, JevError> {
        let corpo = self.corpo(state, perguntas);
        match self.enviar(&corpo).await {
            Err(JevError::Sobrecarga(_)) => {
                tokio::time::sleep(ESPERA_REPETICAO).await;
                self.enviar(&corpo).await
            }
            outro => outro,
        }
    }

    async fn enviar(&self, corpo: &Value) -> Result<RespostasJev, JevError> {
        let resp = self
            .http
            .post(self.url())
            .bearer_auth(&self.api_key)
            // Atribuição do tráfego ao projeto nos rankings do OpenRouter —
            // os mesmos dois cabeçalhos que o chat manda.
            .header(
                "HTTP-Referer",
                "https://github.com/pedro-canedo/openweights",
            )
            .header("X-Title", "OpenWeights")
            .json(corpo)
            .send()
            .await?;

        let status = resp.status();
        match status.as_u16() {
            200..=299 => {}
            401 | 403 => return Err(JevError::Unauthorized),
            429 | 529 | 502 | 503 => return Err(JevError::Sobrecarga(status.as_u16())),
            400 | 422 => {
                let texto = resp.text().await.unwrap_or_default();
                return Err(JevError::Invalido(resumo_de_erro(&texto)));
            }
            _ => {
                let texto = resp.text().await.unwrap_or_default();
                return Err(JevError::Formato(format!(
                    "HTTP {}: {}",
                    status.as_u16(),
                    resumo_de_erro(&texto)
                )));
            }
        }

        let texto = resp.text().await?;
        serde_json::from_str::<RespostasJev>(&texto)
            .map_err(|e| JevError::Formato(format!("{e}: {}", resumo_de_erro(&texto))))
    }
}

/// A mensagem de um erro JSON do OpenRouter (`{"error":{"message":…}}`), ou
/// o começo do texto cru quando não é JSON.
fn resumo_de_erro(texto: &str) -> String {
    let msg = serde_json::from_str::<Value>(texto)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.get("message"))
                .or_else(|| v.get("detail"))
                .and_then(|m| m.as_str().map(str::to_string))
        })
        .unwrap_or_else(|| texto.trim().to_string());
    msg.chars().take(300).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::thread;

    // ---------------------------------------------------- servidor falso ---

    /// Um `/alpha/decisions` de mentira: lê a requisição, conta e responde
    /// o que o teste mandar, por número de chamada.
    struct JevFalso {
        addr: SocketAddr,
        chamadas: Arc<AtomicUsize>,
        parar: Arc<AtomicBool>,
    }

    impl JevFalso {
        fn subir<F>(responder: F) -> Self
        where
            F: Fn(usize, &str) -> (u16, String) + Send + 'static,
        {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let addr = listener.local_addr().expect("addr");
            listener.set_nonblocking(true).expect("nonblocking");
            let chamadas = Arc::new(AtomicUsize::new(0));
            let parar = Arc::new(AtomicBool::new(false));
            let (c, p) = (chamadas.clone(), parar.clone());
            thread::spawn(move || {
                while !p.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut s, _)) => {
                            let _ = s.set_nonblocking(false);
                            if let Some(corpo) = ler_requisicao(&mut s) {
                                let n = c.fetch_add(1, Ordering::SeqCst);
                                let (status, body) = responder(n, &corpo);
                                let _ = write!(
                                    s,
                                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                    body.len()
                                );
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                addr,
                chamadas,
                parar,
            }
        }

        fn cliente(&self) -> ClienteJev {
            ClienteJev::openrouter("sk-teste").with_base_url(&format!("http://{}", self.addr))
        }

        fn chamadas(&self) -> usize {
            self.chamadas.load(Ordering::SeqCst)
        }
    }

    impl Drop for JevFalso {
        fn drop(&mut self) {
            self.parar.store(true, Ordering::SeqCst);
        }
    }

    fn ler_requisicao(s: &mut TcpStream) -> Option<String> {
        let mut cabeca = Vec::new();
        let mut b = [0u8; 1];
        while !cabeca.ends_with(b"\r\n\r\n") {
            if s.read(&mut b).ok()? == 0 {
                return None;
            }
            cabeca.push(b[0]);
        }
        let texto = String::from_utf8_lossy(&cabeca).to_string();
        assert!(
            texto.starts_with(&format!("POST {JEV_DECISIONS_PATH} ")),
            "caminho errado: {}",
            texto.lines().next().unwrap_or("")
        );
        assert!(
            texto
                .to_lowercase()
                .contains("authorization: bearer sk-teste"),
            "sem bearer"
        );
        let tamanho = texto
            .lines()
            .find_map(|l| {
                l.to_lowercase()
                    .strip_prefix("content-length:")
                    .map(|v| v.trim().parse::<usize>().ok())
            })
            .flatten()
            .unwrap_or(0);
        let mut corpo = vec![0u8; tamanho];
        s.read_exact(&mut corpo).ok()?;
        Some(String::from_utf8_lossy(&corpo).to_string())
    }

    fn resposta_ok() -> String {
        json!({
            "model": "typesafe/jev-1.13-20260917",
            "answers": {
                "nivel": {"type": "choice", "choice": "alto",
                          "probabilities": {"nenhum": 0.05, "medio": 0.15, "alto": 0.8},
                          "confidence": 0.71},
                "precisa": {"type": "noul", "noul": 0.93},
                "gravidade": {"type": "score", "score": 1.4, "confidence": 0.6,
                              "legend": {"0": "a", "1": "b"}}
            },
            "usage": {"input_tokens": 456, "output_tokens": 81, "cost": 1.9152e-05},
            "provider": "TypeSafe"
        })
        .to_string()
    }

    fn perguntas() -> BTreeMap<String, Pergunta> {
        let mut p = BTreeMap::new();
        p.insert(
            "nivel".to_string(),
            Pergunta::choice(
                "Quanto raciocínio?",
                [("nenhum", "nada"), ("medio", "algum"), ("alto", "muito")],
            ),
        );
        p.insert("precisa".to_string(), Pergunta::noul("Precisa pensar?"));
        p
    }

    // ------------------------------------------------------------ puros ---

    /// O corpo bate com o formato documentado pela TypeSafe/OpenRouter.
    #[test]
    fn the_request_body_matches_the_documented_shape() {
        let c = ClienteJev::openrouter("k");
        let corpo = c.corpo(&json!({"mensagem_atual": "oi"}), &perguntas());
        assert_eq!(
            corpo,
            json!({
                "model": "typesafe/jev-1.13",
                "state": {"mensagem_atual": "oi"},
                "questions": {
                    "nivel": {"type": "choice", "instructions": "Quanto raciocínio?",
                              "criteria": {"alto": "muito", "medio": "algum", "nenhum": "nada"}},
                    "precisa": {"type": "noul", "instructions": "Precisa pensar?"}
                }
            })
        );
    }

    #[test]
    fn the_url_is_the_decisions_endpoint_not_chat_completions() {
        let c = ClienteJev::openrouter("k");
        assert_eq!(c.url(), "https://openrouter.ai/api/alpha/decisions");
        assert_eq!(
            c.with_base_url("http://127.0.0.1:1/").url(),
            "http://127.0.0.1:1/alpha/decisions"
        );
    }

    #[test]
    fn a_documented_response_deserialises_with_all_three_answer_types() {
        let r: RespostasJev = serde_json::from_str(&resposta_ok()).expect("parse");
        assert_eq!(r.model, "typesafe/jev-1.13-20260917");
        assert_eq!(r.escolha("nivel"), Some(("alto", 0.71)));
        assert_eq!(r.noul("precisa"), Some(0.93));
        assert!(
            matches!(r.answers.get("gravidade"), Some(RespostaJev::Score { score, .. }) if (*score - 1.4).abs() < 1e-9)
        );
        assert_eq!(r.usage.input_tokens, 456);
        assert_eq!(r.usage.cost, Some(1.9152e-05));
        // Tipo trocado não é resposta.
        assert_eq!(r.escolha("precisa"), None);
        assert_eq!(r.noul("nivel"), None);
    }

    #[test]
    fn an_empty_model_override_keeps_the_default() {
        assert_eq!(
            ClienteJev::openrouter("k").with_modelo("  ").modelo(),
            JEV_MODELO_PADRAO
        );
        assert_eq!(
            ClienteJev::openrouter("k")
                .with_modelo("typesafe/jev-1.12")
                .modelo(),
            "typesafe/jev-1.12"
        );
    }

    #[test]
    fn error_messages_are_extracted_from_the_openrouter_envelope() {
        assert_eq!(
            resumo_de_erro(r#"{"error":{"message":"is a decisions model","code":400}}"#),
            "is a decisions model"
        );
        assert_eq!(resumo_de_erro("  texto cru  "), "texto cru");
    }

    // -------------------------------------------------- contra o falso ---

    #[tokio::test]
    async fn a_successful_decision_is_a_single_request() {
        let falso = JevFalso::subir(|_, _| (200, resposta_ok()));
        let r = falso
            .cliente()
            .decidir(&json!("oi"), &perguntas())
            .await
            .expect("ok");
        assert_eq!(r.escolha("nivel"), Some(("alto", 0.71)));
        assert_eq!(falso.chamadas(), 1);
    }

    #[tokio::test]
    async fn a_429_is_retried_once_and_then_succeeds() {
        let falso = JevFalso::subir(|n, _| {
            if n == 0 {
                (429, r#"{"error":{"message":"rate limited"}}"#.to_string())
            } else {
                (200, resposta_ok())
            }
        });
        let r = falso
            .cliente()
            .decidir(&json!("oi"), &perguntas())
            .await
            .expect("ok após retry");
        assert_eq!(r.noul("precisa"), Some(0.93));
        assert_eq!(falso.chamadas(), 2);
    }

    #[tokio::test]
    async fn two_overloads_in_a_row_give_up() {
        let falso = JevFalso::subir(|_, _| (529, "{}".to_string()));
        let e = falso
            .cliente()
            .decidir(&json!("oi"), &perguntas())
            .await
            .unwrap_err();
        assert!(matches!(e, JevError::Sobrecarga(529)), "{e}");
        assert_eq!(falso.chamadas(), 2);
    }

    #[tokio::test]
    async fn a_401_is_unauthorized_without_retry() {
        let falso = JevFalso::subir(|_, _| (401, r#"{"error":{"message":"No auth"}}"#.to_string()));
        let e = falso
            .cliente()
            .decidir(&json!("oi"), &perguntas())
            .await
            .unwrap_err();
        assert!(matches!(e, JevError::Unauthorized), "{e}");
        assert_eq!(falso.chamadas(), 1);
    }

    #[tokio::test]
    async fn a_422_carries_the_validation_message() {
        let falso = JevFalso::subir(|_, _| {
            (
                422,
                r#"{"error":{"message":"questions: too many"}}"#.to_string(),
            )
        });
        let e = falso
            .cliente()
            .decidir(&json!("oi"), &perguntas())
            .await
            .unwrap_err();
        assert!(
            matches!(&e, JevError::Invalido(m) if m == "questions: too many"),
            "{e}"
        );
    }

    #[tokio::test]
    async fn a_slow_server_is_a_timeout_not_a_hang() {
        let falso = JevFalso::subir(|_, _| {
            thread::sleep(Duration::from_millis(700));
            (200, resposta_ok())
        });
        let cliente = falso.cliente().with_timeout(Duration::from_millis(150));
        let e = cliente
            .decidir(&json!("oi"), &perguntas())
            .await
            .unwrap_err();
        assert!(matches!(e, JevError::Tempo), "{e}");
    }

    #[tokio::test]
    async fn a_body_that_is_not_the_answer_shape_is_a_format_error() {
        let falso = JevFalso::subir(|_, _| {
            (
                200,
                r#"{"choices":[{"message":{"content":"oi"}}]}"#.to_string(),
            )
        });
        let e = falso
            .cliente()
            .decidir(&json!("oi"), &perguntas())
            .await
            .unwrap_err();
        assert!(matches!(e, JevError::Formato(_)), "{e}");
    }

    /// Teste live — roda só com `--ignored` e a chave no ambiente.
    #[tokio::test]
    #[ignore = "rede: gasta uma fração de centavo no OpenRouter (OPENROUTER_API_KEY)"]
    async fn jev_ao_vivo() {
        let chave = std::env::var("OPENROUTER_API_KEY").expect("OPENROUTER_API_KEY");
        let c = ClienteJev::openrouter(chave);
        let r = c
            .decidir(
                &json!({"mensagem_atual": "Prove that the square root of 2 is irrational."}),
                &perguntas(),
            )
            .await
            .expect("decisão ao vivo");
        let (escolha, conf) = r.escolha("nivel").expect("choice");
        assert!(["nenhum", "medio", "alto"].contains(&escolha), "{escolha}");
        assert!((0.0..=1.0).contains(&conf));
        eprintln!(
            "jev ao vivo: {escolha} ({conf:.2}) modelo={} custo={:?}",
            r.model, r.usage.cost
        );
    }
}
