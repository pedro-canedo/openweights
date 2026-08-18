//! Ponto de entrada único para os provedores, com Traefik local.
//!
//! **O que isto entrega, sem exagero:** uma URL só, estável, que roteia por
//! prefixo para o llama.cpp local e para o 9router. Serve para apontar uma
//! ferramenta externa (Cursor, Claude Code, um script) para o OpenWeights sem
//! ter de decorar duas portas, e para alcançar tudo isso de outro aparelho da
//! rede quando você escolhe expor.
//!
//! **O que isto NÃO faz**, e vale dizer para ninguém esperar: não cria túnel
//! para a internet (Traefik é proxy reverso, não túnel — e o painel do
//! 9router já traz cloudflared e tailscale por conta própria); não junta os
//! catálogos num `/v1/models` só, porque isso seria código nosso e não
//! roteamento; e não acrescenta autenticação nenhuma.
//!
//! Por isso o gateway é **opcional e desligado por padrão**: nada no chat
//! depende dele.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Versão fixada, pelo mesmo motivo das outras: uma minor do upstream não
/// pode mudar o comportamento do app sem alguém decidir.
pub const PINNED_TRAEFIK: &str = "v3.7.10";

/// Porta do ponto de entrada. Fora da faixa do llama-server (11711) e do
/// 9router (20128) para não competir com eles.
pub const DEFAULT_PORT: u16 = 11700;

const USER_AGENT: &str = concat!("OpenWeights/", env!("CARGO_PKG_VERSION"));
const REPO: &str = "traefik/traefik";

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("plataforma sem binário do Traefik: {0}/{1}")]
    UnsupportedPlatform(String, String),
    #[error("falha ao baixar ou extrair: {0}")]
    Fetch(#[from] lr_fetch::FetchError),
    #[error("falha de rede: {0}")]
    Network(#[from] reqwest::Error),
    #[error("falha de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("o gateway não respondeu em {0} s")]
    Timeout(u64),
    #[error("instalação inválida: {0}")]
    Verification(String),
}

pub fn traefik_asset(os: &str, arch: &str, tag: &str) -> Option<String> {
    let alvo = match (os, arch) {
        ("windows", "x86_64") => "windows_amd64.zip",
        ("windows", "aarch64") => "windows_arm64.zip",
        ("macos", "x86_64") => "darwin_amd64.tar.gz",
        ("macos", "aarch64") => "darwin_arm64.tar.gz",
        ("linux", "x86_64") => "linux_amd64.tar.gz",
        ("linux", "aarch64") => "linux_arm64.tar.gz",
        _ => return None,
    };
    Some(format!("traefik_{tag}_{alvo}"))
}

pub fn asset_url(tag: &str, arquivo: &str) -> String {
    format!("https://github.com/{REPO}/releases/download/{tag}/{arquivo}")
}

pub const fn traefik_exe_name() -> &'static str {
    if cfg!(windows) {
        "traefik.exe"
    } else {
        "traefik"
    }
}

// ------------------------------------------------------------ configuração ---

/// Uma rota do gateway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    /// Nome interno (vira o nome do router e do serviço no YAML).
    pub name: String,
    /// Prefixo da URL, com barra inicial (`/local`).
    pub path_prefix: String,
    /// Para onde encaminhar (`http://127.0.0.1:11711`).
    pub target: String,
}

/// Rotas de quem está no ar agora.
///
/// Serviço parado **não** entra: uma rota apontando para porta morta devolve
/// 502 e faz parecer que o gateway está quebrado, quando o que falta é ligar
/// o provedor.
pub fn rotas_ativas(local: Option<u16>, ninerouter: Option<u16>) -> Vec<Route> {
    let mut rotas = Vec::new();
    if let Some(p) = local {
        rotas.push(Route {
            name: "local".into(),
            path_prefix: "/local".into(),
            target: format!("http://127.0.0.1:{p}"),
        });
    }
    if let Some(p) = ninerouter {
        rotas.push(Route {
            name: "ninerouter".into(),
            path_prefix: "/9router".into(),
            target: format!("http://127.0.0.1:{p}"),
        });
    }
    rotas
}

