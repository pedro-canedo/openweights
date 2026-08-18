//! O 9router instalado, supervisionado e removido pelo app.
//!
//! O 9router é um roteador de IA com painel próprio, distribuído só como
//! pacote npm. O que este crate garante é que ele viva inteiramente numa
//! pasta nossa: o pacote, as dependências que ele baixa sozinho, o banco
//! SQLite e as credenciais. Desinstalar é apagar essa pasta — nada fica em
//! `~/.9router`.
//!
//! Três detalhes do upstream que ditam o formato daqui, e que custaram uma
//! leitura do `cli.js` para descobrir:
//!
//! - **O padrão dele é escutar em `0.0.0.0`.** Como ele guarda credenciais
//!   OAuth de contas de terceiros, deixar isso aberto na rede local seria o
//!   pior desfecho possível desta funcionalidade. Forçamos loopback pelo
//!   argumento e pela env.
//! - **Ele se auto-atualiza com `npm i -g 9router@latest`.** Sem
//!   `--skip-update`, ele fura a versão pinada e escapa da pasta isolada.
//! - **Ele chama `npm` em tempo de execução** para preparar o SQLite dele.
//!   Por isso o Node portátil precisa estar no `PATH` do processo filho.

use std::path::{Path, PathBuf};
use std::time::Duration;

use lr_nodejs::NodeManager;

/// Versão fixada. O projeto é novo e publica com frequência; seguir `latest`
/// significaria que uma minor do upstream pode quebrar o app sem aviso.
pub const PINNED_9ROUTER: &str = "0.5.55";

/// Porta padrão do upstream. Só mudamos se estiver ocupada.
pub const DEFAULT_PORT: u16 = 20128;

/// Cold start de app Next.js em disco frio passa dos 20 s; o llama-server
/// tem 30 s, mas aqui isso daria falso negativo em máquina lenta.
pub const READY_TIMEOUT: Duration = Duration::from_secs(60);

/// Teto do `npm install`. Com antivírus ativo no Windows e dezenas de
/// milhares de arquivos, minutos são normais — pendurar para sempre não.
pub const INSTALL_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, thiserror::Error)]
pub enum NineRouterError {
    #[error("o Node portátil não está instalado")]
    NodeMissing,
    #[error("falha de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("npm falhou ({codigo}): {detalhe}")]
    Npm { codigo: String, detalhe: String },
    #[error("o 9router não respondeu em {0} s")]
    Timeout(u64),
    #[error("instalação inválida: {0}")]
    Verification(String),
}

/// Progresso da instalação.
///
/// `Installing` carrega uma fase nomeada em vez de porcentagem: o `npm` não
/// publica progresso estruturado, e uma barra que avança sozinha mentiria
/// sobre quanto falta.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum NineRouterEvent {
    Installing { phase: String },
    Log { line: String },
    Ready,
    Failed { message: String },
}

/// Fase legível a partir de uma linha de saída do npm.
///
/// É heurística, e assumidamente: serve para a tela dizer algo mais útil que
/// "aguarde". Linha não reconhecida mantém a fase anterior (devolve `None`).
pub fn fase_do_npm(linha: &str) -> Option<&'static str> {
    let l = linha.trim();
    if l.contains("idealTree") || l.contains("resolving") {
        Some("resolving")
    } else if l.contains("reify") || l.contains("extract") {
        Some("extracting")
    } else if l.contains("added") && l.contains("package") {
        Some("finishing")
    } else {
        None
    }
}

/// Onde tudo do 9router vive.
#[derive(Debug, Clone)]
pub struct Layout {
    /// `<data_dir>/providers/9router`
    pub raiz: PathBuf,
}

impl Layout {
    pub fn new(providers_dir: &Path) -> Self {
        Self {
            raiz: providers_dir.join("9router"),
        }
    }

    /// Prefixo do npm: o pacote cai em `app/node_modules/9router`.
    pub fn app(&self) -> PathBuf {
        self.raiz.join("app")
    }

    /// `DATA_DIR` do 9router: SQLite, segredos, dependências de runtime e os
    /// PID files dos túneis dele. Fica DENTRO da nossa pasta — é isso que faz
    /// "desinstalar" apagar tudo de verdade.
    pub fn data(&self) -> PathBuf {
        self.raiz.join("data")
    }

    pub fn cli_js(&self) -> PathBuf {
        self.app().join("node_modules/9router/cli.js")
    }

    pub fn instalado(&self) -> bool {
        self.cli_js().is_file()
    }
}

// --------------------------------------------------------------- segredos ---

