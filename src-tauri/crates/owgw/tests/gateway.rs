//! O gateway contra uma fonte falsa: rota pelo prefixo, chave injetada, token
//! cobrado e a resposta em streaming chegando byte a byte.

use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use lr_owgw::{Configuracao, Gateway, Rota};
use serde_json::Value;
use tokio::net::TcpListener;

#[derive(Debug, Clone, Default)]
struct Visto {
    caminho: String,
    autorizacao: Option<String>,
    modelo: Option<String>,
}

const SSE: [&str; 3] = [
    "event: response.output_text.delta\ndata: {\"delta\":\"ol\"}\n\n",
    "event: response.output_text.delta\ndata: {\"delta\":\"á\"}\n\n",
    "event: response.completed\ndata: {}\n\n",
];

/// Uma fonte falsa que anota o que recebeu e responde um SSE em três pedaços.
async fn fonte_falsa() -> (String, Arc<Mutex<Vec<Visto>>>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let porta = listener.local_addr().unwrap().port();
    let vistos = Arc::new(Mutex::new(Vec::new()));
    let anotar = vistos.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let anotar = anotar.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: Request<Incoming>| {
                    let anotar = anotar.clone();
                    async move {
                        let caminho = req.uri().path().to_string();
                        let autorizacao = req
                            .headers()
                            .get("authorization")
                            .map(|v| v.to_str().unwrap().to_string());
                        let corpo = req.into_body().collect().await.unwrap().to_bytes();
                        let json: Value = serde_json::from_slice(&corpo).unwrap_or(Value::Null);
                        anotar.lock().unwrap().push(Visto {
                            caminho,
                            autorizacao,
                            modelo: json["model"].as_str().map(str::to_string),
                        });
                        let (tx, rx) =
                            tokio::sync::mpsc::channel::<Result<Frame<Bytes>, Infallible>>(4);
                        tokio::spawn(async move {
                            for p in SSE {
                                tokio::time::sleep(Duration::from_millis(30)).await;
                                let _ = tx.send(Ok(Frame::data(Bytes::from(p)))).await;
                            }
                        });
                        let corpo = StreamBody::new(tokio_stream_de(rx));
                        Ok::<_, Infallible>(
                            Response::builder()
                                .header("content-type", "text/event-stream")
                                .body(corpo)
                                .unwrap(),
                        )
                    }
                });
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), svc)
                    .await;
            });
        }
    });
    (format!("http://127.0.0.1:{porta}/v1"), vistos)
}

fn tokio_stream_de(
    mut rx: tokio::sync::mpsc::Receiver<Result<Frame<Bytes>, Infallible>>,
) -> impl futures_util::Stream<Item = Result<Frame<Bytes>, Infallible>> {
    futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx))
}

async fn gateway(base: &str) -> Gateway {
    Gateway::iniciar(
        0,
        Configuracao {
            token: "tok-certo".into(),
            rotas: vec![
                Rota {
                    prefixo: "local".into(),
                    base_url: base.into(),
                    chave: Some("sk-local-abc".into()),
                    modelos: vec!["qwen3-coder".into()],
                },
                Rota {
                    prefixo: "openrouter".into(),
                    base_url: base.into(),
                    chave: Some("sk-or-123".into()),
                    modelos: vec!["openai/gpt-oss-20b:free".into()],
                },
            ],
        },
    )
    .await
    .unwrap()
}

async fn post(gw: &Gateway, token: Option<&str>, modelo: &str) -> reqwest::Response {
    let mut r = reqwest::Client::new()
        .post(format!("{}/responses", gw.base_url()))
        .json(&serde_json::json!({ "model": modelo, "input": "oi", "stream": true }));
    if let Some(t) = token {
        r = r.bearer_auth(t);
    }
    r.send().await.unwrap()
}