/// Configuração estática do Traefik.
///
/// `api.dashboard` e `insecure` ficam desligados: o painel do Traefik não
/// acrescenta nada aqui e seria mais uma superfície aberta.
pub fn render_static(porta: u16, expor_lan: bool, dir_dinamico: &Path) -> String {
    let bind = if expor_lan { "0.0.0.0" } else { "127.0.0.1" };
    format!(
        "entryPoints:\n  \
           web:\n    \
             address: \"{bind}:{porta}\"\n\
         providers:\n  \
           file:\n    \
             directory: \"{}\"\n    \
             watch: true\n\
         api:\n  \
           dashboard: false\n  \
           insecure: false\n\
         ping: {{}}\n\
         log:\n  \
           level: INFO\n",
        dir_dinamico.display().to_string().replace('\\', "/")
    )
}

/// Configuração dinâmica: um router e um serviço por rota.
///
/// Determinística de propósito — o Traefik relê o arquivo a cada mudança, e
/// uma saída instável faria recarga a cada escrita sem nada ter mudado.
pub fn render_dynamic(rotas: &[Route]) -> String {
    if rotas.is_empty() {
        // `http: {}` vazio é válido e limpa as rotas anteriores; um arquivo
        // vazio de verdade faria o Traefik reclamar no log.
        return "http: {}\n".to_string();
    }

    let mut s = String::from("http:\n  routers:\n");
    for r in rotas {
        s.push_str(&format!(
            "    {}:\n      rule: \"PathPrefix(`{}`)\"\n      service: {}\n      middlewares:\n        - {}-strip\n      entryPoints:\n        - web\n",
            r.name, r.path_prefix, r.name, r.name
        ));
    }
    s.push_str("  middlewares:\n");
    for r in rotas {
        // O provedor atrás não conhece o prefixo: `/local/v1/models` tem de
        // chegar nele como `/v1/models`.
        s.push_str(&format!(
            "    {}-strip:\n      stripPrefix:\n        prefixes:\n          - \"{}\"\n",
            r.name, r.path_prefix
        ));
    }
    s.push_str("  services:\n");
    for r in rotas {
        s.push_str(&format!(
            "    {}:\n      loadBalancer:\n        servers:\n          - url: \"{}\"\n",
            r.name, r.target
        ));
    }
    s
}

// --------------------------------------------------------------- instalação ---

#[derive(Debug, Clone, serde::Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum GatewayEvent {
    Progress {
        asset: String,
        received_bytes: u64,
        total_bytes: u64,
    },
    Extracting {
        asset: String,
    },
    Ready,
    Failed {
        message: String,
    },
}

/// Onde o gateway vive.
#[derive(Debug, Clone)]
pub struct Layout {
    pub raiz: PathBuf,
}

impl Layout {
    pub fn new(providers_dir: &Path) -> Self {
        Self {
            raiz: providers_dir.join("traefik"),
        }
    }
    pub fn bin_dir(&self) -> PathBuf {
        self.raiz.join(PINNED_TRAEFIK)
    }
    pub fn exe(&self) -> PathBuf {
        self.bin_dir().join(traefik_exe_name())
    }
    pub fn etc(&self) -> PathBuf {
        self.raiz.join("etc")
    }
    pub fn static_yml(&self) -> PathBuf {
        self.etc().join("traefik.yml")
    }
    pub fn dynamic_dir(&self) -> PathBuf {
        self.etc().join("dynamic")
    }
    pub fn routes_yml(&self) -> PathBuf {
        self.dynamic_dir().join("routes.yml")
    }
    pub fn instalado(&self) -> bool {
        self.exe().is_file()
    }
}

