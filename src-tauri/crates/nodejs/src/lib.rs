//! Node.js portátil, instalado e usado sem tocar no Node do sistema.
//!
//! O 9router só é distribuído como pacote npm — não há binário pronto. Em vez
//! de exigir que a pessoa instale Node (e de brigar com a versão que ela já
//! tem), o app baixa uma distribuição oficial para uma pasta própria. Assim o
//! botão "instalar" continua sendo um clique, e "desinstalar" é apagar pasta.
//!
//! Duas decisões que evitam dor de cabeça e não são óbvias:
//!
//! - **Nunca invocar `npm`/`npm.cmd` pelo nome.** O `.cmd` do Windows é um
//!   batch: exige `cmd.exe`, abre console e tem regras próprias de aspas; no
//!   Unix é symlink. Rodar `<node> <caminho>/npm-cli.js` é idêntico nos três
//!   sistemas.
//! - **`--prefix` sozinho não isola.** Sem apontar `npm_config_userconfig` e
//!   `globalconfig` para um arquivo nosso, o npm lê o `~/.npmrc` da pessoa e
//!   herda registry corporativo e token dela.

use std::path::{Path, PathBuf};

/// Versão fixada.
///
/// **Não rebaixar para menos que 22.** O `engines.node >= 18` do 9router é
/// folgado, mas o Code Mode roda o programa do agente com `--permission
/// --allow-fs-read=<scratch>`, e essa flag só existe a partir do Node 22: numa
/// versão anterior o programa roda sem isolamento nenhum. Pinar também é o que
/// torna a instalação reprodutível — mesma escolha do `PINNED_TAG` do
/// llama.cpp.
pub const PINNED_NODE: &str = "v22.20.0";

const USER_AGENT: &str = concat!("OpenWeights/", env!("CARGO_PKG_VERSION"));

/// Sanidade pós-extração: uma distribuição completa passa fácil disso.
const MIN_INSTALL_SIZE: u64 = 20 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error("plataforma sem distribuição oficial do Node: {0}/{1}")]
    UnsupportedPlatform(String, String),
    #[error("falha ao baixar ou extrair: {0}")]
    Fetch(#[from] lr_fetch::FetchError),
    #[error("falha de rede: {0}")]
    Network(#[from] reqwest::Error),
    #[error("falha de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("instalação inválida: {0}")]
    Verification(String),
}

/// Progresso da instalação, no mesmo formato que a UI já consome do runtime.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum NodeEvent {
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

/// Qual pacote baixar nesta máquina.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeAsset {
    pub file: String,
}

/// Escolhe o pacote a partir do `os`/`arch` do `HardwareProfile`.
///
/// A decisão é em tempo de execução, não de compilação: o build de macOS é
/// universal (um binário para Intel e Apple Silicon), então perguntar ao
/// `cfg!` daria a resposta errada na metade das máquinas.
///
/// Sempre `.tar.gz` fora do Windows — o Node também publica `.tar.xz`, mas
/// usá-lo custaria uma dependência de liblzma para não ganhar nada.
pub fn node_asset(os: &str, arch: &str, versao: &str) -> Option<NodeAsset> {
    let alvo = match (os, arch) {
        ("windows", "x86_64") => "win-x64.zip",
        ("windows", "aarch64") => "win-arm64.zip",
        ("macos", "aarch64") => "darwin-arm64.tar.gz",
        ("macos", "x86_64") => "darwin-x64.tar.gz",
        ("linux", "x86_64") => "linux-x64.tar.gz",
        ("linux", "aarch64") => "linux-arm64.tar.gz",
        _ => return None,
    };
    Some(NodeAsset {
        file: format!("node-{versao}-{alvo}"),
    })
}

pub fn asset_url(versao: &str, arquivo: &str) -> String {
    format!("https://nodejs.org/dist/{versao}/{arquivo}")
}

pub fn shasums_url(versao: &str) -> String {
    format!("https://nodejs.org/dist/{versao}/SHASUMS256.txt")
}

/// Nome do executável do Node no sistema.
pub const fn node_exe_name() -> &'static str {
    if cfg!(windows) { "node.exe" } else { "node" }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeState {
    pub version: String,
    pub installed: bool,
    pub node_exe: Option<PathBuf>,
}

/// Gerencia o Node em `<data_dir>/providers/node/<versão>/`.
pub struct NodeManager {
    raiz: PathBuf,
    os: String,
    arch: String,
    /// Serializa instalações concorrentes, como o `RuntimeManager`.
    install_lock: tokio::sync::Mutex<()>,
}

impl NodeManager {
    /// `raiz` é a pasta de provedores (`<data_dir>/providers`).
    pub fn new(raiz: PathBuf, os: impl Into<String>, arch: impl Into<String>) -> Self {
        Self {
            raiz,
            os: os.into(),
            arch: arch.into(),
            install_lock: tokio::sync::Mutex::new(()),
        }
    }

