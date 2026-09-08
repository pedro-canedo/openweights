//! Entrar com a conta do Hugging Face — o token sem copiar e colar.
//!
//! O que fazia o app pedir um token colado à mão não era falta de padrão: o
//! Hub fala OpenID Connect inteiro. Verificado ao vivo em 2026-09-07, no
//! `/.well-known/openid-configuration`:
//!
//! - `authorization_endpoint` + `token_endpoint`, com PKCE `S256`;
//! - `registration_endpoint` — **registro dinâmico de cliente**, aberto: um
//!   POST anônimo devolve `client_id` e `client_secret` (HTTP 201). É o que
//!   dispensa o usuário de ir criar uma "OAuth App" no site antes de usar o
//!   login. O app se registra sozinho, uma vez, e guarda o registro;
//! - `http://127.0.0.1:<porta>/callback` é aceito como redirect, e o registro
//!   aceita uma LISTA de redirects — daí registrar todas as portas de uma vez
//!   e usar a primeira livre, em vez de re-registrar a cada login;
//! - `refresh_token` entre os `grant_types`, então a sessão se renova sem
//!   mandar a pessoa logar de novo no meio de um download de 70 GB.
//!
//! O que este módulo NÃO faz — de propósito — é aceitar a licença por
//! ninguém. Aceitar é um ato da pessoa sobre um contrato, muitas vezes com
//! formulário e aprovação do autor; automatizar o clique seria assinar em
//! nome dela. O que dá para automatizar é tudo em volta: abrir a página
//! certa, perceber sozinho quando o portão abriu e seguir com o download.

use crate::{HF_BASE, ModelsError};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// As portas que o app registra para receber a volta do navegador.
///
/// São fixas porque o redirect precisa estar no registro, e o registro é
/// guardado entre sessões: porta sorteada obrigaria a registrar de novo toda
/// vez. Quatro bastam para o caso de outra coisa já estar ouvindo em uma.
const PORTAS: [u16; 4] = [53682, 53683, 53684, 53685];

/// `read-repos` é o que dá acesso ao conteúdo dos repositórios em nome da
/// pessoa — inclusive os com licença que ela já aceitou. `openid profile`
/// vem junto porque é o que devolve o nome da conta para a tela mostrar.
const SCOPE: &str = "openid profile read-repos";

/// O registro deste app no Hub, obtido uma vez e guardado.
///
/// O `client_secret` não é segredo de verdade num app que roda na máquina da
/// pessoa — e por isso o fluxo usa PKCE, que é o que impede um código
/// interceptado de virar token. O registro é de ESTA instalação: quem apagar
/// os dados do app simplesmente registra outro.
///
/// Os nomes dos campos ficam como o OAuth os escreve (`client_id`), e não em
/// camelCase como o resto do app: esta struct é lida DIRETO da resposta do
/// Hub, e renomear aqui foi exatamente o que quebrou o registro no primeiro
/// teste contra o servidor de verdade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OauthClient {
    pub client_id: String,
    pub client_secret: String,
}

/// A sessão viva: o token que vai nas requisições e como renová-lo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HfSession {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Quando o `access_token` deixa de valer, em ms desde a época. Zero
    /// quando o Hub não disse — aí não há o que antecipar, e a renovação só
    /// acontece se uma chamada voltar 401.
    pub expires_at_ms: i64,
    /// O registro que emitiu esta sessão; é ele que renova.
    pub client: OauthClient,
}

impl HfSession {
    /// Se vale a pena renovar antes de usar.
    ///
    /// A folga de um minuto é para o token não expirar no meio da requisição
    /// que ele acabou de autorizar.
    pub fn expirando(&self, agora_ms: i64) -> bool {
        self.expires_at_ms > 0 && agora_ms + 60_000 >= self.expires_at_ms
    }
}

/// Registra este app no Hub (uma vez por instalação).
pub async fn register(http: &reqwest::Client) -> Result<OauthClient, ModelsError> {
    let redirects: Vec<String> = PORTAS.iter().map(|p| redirect_uri(*p)).collect();
    let corpo = serde_json::json!({
        "client_name": "OpenWeights",
        "redirect_uris": redirects,
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "client_secret_post",
        "scope": SCOPE,
    });
    let resp = http
        .post(format!("{HF_BASE}/oauth/register"))
        .json(&corpo)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(ModelsError::Api(format!(
            "registro do app retornou HTTP {}",
            resp.status()
        )));
    }
    Ok(resp.json::<OauthClient>().await?)
}

/// Um login em andamento: a URL para abrir e a porta que espera a volta.
pub struct Login {
    listener: TcpListener,
    redirect_uri: String,
    verifier: String,
    state: String,
    client: OauthClient,
    url: String,
}

