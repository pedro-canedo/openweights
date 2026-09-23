//! O Host do AgenticOw em execução: o Node portátil do app rodando
//! `bin/agenticow-host.mjs` do runtime, com o protocolo de controle no
//! stdin/stdout.
//!
//! A URL autenticada (com o token de lançamento) fica só aqui dentro: quem
//! precisa dela é a webview do app, nunca um log ou evento. O stderr do Host
//! vira log já redigido.
//!
//! Desligar é gracioso nos três sistemas pelo próprio protocolo: `shutdown`
//! ou EOF no stdin fazem o Host descartar a árvore (sessões gravadas,
//! processos filhos encerrados) e sair com 0. Só depois do prazo entram o Job
//! Object e o kill da árvore — a rede de segurança.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};

use crate::install::ENTRADA;
use crate::protocol::{self, Comando, Mensagem, PROTOCOLO};

/// O dispose do Host tem prazo interno de 5 s; uma folga por cima.
pub const PRAZO_PARA_SAIR: Duration = Duration::from_secs(7);

/// O primeiro boot inicializa o profile e, no Windows, passa pelo antivírus
/// numa árvore de ~12 mil arquivos.
pub const PRAZO_PARA_SUBIR: Duration = Duration::from_secs(120);

#[derive(Clone, Debug)]
pub struct Config {
    pub node_exe: PathBuf,
    /// PATH com o Node portátil e o isolamento do npm (`NodeManager::env_isolado`).
    pub node_env: Vec<(String, String)>,
    pub runtime_dir: PathBuf,
    /// `DSH_HOME`: profiles, sessões, configurações.
    pub home: PathBuf,
    pub porta: u16,
}

/// Quem recebe os eventos do Host (chamado de tarefas próprias).
pub type OuvinteDoHost = Arc<dyn Fn(EventoDoHost) + Send + Sync>;

/// O que o Host contou, para a tela e o log. Sem a URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventoDoHost {
    Saudou {
        revisao: String,
        upstream_tag: String,
        dsh: String,
    },
    Pronto {
        porta: u16,
    },
    Fatal {
        mensagem: String,
    },
    CatalogoAplicado {
        revisao: Option<u64>,
    },
    CatalogoRecusado {
        revisao: Option<u64>,
        mensagem: String,
    },
    /// Uma linha do stderr (ou do stdout fora do protocolo), já redigida.
    Log {
        linha: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum ErroDoHost {
    #[error("o runtime do AgenticOw não está instalado em {0}")]
    SemRuntime(PathBuf),
    #[error("o Host do AgenticOw falhou: {0}")]
    Fatal(String),
    #[error("o Host do AgenticOw saiu antes de ficar pronto")]
    Saiu,
    #[error("o Host do AgenticOw não ficou pronto em {0} s")]
    Prazo(u64),
    #[error("o Host fala o protocolo {0}; este app fala o {PROTOCOLO}")]
    Protocolo(u64),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Default)]
struct Estado {
    url: Option<String>,
    porta: Option<u16>,
    fatal: Option<String>,
    protocolo: Option<u64>,
}

pub struct AgenticowHost {
    filho: Option<Child>,
    job: Option<lr_proc::JobGuard>,
    stdin: Arc<tokio::sync::Mutex<Option<ChildStdin>>>,
    estado: Arc<Mutex<Estado>>,
}

