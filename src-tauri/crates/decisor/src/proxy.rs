//! O tratamento de cada requisição: passagem direta para tudo, decisão e
//! reescrita só no `chat/completions`.

use std::convert::Infallible;
use std::sync::Arc;

use bytes::Bytes;
use http::header::{
    CONNECTION, CONTENT_LENGTH, CONTENT_TYPE, HOST, HeaderName, HeaderValue, TRANSFER_ENCODING,
};
use http::{Method, Request, Response, StatusCode, Uri};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use serde_json::Value;

use lr_providers::jev_esforco::decidir_esforco;
use lr_providers::{ContextoDecisao, DecisaoEsforco, Superficie};

use crate::reescrita::{chave_da_conversa, reescrever, resumir_mensagens, tem_ferramentas};
use crate::{Ctx, TETO_CORPO};

pub(crate) type Corpo = BoxBody<Bytes, hyper::Error>;

/// Cabeçalho de resposta que conta o que o proxy fez.
pub const CABECALHO_JEV: &str = "x-openweights-jev";

/// Cabeçalhos que pertencem a UMA conexão e não podem atravessar um proxy
/// (RFC 7230 §6.1). `host` entra porque o cliente upstream monta o seu.
const SALTO_A_SALTO: [HeaderName; 9] = [
    CONNECTION,
    HOST,
    TRANSFER_ENCODING,
    http::header::PROXY_AUTHENTICATE,
    http::header::PROXY_AUTHORIZATION,
    http::header::TE,
    http::header::TRAILER,
    http::header::UPGRADE,
    HeaderName::from_static("keep-alive"),
];

#[derive(Debug, thiserror::Error)]
enum ErroProxy {
    #[error("servidor local indisponível: {0}")]
    Upstream(#[from] hyper_util::client::legacy::Error),
    #[error("requisição malformada: {0}")]
    Http(#[from] http::Error),
    #[error("URL inválida: {0}")]
    Uri(#[from] http::uri::InvalidUri),
    #[error("corpo maior que o teto de {TETO_CORPO} bytes")]
    CorpoGrande,
    #[error("falha ao ler o corpo: {0}")]
    Corpo(String),
}

impl ErroProxy {
    fn status(&self) -> StatusCode {
        match self {
            Self::Upstream(_) => StatusCode::BAD_GATEWAY,
            Self::CorpoGrande => StatusCode::PAYLOAD_TOO_LARGE,
            Self::Http(_) | Self::Uri(_) | Self::Corpo(_) => StatusCode::BAD_REQUEST,
        }
    }
}

pub(crate) async fn atender(
    req: Request<Incoming>,
    ctx: Arc<Ctx>,
) -> Result<Response<Corpo>, Infallible> {
    Ok(match tratar(req, &ctx).await {
        Ok(r) => r,
        Err(e) => resposta_de_erro(&e),
    })
}

fn cheio(bytes: Bytes) -> Corpo {
    Full::new(bytes).map_err(|nunca| match nunca {}).boxed()
}

fn resposta_de_erro(e: &ErroProxy) -> Response<Corpo> {
    let corpo = serde_json::json!({
        "error": { "message": e.to_string(), "type": "proxy_error" }
    });
    Response::builder()
        .status(e.status())
        .header(CONTENT_TYPE, "application/json")
        .body(cheio(Bytes::from(corpo.to_string())))
        .unwrap_or_else(|_| Response::new(cheio(Bytes::new())))
}

fn e_chat_completions(metodo: &Method, caminho: &str) -> bool {
    *metodo == Method::POST && matches!(caminho, "/v1/chat/completions" | "/chat/completions")
}

async fn tratar(req: Request<Incoming>, ctx: &Ctx) -> Result<Response<Corpo>, ErroProxy> {
    let (mut partes, corpo) = req.into_parts();
    let chat = e_chat_completions(&partes.method, partes.uri.path());

    let (corpo, marca): (Corpo, Option<String>) = if chat {
        let bytes = Limited::new(corpo, TETO_CORPO)
            .collect()
            .await
            .map_err(|e| {
                if e.downcast_ref::<http_body_util::LengthLimitError>()
                    .is_some()
                {
                    ErroProxy::CorpoGrande
                } else {
                    ErroProxy::Corpo(e.to_string())
                }
            })?
            .to_bytes();
        let (bytes, marca) = decidir_e_reescrever(bytes, ctx).await;
        partes.headers.remove(TRANSFER_ENCODING);
        partes
            .headers
            .insert(CONTENT_LENGTH, HeaderValue::from(bytes.len()));
        (cheio(bytes), Some(marca))
    } else {
        (corpo.boxed(), None)
    };

    let caminho = partes
        .uri
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("/");
    let uri: Uri = format!("{}{caminho}", ctx.upstream).parse()?;
    let mut pedido = Request::builder().method(partes.method).uri(uri);
    for (nome, valor) in &partes.headers {
        if !SALTO_A_SALTO.contains(nome) {
            pedido = pedido.header(nome, valor);
        }
    }
    let pedido = pedido.body(corpo)?;

    let resposta = ctx.http.request(pedido).await?;
    let (mut rp, rb) = resposta.into_parts();
    for nome in &SALTO_A_SALTO {
        rp.headers.remove(nome);
    }
    if let Some(v) = marca.and_then(|m| HeaderValue::from_str(&ascii(&m)).ok()) {
        rp.headers.insert(CABECALHO_JEV, v);
    }
    Ok(Response::from_parts(rp, rb.boxed()))
}

/// Só ASCII imprimível: um cabeçalho HTTP não carrega acento.
fn ascii(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '?'
            }
        })
        .take(200)
        .collect()
}