    fn dir_versao(&self, versao: &str) -> PathBuf {
        self.raiz.join("node").join(versao)
    }

    /// Pasta que contém o executável. No Windows o Node fica na raiz da
    /// distribuição; nos demais, em `bin/`.
    pub fn bin_dir(&self) -> PathBuf {
        let base = self.dir_versao(PINNED_NODE);
        if self.os == "windows" {
            base
        } else {
            base.join("bin")
        }
    }

    pub fn node_exe(&self) -> Option<PathBuf> {
        let p = self.bin_dir().join(node_exe_name());
        p.is_file().then_some(p)
    }

    /// Caminho do `npm-cli.js`, que é como invocamos o npm.
    pub fn npm_cli(&self) -> Option<PathBuf> {
        let base = self.dir_versao(PINNED_NODE);
        // Windows: `node_modules/npm`; Unix: `lib/node_modules/npm`.
        [
            base.join("node_modules/npm/bin/npm-cli.js"),
            base.join("lib/node_modules/npm/bin/npm-cli.js"),
        ]
        .into_iter()
        .find(|c| c.is_file())
    }

    pub fn state(&self) -> NodeState {
        let exe = self.node_exe();
        NodeState {
            version: PINNED_NODE.to_string(),
            installed: exe.is_some(),
            node_exe: exe,
        }
    }

    /// Garante o Node instalado, baixando se preciso.
    pub async fn ensure(
        &self,
        on_event: impl Fn(NodeEvent) + Send + Sync,
    ) -> Result<NodeState, NodeError> {
        let _guarda = self.install_lock.lock().await;
        let estado = self.state();
        if estado.installed {
            return Ok(estado);
        }
        match self.instalar(&on_event).await {
            Ok(()) => {
                on_event(NodeEvent::Ready);
                Ok(self.state())
            }
            Err(e) => {
                on_event(NodeEvent::Failed {
                    message: e.to_string(),
                });
                Err(e)
            }
        }
    }

    async fn instalar<F>(&self, on_event: &F) -> Result<(), NodeError>
    where
        F: Fn(NodeEvent) + Send + Sync,
    {
        let asset = node_asset(&self.os, &self.arch, PINNED_NODE)
            .ok_or_else(|| NodeError::UnsupportedPlatform(self.os.clone(), self.arch.clone()))?;
        let sessao = lr_fetch::Session::new(&self.raiz)?;
        let client = lr_fetch::client(USER_AGENT)?;

        // Checksum obrigatório: o nodejs.org publica o SHASUMS de todo
        // release, então aqui não há a desculpa de "digest indisponível" que
        // o runtime do llama.cpp precisa aceitar.
        let esperado = self.baixar_checksum(&client, &asset.file).await?;

        let parte = sessao.path().join(format!("{}.part", asset.file));
        let nome = asset.file.clone();
        lr_fetch::download_to(
            &client,
            &asset_url(PINNED_NODE, &asset.file),
            &parte,
            &|received_bytes, total_bytes| {
                on_event(NodeEvent::Progress {
                    asset: nome.clone(),
                    received_bytes,
                    total_bytes,
                });
            },
        )
        .await?;
        lr_fetch::verify_sha256_strict(&parte, &asset.file, &esperado).await?;

        on_event(NodeEvent::Extracting {
            asset: asset.file.clone(),
        });
        let extraido = sessao.path().join("extract");
        lr_fetch::extract_archive_async(parte.clone(), asset.file.clone(), extraido.clone())
            .await?;
        let _ = std::fs::remove_file(&parte);

        // O pacote traz uma pasta `node-<versão>-<plataforma>/` por fora;
        // procurar pelo executável dispensa reproduzir esse nome aqui.
        let raiz_exe =
            lr_fetch::find_dir_containing(&extraido, node_exe_name()).ok_or_else(|| {
                NodeError::Verification(format!("{} não encontrado no pacote", node_exe_name()))
            })?;
        // No Unix o exe está em `bin/`: a raiz da distribuição é o pai.
        let raiz_dist = if self.os == "windows" {
            raiz_exe
        } else {
            raiz_exe
                .parent()
                .ok_or_else(|| NodeError::Verification("pacote com layout inesperado".into()))?
                .to_path_buf()
        };

        let staging = sessao.path().join("install");
        lr_fetch::move_dir_contents(&raiz_dist, &staging)?;

        let tamanho = lr_fetch::dir_size(&staging);
        if tamanho < MIN_INSTALL_SIZE {
            return Err(NodeError::Verification(format!(
                "distribuição do Node suspeita de incompleta ({tamanho} bytes)"
            )));
        }

        lr_fetch::install_atomically(&staging, &self.dir_versao(PINNED_NODE))?;
        log::info!(
            "Node {PINNED_NODE} instalado em {}",
            self.bin_dir().display()
        );
        Ok(())
    }

