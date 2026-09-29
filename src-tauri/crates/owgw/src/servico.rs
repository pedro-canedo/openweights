//! O atendimento de cada requisição do gateway.

use std::convert::Infallible;
use std::sync::Arc;

use bytes::Bytes;
use futures_util::StreamExt;
use http::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, HeaderValue};
use http::{Method, Request, Response, StatusCode};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full, Limited, StreamBody};
use hyper::body::{Frame, Incoming};
use serde_json::{Value, json};

use crate::{Configuracao, Ctx, PREFIXO_DO_CAMINHO};

pub(crate) type Corpo = BoxBody<Bytes, std::io::Error>;

/// Teto do corpo de uma requisição. Uma conversa longa com o histórico inteiro
/// passa de alguns MB; 64 MB separa isso de um corpo absurdo.
const TETO_DO_CORPO: usize = 64 * 1024 * 1024;

struct Falha {
    status: StatusCode,
    mensagem: String,
}

fn falha(status: StatusCode, mensagem: impl Into<String>) -> Falha {
    Falha {
        status,
        mensagem: mensagem.into(),
    }
}

pub(crate) async fn atender(
    req: Request<Incoming>,
    ctx: Arc<Ctx>,
) -> Result<Response<Corpo>, Infallible> {
    Ok(match tratar(req, &ctx).await {
        Ok(r) => r,
        Err(f) => json_de(
            f.status,
            &json!({ "error": { "message": f.mensagem, "type": "openweights_gateway" } }),
        ),
    })
}

fn json_de(status: StatusCode, valor: &Value) -> Response<Corpo> {
    let mut r = Response::new(
        Full::new(Bytes::from(valor.to_string()))
            .map_err(|never| match never {})
            .boxed(),
    );
    *r.status_mut() = status;
    r.headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    r
}

/// Comparação que não entrega, pelo tempo, quantos caracteres acertaram.
fn iguais(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn autenticado(req: &Request<Incoming>, token: &str) -> bool {
    req.headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| !token.is_empty() && iguais(t.trim().as_bytes(), token.as_bytes()))
}

async fn tratar(req: Request<Incoming>, ctx: &Ctx) -> Result<Response<Corpo>, Falha> {
    let caminho = req.uri().path().to_string();
    let Some(resto) = caminho.strip_prefix(PREFIXO_DO_CAMINHO) else {
        return Err(falha(
            StatusCode::NOT_FOUND,
            "caminho fora do gateway do OwCLI",
        ));
    };
    let resto = resto.to_string();
    let config = ctx.config.read().unwrap_or_else(|e| e.into_inner()).clone();
    if !autenticado(&req, &config.token) {
        return Err(falha(
            StatusCode::UNAUTHORIZED,
            "token do OwCLI ausente ou inválido — abra o OpenWeights",
        ));
    }
    match (req.method().clone(), resto.as_str()) {
        (Method::GET, "/models") => Ok(json_de(StatusCode::OK, &listar(&config))),
        (Method::POST, _) => encaminhar(req, &resto, &config, &ctx.http).await,
        _ => Err(falha(StatusCode::METHOD_NOT_ALLOWED, "método não atendido")),
    }
}

fn listar(config: &Configuracao) -> Value {
    let data: Vec<Value> = config
        .rotas
        .iter()
        .flat_map(|r| {
            r.modelos.iter().map(move |m| {
                json!({ "id": format!("{}:{m}", r.prefixo), "object": "model", "owned_by": r.prefixo })
            })
        })
        .collect();
    json!({ "object": "list", "data": data })
}

async fn encaminhar(
    req: Request<Incoming>,
    resto: &str,
    config: &Configuracao,
    http: &reqwest::Client,
) -> Result<Response<Corpo>, Falha> {
    let consulta = req
        .uri()
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    let aceita = req.headers().get(http::header::ACCEPT).cloned();
    let bytes = Limited::new(req.into_body(), TETO_DO_CORPO)
        .collect()
        .await
        .map_err(|e| {
            if e.downcast_ref::<http_body_util::LengthLimitError>()
                .is_some()
            {
                falha(StatusCode::PAYLOAD_TOO_LARGE, "corpo grande demais")
            } else {
                falha(StatusCode::BAD_REQUEST, format!("corpo ilegível: {e}"))
            }
        })?
        .to_bytes();
    let mut corpo: Value = serde_json::from_slice(&bytes)
        .map_err(|e| falha(StatusCode::BAD_REQUEST, format!("JSON inválido: {e}")))?;
    let modelo = corpo
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| falha(StatusCode::BAD_REQUEST, "requisição sem `model`"))?
        .to_string();
    // O prefixo é o que vem antes do PRIMEIRO `:` — ids do OpenRouter têm `:`
    // próprio (`openai/gpt-oss-20b:free`).
    let (prefixo, id) = modelo.split_once(':').ok_or_else(|| {
        falha(
            StatusCode::NOT_FOUND,
            format!("modelo `{modelo}` sem fonte (esperado `local:…`, `openrouter:…`)"),
        )
    })?;
    let rota = config
        .rotas
        .iter()
        .find(|r| r.prefixo == prefixo)
        .ok_or_else(|| {
            falha(
                StatusCode::NOT_FOUND,
                format!("a fonte `{prefixo}` não está disponível no OpenWeights agora"),
            )
        })?;
    corpo["model"] = Value::String(id.to_string());
    // O llama.cpp entrega o chat template do modelo mensagens de sistema em
    // posições que vários templates recusam (ver `reescritas`).
    if prefixo == "local"
        && resto == "/responses"
        && crate::reescritas::unificar_sistema(&mut corpo)
    {
        log::debug!("gateway: mensagens de sistema reunidas em `instructions`");
    }
    let url = format!("{}{resto}{consulta}", rota.base_url.trim_end_matches('/'));

    let mut pedido = http
        .post(url)
        .header(CONTENT_TYPE, "application/json")
        .body(serde_json::to_vec(&corpo).unwrap_or_default());
    if let Some(a) = aceita {
        pedido = pedido.header(http::header::ACCEPT, a);
    }
    if let Some(chave) = rota.chave.as_deref().filter(|c| !c.is_empty()) {
        pedido = pedido.bearer_auth(chave);
    }
    if prefixo == "openrouter" {
        // Atribuição que o OpenRouter pede aos aplicativos.
        pedido = pedido
            .header(
                "HTTP-Referer",
                "https://github.com/pedro-canedo/openweights",
            )
            .header("X-Title", "OwCLI (OpenWeights)");
    }
    let resposta = pedido.send().await.map_err(|e| {
        falha(
            StatusCode::BAD_GATEWAY,
            format!("a fonte `{prefixo}` não respondeu: {e}"),
        )
    })?;

    let status = resposta.status();
    let mut saida = Response::builder().status(status.as_u16());
    for (nome, valor) in resposta.headers() {
        // Comprimento e codificação de transferência são da conexão de cá.
        if nome == CONTENT_LENGTH
            || nome == http::header::TRANSFER_ENCODING
            || nome == http::header::CONNECTION
        {
            continue;
        }
        saida = saida.header(nome, valor);
    }
    let fluxo = resposta
        .bytes_stream()
        .map(|pedaco| pedaco.map(Frame::data).map_err(std::io::Error::other));
    saida
        .body(BodyExt::boxed(StreamBody::new(fluxo)))
        .map_err(|e| falha(StatusCode::BAD_GATEWAY, e.to_string()))
}