impl Login {
    /// Prepara o login: garante o registro, abre a porta e monta a URL.
    ///
    /// A porta é aberta ANTES de o navegador ir para o Hub. Ao contrário,
    /// haveria uma janela em que a pessoa já autorizou e ninguém está
    /// escutando — e o erro apareceria como uma página de conexão recusada,
    /// que ninguém consegue interpretar.
    pub async fn start(
        http: &reqwest::Client,
        registrado: Option<OauthClient>,
    ) -> Result<Self, ModelsError> {
        let (listener, porta) = ouvir().await?;
        let client = match registrado {
            Some(c) => c,
            None => register(http).await?,
        };
        let verifier = aleatorio_url_safe();
        let state = aleatorio_url_safe();
        let redirect_uri = redirect_uri(porta);
        let url = format!(
            "{HF_BASE}/oauth/authorize?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
            urlencode(&client.client_id),
            urlencode(&redirect_uri),
            urlencode(SCOPE),
            urlencode(&state),
            urlencode(&challenge(&verifier)),
        );
        Ok(Self {
            listener,
            redirect_uri,
            verifier,
            state,
            client,
            url,
        })
    }

    /// A URL a abrir no navegador da pessoa.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// O registro em uso — para quem chamou guardá-lo e não registrar de novo.
    pub fn client(&self) -> &OauthClient {
        &self.client
    }

    /// Espera a volta do navegador e troca o código pela sessão.
    pub async fn finish(
        self,
        http: &reqwest::Client,
        espera: Duration,
    ) -> Result<HfSession, ModelsError> {
        let code = tokio::time::timeout(espera, receber_codigo(&self.listener, &self.state))
            .await
            .map_err(|_| ModelsError::Api("o login expirou sem resposta do navegador".into()))??;

        let client = self.client.clone();
        let form = [
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &self.redirect_uri),
            ("client_id", &self.client.client_id),
            ("client_secret", &self.client.client_secret),
            ("code_verifier", &self.verifier),
        ];
        trocar(http, &form, client).await
    }
}

/// Renova a sessão com o `refresh_token`, sem passar pelo navegador.
pub async fn refresh(http: &reqwest::Client, sessao: &HfSession) -> Result<HfSession, ModelsError> {
    let Some(rt) = sessao.refresh_token.as_deref() else {
        return Err(ModelsError::Api("sessão sem refresh token".into()));
    };
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", rt),
        ("client_id", sessao.client.client_id.as_str()),
        ("client_secret", sessao.client.client_secret.as_str()),
    ];
    trocar(http, &form, sessao.client.clone()).await
}

/// O POST no `token_endpoint`, que é o mesmo nas duas trocas.
async fn trocar(
    http: &reqwest::Client,
    form: &[(&str, &str)],
    client: OauthClient,
) -> Result<HfSession, ModelsError> {
    #[derive(Deserialize)]
    struct Resp {
        access_token: String,
        #[serde(default)]
        refresh_token: Option<String>,
        #[serde(default)]
        expires_in: Option<i64>,
    }
    let resp = http
        .post(format!("{HF_BASE}/oauth/token"))
        .form(form)
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        // O corpo do erro do OAuth diz qual é o problema (`invalid_grant`
        // quando a sessão foi revogada, por exemplo); sem ele sobra o número.
        let detalhe = resp.text().await.unwrap_or_default();
        let detalhe: String = detalhe.chars().take(200).collect();
        return Err(ModelsError::Api(format!(
            "troca de token retornou HTTP {status}: {detalhe}"
        )));
    }
    let t: Resp = resp.json().await?;
    Ok(HfSession {
        access_token: t.access_token,
        refresh_token: t.refresh_token,
        expires_at_ms: t
            .expires_in
            .map(|s| agora_ms() + s.saturating_mul(1000))
            .unwrap_or(0),
        client,
    })
}

// ------------------------------------------------------------ callback ---

/// Abre a primeira porta livre da lista registrada.
async fn ouvir() -> Result<(TcpListener, u16), ModelsError> {
    for porta in PORTAS {
        if let Ok(l) = TcpListener::bind(("127.0.0.1", porta)).await {
            return Ok((l, porta));
        }
    }
    Err(ModelsError::Api(
        "nenhuma porta local livre para receber o login".into(),
    ))
}

fn redirect_uri(porta: u16) -> String {
    format!("http://127.0.0.1:{porta}/callback")
}