/// Decide e reescreve. Devolve os bytes a encaminhar (os originais quando
/// nada foi decidido) e a marca do cabeçalho.
async fn decidir_e_reescrever(bytes: Bytes, ctx: &Ctx) -> (Bytes, String) {
    let Ok(mut corpo) = serde_json::from_slice::<Value>(&bytes) else {
        return (bytes, "default;corpo ilegivel".into());
    };
    if corpo.get("messages").and_then(Value::as_array).is_none() {
        return (bytes, "default;sem messages".into());
    }
    let (cliente, min) = {
        let p = ctx.politica.read().await;
        (p.jev.clone(), p.min_confianca)
    };
    let Some(cliente) = cliente else {
        return (bytes, "default;jev desligado".into());
    };
    let modelo = corpo
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let cap = (ctx.capacidade)(&modelo);
    if !cap.pode_raciocinar() {
        return (bytes, "default;modelo sem raciocinio".into());
    }

    let mensagens = resumir_mensagens(&corpo);
    let chave = chave_da_conversa(&mensagens);
    let lembrado = match chave {
        Some(k) => ctx.lembrar(k).await,
        None => None,
    };
    let (nivel, marca) = match lembrado {
        Some(n) => (n, "cache".to_string()),
        None => {
            let contexto = ContextoDecisao {
                tem_ferramentas: tem_ferramentas(&corpo),
                ..Default::default()
            };
            let decisao = match tokio::time::timeout(
                ctx.tempo_decisao,
                decidir_esforco(
                    &cliente,
                    &mensagens,
                    &contexto,
                    min,
                    Superficie::Proxy,
                    Some(&ctx.contadores),
                ),
            )
            .await
            {
                Ok(d) => d,
                Err(_) => DecisaoEsforco::padrao("prazo da decisao esgotado"),
            };
            match decisao.nivel.filter(|_| decisao.aplicada()) {
                Some(n) => {
                    if let Some(k) = chave {
                        ctx.guardar(k, n).await;
                    }
                    (n, format!("{:.2}", decisao.confianca.unwrap_or(0.0)))
                }
                None => {
                    let motivo = decisao.motivo.unwrap_or_else(|| "sem decisao".into());
                    log::debug!("proxy do Jev manteve o padrão: {motivo}");
                    return (bytes, format!("default;{motivo}"));
                }
            }
        }
    };

    reescrever(&mut corpo, nivel, Some(&cap));
    log::debug!("proxy do Jev: {} ({marca}) para {modelo}", nivel.as_str());
    match serde_json::to_vec(&corpo) {
        Ok(v) => (Bytes::from(v), format!("{};{marca}", nivel.as_str())),
        Err(_) => (bytes, "default;falha ao serializar".into()),
    }
}