#[tokio::test]
async fn sem_token_ou_com_token_errado_e_401() {
    let (base, vistos) = fonte_falsa().await;
    let gw = gateway(&base).await;
    assert_eq!(post(&gw, None, "local:qwen3-coder").await.status(), 401);
    assert_eq!(
        post(&gw, Some("tok-errado"), "local:qwen3-coder")
            .await
            .status(),
        401
    );
    assert!(
        vistos.lock().unwrap().is_empty(),
        "nada pode chegar à fonte"
    );
}

#[tokio::test]
async fn rota_pelo_prefixo_troca_o_modelo_e_injeta_a_chave_da_fonte() {
    let (base, vistos) = fonte_falsa().await;
    let gw = gateway(&base).await;

    let r = post(&gw, Some("tok-certo"), "local:qwen3-coder").await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["content-type"], "text/event-stream");
    let corpo = r.bytes().await.unwrap();
    assert_eq!(
        corpo,
        SSE.concat().as_bytes(),
        "o SSE atravessa byte a byte"
    );

    let r = post(&gw, Some("tok-certo"), "openrouter:openai/gpt-oss-20b:free").await;
    assert_eq!(r.status(), 200);
    let _ = r.bytes().await;

    let v = vistos.lock().unwrap().clone();
    assert_eq!(v[0].caminho, "/v1/responses");
    assert_eq!(v[0].modelo.as_deref(), Some("qwen3-coder"));
    assert_eq!(v[0].autorizacao.as_deref(), Some("Bearer sk-local-abc"));
    // O `:` do id do OpenRouter sobrevive: só o primeiro é o prefixo.
    assert_eq!(v[1].modelo.as_deref(), Some("openai/gpt-oss-20b:free"));
    assert_eq!(v[1].autorizacao.as_deref(), Some("Bearer sk-or-123"));
}

#[tokio::test]
async fn a_resposta_chega_em_pedacos_e_nao_de_uma_vez() {
    let (base, _) = fonte_falsa().await;
    let gw = gateway(&base).await;
    let mut r = post(&gw, Some("tok-certo"), "local:qwen3-coder").await;
    let primeiro = r.chunk().await.unwrap().unwrap();
    assert_eq!(
        primeiro,
        SSE[0].as_bytes(),
        "o primeiro evento não esperou os outros"
    );
}

#[tokio::test]
async fn fonte_desconhecida_e_modelo_sem_prefixo_dizem_o_que_falta() {
    let (base, vistos) = fonte_falsa().await;
    let gw = gateway(&base).await;
    let r = post(&gw, Some("tok-certo"), "ninerouter:x").await;
    assert_eq!(r.status(), 404);
    let erro: Value = r.json().await.unwrap();
    assert!(
        erro["error"]["message"]
            .as_str()
            .unwrap()
            .contains("ninerouter")
    );
    assert_eq!(
        post(&gw, Some("tok-certo"), "qwen3-coder").await.status(),
        404
    );
    assert!(vistos.lock().unwrap().is_empty());
}

#[tokio::test]
async fn models_lista_com_prefixo_e_atualizar_troca_o_token() {
    let (base, _) = fonte_falsa().await;
    let gw = gateway(&base).await;
    let lista: Value = reqwest::Client::new()
        .get(format!("{}/models", gw.base_url()))
        .bearer_auth("tok-certo")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ids: Vec<&str> = lista["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        ["local:qwen3-coder", "openrouter:openai/gpt-oss-20b:free"]
    );

    gw.atualizar(Configuracao {
        token: "tok-novo".into(),
        rotas: vec![],
    });
    assert_eq!(
        post(&gw, Some("tok-certo"), "local:qwen3-coder")
            .await
            .status(),
        401
    );
    assert_eq!(
        post(&gw, Some("tok-novo"), "local:qwen3-coder")
            .await
            .status(),
        404
    );
}

#[test]
fn token_novo_tem_64_hex_e_nao_se_repete() {
    let a = lr_owgw::novo_token().unwrap();
    let b = lr_owgw::novo_token().unwrap();
    assert_eq!(a.len(), 64);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a, b);
}