    async fn baixar_checksum(
        &self,
        client: &reqwest::Client,
        arquivo: &str,
    ) -> Result<String, NodeError> {
        let texto = client
            .get(shasums_url(PINNED_NODE))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        lr_fetch::parse_shasums256(&texto)
            .remove(arquivo)
            .ok_or_else(|| {
                NodeError::Verification(format!("SHASUMS256.txt sem entrada para {arquivo}"))
            })
    }

    /// Remove a instalação. Desinstalar é apagar pasta — a promessa do
    /// isolamento.
    pub fn uninstall(&self) -> std::io::Result<()> {
        lr_fetch::remove_dir_all_retrying(&self.raiz.join("node"))
    }

    /// Arquivo `.npmrc` vazio e nosso, no nível "user".
    ///
    /// Sem apontar o npm para ele, a instalação herdaria o `~/.npmrc` da
    /// pessoa — inclusive registry privado e token de autenticação.
    pub fn npmrc(&self) -> PathBuf {
        self.raiz.join("npmrc")
    }

    /// O mesmo, no nível "global".
    ///
    /// Precisa ser um arquivo DIFERENTE do `npmrc`: apontar o mesmo caminho
    /// para os dois níveis faz o npm abortar antes de resolver a
    /// configuração, com "double-loading config ... as global, previously
    /// loaded as user". Descoberto rodando a instalação de verdade.
    pub fn npmrc_global(&self) -> PathBuf {
        self.raiz.join("npmrc-global")
    }

    pub fn npm_cache(&self) -> PathBuf {
        self.raiz.join("npm-cache")
    }

    /// Variáveis que isolam o npm e deixam o Node portátil no `PATH`.
    ///
    /// O `PATH` importa mesmo depois da instalação: o 9router chama `npm` em
    /// tempo de execução para preparar as dependências de SQLite dele.
    pub fn env_isolado(&self, prefix: &Path) -> Vec<(String, String)> {
        let path_atual = std::env::var("PATH").unwrap_or_default();
        let separador = if cfg!(windows) { ";" } else { ":" };
        vec![
            (
                "PATH".into(),
                format!("{}{separador}{path_atual}", self.bin_dir().display()),
            ),
            ("npm_config_prefix".into(), prefix.display().to_string()),
            (
                "npm_config_cache".into(),
                self.npm_cache().display().to_string(),
            ),
            (
                "npm_config_userconfig".into(),
                self.npmrc().display().to_string(),
            ),
            (
                "npm_config_globalconfig".into(),
                self.npmrc_global().display().to_string(),
            ),
            ("npm_config_audit".into(), "false".into()),
            ("npm_config_fund".into(), "false".into()),
            ("npm_config_update_notifier".into(), "false".into()),
            ("NO_UPDATE_NOTIFIER".into(), "1".into()),
        ]
    }