/// Bytes aleatórios em hex. Usa o `getrandom`, que já está no grafo por conta
/// do rustls — não vale puxar o `rand` inteiro para gerar duas strings.
fn hex_aleatorio(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("fonte de aleatoriedade do sistema indisponível");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn gerar_jwt_secret() -> String {
    hex_aleatorio(32)
}

/// Alfabeto sem os caracteres que se confundem ao ler da tela e digitar no
/// painel (`0/O`, `1/l/I`) — esta senha é feita para ser copiada à mão.
const ALFABETO: &[u8] = b"abcdefghijkmnpqrstuvwxyz23456789";

pub fn gerar_senha() -> String {
    let mut buf = [0u8; 16];
    getrandom::fill(&mut buf).expect("fonte de aleatoriedade do sistema indisponível");
    let chars: Vec<char> = buf
        .iter()
        .map(|b| ALFABETO[*b as usize % ALFABETO.len()] as char)
        .collect();
    chars
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

// ------------------------------------------------------------- instalação ---

/// Argumentos do `npm install` do 9router.
///
/// Função pura para poder travar em teste as duas decisões que causam dano se
/// regredirem: a versão pinada e o prefixo isolado.
pub fn npm_install_args(versao: &str, ignorar_scripts: bool) -> Vec<String> {
    let mut args = vec![
        "install".to_string(),
        format!("9router@{versao}"),
        "--no-audit".to_string(),
        "--no-fund".to_string(),
        "--loglevel=info".to_string(),
    ];
    if ignorar_scripts {
        args.push("--ignore-scripts".to_string());
    }
    args
}

/// Instala o 9router na pasta isolada.
///
/// Tenta primeiro com `--ignore-scripts`: as dependências declaradas são JS
/// puro, e não rodar script de ciclo de vida de terceiros é a única mitigação
/// real do maior risco desta funcionalidade. Se falhar, repete sem a flag e
/// avisa — melhor instalar com aviso do que não instalar.
pub async fn instalar(
    node: &NodeManager,
    layout: &Layout,
    on_event: &(dyn Fn(NineRouterEvent) + Send + Sync),
) -> Result<(), NineRouterError> {
    std::fs::create_dir_all(layout.app())?;
    std::fs::create_dir_all(layout.data())?;

    match rodar_npm(node, layout, true, on_event).await {
        Ok(()) => Ok(()),
        Err(e) => {
            log::warn!("npm com --ignore-scripts falhou ({e}); repetindo sem a flag");
            on_event(NineRouterEvent::Log {
                line: "scripts de instalação do pacote serão executados".to_string(),
            });
            rodar_npm(node, layout, false, on_event).await
        }
    }
}

async fn rodar_npm(
    node: &NodeManager,
    layout: &Layout,
    ignorar_scripts: bool,
    on_event: &(dyn Fn(NineRouterEvent) + Send + Sync),
) -> Result<(), NineRouterError> {
    use tokio::io::AsyncBufReadExt as _;

    let args = npm_install_args(PINNED_9ROUTER, ignorar_scripts);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let app = layout.app();
    let mut cmd = node
        .npm_command(&refs, &app, &app)
        .ok_or(NineRouterError::NodeMissing)?;
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    lr_proc::prepare(&mut cmd);

    let mut filho = lr_proc::spawn_supervised(&mut cmd)?;
    let stdout = filho.stdout.take();
    let stderr = filho.stderr.take();

    // O log ao vivo é o que substitui a barra de progresso: sem ele a tela
    // fica muda por minutos e parece travada.
    let mut linhas = Vec::new();
    if let Some(saida) = stdout {
        let mut leitor = tokio::io::BufReader::new(saida).lines();
        while let Ok(Some(linha)) = leitor.next_line().await {
            if let Some(fase) = fase_do_npm(&linha) {
                on_event(NineRouterEvent::Installing {
                    phase: fase.to_string(),
                });
            }
            on_event(NineRouterEvent::Log {
                line: linha.clone(),
            });
            linhas.push(linha);
        }
    }

    let status = match tokio::time::timeout(INSTALL_TIMEOUT, filho.wait()).await {
        Ok(r) => r?,
        Err(_) => {
            if let Some(pid) = filho.id() {
                lr_proc::kill_process_tree(pid);
            }
            return Err(NineRouterError::Timeout(INSTALL_TIMEOUT.as_secs()));
        }
    };

    if !status.success() {
        let mut detalhe = String::new();
        if let Some(saida) = stderr {
            let mut leitor = tokio::io::BufReader::new(saida).lines();
            while let Ok(Some(l)) = leitor.next_line().await {
                detalhe.push_str(&l);
                detalhe.push('\n');
            }
        }
        if detalhe.trim().is_empty() {
            detalhe = linhas.join("\n");
        }
        return Err(NineRouterError::Npm {
            codigo: status.code().map(|c| c.to_string()).unwrap_or_default(),
            detalhe: detalhe
                .chars()
                .rev()
                .take(600)
                .collect::<String>()
                .chars()
                .rev()
                .collect(),
        });
    }

    if !layout.instalado() {
        return Err(NineRouterError::Verification(
            "cli.js não encontrado após a instalação".to_string(),
        ));
    }
    Ok(())
}

/// Para o processo e apaga tudo. `remover_dados` inclui o SQLite — que leva
/// junto as contas configuradas dentro do painel.
pub fn desinstalar(layout: &Layout, remover_dados: bool) -> std::io::Result<()> {
    lr_fetch::remove_dir_all_retrying(&layout.app())?;
    if remover_dados {
        lr_fetch::remove_dir_all_retrying(&layout.data())?;
        lr_fetch::remove_dir_all_retrying(&layout.raiz)?;
    }
    Ok(())
}

// --------------------------------------------------------------- execução ---

#[derive(Debug, Clone)]
pub struct RunConfig {
    pub port: u16,
    pub password: String,
    pub jwt_secret: String,
}

/// Argumentos do `cli.js`.
///
/// `--host 127.0.0.1` e `--skip-update` não são preferência: sem o primeiro o
/// 9router escuta em `0.0.0.0` (o padrão dele), e sem o segundo ele se
/// reinstala globalmente e escapa da pasta isolada.
pub fn run_args(cfg: &RunConfig) -> Vec<String> {
    vec![
        "--port".to_string(),
        cfg.port.to_string(),
        "--host".to_string(),
        "127.0.0.1".to_string(),
        "--no-browser".to_string(),
        "--log".to_string(),
        "--skip-update".to_string(),
    ]
}

/// Ambiente do processo. O `DATA_DIR` é o que mantém banco, segredos e
/// dependências dentro da nossa pasta.
pub fn run_env(layout: &Layout, cfg: &RunConfig) -> Vec<(String, String)> {
    let base = format!("http://127.0.0.1:{}", cfg.port);
    vec![
        ("PORT".into(), cfg.port.to_string()),
        ("HOSTNAME".into(), "127.0.0.1".into()),
        ("DATA_DIR".into(), layout.data().display().to_string()),
        ("BASE_URL".into(), base.clone()),
        ("NEXT_PUBLIC_BASE_URL".into(), base),
        ("JWT_SECRET".into(), cfg.jwt_secret.clone()),
        // Só vale no PRIMEIRO boot: depois o 9router grava o hash e ignora
        // esta variável. Por isso ela precisa estar aqui desde o começo.
        ("INITIAL_PASSWORD".into(), cfg.password.clone()),
        ("NODE_ENV".into(), "production".into()),
        ("ENABLE_REQUEST_LOGS".into(), "false".into()),
    ]
}

/// Espera a porta atender, sem depender do objeto do processo.
///
/// Existe como função livre porque quem chama precisa soltar o mutex do
/// processo antes de esperar: o cold start do Next.js leva dezenas de
/// segundos, e segurar o lock nesse intervalo travaria `status` e `stop`
/// junto — o mesmo cuidado que o `start_engine` do llama-server já toma.
pub async fn aguardar_pronto(porta: u16, prazo: Duration) -> Result<(), NineRouterError> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default();
    let limite = tokio::time::Instant::now() + prazo;
    loop {
        // TCP primeiro: durante o cold start a porta já aceita conexão antes
        // de responder rota, e é isso que distingue "subindo" de "morto".
        if lr_proc::port_in_use(porta)
            && http
                .get(format!("http://127.0.0.1:{porta}/v1/models"))
                .send()
                .await
                .is_ok()
        {
            return Ok(());
        }
        if tokio::time::Instant::now() >= limite {
            return Err(NineRouterError::Timeout(prazo.as_secs()));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

// ------------------------------------------------------------- chave de API ---

/// Sal do token de CLI do 9router. Vem do `src/cli/api/client.js` dele.
const CLI_TOKEN_SALT: &str = "9r-cli-auth";

/// Token que autentica a API interna do 9router sem passar pelo painel.
///
/// O 9router aceita o header `x-9r-cli-token` como credencial equivalente à
/// sessão do painel, e o valor é derivado de dois arquivos que ele mesmo
/// escreve no `DATA_DIR` — que é NOSSO. É por isso que o app consegue pegar
/// a chave de API sem pedir para a pessoa copiar nada da tela dele:
///
/// ```text
/// sha256(machine-id + "9r-cli-auth" + auth/cli-secret)[..16]
/// ```
///
/// `None` antes do primeiro boot: os dois arquivos nascem com o servidor.
pub fn cli_token(layout: &Layout) -> Option<String> {
    use sha2::{Digest as _, Sha256};
    let ler = |p: PathBuf| -> Option<String> {
        let texto = std::fs::read_to_string(p).ok()?;
        let texto = texto.trim().to_string();
        (!texto.is_empty()).then_some(texto)
    };
    let machine = ler(layout.data().join("machine-id"))?;
    let segredo = ler(layout.data().join("auth/cli-secret"))?;
    let hash = Sha256::digest(format!("{machine}{CLI_TOKEN_SALT}{segredo}").as_bytes());
    Some(hash.iter().map(|b| format!("{b:02x}")).collect::<String>()[..16].to_string())
}

/// Nome da chave que o app cria para si no 9router.
///
/// Nomeada em vez de reaproveitar a "Default Key": quem olhar a lista de
/// chaves no painel precisa conseguir dizer de quem é cada uma — e revogar a
/// nossa sem derrubar as outras.
pub const NOME_DA_CHAVE: &str = "OpenWeights";

/// Escolhe a chave a usar entre as que o 9router já tem.
///
/// Prefere a nossa; qualquer outra serve como segunda opção — se a pessoa já
/// criou uma chave à mão, reaproveitá-la evita encher a lista dela.
pub fn escolher_chave(json: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Chave {
        key: Option<String>,
        name: Option<String>,
    }
    #[derive(serde::Deserialize)]
    struct Lista {
        keys: Vec<Chave>,
    }
    let lista = serde_json::from_str::<Lista>(json).ok()?;
    let validas: Vec<&Chave> = lista
        .keys
        .iter()
        .filter(|c| c.key.as_deref().is_some_and(|k| !k.trim().is_empty()))
        .collect();
    validas
        .iter()
        .find(|c| c.name.as_deref() == Some(NOME_DA_CHAVE))
        .or_else(|| validas.first())
        .and_then(|c| c.key.clone())
}

/// Garante uma chave de API utilizável e devolve o valor dela.
///
/// Existe porque o 9router tem um interruptor "Require API key" no painel: se
/// a pessoa o ligar, todo `/v1/chat/completions` sem `Authorization` passa a
/// responder 401 — e a conversa morre com "Missing API key" sem que ela
/// tenha mudado nada do lado de cá. Pegar a chave ANTES de precisar dela faz
/// o interruptor deixar de ser um problema nosso.
pub async fn garantir_api_key(
    layout: &Layout,
    porta: u16,
) -> Result<String, NineRouterError> {
    let token = cli_token(layout)
        .ok_or_else(|| NineRouterError::Verification("sem token de CLI do 9router".into()))?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default();
    let base = format!("http://127.0.0.1:{porta}/api/keys");

    let existentes = http
        .get(&base)
        .header("x-9r-cli-token", &token)
        .send()
        .await
        .map_err(|e| NineRouterError::Verification(e.to_string()))?;
    if existentes.status().is_success() {
        let corpo = existentes.text().await.unwrap_or_default();
        if let Some(chave) = escolher_chave(&corpo) {
            return Ok(chave);
        }
    }

    let criada = http
        .post(&base)
        .header("x-9r-cli-token", &token)
        .json(&serde_json::json!({ "name": NOME_DA_CHAVE }))
        .send()
        .await
        .map_err(|e| NineRouterError::Verification(e.to_string()))?;
    if !criada.status().is_success() {
        return Err(NineRouterError::Verification(format!(
            "POST /api/keys devolveu {}",
            criada.status()
        )));
    }
    let corpo = criada.text().await.unwrap_or_default();
    serde_json::from_str::<serde_json::Value>(&corpo)
        .ok()
        .and_then(|v| v.get("key")?.as_str().map(str::to_string))
        .filter(|k| !k.trim().is_empty())
        .ok_or_else(|| NineRouterError::Verification("resposta sem chave".into()))
}

/// Um modelo publicado pelo 9router em `GET /v1/models`.
///
/// O que interessa aqui não é o catálogo inteiro do upstream: é o que a
/// pessoa já conectou no painel dele. Um combo (`owned_by: "combo"`) aparece
/// como qualquer outro modelo — do lado de cá é só um id que o 9router sabe
/// atender, e é exatamente assim que ele deve entrar no seletor do chat.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeloNine {
    pub id: String,
    /// Quem serve: o provedor conectado (`cx`, `gcli`) ou `combo`.
    pub owned_by: String,
    pub context_length: Option<u32>,
    /// `None` quando o 9router não declarou capacidades — é o caso dos
    /// combos, cujo suporte depende do modelo que atender a vez.
    pub supports_tools: Option<bool>,
    pub vision: Option<bool>,
}

/// Lê a resposta de `GET /v1/models`.
///
/// Função pura para poder travar em teste o formato real do upstream, que
/// mistura entradas ricas (com `capabilities`) e entradas mínimas.
pub fn parse_modelos(json: &str) -> Vec<ModeloNine> {
    #[derive(serde::Deserialize)]
    struct Caps {
        tools: Option<bool>,
        vision: Option<bool>,
    }
    #[derive(serde::Deserialize)]
    struct Item {
        id: String,
        owned_by: Option<String>,
        context_length: Option<u32>,
        capabilities: Option<Caps>,
    }
    #[derive(serde::Deserialize)]
    struct Lista {
        data: Vec<Item>,
    }

    serde_json::from_str::<Lista>(json)
        .map(|l| {
            l.data
                .into_iter()
                .filter(|i| !i.id.trim().is_empty())
                .map(|i| ModeloNine {
                    id: i.id,
                    owned_by: i.owned_by.unwrap_or_default(),
                    context_length: i.context_length,
                    supports_tools: i.capabilities.as_ref().and_then(|c| c.tools),
                    vision: i.capabilities.as_ref().and_then(|c| c.vision),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Pergunta ao 9router no ar quais modelos ele atende.
///
/// Não exige chave: o endpoint é loopback e o 9router aceita o `/v1` local
/// sem autenticação — a chave do painel é para quem chama de fora.
pub async fn listar_modelos(porta: u16) -> Result<Vec<ModeloNine>, NineRouterError> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default();
    let resposta = http
        .get(format!("http://127.0.0.1:{porta}/v1/models"))
        .send()
        .await
        .map_err(|e| NineRouterError::Verification(e.to_string()))?;
    if !resposta.status().is_success() {
        return Err(NineRouterError::Verification(format!(
            "GET /v1/models devolveu {}",
            resposta.status()
        )));
    }
    let corpo = resposta
        .text()
        .await
        .map_err(|e| NineRouterError::Verification(e.to_string()))?;
    Ok(parse_modelos(&corpo))
}

/// Processo do 9router em execução.
pub struct NineRouter {
    filho: Option<tokio::process::Child>,
    job: Option<lr_proc::JobGuard>,
    porta: u16,
    http: reqwest::Client,
}

impl NineRouter {
    pub fn spawn(
        node: &NodeManager,
        layout: &Layout,
        cfg: &RunConfig,
    ) -> Result<Self, NineRouterError> {
        let exe = node.node_exe().ok_or(NineRouterError::NodeMissing)?;
        if !layout.instalado() {
            return Err(NineRouterError::Verification(
                "9router não instalado".to_string(),
            ));
        }

        let mut cmd = tokio::process::Command::new(exe);
        cmd.arg(layout.cli_js())
            .args(run_args(cfg))
            .current_dir(layout.app())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        // O Node portátil precisa estar no PATH: o 9router chama `npm` em
        // tempo de execução para preparar o SQLite dele.
        for (k, v) in node.env_isolado(&layout.app()) {
            cmd.env(k, v);
        }
        for (k, v) in run_env(layout, cfg) {
            cmd.env(k, v);
        }
        lr_proc::prepare(&mut cmd);

        let filho = lr_proc::spawn_supervised(&mut cmd)?;
        let job = lr_proc::attach_job(&filho);
        Ok(Self {
            filho: Some(filho),
            job,
            porta: cfg.port,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
        })
    }

    pub fn pid(&self) -> Option<u32> {
        self.filho.as_ref().and_then(|c| c.id())
    }

    pub fn take_output(
        &mut self,
    ) -> (
        Option<tokio::process::ChildStdout>,
        Option<tokio::process::ChildStderr>,
    ) {
        match self.filho.as_mut() {
            Some(c) => (c.stdout.take(), c.stderr.take()),
            None => (None, None),
        }
    }

    /// Está atendendo?
    ///
    /// Sonda o TCP antes do HTTP: durante o cold start do Next.js a porta já
    /// aceita conexão mas ainda não responde rota, e distinguir "subindo" de
    /// "morto" é o que evita desistir cedo demais.
    pub async fn pronto(&self) -> bool {
        if lr_proc::port_in_use(self.porta) {
            // Porta ocupada = alguém escutando. Confirma com uma requisição.
            return self
                .http
                .get(format!("http://127.0.0.1:{}/v1/models", self.porta))
                .send()
                .await
                .is_ok();
        }
        false
    }

    pub async fn wait_ready(&self, prazo: Duration) -> Result<(), NineRouterError> {
        aguardar_pronto(self.porta, prazo).await
    }

    /// Mata o processo e a árvore. O 9router é Next.js e gera netos: sem
    /// matar a árvore sobra `node` segurando a porta depois de fechar o app.
    pub fn stop_blocking(&mut self) {
        let pid = self.pid();
        if let Some(job) = self.job.take() {
            lr_proc::terminate_job(&job);
        }
        // O `taskkill /T` roda MESMO com o job encerrado, e não é redundância
        // barata: o `cli.js` sobe o servidor Next com `detached: true` e, se
        // um neto tiver escapado do job (elevação, breakaway de terceiro), o
        // job fecha sem levá-lo junto. O sintoma é o pior possível — a porta
        // do 9router continua ocupada com o app já fechado, e a próxima
        // abertura acha "outra instância" e cai numa porta efêmera.
        if let Some(pid) = pid {
            lr_proc::kill_process_tree(pid);
        }
        if let Some(mut filho) = self.filho.take() {
            let _ = filho.start_kill();
            lr_proc::reap_child(&mut filho);
        }
    }
}

impl Drop for NineRouter {
    fn drop(&mut self) {
        self.stop_blocking();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> RunConfig {
        RunConfig {
            port: 20500,
            password: "abcd-efgh".into(),
            jwt_secret: "segredo".into(),
        }
    }

    /// As duas flags que causam dano real se sumirem: sem `--host` o 9router
    /// escuta em 0.0.0.0 (o padrão dele) expondo credenciais OAuth na rede;
    /// sem `--skip-update` ele se reinstala globalmente e fura o isolamento.
    #[test]
    fn the_run_arguments_force_loopback_and_skip_update() {
        let args = run_args(&cfg()).join(" ");
        assert!(args.contains("--host 127.0.0.1"));
        assert!(args.contains("--skip-update"));
        assert!(args.contains("--no-browser"));
        // `--log` é o que alimenta o painel de log da tela.
        assert!(args.contains("--log"));
        // Bandeja seria um segundo ícone para um app que já tem janela.
        assert!(!args.contains("--tray"));
    }

    #[test]
    fn the_run_arguments_carry_the_chosen_port() {
        let args = run_args(&cfg()).join(" ");
        assert!(args.contains("--port 20500"));
    }

    /// Se o `DATA_DIR` escapar, o 9router grava em `~/.9router` e o
    /// "desinstalar apaga tudo" deixa de ser verdade.
    #[test]
    fn the_environment_keeps_the_data_directory_inside_our_folder() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path());
        let env: std::collections::HashMap<_, _> = run_env(&layout, &cfg()).into_iter().collect();
        assert_eq!(
            env.get("DATA_DIR").unwrap(),
            &layout.data().display().to_string()
        );
        assert!(layout.data().starts_with(dir.path()));
        assert_eq!(env.get("HOSTNAME").unwrap(), "127.0.0.1");
        assert_eq!(env.get("NODE_ENV").unwrap(), "production");
    }

    /// Vetor conferido contra a implementação do upstream
    /// (`src/cli/api/client.js`): mesmo `machine-id`, mesmo segredo, mesmo
    /// token. Se o 9router mudar o sal ou a fatia, é aqui que quebra — e o
    /// sintoma sem este teste seria um 401 mudo na conversa.
    #[test]
    fn the_cli_token_matches_the_upstream_derivation() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path());
        std::fs::create_dir_all(layout.data().join("auth")).unwrap();
        std::fs::write(layout.data().join("machine-id"), "  abc123\n").unwrap();
        std::fs::write(layout.data().join("auth/cli-secret"), "segredo\n").unwrap();

        // sha256("abc123" + "9r-cli-auth" + "segredo")[..16]
        let esperado = {
            use sha2::{Digest as _, Sha256};
            let h = Sha256::digest(b"abc1239r-cli-authsegredo");
            h.iter().map(|b| format!("{b:02x}")).collect::<String>()[..16].to_string()
        };
        assert_eq!(cli_token(&layout).as_deref(), Some(esperado.as_str()));
        assert_eq!(esperado.len(), 16);
    }

    /// Antes do primeiro boot os arquivos não existem — e pedir a chave nesse
    /// momento não pode virar pânico.
    #[test]
    fn without_the_secret_files_there_is_no_cli_token() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(cli_token(&Layout::new(dir.path())), None);
    }

    /// A chave do app tem precedência sobre a "Default Key" do 9router: é a
    /// que a pessoa pode revogar sem derrubar o resto.
    #[test]
    fn our_own_key_wins_over_the_others() {
        let json = r#"{"keys":[
            {"id":1,"name":"Default Key","key":"sk-outra"},
            {"id":2,"name":"OpenWeights","key":"sk-nossa"}
        ]}"#;
        assert_eq!(escolher_chave(json).as_deref(), Some("sk-nossa"));
    }

    /// Sem chave nossa, aproveitar a que já existe evita encher a lista dela.
    /// Entrada sem valor de chave não conta.
    #[test]
    fn an_existing_key_is_reused_and_empty_ones_are_ignored() {
        let json = r#"{"keys":[{"name":"vazia","key":"  "},{"name":"Default Key","key":"sk-outra"}]}"#;
        assert_eq!(escolher_chave(json).as_deref(), Some("sk-outra"));
        assert_eq!(escolher_chave(r#"{"keys":[]}"#), None);
        assert_eq!(escolher_chave("não é json"), None);
    }

    /// Payload real do 9router 0.5.55 com um combo e um modelo conectado.
    /// O combo vem sem `capabilities` — e é justamente ele que quebrava um
    /// parse ingênuo que assumisse o campo presente.
    #[test]
    fn the_model_list_keeps_combos_and_reads_capabilities() {
        let json = r#"{"object":"list","data":[
            {"id":"Compo-Premium","object":"model","owned_by":"combo"},
            {"id":"gcli/grok-4.6","object":"model","owned_by":"gcli",
             "capabilities":{"vision":true,"tools":true,"reasoning":true},
             "context_length":256000,"max_completion_tokens":64000},
            {"id":"   ","object":"model","owned_by":"lixo"}
        ]}"#;
        let modelos = parse_modelos(json);
        assert_eq!(modelos.len(), 2, "a entrada sem id é descartada");

        assert_eq!(modelos[0].id, "Compo-Premium");
        assert_eq!(modelos[0].owned_by, "combo");
        assert_eq!(modelos[0].supports_tools, None);
        assert_eq!(modelos[0].context_length, None);

        assert_eq!(modelos[1].id, "gcli/grok-4.6");
        assert_eq!(modelos[1].supports_tools, Some(true));
        assert_eq!(modelos[1].vision, Some(true));
        assert_eq!(modelos[1].context_length, Some(256_000));
    }

    /// Resposta ilegível não pode derrubar o seletor do chat: sem modelos do
    /// 9router a lista ainda tem os locais.
    #[test]
    fn a_broken_model_list_is_empty_not_an_error() {
        assert!(parse_modelos("não é json").is_empty());
        assert!(parse_modelos(r#"{"data":"nem lista"}"#).is_empty());
    }

    /// A senha só é aceita no primeiro boot; precisa ir desde o spawn.
    #[test]
    fn the_initial_password_is_present_from_the_first_spawn() {
        let dir = tempfile::tempdir().unwrap();
        let env: std::collections::HashMap<_, _> = run_env(&Layout::new(dir.path()), &cfg())
            .into_iter()
            .collect();
        assert_eq!(env.get("INITIAL_PASSWORD").unwrap(), "abcd-efgh");
    }

    #[test]
    fn the_install_arguments_pin_the_version() {
        let args = npm_install_args(PINNED_9ROUTER, true).join(" ");
        assert!(args.contains(&format!("9router@{PINNED_9ROUTER}")));
        assert!(args.contains("--ignore-scripts"));
        // Nunca global: `-g` escreveria no prefixo do Node portátil e
        // misturaria o app com o runtime.
        assert!(!args.contains(" -g"));
        assert!(!args.contains("--global"));
    }

    #[test]
    fn the_install_can_fall_back_to_running_scripts() {
        let args = npm_install_args(PINNED_9ROUTER, false).join(" ");
        assert!(!args.contains("--ignore-scripts"));
    }

    #[test]
    fn the_generated_password_avoids_ambiguous_characters() {
        let senha = gerar_senha();
        assert!(!senha.contains('0'));
        assert!(!senha.contains('O'));
        assert!(!senha.contains('1'));
        assert!(!senha.contains('l'));
        assert_eq!(senha.len(), 19, "4 grupos de 4 com 3 hífens");
    }

    #[test]
    fn two_generated_secrets_differ() {
        assert_ne!(gerar_jwt_secret(), gerar_jwt_secret());
        assert_ne!(gerar_senha(), gerar_senha());
        assert_eq!(gerar_jwt_secret().len(), 64);
    }

    #[test]
    fn the_layout_keeps_everything_under_one_folder() {
        let dir = tempfile::tempdir().unwrap();
        let l = Layout::new(dir.path());
        assert!(l.app().starts_with(&l.raiz));
        assert!(l.data().starts_with(&l.raiz));
        assert!(l.cli_js().starts_with(l.app()));
        assert!(!l.instalado());
    }

    #[test]
    fn the_npm_phase_is_read_from_a_reify_line() {
        assert_eq!(
            fase_do_npm("npm http fetch reify:9router"),
            Some("extracting")
        );
        assert_eq!(fase_do_npm("timing idealTree Completed"), Some("resolving"));
        assert_eq!(fase_do_npm("added 42 packages in 3s"), Some("finishing"));
    }

    /// Linha desconhecida não pode zerar a fase que já estava na tela.
    #[test]
    fn an_unrecognised_line_keeps_the_previous_phase() {
        assert_eq!(fase_do_npm("qualquer coisa"), None);
        assert_eq!(fase_do_npm(""), None);
    }

    #[test]
    fn uninstalling_without_data_keeps_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let l = Layout::new(dir.path());
        std::fs::create_dir_all(l.app()).unwrap();
        std::fs::create_dir_all(l.data()).unwrap();
        std::fs::write(l.data().join("data.sqlite"), b"x").unwrap();

        desinstalar(&l, false).unwrap();
        assert!(!l.app().exists());
        assert!(l.data().join("data.sqlite").is_file());
    }

    /// Ciclo completo de verdade — roda só com `--ignored` porque baixa
    /// centenas de MB e leva minutos.
    ///
    /// É o único teste que prova a promessa inteira: instala o Node portátil
    /// e o 9router numa pasta temporária, sobe o processo, confere que ele
    /// responde `/v1/models`, derruba e apaga tudo. Se o upstream mudar o
    /// nome de uma flag ou o layout do pacote, é aqui que aparece.
    #[tokio::test]
    #[ignore = "rede + minutos: baixa Node e o pacote do 9router"]
    async fn live_full_install_start_and_uninstall_cycle() {
        let dir = tempfile::tempdir().unwrap();
        let raiz = dir.path().to_path_buf();
        let node = lr_nodejs::NodeManager::new(
            raiz.clone(),
            if cfg!(windows) {
                "windows"
            } else if cfg!(target_os = "macos") {
                "macos"
            } else {
                "linux"
            },
            if cfg!(target_arch = "aarch64") {
                "aarch64"
            } else {
                "x86_64"
            },
        );
        node.ensure(|_| {}).await.expect("Node portátil");
        assert!(node.node_exe().is_some());
        assert!(node.npm_cli().is_some(), "npm precisa vir na distribuição");

        let layout = Layout::new(&raiz);
        instalar(&node, &layout, &|_| {})
            .await
            .expect("instalação do 9router");
        assert!(layout.instalado());

        let cfg = RunConfig {
            port: lr_proc::free_port(0),
            password: gerar_senha(),
            jwt_secret: gerar_jwt_secret(),
        };
        let mut nr = NineRouter::spawn(&node, &layout, &cfg).expect("spawn");
        let subiu = nr.wait_ready(READY_TIMEOUT).await;
        nr.stop_blocking();
        subiu.expect("9router deveria atender");

        // Parar tem de LIBERAR A PORTA, não só matar o `cli.js`: quem escuta
        // é o servidor Next, que é neto e nasce `detached`. Se ele sobreviver,
        // fica um processo órfão segurando a porta com o app fechado — e é
        // exatamente isso que este assert impede de voltar.
        let livre = tokio::time::timeout(Duration::from_secs(15), async {
            while lr_proc::port_in_use(cfg.port) {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await;
        assert!(
            livre.is_ok(),
            "a porta {} continuou ocupada depois do stop",
            cfg.port
        );

        // A promessa do isolamento, verificada pelo lado positivo: o banco
        // nasceu DENTRO do nosso DATA_DIR.
        //
        // Verificar pelo lado negativo ("`~/.9router` não existe") daria
        // falso positivo em quem já tem o 9router instalado por conta
        // própria — foi o que aconteceu na primeira execução deste teste.
        let banco = layout.data().join("db/data.sqlite");
        assert!(
            banco.is_file(),
            "o banco deveria estar em {}",
            banco.display()
        );

        desinstalar(&layout, true).unwrap();
        assert!(!layout.raiz.exists());
    }

    #[test]
    fn uninstalling_with_data_removes_the_whole_folder() {
        let dir = tempfile::tempdir().unwrap();
        let l = Layout::new(dir.path());
        std::fs::create_dir_all(l.app()).unwrap();
        std::fs::create_dir_all(l.data()).unwrap();
        desinstalar(&l, true).unwrap();
        assert!(!l.raiz.exists());
    }
}
