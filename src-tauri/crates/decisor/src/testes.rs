//! Testes de integração do proxy contra um motor falso e um Jev falso, os
//! dois em hyper. O caso que mais importa é o streaming: o motor falso segura
//! o segundo pedaço até o teste ter LIDO o primeiro pela ponta do cliente —
//! se o proxy bufferizasse, o teste travaria e falharia por prazo.

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use http::{Request, Response};
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::body::{Body, Frame, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use lr_providers::{CapacidadeModelo, ClienteJev, Contadores};

use super::*;

type CorpoFalso = BoxBody<Bytes, hyper::Error>;

#[derive(Debug, Clone)]
struct Registro {
    metodo: String,
    caminho: String,
    authorization: Option<String>,
    corpo: Bytes,
}

/// Um corpo alimentado por um canal: cada `send` vira um pedaço no fio.
struct CorpoCanal(mpsc::Receiver<Bytes>);

impl Body for CorpoCanal {
    type Data = Bytes;
    type Error = hyper::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, hyper::Error>>> {
        match self.0.poll_recv(cx) {
            Poll::Ready(Some(b)) => Poll::Ready(Some(Ok(Frame::data(b)))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

enum Resposta {
    Json(u16, Value),
    Sse(mpsc::Receiver<Bytes>),
}

type Tratador = Arc<dyn Fn(&Registro) -> Resposta + Send + Sync>;

struct Falso {
    addr: SocketAddr,
    registros: Arc<Mutex<Vec<Registro>>>,
    chamadas: Arc<AtomicUsize>,
}

impl Falso {
    async fn subir(tratador: Tratador) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let registros = Arc::new(Mutex::new(Vec::new()));
        let chamadas = Arc::new(AtomicUsize::new(0));
        let (r, c) = (registros.clone(), chamadas.clone());
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let (t, r, c) = (tratador.clone(), r.clone(), c.clone());
                tokio::spawn(async move {
                    let servico = service_fn(move |req: Request<Incoming>| {
                        let (t, r, c) = (t.clone(), r.clone(), c.clone());
                        async move {
                            let (partes, corpo) = req.into_parts();
                            let corpo = corpo.collect().await.unwrap().to_bytes();
                            let reg = Registro {
                                metodo: partes.method.to_string(),
                                caminho: partes.uri.path().to_string(),
                                authorization: partes
                                    .headers
                                    .get("authorization")
                                    .and_then(|v| v.to_str().ok())
                                    .map(str::to_string),
                                corpo,
                            };
                            c.fetch_add(1, Ordering::SeqCst);
                            let resposta = t(&reg);
                            r.lock().unwrap().push(reg);
                            let resp: Response<CorpoFalso> = match resposta {
                                Resposta::Json(status, v) => Response::builder()
                                    .status(status)
                                    .header("content-type", "application/json")
                                    .body(
                                        Full::new(Bytes::from(v.to_string()))
                                            .map_err(|e| match e {})
                                            .boxed(),
                                    )
                                    .unwrap(),
                                Resposta::Sse(rx) => Response::builder()
                                    .status(200)
                                    .header("content-type", "text/event-stream")
                                    .body(CorpoCanal(rx).boxed())
                                    .unwrap(),
                            };
                            Ok::<_, std::convert::Infallible>(resp)
                        }
                    });
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), servico)
                        .await;
                });
            }
        });
        Self {
            addr,
            registros,
            chamadas,
        }
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn registros(&self) -> Vec<Registro> {
        self.registros.lock().unwrap().clone()
    }

    fn chamadas(&self) -> usize {
        self.chamadas.load(Ordering::SeqCst)
    }
}

fn resposta_jev(nivel: &str, conf: f64, noul: f64) -> Value {
    json!({
        "model": "typesafe/jev-1.13-x",
        "answers": {
            "nivel": {"type": "choice", "choice": nivel, "probabilities": {}, "confidence": conf},
            "precisa_raciocinar": {"type": "noul", "noul": noul}
        },
        "usage": {"input_tokens": 50, "output_tokens": 5, "cost": 2.1e-6}
    })
}

fn qwen3() -> ResolverCapacidade {
    Arc::new(|_| CapacidadeModelo {
        thinking_toggle: true,
        efforts: vec![],
    })
}