    /// Monta `<node> <npm-cli.js> <args>` com o ambiente isolado.
    pub fn npm_command(
        &self,
        args: &[&str],
        cwd: &Path,
        prefix: &Path,
    ) -> Option<tokio::process::Command> {
        let node = self.node_exe()?;
        let npm = self.npm_cli()?;
        // Garante que o `.npmrc` nosso existe: o npm ignora o que não acha,
        // mas criá-lo deixa explícito que a configuração é vazia de propósito.
        let _ = std::fs::create_dir_all(&self.raiz);
        for arquivo in [self.npmrc(), self.npmrc_global()] {
            if !arquivo.exists() {
                let _ = std::fs::write(arquivo, b"");
            }
        }
        let mut cmd = tokio::process::Command::new(node);
        // A flag vai na origem, não no chamador: quem recebe este `Command`
        // não tem como saber que faltava, e um `npm install` com console
        // aberto é uma janela preta na cara do usuário.
        lr_proc::no_window(&mut cmd);
        cmd.arg(npm).args(args).current_dir(cwd);
        for (k, v) in self.env_isolado(prefix) {
            cmd.env(k, v);
        }
        Some(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_windows_asset_is_a_zip_and_the_others_are_tar_gz() {
        assert!(
            node_asset("windows", "x86_64", "v22.20.0")
                .unwrap()
                .file
                .ends_with("win-x64.zip")
        );
        assert!(
            node_asset("macos", "aarch64", "v22.20.0")
                .unwrap()
                .file
                .ends_with("darwin-arm64.tar.gz")
        );
    }

    /// O release de macOS é universal: a arquitetura só se sabe rodando.
    #[test]
    fn macos_resolves_each_architecture_separately() {
        assert_eq!(
            node_asset("macos", "aarch64", "v1").unwrap().file,
            "node-v1-darwin-arm64.tar.gz"
        );
        assert_eq!(
            node_asset("macos", "x86_64", "v1").unwrap().file,
            "node-v1-darwin-x64.tar.gz"
        );
    }

    /// `.tar.gz` e não `.tar.xz`: é o que dispensa uma dependência de liblzma.
    #[test]
    fn linux_x64_uses_tar_gz_so_no_xz_dependency_is_needed() {
        assert_eq!(
            node_asset("linux", "x86_64", "v22.20.0").unwrap().file,
            "node-v22.20.0-linux-x64.tar.gz"
        );
    }

    /// O Code Mode depende de `--permission`, que não existe antes do Node 22:
    /// rebaixar o pino tiraria o isolamento do programa do agente junto.
    #[test]
    fn the_pinned_node_is_at_least_22() {
        let maior: u32 = PINNED_NODE
            .trim_start_matches('v')
            .split('.')
            .next()
            .and_then(|n| n.parse().ok())
            .expect("versão no formato vX.Y.Z");
        assert!(maior >= 22, "Node {PINNED_NODE} não tem --permission");
    }

    #[test]
    fn an_unknown_platform_has_no_asset() {
        assert!(node_asset("freebsd", "x86_64", "v1").is_none());
        assert!(node_asset("linux", "riscv64", "v1").is_none());
    }

    #[test]
    fn the_asset_url_points_at_the_official_dist() {
        assert_eq!(
            asset_url("v22.20.0", "node-v22.20.0-linux-x64.tar.gz"),
            "https://nodejs.org/dist/v22.20.0/node-v22.20.0-linux-x64.tar.gz"
        );
    }

    fn manager(os: &str) -> (tempfile::TempDir, NodeManager) {
        let dir = tempfile::tempdir().unwrap();
        let m = NodeManager::new(dir.path().to_path_buf(), os, "x86_64");
        (dir, m)
    }

    #[test]
    fn windows_keeps_node_at_the_distribution_root_and_unix_uses_bin() {
        let (_d, win) = manager("windows");
        assert!(win.bin_dir().ends_with(PINNED_NODE));
        let (_d2, nix) = manager("linux");
        assert!(nix.bin_dir().ends_with("bin"));
    }

    /// Sem isto o npm lê o `~/.npmrc` da pessoa: registry privado e token.
    #[test]
    fn the_npm_environment_ignores_the_user_npmrc() {
        let (_d, m) = manager("linux");
        let env = m.env_isolado(Path::new("/tmp/prefixo"));
        let mapa: std::collections::HashMap<_, _> = env.into_iter().collect();
        assert_eq!(
            mapa.get("npm_config_userconfig").map(String::as_str),
            Some(m.npmrc().display().to_string()).as_deref()
        );
        // Precisam ser arquivos DIFERENTES: o npm aborta se o mesmo caminho
        // for carregado nos dois níveis.
        assert_eq!(
            mapa.get("npm_config_globalconfig").map(String::as_str),
            Some(m.npmrc_global().display().to_string()).as_deref()
        );
        assert_ne!(m.npmrc(), m.npmrc_global());
        assert_eq!(mapa.get("npm_config_prefix").unwrap(), "/tmp/prefixo");
    }

    /// O 9router chama `npm` em tempo de execução; sem o Node no PATH ele
    /// acharia o do sistema, ou nenhum.
    #[test]
    fn the_portable_node_comes_first_in_the_path() {
        let (_d, m) = manager("linux");
        let env = m.env_isolado(Path::new("/tmp/p"));
        let path = env.iter().find(|(k, _)| k == "PATH").unwrap().1.clone();
        assert!(path.starts_with(&m.bin_dir().display().to_string()));
    }

    #[test]
    fn a_missing_install_reports_not_installed() {
        let (_d, m) = manager("linux");
        assert!(!m.state().installed);
        assert!(m.node_exe().is_none());
    }

    /// Teste live: confere que a versão pinada existe e tem o checksum.
    #[tokio::test]
    #[ignore = "rede: consulta o nodejs.org"]
    async fn live_pinned_version_publishes_our_assets() {
        let client = lr_fetch::client(USER_AGENT).unwrap();
        let texto = client
            .get(shasums_url(PINNED_NODE))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let mapa = lr_fetch::parse_shasums256(&texto);
        for (os, arch) in [
            ("windows", "x86_64"),
            ("macos", "aarch64"),
            ("macos", "x86_64"),
            ("linux", "x86_64"),
            ("linux", "aarch64"),
        ] {
            let a = node_asset(os, arch, PINNED_NODE).unwrap();
            assert!(mapa.contains_key(&a.file), "sem checksum para {}", a.file);
        }
    }
}