impl AgenticowHost {
    /// Sobe o Host. Os eventos chegam por `on_event`, de tarefas próprias.
    pub fn spawn(config: &Config, on_event: OuvinteDoHost) -> Result<Self, ErroDoHost> {
        let entrada = config.runtime_dir.join(ENTRADA);
        if !entrada.is_file() {
            return Err(ErroDoHost::SemRuntime(config.runtime_dir.clone()));
        }
        std::fs::create_dir_all(&config.home)?;
        let mut cmd = tokio::process::Command::new(&config.node_exe);
        cmd.arg(&entrada)
            .arg("--port")
            .arg(config.porta.to_string())
            .current_dir(&config.runtime_dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        for (k, v) in &config.node_env {
            cmd.env(k, v);
        }
        cmd.env("DSH_HOME", &config.home);
        lr_proc::prepare(&mut cmd);
        let mut filho = lr_proc::spawn_supervised(&mut cmd)?;
        let job = lr_proc::attach_job(&filho);

        let estado = Arc::new(Mutex::new(Estado::default()));
        if let Some(stdout) = filho.stdout.take() {
            let estado = Arc::clone(&estado);
            let on_event = Arc::clone(&on_event);
            tokio::spawn(async move {
                let mut linhas = BufReader::new(stdout).lines();
                while let Ok(Some(linha)) = linhas.next_line().await {
                    tratar_linha(&linha, &estado, on_event.as_ref());
                }
            });
        }
        if let Some(stderr) = filho.stderr.take() {
            let on_event = Arc::clone(&on_event);
            tokio::spawn(async move {
                let mut linhas = BufReader::new(stderr).lines();
                while let Ok(Some(linha)) = linhas.next_line().await {
                    on_event(EventoDoHost::Log {
                        linha: protocol::redigir(&linha),
                    });
                }
            });
        }
        let stdin = Arc::new(tokio::sync::Mutex::new(filho.stdin.take()));
        Ok(Self {
            filho: Some(filho),
            job,
            stdin,
            estado,
        })
    }

    pub fn pid(&self) -> Option<u32> {
        self.filho.as_ref().and_then(|c| c.id())
    }

    /// URL autenticada, quando o Host já disse `ready`. Só para a webview.
    pub fn url(&self) -> Option<String> {
        self.estado
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .url
            .clone()
    }

    pub fn porta(&self) -> Option<u16> {
        self.estado.lock().unwrap_or_else(|e| e.into_inner()).porta
    }

    /// O processo já saiu? Consulta sem bloquear.
    pub fn morreu(&mut self) -> bool {
        match self.filho.as_mut() {
            Some(c) => matches!(c.try_wait(), Ok(Some(_)) | Err(_)),
            None => true,
        }
    }

    /// Espera o `ready`. Falha cedo em `fatal`, protocolo incompatível ou saída.
    pub async fn aguardar_pronto(&mut self, prazo: Duration) -> Result<(String, u16), ErroDoHost> {
        let limite = Instant::now() + prazo;
        loop {
            {
                let e = self.estado.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(p) = e.protocolo.filter(|p| *p != PROTOCOLO) {
                    return Err(ErroDoHost::Protocolo(p));
                }
                if let Some(msg) = &e.fatal {
                    return Err(ErroDoHost::Fatal(msg.clone()));
                }
                if let (Some(url), Some(porta)) = (&e.url, e.porta) {
                    return Ok((url.clone(), porta));
                }
            }
            if self.morreu() {
                // O fatal pode chegar logo antes do fim do stdout.
                tokio::time::sleep(Duration::from_millis(200)).await;
                let e = self.estado.lock().unwrap_or_else(|e| e.into_inner());
                return Err(e
                    .fatal
                    .clone()
                    .map(ErroDoHost::Fatal)
                    .unwrap_or(ErroDoHost::Saiu));
            }
            if Instant::now() >= limite {
                return Err(ErroDoHost::Prazo(prazo.as_secs()));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Manda um comando pelo stdin.
    pub async fn enviar(&self, comando: &Comando) -> Result<(), ErroDoHost> {
        let mut guarda = self.stdin.lock().await;
        let Some(stdin) = guarda.as_mut() else {
            return Err(ErroDoHost::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "stdin do Host fechado",
            )));
        };
        stdin.write_all(comando.linha().as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Desliga pelo protocolo e espera; força só depois do prazo.
    pub async fn parar(&mut self) {
        let _ = self.enviar(&Comando::Shutdown).await;
        self.stdin.lock().await.take();
        let limite = Instant::now() + PRAZO_PARA_SAIR;
        while Instant::now() < limite && !self.morreu() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        self.forcar();
    }

    /// Versão síncrona (Drop, fechamento do app): o EOF no stdin já é um
    /// desligamento gracioso no Host.
    pub fn parar_bloqueando(&mut self) {
        if let Ok(mut g) = self.stdin.try_lock() {
            g.take();
        }
        let limite = Instant::now() + PRAZO_PARA_SAIR;
        while Instant::now() < limite && !self.morreu() {
            std::thread::sleep(Duration::from_millis(100));
        }
        self.forcar();
    }

    fn forcar(&mut self) {
        let pid = self.pid();
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

impl Drop for AgenticowHost {
    fn drop(&mut self) {
        if self.filho.is_some() {
            self.parar_bloqueando();
        }
    }
}

fn tratar_linha(
    linha: &str,
    estado: &Mutex<Estado>,
    on_event: &(dyn Fn(EventoDoHost) + Send + Sync),
) {
    let Some(msg) = protocol::ler(linha) else {
        if !linha.trim().is_empty() {
            on_event(EventoDoHost::Log {
                linha: protocol::redigir(linha),
            });
        }
        return;
    };
    let mut e = estado.lock().unwrap_or_else(|e| e.into_inner());
    match msg {
        Mensagem::Hello {
            protocol,
            revision,
            upstream_tag,
            dsh,
            ..
        } => {
            e.protocolo = Some(protocol);
            drop(e);
            on_event(EventoDoHost::Saudou {
                revisao: revision,
                upstream_tag,
                dsh,
            });
        }
        Mensagem::Ready { url, port } => {
            e.url = Some(url);
            e.porta = Some(port);
            drop(e);
            on_event(EventoDoHost::Pronto { porta: port });
        }
        Mensagem::Fatal { message } => {
            let redigida = protocol::redigir(&message);
            e.fatal = Some(redigida.clone());
            drop(e);
            on_event(EventoDoHost::Fatal { mensagem: redigida });
        }
        Mensagem::CatalogApplied { revision } => {
            drop(e);
            on_event(EventoDoHost::CatalogoAplicado { revisao: revision });
        }
        Mensagem::CatalogError { revision, message } => {
            drop(e);
            on_event(EventoDoHost::CatalogoRecusado {
                revisao: revision,
                mensagem: message,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Contra um Host FALSO (um script Node que fala o protocolo): exercita
    //! ready, shutdown, EOF, fatal, catálogo e a redação do token sem precisar
    //! do runtime real. Sem `node` no PATH (nem em AGENTICOW_TEST_NODE), pula.
    use super::*;

    const HOST_FALSO: &str = r#"
const modo = process.argv[process.argv.indexOf('--port') + 1] === '1' ? 'fatal' : 'normal'
const w = (m) => process.stdout.write(JSON.stringify({ ow: 1, ...m }) + '\n')
w({ type: 'hello', protocol: 1, host: '0.1.0', revision: 'r', upstreamTag: 'dsh-v0', dsh: '0.1.5', node: process.version, nodeAbi: process.versions.modules, pid: process.pid })
process.stderr.write('log com a URL http://127.0.0.1:9/?token=SEGREDO123 no meio\n')
if (modo === 'fatal') { w({ type: 'fatal', message: 'porta ocupada' }); process.exit(1) }
setTimeout(() => w({ type: 'ready', url: 'http://127.0.0.1:4242/?token=SEGREDO123', port: 4242 }), 150)
let buf = ''
process.stdin.on('data', (d) => {
  buf += d
  let i
  while ((i = buf.indexOf('\n')) !== -1) {
    const m = JSON.parse(buf.slice(0, i)); buf = buf.slice(i + 1)
    if (m.type === 'shutdown') process.exit(0)
    if (m.type === 'catalog') w({ type: 'catalog-applied', revision: m.revision })
  }
})
process.stdin.on('end', () => process.exit(0))
setInterval(() => {}, 1000)
"#;

    fn node() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("AGENTICOW_TEST_NODE") {
            return Some(PathBuf::from(p));
        }
        let nome = if cfg!(windows) { "node.exe" } else { "node" };
        std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|d| d.join(nome))
            .find(|p| p.is_file())
    }

    fn preparar(tmp: &std::path::Path) -> Option<Config> {
        let node = node()?;
        let rt = tmp.join("runtime");
        std::fs::create_dir_all(rt.join("bin")).unwrap();
        std::fs::write(rt.join(ENTRADA), HOST_FALSO).unwrap();
        Some(Config {
            node_exe: node,
            node_env: vec![],
            runtime_dir: rt,
            home: tmp.join("home"),
            porta: 0,
        })
    }

    fn coletor() -> (OuvinteDoHost, Arc<Mutex<Vec<EventoDoHost>>>) {
        let eventos = Arc::new(Mutex::new(Vec::new()));
        let e2 = Arc::clone(&eventos);
        (Arc::new(move |ev| e2.lock().unwrap().push(ev)), eventos)
    }

    #[tokio::test]
    async fn sobe_manda_catalogo_e_desliga_pelo_protocolo() {
        let tmp = tempfile::tempdir().unwrap();
        let Some(cfg) = preparar(tmp.path()) else {
            return;
        };
        let (on_event, eventos) = coletor();
        let mut host = AgenticowHost::spawn(&cfg, on_event).unwrap();
        let (url, porta) = host.aguardar_pronto(Duration::from_secs(20)).await.unwrap();
        assert_eq!(porta, 4242);
        assert!(
            url.contains("token=SEGREDO123"),
            "a URL inteira fica com o app"
        );
        host.enviar(&Comando::Catalog {
            revision: 9,
            pi_ai: serde_json::json!({}),
            env: Default::default(),
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        let inicio = Instant::now();
        host.parar().await;
        assert!(
            inicio.elapsed() < PRAZO_PARA_SAIR,
            "saiu pelo shutdown, não pelo prazo"
        );
        let eventos = eventos.lock().unwrap();
        assert!(eventos.contains(&EventoDoHost::Pronto { porta: 4242 }));
        assert!(eventos.contains(&EventoDoHost::CatalogoAplicado { revisao: Some(9) }));
        // Nada que sai do Host para a tela carrega o token.
        assert!(
            !format!("{eventos:?}").contains("SEGREDO123"),
            "{eventos:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn eof_no_stdin_desliga() {
        let tmp = tempfile::tempdir().unwrap();
        let Some(cfg) = preparar(tmp.path()) else {
            return;
        };
        let (on_event, _) = coletor();
        let mut host = AgenticowHost::spawn(&cfg, on_event).unwrap();
        host.aguardar_pronto(Duration::from_secs(20)).await.unwrap();
        let inicio = Instant::now();
        tokio::task::block_in_place(|| host.parar_bloqueando());
        assert!(
            inicio.elapsed() < PRAZO_PARA_SAIR,
            "saiu pelo EOF, não pelo prazo"
        );
    }

    #[tokio::test]
    async fn fatal_vira_erro_redigido() {
        let tmp = tempfile::tempdir().unwrap();
        let Some(mut cfg) = preparar(tmp.path()) else {
            return;
        };
        cfg.porta = 1; // o falso trata a porta 1 como "ocupada"
        let (on_event, _) = coletor();
        let mut host = AgenticowHost::spawn(&cfg, on_event).unwrap();
        match host.aguardar_pronto(Duration::from_secs(20)).await {
            Err(ErroDoHost::Fatal(m)) => assert_eq!(m, "porta ocupada"),
            outro => panic!("esperava fatal, veio {outro:?}"),
        }
    }

    #[test]
    fn sem_runtime_nem_tenta() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = Config {
            node_exe: PathBuf::from("node"),
            node_env: vec![],
            runtime_dir: tmp.path().join("nada"),
            home: tmp.path().join("home"),
            porta: 0,
        };
        let (on_event, _) = coletor();
        assert!(matches!(
            AgenticowHost::spawn(&cfg, on_event),
            Err(ErroDoHost::SemRuntime(_))
        ));
    }
}