fn sem_raciocinio() -> ResolverCapacidade {
    Arc::new(|_| CapacidadeModelo::default())
}

async fn shim(
    upstream: &str,
    jev: Option<ClienteJev>,
    capacidade: ResolverCapacidade,
) -> (Shim, Arc<Contadores>) {
    let contadores = Arc::new(Contadores::default());
    let s = Shim::iniciar(ConfigShim {
        upstream: upstream.to_string(),
        porta_preferida: 0,
        jev,
        min_confianca: 0.6,
        capacidade,
        contadores: contadores.clone(),
        tempo_decisao: Duration::from_millis(800),
    })
    .await
    .unwrap();
    (s, contadores)
}

fn jev_em(url: &str) -> ClienteJev {
    ClienteJev::openrouter("sk-teste")
        .with_base_url(url)
        .with_timeout(Duration::from_millis(500))
}

fn corpo_chat(stream: bool) -> Value {
    json!({
        "model": "qwen3", "stream": stream,
        "messages": [{"role": "user", "content": "prove que raiz de 2 é irracional"}],
        "chat_template_kwargs": {"enable_thinking": false}
    })
}

// ------------------------------------------------------------------ testes ---

#[tokio::test]
async fn a_get_passes_through_with_its_authorization_header() {
    let motor = Falso::subir(Arc::new(|_| {
        Resposta::Json(200, json!({"data": [{"id": "qwen3"}]}))
    }))
    .await;
    let (s, _) = shim(&motor.url(), None, qwen3()).await;

    let r = reqwest::Client::new()
        .get(format!("{}/v1/models", s.base_url()))
        .header("authorization", "Bearer local")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert!(
        r.headers().get("x-openweights-jev").is_none(),
        "GET não é decidido"
    );
    assert_eq!(r.json::<Value>().await.unwrap()["data"][0]["id"], "qwen3");

    let regs = motor.registros();
    assert_eq!(regs[0].metodo, "GET");
    assert_eq!(regs[0].caminho, "/v1/models");
    assert_eq!(regs[0].authorization.as_deref(), Some("Bearer local"));
    s.parar().await;
}

/// O segundo pedaço só sai do motor depois de o cliente ter recebido o
/// primeiro: prova que nada no meio está bufferizando.
#[tokio::test]
async fn sse_is_forwarded_chunk_by_chunk_without_buffering() {
    let (tx, rx) = mpsc::channel::<Bytes>(4);
    let rx = Arc::new(Mutex::new(Some(rx)));
    let motor = Falso::subir(Arc::new(move |_| {
        Resposta::Sse(rx.lock().unwrap().take().expect("uma resposta só"))
    }))
    .await;
    let (s, _) = shim(&motor.url(), None, qwen3()).await;

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", s.base_url()))
        .json(&corpo_chat(true))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["x-openweights-jev"], "default;jev desligado");

    use futures_util::StreamExt;
    let mut fluxo = resp.bytes_stream();

    tx.send(Bytes::from("data: {\"n\":1}\n\n")).await.unwrap();
    let primeiro = tokio::time::timeout(Duration::from_secs(3), fluxo.next())
        .await
        .expect("o primeiro pedaço tem de chegar antes do segundo existir")
        .unwrap()
        .unwrap();
    assert!(std::str::from_utf8(&primeiro).unwrap().contains("\"n\":1"));

    tx.send(Bytes::from("data: [DONE]\n\n")).await.unwrap();
    drop(tx);
    let mut resto = Vec::new();
    while let Some(p) = fluxo.next().await {
        resto.extend_from_slice(&p.unwrap());
    }
    assert!(std::str::from_utf8(&resto).unwrap().contains("[DONE]"));
    s.parar().await;
}

#[tokio::test]
async fn when_jev_is_unreachable_the_body_reaches_the_engine_untouched() {
    let motor = Falso::subir(Arc::new(|_| Resposta::Json(200, json!({"ok": true})))).await;
    // Porta fechada: nada escuta ali.
    let jev = jev_em("http://127.0.0.1:1");
    let (s, contadores) = shim(&motor.url(), Some(jev), qwen3()).await;

    let corpo = corpo_chat(false);
    let r = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", s.base_url()))
        .json(&corpo)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let marca = r.headers()["x-openweights-jev"]
        .to_str()
        .unwrap()
        .to_string();
    assert!(marca.starts_with("default;"), "{marca}");

    let visto: Value = serde_json::from_slice(&motor.registros()[0].corpo).unwrap();
    assert_eq!(visto, corpo, "o corpo original, byte a byte em JSON");
    let resumo = contadores.resumo();
    assert_eq!(
        (resumo.chamadas, resumo.falhas, resumo.aplicadas),
        (1, 1, 0)
    );
    s.parar().await;
}