pub async fn instalar(
    layout: &Layout,
    os: &str,
    arch: &str,
    on_event: &(dyn Fn(GatewayEvent) + Send + Sync),
) -> Result<(), GatewayError> {
    let asset = traefik_asset(os, arch, PINNED_TRAEFIK)
        .ok_or_else(|| GatewayError::UnsupportedPlatform(os.into(), arch.into()))?;

    let sessao = lr_fetch::Session::new(&layout.raiz)?;
    let client = lr_fetch::client(USER_AGENT)?;
    // Os assets do Traefik trazem `digest` na API do GitHub, então o mesmo
    // caminho de verificação do runtime do llama.cpp serve sem código novo.
    let digests = lr_fetch::github_release_digests(&client, REPO, PINNED_TRAEFIK).await;

    let parte = sessao.path().join(format!("{asset}.part"));
    let nome = asset.clone();
    lr_fetch::download_to(
        &client,
        &asset_url(PINNED_TRAEFIK, &asset),
        &parte,
        &|received_bytes, total_bytes| {
            on_event(GatewayEvent::Progress {
                asset: nome.clone(),
                received_bytes,
                total_bytes,
            });
        },
    )
    .await?;
    lr_fetch::verify_sha256(&parte, &asset, digests.as_ref()).await?;

    on_event(GatewayEvent::Extracting {
        asset: asset.clone(),
    });
    let extraido = sessao.path().join("extract");
    lr_fetch::extract_archive_async(parte.clone(), asset.clone(), extraido.clone()).await?;
    let _ = std::fs::remove_file(&parte);

    let raiz = lr_fetch::find_dir_containing(&extraido, traefik_exe_name()).ok_or_else(|| {
        GatewayError::Verification(format!("{} não encontrado no pacote", traefik_exe_name()))
    })?;
    let staging = sessao.path().join("install");
    lr_fetch::move_dir_contents(&raiz, &staging)?;
    lr_fetch::install_atomically(&staging, &layout.bin_dir())?;
    Ok(())
}

pub fn desinstalar(layout: &Layout) -> std::io::Result<()> {
    lr_fetch::remove_dir_all_retrying(&layout.raiz)
}

/// Grava as duas configurações. As rotas podem ser reescritas com o Traefik
/// no ar: o provider `file` com `watch: true` recarrega sozinho, e é a única
/// elegância que ele traz de graça aqui.
pub fn escrever_config(
    layout: &Layout,
    porta: u16,
    expor_lan: bool,
    rotas: &[Route],
) -> std::io::Result<()> {
    std::fs::create_dir_all(layout.dynamic_dir())?;
    std::fs::write(
        layout.static_yml(),
        render_static(porta, expor_lan, &layout.dynamic_dir()),
    )?;
    std::fs::write(layout.routes_yml(), render_dynamic(rotas))
}

// ----------------------------------------------------------------- processo ---

