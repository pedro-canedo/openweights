//! O DeepSeek Harness (dsh) instalado, supervisionado e configurado pelo app.
//!
//! O dsh é distribuído só como pacote npm (`@deepseek-ai/dsh`). Este crate
//! segue o molde do `lr_ninerouter`, simplificado: pacote pinado numa pasta
//! nossa (`<providers>/dsh/app`, npm com prefixo isolado, nunca `-g`),
//! processo `dsh web` supervisionado e derrubado junto do app.
//!
//! Fatos do pacote real (0.1.1-rc.2) que ditam o formato daqui — verificados
//! numa instalação de verdade, não na documentação:
//!
//! - **`--port 0` + porta lida do stdout.** `EADDRINUSE` MATA o processo (sem
//!   fallback de porta), então pedimos porta 0 ao SO e parseamos a linha
//!   `dsh web: http://127.0.0.1:<porta>` que o próprio dsh imprime. Com
//!   `--no-open` o filho que abriria o browser (e que herda o stdout,
//!   poluindo o stream) nem é criado.
//! - **A UI web não tem autenticação.** A única proteção é o bind em
//!   loopback — que é o padrão do dsh, e `--host 0.0.0.0` é recusado por ele
//!   mesmo. Nada a forçar por aqui, só a não mudar.
//! - **Encerramento**: `SIGTERM` → dispose com timeout interno de 5 s → exit
//!   0. Por isso a graça de ~6 s antes do `kill_process_tree`. No Windows não
//!   há sinal equivalente: o Job Object encerra a árvore.
//! - **`DSH_HOME`** aponta o perfil (settings, sessões, credenciais) para uma
//!   pasta do app; vazio é ignorado e cai em `~/.dsh` — nunca mandar vazio.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use lr_nodejs::NodeManager;

pub mod settings;

/// Versão fixada do `@deepseek-ai/dsh`. O monorepo publica todos os ~190
/// pacotes `dsh-*` em lockstep, então pinar o CLI pina o conjunto.
pub const PINNED_DSH: &str = "0.1.1-rc.2";

/// Nome do pacote npm.
pub const NPM_PACKAGE: &str = "@deepseek-ai/dsh";

/// Prazo de readiness. O primeiro boot inicializa o perfil em
/// `$DSH_HOME/profiles/web` além de subir o servidor — em disco frio passa
/// fácil dos 20 s.
pub const READY_TIMEOUT: Duration = Duration::from_secs(60);

/// Teto do `npm install` — mesmo racional do 9router: antivírus + dezenas de
/// milhares de arquivos tornam minutos normais, pendurar para sempre não.
pub const INSTALL_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Graça entre o pedido de parada e a força bruta. O dispose do dsh tem
/// timeout interno de 5 s; 6 dá folga para ele terminar por bem.
pub const STOP_GRACE: Duration = Duration::from_secs(6);

#[derive(Debug, thiserror::Error)]
pub enum DshError {
    #[error("o Node portátil não está instalado")]
    NodeMissing,
    #[error("falha de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("npm falhou ({codigo}): {detalhe}")]
    Npm { codigo: String, detalhe: String },
    #[error("o dsh não respondeu em {0} s")]
    Timeout(u64),
    #[error("instalação inválida: {0}")]
    Verification(String),
    #[error("settings.yaml travado por outro processo (lock não liberou)")]
    SettingsLock,
}

/// Progresso de instalação/subida, no MESMO formato serializado dos eventos
/// do 9router (`kind` + camelCase): a tela já sabe desenhar esse shape e o
/// canal (`provider`) é o mesmo.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum DshEvent {
    Installing { phase: String },
    Log { line: String },
    Ready,
    Failed { message: String },
}

/// Fase legível a partir de uma linha do npm — mesma heurística (assumida) do
/// 9router: serve para a tela dizer algo mais útil que "aguarde". Linha não
/// reconhecida mantém a fase anterior (devolve `None`).
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

/// Onde a instalação do dsh vive.
#[derive(Debug, Clone)]
pub struct Layout {
    /// `<data_dir>/providers/dsh`
    pub raiz: PathBuf,
}

impl Layout {
    pub fn new(providers_dir: &Path) -> Self {
        Self {
            raiz: providers_dir.join("dsh"),
        }
    }