#[tokio::test]
async fn a_confident_alto_turns_thinking_on_before_the_engine_sees_it() {
    let motor = Falso::subir(Arc::new(|_| Resposta::Json(200, json!({"ok": true})))).await;
    let jev_falso = Falso::subir(Arc::new(|_| {
        Resposta::Json(200, resposta_jev("alto", 0.9, 0.95))
    }))
    .await;
    let (s, contadores) = shim(&motor.url(), Some(jev_em(&jev_falso.url())), qwen3()).await;

    let r = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", s.base_url()))
        .json(&corpo_chat(false))
        .send()
        .await
        .unwrap();
    assert_eq!(r.headers()["x-openweights-jev"], "alto;0.90");

    let visto: Value = serde_json::from_slice(&motor.registros()[0].corpo).unwrap();
    assert_eq!(visto["chat_template_kwargs"]["enable_thinking"], true);
    assert_eq!(
        visto["messages"][0]["content"],
        "prove que raiz de 2 é irracional"
    );
    // O Jev viu a conversa resumida e com o caminho certo.
    let pedido = &jev_falso.registros()[0];
    assert_eq!(pedido.caminho, "/alpha/decisions");
    let estado: Value = serde_json::from_slice(&pedido.corpo).unwrap();
    assert_eq!(
        estado["state"]["mensagem_atual"],
        "prove que raiz de 2 é irracional"
    );
    assert_eq!(contadores.resumo().aplicadas, 1);
    s.parar().await;
}

/// A continuação de um laço (mesma pergunta, mais turnos de ferramenta)
/// não paga o Jev de novo.
#[tokio::test]
async fn an_agent_continuation_reuses_the_cached_decision() {
    let motor = Falso::subir(Arc::new(|_| Resposta::Json(200, json!({"ok": true})))).await;
    let jev_falso = Falso::subir(Arc::new(|_| {
        Resposta::Json(200, resposta_jev("nenhum", 0.8, 0.1))
    }))
    .await;
    let (s, _) = shim(&motor.url(), Some(jev_em(&jev_falso.url())), qwen3()).await;
    let cliente = reqwest::Client::new();
    let url = format!("{}/v1/chat/completions", s.base_url());

    let r1 = cliente
        .post(&url)
        .json(&corpo_chat(false))
        .send()
        .await
        .unwrap();
    assert_eq!(r1.headers()["x-openweights-jev"], "nenhum;0.80");

    let mut continuacao = corpo_chat(false);
    continuacao["messages"].as_array_mut().unwrap().extend([
        json!({"role": "assistant", "content": null, "tool_calls": [{"id": "1"}]}),
        json!({"role": "tool", "content": "resultado"}),
    ]);
    let r2 = cliente.post(&url).json(&continuacao).send().await.unwrap();
    assert_eq!(r2.headers()["x-openweights-jev"], "nenhum;cache");
    assert_eq!(jev_falso.chamadas(), 1, "uma consulta para as duas idas");

    let visto: Value = serde_json::from_slice(&motor.registros()[1].corpo).unwrap();
    assert_eq!(visto["chat_template_kwargs"]["enable_thinking"], false);
    s.parar().await;
}

#[tokio::test]
async fn a_model_that_cannot_reason_never_costs_a_jev_call() {
    let motor = Falso::subir(Arc::new(|_| Resposta::Json(200, json!({"ok": true})))).await;
    let jev_falso = Falso::subir(Arc::new(|_| {
        Resposta::Json(200, resposta_jev("alto", 0.9, 0.9))
    }))
    .await;
    let (s, _) = shim(
        &motor.url(),
        Some(jev_em(&jev_falso.url())),
        sem_raciocinio(),
    )
    .await;

    let r = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", s.base_url()))
        .json(&corpo_chat(false))
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.headers()["x-openweights-jev"],
        "default;modelo sem raciocinio"
    );
    assert_eq!(jev_falso.chamadas(), 0);
    s.parar().await;
}