/// Espera a volta do navegador e devolve o código de autorização.
///
/// Aceita em laço porque nem toda conexão é a resposta: o navegador costuma
/// pedir `/favicon.ico` na mesma porta, e desistir na primeira conexão
/// deixaria o login pendurado esperando algo que já chegou.
async fn receber_codigo(listener: &TcpListener, state: &str) -> Result<String, ModelsError> {
    loop {
        let (mut sock, _) = listener.accept().await?;
        let mut buf = vec![0u8; 4096];
        let n = sock.read(&mut buf).await?;
        let pedido = String::from_utf8_lossy(&buf[..n]).to_string();
        let Some(alvo) = linha_de_pedido(&pedido) else {
            let _ = responder(&mut sock, 400, "pedido inválido").await;
            continue;
        };
        if !alvo.starts_with("/callback") {
            let _ = responder(&mut sock, 404, "não é aqui").await;
            continue;
        }
        let params = query(&alvo);
        let achar = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());

        if let Some(erro) = achar("error") {
            responder(&mut sock, 400, "Login recusado. Pode fechar esta aba.").await?;
            return Err(ModelsError::Api(format!("login recusado pelo Hub: {erro}")));
        }
        // O `state` é o que garante que este código é resposta ao NOSSO
        // pedido, e não a um que alguém induziu o navegador a fazer.
        if achar("state").as_deref() != Some(state) {
            responder(&mut sock, 400, "Resposta inesperada. Pode fechar esta aba.").await?;
            return Err(ModelsError::Api("state do login não confere".into()));
        }
        let Some(code) = achar("code") else {
            responder(&mut sock, 400, "Resposta sem código. Pode fechar esta aba.").await?;
            return Err(ModelsError::Api("callback sem código".into()));
        };
        responder(
            &mut sock,
            200,
            "Conta conectada. Pode fechar esta aba e voltar ao OpenWeights.",
        )
        .await?;
        return Ok(code);
    }
}

/// O alvo da requisição (`GET <alvo> HTTP/1.1`).
fn linha_de_pedido(pedido: &str) -> Option<String> {
    let primeira = pedido.lines().next()?;
    let mut partes = primeira.split_whitespace();
    let metodo = partes.next()?;
    let alvo = partes.next()?;
    (metodo == "GET").then(|| alvo.to_string())
}

/// Os pares da query string, já decodificados.
fn query(alvo: &str) -> Vec<(String, String)> {
    let Some((_, qs)) = alvo.split_once('?') else {
        return Vec::new();
    };
    qs.split('&')
        .filter_map(|par| par.split_once('='))
        .map(|(k, v)| (urldecode(k), urldecode(v)))
        .collect()
}

async fn responder(
    sock: &mut tokio::net::TcpStream,
    status: u16,
    texto: &str,
) -> Result<(), ModelsError> {
    let corpo = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>OpenWeights</title>\
         <body style=\"font:16px system-ui;background:#111318;color:#e6e8ee;\
         display:grid;place-items:center;height:100vh;margin:0\">\
         <p>{texto}</p>"
    );
    let resp = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{corpo}",
        corpo.len()
    );
    sock.write_all(resp.as_bytes()).await?;
    let _ = sock.flush().await;
    Ok(())
}

// -------------------------------------------------------------- PKCE ---

/// 32 bytes de acaso em base64url — serve de `code_verifier` e de `state`.
fn aleatorio_url_safe() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("fonte de aleatoriedade do sistema");
    base64url(&bytes)
}

/// O desafio PKCE: base64url(sha256(verifier)).
fn challenge(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    base64url(&Sha256::digest(verifier.as_bytes()))
}

/// Base64 na variante URL, sem preenchimento — o que o PKCE pede.
///
/// São vinte linhas contra uma dependência que só faria isto; o teste abaixo
/// usa os vetores do RFC 4648 e o exemplo do próprio RFC 7636.
fn base64url(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for pedaco in bytes.chunks(3) {
        let b = [
            pedaco[0],
            pedaco.get(1).copied().unwrap_or(0),
            pedaco.get(2).copied().unwrap_or(0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        let saida = pedaco.len() + 1; // 3 bytes → 4 caracteres; 2 → 3; 1 → 2.
        for i in 0..saida {
            s.push(A[(n >> (18 - 6 * i) & 0x3F) as usize] as char);
        }
    }
    s
}

fn agora_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn urlencode(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => vec![c],
            outro => format!("%{:02X}", outro as u32).chars().collect(),
        })
        .collect()
}