    /// Prefixo do npm: o pacote cai em `app/node_modules/@deepseek-ai/dsh`.
    pub fn app(&self) -> PathBuf {
        self.raiz.join("app")
    }

    /// Raiz do pacote instalado.
    pub fn pacote(&self) -> PathBuf {
        self.app().join("node_modules/@deepseek-ai/dsh")
    }

    /// O script que o Node executa (`dsh web ...`).
    ///
    /// Preferimos `lib/bin.js` direto (caminho verificado na 0.1.1-rc.2) a
    /// `node_modules/.bin/dsh`: o `.bin` do Windows é um shim `.cmd`/`.ps1`
    /// que exigiria `cmd.exe`, e no Unix é só um symlink para este arquivo.
    /// Se o upstream mover o arquivo numa versão futura, o campo `bin` do
    /// `package.json` do pacote é o fallback.
    pub fn bin_js(&self) -> PathBuf {
        let fixo = self.pacote().join("lib/bin.js");
        if fixo.is_file() {
            return fixo;
        }
        bin_do_manifesto(&self.pacote()).unwrap_or(fixo)
    }

    pub fn instalado(&self) -> bool {
        self.bin_js().is_file()
    }
}

/// Resolve o script do CLI pelo campo `bin` do `package.json` do pacote —
/// aceita as duas formas do npm (string ou objeto `{ "dsh": "caminho" }`).
fn bin_do_manifesto(pacote: &Path) -> Option<PathBuf> {
    let texto = std::fs::read_to_string(pacote.join("package.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&texto).ok()?;
    let rel = match v.get("bin")? {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(m) => m.get("dsh")?.as_str()?.to_string(),
        _ => return None,
    };
    let p = pacote.join(rel.trim_start_matches("./"));
    p.is_file().then_some(p)
}

// ------------------------------------------------------------- instalação ---

/// Argumentos do `npm install` do dsh.
///
/// Função pura para travar em teste as decisões que causam dano se
/// regredirem: a versão pinada e a ausência de `-g` (global escaparia da
/// pasta isolada e misturaria o app com o runtime).
pub fn npm_install_args(versao: &str, ignorar_scripts: bool) -> Vec<String> {
    let mut args = vec![
        "install".to_string(),
        format!("{NPM_PACKAGE}@{versao}"),
        "--no-audit".to_string(),
        "--no-fund".to_string(),
        "--loglevel=info".to_string(),
    ];
    if ignorar_scripts {
        args.push("--ignore-scripts".to_string());
    }
    args
}

/// Instala o dsh na pasta isolada.
///
/// Primeiro com `--ignore-scripts` (não rodar script de ciclo de vida de
/// terceiros é a única mitigação real de um `npm install`); se falhar, repete
/// sem a flag avisando — mesmo desenho do 9router.
pub async fn instalar(
    node: &NodeManager,
    layout: &Layout,
    on_event: &(dyn Fn(DshEvent) + Send + Sync),
) -> Result<(), DshError> {
    std::fs::create_dir_all(layout.app())?;

    match rodar_npm(node, layout, true, on_event).await {
        Ok(()) => Ok(()),
        Err(e) => {
            log::warn!("npm com --ignore-scripts falhou ({e}); repetindo sem a flag");
            on_event(DshEvent::Log {
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
    on_event: &(dyn Fn(DshEvent) + Send + Sync),
) -> Result<(), DshError> {
    use tokio::io::AsyncBufReadExt as _;

    let args = npm_install_args(PINNED_DSH, ignorar_scripts);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let app = layout.app();
    let mut cmd = node
        .npm_command(&refs, &app, &app)
        .ok_or(DshError::NodeMissing)?;
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    lr_proc::prepare(&mut cmd);

    let mut filho = lr_proc::spawn_supervised(&mut cmd)?;
    let stdout = filho.stdout.take();
    let stderr = filho.stderr.take();

    // O log ao vivo substitui a barra de progresso: o npm não publica
    // porcentagem, e uma tela muda por minutos parece travada.
    let mut linhas = Vec::new();
    if let Some(saida) = stdout {
        let mut leitor = tokio::io::BufReader::new(saida).lines();
        while let Ok(Some(linha)) = leitor.next_line().await {
            if let Some(fase) = fase_do_npm(&linha) {
                on_event(DshEvent::Installing {
                    phase: fase.to_string(),
                });
            }
            on_event(DshEvent::Log {
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
            return Err(DshError::Timeout(INSTALL_TIMEOUT.as_secs()));
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
        return Err(DshError::Npm {
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
        return Err(DshError::Verification(
            "bin do dsh não encontrado após a instalação".to_string(),
        ));
    }
    Ok(())
}

// --------------------------------------------------------------- execução ---

/// Argumentos do `dsh web`.
///
/// `--port 0` porque `EADDRINUSE` mata o processo do dsh sem fallback — pedir
/// porta ao SO nunca colide; a real sai no stdout. `--no-open` porque quem
/// abre a UI é o app (janela própria), e o filho abridor de browser herdaria
/// o stdout que parseamos.
pub fn run_args() -> Vec<String> {
    vec![
        "web".to_string(),
        "--port".to_string(),
        "0".to_string(),
        "--no-open".to_string(),
    ]
}

/// Ambiente do processo `dsh web`.
#[derive(Debug, Clone)]
pub struct RunEnv {
    /// `DSH_HOME`: settings, sessões e credenciais numa pasta do app.
    pub dsh_home: PathBuf,
    /// Envs de chave (`OPENWEIGHTS_API_KEY`, …). Valor vazio não é exportado:
    /// env vazia dispara MISSING_CREDENTIAL no adapter do dsh.
    pub extra_env: Vec<(String, String)>,
}

/// Lê a porta real da linha que o dsh imprime no stdout.
///
/// Formato verificado no pacote (`dsh-web-app/lib/index.js`):
/// `dsh web: http://127.0.0.1:<porta>` — possivelmente com um sufixo
/// ` (LAN: …)` que aqui nunca aparece (host loopback). A linha
/// `dsh web: opening the default browser…` não casa com o prefixo numérico.
pub fn parse_porta(linha: &str) -> Option<u16> {
    let resto = linha.trim().strip_prefix("dsh web: http://127.0.0.1:")?;
    let digitos: String = resto.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digitos.is_empty() {
        return None;
    }
    digitos.parse().ok()
}

/// Espera a porta aparecer no handle preenchido pelo leitor de stdout.
///
/// Função livre (como o `aguardar_pronto` do 9router) para quem chama poder
/// soltar o mutex do processo antes de esperar.
pub async fn aguardar_porta(porta: &Arc<AtomicU16>, prazo: Duration) -> Result<u16, DshError> {
    let limite = tokio::time::Instant::now() + prazo;
    loop {
        let p = porta.load(Ordering::SeqCst);
        if p != 0 {
            return Ok(p);
        }
        if tokio::time::Instant::now() >= limite {
            return Err(DshError::Timeout(prazo.as_secs()));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Espera o servidor web do dsh atender: TCP primeiro (distingue "subindo" de
/// "morto"), depois `GET /` respondendo 200 (o SPA estático).
pub async fn aguardar_pronto(porta: u16, prazo: Duration) -> Result<(), DshError> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default();
    let limite = tokio::time::Instant::now() + prazo;
    loop {
        if lr_proc::port_in_use(porta)
            && http
                .get(format!("http://127.0.0.1:{porta}/"))
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
        {
            return Ok(());
        }
        if tokio::time::Instant::now() >= limite {
            return Err(DshError::Timeout(prazo.as_secs()));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Processo `dsh web` em execução.
pub struct DshHost {
    filho: Option<tokio::process::Child>,
    job: Option<lr_proc::JobGuard>,
    /// Porta real, preenchida por quem lê o stdout (0 = ainda desconhecida).
    porta: Arc<AtomicU16>,
    http: reqwest::Client,
}

impl DshHost {
    pub fn spawn(node: &NodeManager, layout: &Layout, env: &RunEnv) -> Result<Self, DshError> {
        let exe = node.node_exe().ok_or(DshError::NodeMissing)?;
        if !layout.instalado() {
            return Err(DshError::Verification("dsh não instalado".to_string()));
        }
        // O DSH_HOME precisa existir antes do boot: é onde o launcher
        // inicializa o perfil e onde o settings.yaml (já escrito pelo app)
        // mora.
        std::fs::create_dir_all(&env.dsh_home)?;

        let mut cmd = tokio::process::Command::new(exe);
        cmd.arg(layout.bin_js())
            .args(run_args())
            .current_dir(layout.app())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        // O Node portátil no PATH: o dsh delega a gestão de plugins ao pnpm
        // em runtime, e o isolamento do npm evita herdar o ~/.npmrc alheio.
        for (k, v) in node.env_isolado(&layout.app()) {
            cmd.env(k, v);
        }
        cmd.env("DSH_HOME", &env.dsh_home);
        for (k, v) in &env.extra_env {
            // Env vazia é pior que ausente: MISSING_CREDENTIAL no adapter.
            if !v.is_empty() {
                cmd.env(k, v);
            }
        }
        lr_proc::prepare(&mut cmd);

        let filho = lr_proc::spawn_supervised(&mut cmd)?;
        let job = lr_proc::attach_job(&filho);
        Ok(Self {
            filho: Some(filho),
            job,
            porta: Arc::new(AtomicU16::new(0)),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
        })
    }

    pub fn pid(&self) -> Option<u32> {
        self.filho.as_ref().and_then(|c| c.id())
    }

    /// Handle compartilhado da porta, para o leitor de stdout preencher e a
    /// espera de readiness consultar sem segurar o mutex do processo.
    pub fn porta_handle(&self) -> Arc<AtomicU16> {
        Arc::clone(&self.porta)
    }

    /// Porta real, quando já conhecida.
    pub fn porta(&self) -> Option<u16> {
        let p = self.porta.load(Ordering::SeqCst);
        (p != 0).then_some(p)
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

    /// Está atendendo? TCP primeiro, depois `GET /` 200.
    pub async fn pronto(&self) -> bool {
        let Some(porta) = self.porta() else {
            return false;
        };
        if !lr_proc::port_in_use(porta) {
            return false;
        }
        self.http
            .get(format!("http://127.0.0.1:{porta}/"))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
    }

    /// Derruba o processo, com chance de sair por bem primeiro.
    ///
    /// Unix: `SIGTERM` (o launcher do dsh dispara o dispose, com timeout
    /// interno de 5 s, e sai com código 0), graça de [`STOP_GRACE`], e só
    /// então a árvore inteira. Windows: não há sinal equivalente — o Job
    /// Object encerra a árvore. O `kill_process_tree` no fim é a rede de
    /// segurança dos dois mundos.
    pub fn stop_blocking(&mut self) {
        let pid = self.pid();
        #[cfg(unix)]
        if let Some(pid) = pid {
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
            if let Some(filho) = self.filho.as_mut() {
                let limite = std::time::Instant::now() + STOP_GRACE;
                while std::time::Instant::now() < limite {
                    match filho.try_wait() {
                        Ok(Some(_)) => break,
                        Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                        Err(_) => break,
                    }
                }
            }
        }
        if let Some(job) = self.job.take() {
            lr_proc::terminate_job(&job);
        }
        if let Some(pid) = pid {
            lr_proc::kill_process_tree(pid);
        }
        if let Some(mut filho) = self.filho.take() {
            let _ = filho.start_kill();
            lr_proc::reap_child(&mut filho);
        }
    }
}

impl Drop for DshHost {
    fn drop(&mut self) {
        self.stop_blocking();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------- parse da porta ---

    /// A linha real do dsh (`--port 0` resolvido pelo SO).
    #[test]
    fn the_port_is_read_from_the_stdout_line() {
        assert_eq!(parse_porta("dsh web: http://127.0.0.1:3080"), Some(3080));
        assert_eq!(parse_porta("dsh web: http://127.0.0.1:49321"), Some(49321));
    }

    /// Sufixo de LAN não atrapalha (o formato do upstream o prevê, mesmo que
    /// com loopback ele nunca apareça).
    #[test]
    fn a_lan_suffix_does_not_break_the_port_parse() {
        assert_eq!(
            parse_porta("dsh web: http://127.0.0.1:3080 (LAN: http://192.168.0.5:3080)"),
            Some(3080)
        );
    }

    /// A linha do abridor de browser — que com `--no-open` nem existe — e
    /// qualquer outro log não podem virar porta.
    #[test]
    fn other_lines_do_not_parse_as_a_port() {
        assert_eq!(
            parse_porta("dsh web: opening the default browser; pass --no-open to disable"),
            None
        );
        assert_eq!(parse_porta("dsh web: http://127.0.0.1:"), None);
        assert_eq!(parse_porta("qualquer coisa"), None);
        assert_eq!(parse_porta(""), None);
        // Porta fora do u16 é lixo, não pânico.
        assert_eq!(parse_porta("dsh web: http://127.0.0.1:99999999"), None);
    }

    // ---------------------------------------------------- npm install ---

    #[test]
    fn the_install_arguments_pin_the_version() {
        let args = npm_install_args(PINNED_DSH, true).join(" ");
        assert!(args.contains(&format!("{NPM_PACKAGE}@{PINNED_DSH}")));
        assert!(args.contains("--ignore-scripts"));
        // Nunca global: `-g` escreveria no prefixo do Node portátil e
        // escaparia da pasta isolada.
        assert!(!args.contains(" -g"));
        assert!(!args.contains("--global"));
    }

    #[test]
    fn the_install_can_fall_back_to_running_scripts() {
        let args = npm_install_args(PINNED_DSH, false).join(" ");
        assert!(!args.contains("--ignore-scripts"));
    }

    // --------------------------------------------------------- layout ---

    #[test]
    fn the_layout_keeps_everything_under_one_folder() {
        let dir = tempfile::tempdir().unwrap();
        let l = Layout::new(dir.path());
        assert!(l.app().starts_with(&l.raiz));
        assert!(l.pacote().starts_with(l.app()));
        assert!(l.bin_js().starts_with(l.pacote()));
        assert!(!l.instalado());
    }

    #[test]
    fn the_bin_is_the_lib_bin_js_of_the_pinned_package() {
        let dir = tempfile::tempdir().unwrap();
        let l = Layout::new(dir.path());
        let lib = l.pacote().join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(lib.join("bin.js"), b"// cli").unwrap();
        assert!(l.instalado());
        assert_eq!(l.bin_js(), lib.join("bin.js"));
    }

    /// Se o upstream mover o `lib/bin.js`, o campo `bin` do package.json é o
    /// fallback — nas duas formas que o npm aceita.
    #[test]
    fn a_moved_bin_is_found_via_the_package_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let l = Layout::new(dir.path());
        std::fs::create_dir_all(l.pacote().join("dist")).unwrap();
        std::fs::write(l.pacote().join("dist/cli.js"), b"// cli").unwrap();

        std::fs::write(
            l.pacote().join("package.json"),
            br#"{"name":"@deepseek-ai/dsh","bin":{"dsh":"./dist/cli.js"}}"#,
        )
        .unwrap();
        assert_eq!(l.bin_js(), l.pacote().join("dist/cli.js"));
        assert!(l.instalado());

        std::fs::write(
            l.pacote().join("package.json"),
            br#"{"name":"@deepseek-ai/dsh","bin":"dist/cli.js"}"#,
        )
        .unwrap();
        assert_eq!(l.bin_js(), l.pacote().join("dist/cli.js"));
    }

    // ------------------------------------------------------ run_args ---

    /// `--port 0` (EADDRINUSE mata o dsh sem fallback) e `--no-open` (o
    /// filho abridor de browser herdaria o stdout que parseamos).
    #[test]
    fn the_run_arguments_ask_for_an_os_port_and_suppress_the_browser() {
        let args = run_args().join(" ");
        assert!(args.starts_with("web"));
        assert!(args.contains("--port 0"));
        assert!(args.contains("--no-open"));
        // `--no-browser` não existe no dsh; a flag certa é `--no-open`.
        assert!(!args.contains("--no-browser"));
        // O host default já é loopback e 0.0.0.0 é recusado pelo próprio dsh.
        assert!(!args.contains("--host"));
    }

    #[test]
    fn the_npm_phase_is_read_from_a_reify_line() {
        assert_eq!(
            fase_do_npm("npm http fetch reify:@deepseek-ai/dsh"),
            Some("extracting")
        );
        assert_eq!(fase_do_npm("timing idealTree Completed"), Some("resolving"));
        assert_eq!(fase_do_npm("added 190 packages in 40s"), Some("finishing"));
        assert_eq!(fase_do_npm("qualquer coisa"), None);
    }
}