#[tokio::test]
async fn low_confidence_keeps_the_body_as_it_came() {
    let motor = Falso::subir(Arc::new(|_| Resposta::Json(200, json!({"ok": true})))).await;
    let jev_falso = Falso::subir(Arc::new(|_| {
        Resposta::Json(200, resposta_jev("alto", 0.3, 0.9))
    }))
    .await;
    let (s, _) = shim(&motor.url(), Some(jev_em(&jev_falso.url())), qwen3()).await;

    let r = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", s.base_url()))
        .json(&corpo_chat(false))
        .send()
        .await
        .unwrap();
    let marca = r.headers()["x-openweights-jev"]
        .to_str()
        .unwrap()
        .to_string();
    assert!(marca.starts_with("default;confian"), "{marca}");
    let visto: Value = serde_json::from_slice(&motor.registros()[0].corpo).unwrap();
    assert_eq!(visto["chat_template_kwargs"]["enable_thinking"], false);
    s.parar().await;
}

#[tokio::test]
async fn a_non_json_body_is_forwarded_untouched() {
    let motor = Falso::subir(Arc::new(|_| Resposta::Json(400, json!({"error": "bad"})))).await;
    let jev_falso = Falso::subir(Arc::new(|_| {
        Resposta::Json(200, resposta_jev("alto", 0.9, 0.9))
    }))
    .await;
    let (s, _) = shim(&motor.url(), Some(jev_em(&jev_falso.url())), qwen3()).await;

    let r = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", s.base_url()))
        .body("isto não é json")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400, "o status do motor atravessa");
    assert_eq!(r.headers()["x-openweights-jev"], "default;corpo ilegivel");
    assert_eq!(motor.registros()[0].corpo, Bytes::from("isto não é json"));
    assert_eq!(jev_falso.chamadas(), 0);
    s.parar().await;
}

#[tokio::test]
async fn an_engine_that_is_down_is_a_502_with_a_json_error() {
    let (s, _) = shim("http://127.0.0.1:1", None, qwen3()).await;
    let r = reqwest::Client::new()
        .get(format!("{}/v1/models", s.base_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 502);
    let v: Value = r.json().await.unwrap();
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("indisponível")
    );
    s.parar().await;
}

#[tokio::test]
async fn a_busy_preferred_port_falls_back_to_an_ephemeral_one() {
    let ocupada = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let porta = ocupada.local_addr().unwrap().port();
    let s = Shim::iniciar(ConfigShim {
        upstream: "http://127.0.0.1:1".into(),
        porta_preferida: porta,
        jev: None,
        min_confianca: 0.6,
        capacidade: qwen3(),
        contadores: Arc::new(Contadores::default()),
        tempo_decisao: TEMPO_DECISAO_PADRAO,
    })
    .await
    .unwrap();
    assert_ne!(s.porta(), porta);
    assert_eq!(s.base_url(), format!("http://127.0.0.1:{}", s.porta()));
    s.parar().await;
}

#[tokio::test]
async fn updating_the_policy_switches_decisions_on_without_a_restart() {
    let motor = Falso::subir(Arc::new(|_| Resposta::Json(200, json!({"ok": true})))).await;
    let jev_falso = Falso::subir(Arc::new(|_| {
        Resposta::Json(200, resposta_jev("medio", 0.9, 0.5))
    }))
    .await;
    let (s, _) = shim(&motor.url(), None, qwen3()).await;
    let cliente = reqwest::Client::new();
    let url = format!("{}/v1/chat/completions", s.base_url());

    let r = cliente
        .post(&url)
        .json(&corpo_chat(false))
        .send()
        .await
        .unwrap();
    assert_eq!(r.headers()["x-openweights-jev"], "default;jev desligado");

    s.atualizar(Some(jev_em(&jev_falso.url())), 0.6).await;
    let r = cliente
        .post(&url)
        .json(&corpo_chat(false))
        .send()
        .await
        .unwrap();
    assert_eq!(r.headers()["x-openweights-jev"], "medio;0.90");
    s.parar().await;
}