/// Espera o `/ping` do Traefik responder, sem depender do objeto.
///
/// Mesmo motivo do 9router: quem chama solta o mutex antes de esperar.
pub async fn aguardar_pronto(porta: u16, prazo: Duration) -> Result<(), GatewayError> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap_or_default();
    let limite = tokio::time::Instant::now() + prazo;
    loop {
        if http
            .get(format!("http://127.0.0.1:{porta}/ping"))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
        {
            return Ok(());
        }
        if tokio::time::Instant::now() >= limite {
            return Err(GatewayError::Timeout(prazo.as_secs()));
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

pub struct Gateway {
    filho: Option<tokio::process::Child>,
    job: Option<lr_proc::JobGuard>,
    porta: u16,
    http: reqwest::Client,
}

impl Gateway {
    pub fn spawn(layout: &Layout, porta: u16) -> Result<Self, GatewayError> {
        if !layout.instalado() {
            return Err(GatewayError::Verification("Traefik não instalado".into()));
        }
        let mut cmd = tokio::process::Command::new(layout.exe());
        cmd.arg(format!("--configFile={}", layout.static_yml().display()))
            .current_dir(layout.bin_dir())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        lr_proc::prepare(&mut cmd);

        let filho = lr_proc::spawn_supervised(&mut cmd)?;
        let job = lr_proc::attach_job(&filho);
        Ok(Self {
            filho: Some(filho),
            job,
            porta,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(3))
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

    /// `/ping` é o health check nativo do Traefik.
    pub async fn pronto(&self) -> bool {
        self.http
            .get(format!("http://127.0.0.1:{}/ping", self.porta))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    pub async fn wait_ready(&self, prazo: Duration) -> Result<(), GatewayError> {
        aguardar_pronto(self.porta, prazo).await
    }

    pub fn stop_blocking(&mut self) {
        let pid = self.pid();
        if let Some(job) = self.job.take() {
            lr_proc::terminate_job(&job);
        } else if let Some(pid) = pid {
            lr_proc::kill_process_tree(pid);
        }
        if let Some(mut filho) = self.filho.take() {
            let _ = filho.start_kill();
            lr_proc::reap_child(&mut filho);
        }
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        self.stop_blocking();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_traefik_asset_matches_the_platform() {
        assert_eq!(
            traefik_asset("linux", "x86_64", "v3.7.10").unwrap(),
            "traefik_v3.7.10_linux_amd64.tar.gz"
        );
        assert_eq!(
            traefik_asset("windows", "x86_64", "v3.7.10").unwrap(),
            "traefik_v3.7.10_windows_amd64.zip"
        );
        assert!(traefik_asset("freebsd", "x86_64", "v1").is_none());
    }

    #[test]
    fn the_dynamic_config_routes_the_local_prefix_to_the_engine_port() {
        let yaml = render_dynamic(&rotas_ativas(Some(11711), None));
        assert!(yaml.contains("PathPrefix(`/local`)"));
        assert!(yaml.contains("http://127.0.0.1:11711"));
    }

    /// Rota para serviço parado devolveria 502 e pareceria defeito do gateway.
    #[test]
    fn a_route_to_a_stopped_service_is_not_rendered() {
        let yaml = render_dynamic(&rotas_ativas(Some(11711), None));
        assert!(!yaml.contains("/9router"));

        let com_ambos = render_dynamic(&rotas_ativas(Some(11711), Some(20128)));
        assert!(com_ambos.contains("/9router"));
        assert!(com_ambos.contains("http://127.0.0.1:20128"));
    }

    /// O provedor atrás não conhece o prefixo: `/local/v1/models` precisa
    /// chegar nele como `/v1/models`.
    #[test]
    fn every_route_strips_its_prefix_before_forwarding() {
        let yaml = render_dynamic(&rotas_ativas(Some(11711), Some(20128)));
        assert!(yaml.contains("local-strip"));
        assert!(yaml.contains("ninerouter-strip"));
        assert_eq!(yaml.matches("stripPrefix").count(), 2);
    }

    /// Saída instável faria o Traefik recarregar a cada escrita sem motivo.
    #[test]
    fn the_rendered_yaml_is_stable_for_the_same_routes() {
        let r = rotas_ativas(Some(1), Some(2));
        assert_eq!(render_dynamic(&r), render_dynamic(&r));
    }

    /// Arquivo vazio faz o Traefik reclamar; `http: {}` limpa as rotas.
    #[test]
    fn no_active_routes_render_an_empty_but_valid_document() {
        assert_eq!(render_dynamic(&[]), "http: {}\n");
    }

    #[test]
    fn the_static_config_binds_to_loopback_by_default() {
        let yaml = render_static(11700, false, Path::new("/tmp/d"));
        assert!(yaml.contains("\"127.0.0.1:11700\""));
        // Painel do Traefik desligado: seria mais superfície sem ganho.
        assert!(yaml.contains("dashboard: false"));
        assert!(yaml.contains("insecure: false"));
    }

    /// Expor na rede é opt-in explícito, nunca padrão.
    #[test]
    fn exposing_on_the_network_is_opt_in() {
        let yaml = render_static(11700, true, Path::new("/tmp/d"));
        assert!(yaml.contains("\"0.0.0.0:11700\""));
    }

    #[test]
    fn writing_the_configuration_creates_both_files() {
        let dir = tempfile::tempdir().unwrap();
        let l = Layout::new(dir.path());
        escrever_config(&l, 11700, false, &rotas_ativas(Some(11711), None)).unwrap();
        assert!(l.static_yml().is_file());
        assert!(l.routes_yml().is_file());
        assert!(!l.instalado());
    }

    /// Live: confere que a versão pinada existe com os assets que pedimos.
    #[tokio::test]
    #[ignore = "rede: consulta a API de releases do GitHub"]
    async fn live_pinned_release_has_our_assets() {
        let client = lr_fetch::client(USER_AGENT).unwrap();
        let digests = lr_fetch::github_release_digests(&client, REPO, PINNED_TRAEFIK)
            .await
            .expect("API do GitHub indisponível");
        for (os, arch) in [
            ("linux", "x86_64"),
            ("windows", "x86_64"),
            ("macos", "aarch64"),
        ] {
            let a = traefik_asset(os, arch, PINNED_TRAEFIK).unwrap();
            assert!(digests.contains_key(&a), "sem digest para {a}");
        }
    }
}