fn urldecode(s: &str) -> String {
    let bytes = s.replace('+', " ").into_bytes();
    let mut saida: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                saida.push(b);
                i += 3;
                continue;
            }
        }
        saida.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&saida).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vetores do RFC 4648 (base64url) e o exemplo do RFC 7636 (PKCE).
    #[test]
    fn base64url_matches_the_rfc_vectors() {
        assert_eq!(base64url(b""), "");
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        assert_eq!(base64url(b"foob"), "Zm9vYg");
        assert_eq!(base64url(b"fooba"), "Zm9vYmE");
        assert_eq!(base64url(b"foobar"), "Zm9vYmFy");
        // Sem '+' nem '/': o alfabeto é o de URL, e é isso que faz o desafio
        // sobreviver à query string sem escape.
        assert_eq!(base64url(&[0xFB, 0xFF, 0xFE]), "-__-");

        // RFC 7636, apêndice B: este verifier tem este desafio.
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    /// O callback é HTTP cru lido de um socket: o que chega é uma linha de
    /// pedido, e é dela que sai o código.
    #[test]
    fn the_callback_request_is_parsed_into_params() {
        let pedido = "GET /callback?code=abc%2F123&state=xyz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        let alvo = linha_de_pedido(pedido).unwrap();
        assert!(alvo.starts_with("/callback"));
        let p = query(&alvo);
        assert_eq!(p[0], ("code".into(), "abc/123".into()));
        assert_eq!(p[1], ("state".into(), "xyz".into()));

        // O favicon que o navegador pede na mesma porta não é o callback.
        assert_eq!(
            linha_de_pedido("GET /favicon.ico HTTP/1.1\r\n\r\n").as_deref(),
            Some("/favicon.ico")
        );
        assert_eq!(linha_de_pedido("lixo"), None);
    }

    /// O callback é HTTP cru num socket local: este teste faz o papel do
    /// navegador, inclusive o pedido de favicon que ele manda antes — foi
    /// justamente isso que deixaria um laço ingênuo pendurado.
    #[tokio::test]
    async fn the_browser_callback_hands_over_the_code() {
        use tokio::net::TcpStream;

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let porta = listener.local_addr().unwrap().port();
        let tarefa = tokio::spawn(async move { receber_codigo(&listener, "st4te").await });

        let mut favicon = TcpStream::connect(("127.0.0.1", porta)).await.unwrap();
        favicon
            .write_all(b"GET /favicon.ico HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();

        let mut volta = TcpStream::connect(("127.0.0.1", porta)).await.unwrap();
        volta
            .write_all(b"GET /callback?code=c0de&state=st4te HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();

        assert_eq!(tarefa.await.unwrap().unwrap(), "c0de");
        // E o navegador recebe uma página, não uma conexão cortada.
        let mut resposta = String::new();
        volta.read_to_string(&mut resposta).await.unwrap();
        assert!(resposta.starts_with("HTTP/1.1 200"), "{resposta}");
    }

    /// Um código que volta com outro `state` não é resposta ao nosso pedido.
    #[tokio::test]
    async fn a_callback_with_the_wrong_state_is_refused() {
        use tokio::net::TcpStream;

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let porta = listener.local_addr().unwrap().port();
        let tarefa = tokio::spawn(async move { receber_codigo(&listener, "esperado").await });

        let mut volta = TcpStream::connect(("127.0.0.1", porta)).await.unwrap();
        volta
            .write_all(b"GET /callback?code=c0de&state=outro HTTP/1.1\r\n\r\n")
            .await
            .unwrap();

        assert!(tarefa.await.unwrap().is_err());
    }

    #[test]
    fn a_session_renews_before_it_expires_not_after() {
        let s = HfSession {
            access_token: "t".into(),
            refresh_token: Some("r".into()),
            expires_at_ms: 1_000_000,
            client: OauthClient {
                client_id: "c".into(),
                client_secret: "s".into(),
            },
        };
        assert!(!s.expirando(800_000));
        // Um minuto antes de vencer já conta como vencida: o token não pode
        // expirar no meio da requisição que ele acabou de autorizar.
        assert!(s.expirando(950_000));
        assert!(s.expirando(1_100_000));

        // Sem prazo declarado não há o que antecipar.
        let sem_prazo = HfSession {
            expires_at_ms: 0,
            ..s
        };
        assert!(!sem_prazo.expirando(i64::MAX / 2));
    }

    /// O registro dinâmico é o que dispensa a pessoa de criar uma "OAuth App"
    /// no site antes de usar o login. Se o Hub fechar esse endpoint, o login
    /// inteiro para de existir — e é aqui que se descobre.
    #[tokio::test]
    #[ignore = "acessa a rede (Hugging Face)"]
    async fn the_hub_still_registers_this_app_by_itself() {
        let http = reqwest::Client::new();
        let login = Login::start(&http, None).await.unwrap();
        assert!(!login.client().client_id.is_empty());
        assert!(
            login
                .url()
                .starts_with(&format!("{HF_BASE}/oauth/authorize?"))
        );
        assert!(login.url().contains("code_challenge_method=S256"));
        assert!(login.url().contains("read-repos"));
    }
}
