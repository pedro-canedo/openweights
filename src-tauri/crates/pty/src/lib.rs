//! Terminais embutidos: um pseudoterminal por sessão, supervisionado pelo app.
//!
//! O que roda aqui é o que a tela do OwCLI mostra em abas e grade — o agente
//! (OwCLI), um shell comum ou um harness de terceiros (Claude Code, opencode,
//! aider). A emulação do terminal é da interface (xterm.js); este crate cuida
//! do processo e dos bytes:
//!
//! - **Endereço por byte.** Toda saída entra num anel ([`anel`]) com posição
//!   absoluta. A interface diz até onde desenhou; ao voltar, pede "desde N" e
//!   recebe só o que falta. Anexar de novo SUBSTITUI o destino anterior, e o
//!   replay e a saída ao vivo passam pelo mesmo lock — sem buraco nem repetição.
//! - **Controle de fluxo.** Com a interface olhando, o leitor para de ler
//!   quando há mais de [`ALTA`] bytes sem confirmação (`confirmar`); o kernel
//!   segura o programa até a tela alcançar. Sem ninguém olhando, o programa
//!   segue e o anel guarda o fim.
//! - **Sinais para a lista de sessões** ([`osc`]): título, pasta e "preciso
//!   de você" (OSC 9), lidos aqui para funcionarem com a tela fechada.
//! - **Encerramento da árvore** ([`encerrar`]): fechar uma aba é o SIGHUP de
//!   um terminal comum (quem usou `nohup` sobrevive); sair do app mata a
//!   sessão inteira, para não sobrar órfão.
//! - **Ambiente do sistema**: o `CommandBuilder` parte do ambiente do app; aqui
//!   ele é limpo e recebe o do sistema ([`lr_proc::host_environment`]), senão
//!   o shell de um AppImage herdaria o `PATH` e o `LD_LIBRARY_PATH` do pacote.

mod anel;
mod encerrar;
pub mod osc;

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::Serialize;

use anel::Anel;
use osc::Sinal;

/// Bytes sem confirmação a partir dos quais o leitor espera a tela.
pub const ALTA: u64 = 512 * 1024;
/// Quanto de saída cada sessão guarda para replay.
pub const CAPACIDADE_DO_ANEL: usize = 1024 * 1024;
/// Sem nenhuma confirmação por este tempo com a fila cheia, a tela é dada
/// como perdida e o programa volta a correr solto.
const PACIENCIA: Duration = Duration::from_secs(30);

pub type SessaoId = u64;

/// O que roda na sessão — decide o ícone, o rótulo e o que conta como "sino".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Tipo {
    Shell,
    OwCli,
    Harness { id: String },
}

/// Como abrir uma sessão.
#[derive(Debug, Clone)]
pub struct Pedido {
    pub programa: OsString,
    pub args: Vec<OsString>,
    /// Sempre explícita: o AppRun do AppImage muda a pasta do app.
    pub pasta: PathBuf,
    /// Variáveis a mais sobre o ambiente do sistema.
    pub env: Vec<(OsString, OsString)>,
    pub colunas: u16,
    pub linhas: u16,
    pub titulo: String,
    pub tipo: Tipo,
}

/// Um pedaço da saída, com o endereço do primeiro byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bloco {
    pub offset: u64,
    pub dados: Vec<u8>,
    /// Só no replay: parte do pedido já tinha saído do anel.
    pub truncado: bool,
}

/// Para onde vão os blocos de uma sessão anexada (o canal da webview).
pub trait Destino: Send {
    /// `false` quando o destino morreu: a sessão se desanexa sozinha.
    fn enviar(&self, bloco: Bloco) -> bool;
}

/// O que a lista de sessões precisa saber, sem abrir a tela.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Aviso {
    Titulo {
        titulo: String,
    },
    Pasta {
        pasta: String,
    },
    /// "Preciso de você": OSC 9 (texto) ou o sino (texto vazio).
    Atencao {
        texto: String,
    },
    Saiu {
        codigo: Option<i32>,
    },
}

/// A sessão vista de fora.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Resumo {
    pub id: SessaoId,
    pub tipo: Tipo,
    pub titulo: String,
    pub pasta: String,
    pub atencao: bool,
    pub viva: bool,
    pub codigo_saida: Option<i32>,
    pub pid: Option<u32>,
    pub criada_em_ms: u64,
    /// Endereço do próximo byte: quanto a sessão já produziu.
    pub fim: u64,
}

pub type Ouvinte = Arc<dyn Fn(SessaoId, Aviso) + Send + Sync>;

pub struct Gerente {
    sessoes: Mutex<HashMap<SessaoId, Arc<Sessao>>>,
    proximo: AtomicU64,
    ouvinte: Ouvinte,
}

struct Sessao {
    id: SessaoId,
    tipo: Tipo,
    pid: Option<u32>,
    criada_em_ms: u64,
    mestre: Mutex<Box<dyn MasterPty + Send>>,
    escritor: Mutex<Box<dyn Write + Send>>,
    matador: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    saida: Mutex<Saida>,
    fluxo: Condvar,
    estado: Mutex<Estado>,
    #[cfg(windows)]
    job: Mutex<Option<lr_proc::JobGuard>>,
}

struct Saida {
    anel: Anel,
    destino: Option<Box<dyn Destino>>,
    /// Até onde a tela confirmou ter desenhado.
    confirmado: u64,
    fechando: bool,
}

struct Estado {
    titulo: String,
    pasta: String,
    atencao: bool,
    viva: bool,
    codigo_saida: Option<i32>,
}

/// Recupera o conteúdo de um mutex envenenado: um pânico num leitor não pode
/// derrubar o encerramento do app.
fn trava<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn agora_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Gerente {
    pub fn new(ouvinte: Ouvinte) -> Self {
        Self {
            sessoes: Mutex::new(HashMap::new()),
            proximo: AtomicU64::new(1),
            ouvinte,
        }
    }

    /// Abre o pseudoterminal e sobe o programa nele.
    pub fn abrir(&self, pedido: Pedido) -> std::io::Result<SessaoId> {
        let par = native_pty_system()
            .openpty(PtySize {
                rows: pedido.linhas.max(1),
                cols: pedido.colunas.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(erro)?;

        let mut cmd = CommandBuilder::new(&pedido.programa);
        cmd.args(&pedido.args);
        cmd.cwd(&pedido.pasta);
        cmd.env_clear();
        for (k, v) in lr_proc::host_environment() {
            cmd.env(k, v);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        for (k, v) in &pedido.env {
            cmd.env(k, v);
        }

        let filho = par.slave.spawn_command(cmd).map_err(erro)?;
        // Sem soltar o escravo, o leitor nunca vê o fim quando o filho sai.
        drop(par.slave);
        let pid = filho.process_id();
        #[cfg(windows)]
        let job = filho.as_raw_handle().and_then(lr_proc::attach_job_raw);

        let leitor = par.master.try_clone_reader().map_err(erro)?;
        let escritor = par.master.take_writer().map_err(erro)?;
        let matador = filho.clone_killer();

        let id = self.proximo.fetch_add(1, Ordering::Relaxed);
        let sessao = Arc::new(Sessao {
            id,
            tipo: pedido.tipo,
            pid,
            criada_em_ms: agora_ms(),
            mestre: Mutex::new(par.master),
            escritor: Mutex::new(escritor),
            matador: Mutex::new(matador),
            saida: Mutex::new(Saida {
                anel: Anel::new(CAPACIDADE_DO_ANEL),
                destino: None,
                confirmado: 0,
                fechando: false,
            }),
            fluxo: Condvar::new(),
            estado: Mutex::new(Estado {
                titulo: pedido.titulo,
                pasta: pedido.pasta.to_string_lossy().into_owned(),
                atencao: false,
                viva: true,
                codigo_saida: None,
            }),
            #[cfg(windows)]
            job: Mutex::new(job),
        });
        trava(&self.sessoes).insert(id, Arc::clone(&sessao));

        let ouvinte = Arc::clone(&self.ouvinte);
        std::thread::Builder::new()
            .name(format!("pty-{id}"))
            .spawn(move || ler_ate_o_fim(sessao, leitor, filho, ouvinte))?;
        Ok(id)
    }

    fn sessao(&self, id: SessaoId) -> Option<Arc<Sessao>> {
        trava(&self.sessoes).get(&id).cloned()
    }

    /// Liga a sessão a um destino, a partir do byte `desde` (`None`: tudo o
    /// que o anel guarda). Substitui o destino anterior. `false` se a sessão
    /// não existe.
    pub fn anexar(&self, id: SessaoId, desde: Option<u64>, destino: Box<dyn Destino>) -> bool {
        let Some(sessao) = self.sessao(id) else {
            return false;
        };
        let mut saida = trava(&sessao.saida);
        let trecho = saida.anel.desde(desde);
        let offset = trecho.offset;
        let vivo = destino.enviar(Bloco {
            offset,
            dados: trecho.dados,
            truncado: trecho.truncado,
        });
        saida.destino = vivo.then_some(destino);
        // O replay conta no controle de fluxo como qualquer outro bloco.
        saida.confirmado = offset;
        sessao.fluxo.notify_all();
        true
    }

    /// Solta o destino (a tela saiu): o programa segue sem esperar ninguém.
    pub fn desanexar(&self, id: SessaoId) {
        if let Some(sessao) = self.sessao(id) {
            trava(&sessao.saida).destino = None;
            sessao.fluxo.notify_all();
        }
    }

    /// A tela desenhou até o byte `ate`.
    pub fn confirmar(&self, id: SessaoId, ate: u64) {
        if let Some(sessao) = self.sessao(id) {
            let mut saida = trava(&sessao.saida);
            if ate > saida.confirmado {
                saida.confirmado = ate;
                sessao.fluxo.notify_all();
            }
        }
    }

    pub fn escrever(&self, id: SessaoId, bytes: &[u8]) -> std::io::Result<()> {
        let sessao = self.sessao(id).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "sessão inexistente")
        })?;
        let mut escritor = trava(&sessao.escritor);
        escritor.write_all(bytes)?;
        escritor.flush()
    }

    /// Muda o tamanho do terminal numa thread à parte: no Windows o
    /// `ResizePseudoConsole` pode esperar a saída ser drenada, e quem chama é o
    /// comando da interface.
    pub fn redimensionar(&self, id: SessaoId, colunas: u16, linhas: u16) {
        let Some(sessao) = self.sessao(id) else {
            return;
        };
        let _ = std::thread::Builder::new()
            .name(format!("pty-{id}-resize"))
            .spawn(move || {
                let tamanho = PtySize {
                    rows: linhas.max(1),
                    cols: colunas.max(1),
                    pixel_width: 0,
                    pixel_height: 0,
                };
                if let Err(e) = trava(&sessao.mestre).resize(tamanho) {
                    log::debug!("pty {}: resize falhou: {e}", sessao.id);
                }
            });
    }

    /// A pessoa viu a sessão: apaga o "precisa de você".
    pub fn visto(&self, id: SessaoId) {
        if let Some(sessao) = self.sessao(id) {
            trava(&sessao.estado).atencao = false;
        }
    }

    pub fn listar(&self) -> Vec<Resumo> {
        let mut lista: Vec<Resumo> = trava(&self.sessoes).values().map(|s| resumo(s)).collect();
        lista.sort_by_key(|r| r.id);
        lista
    }

    /// Fecha a aba: tira a sessão da lista e encerra como um terminal comum
    /// (SIGHUP; o líder que não sair em 1,5 s leva SIGKILL). Não bloqueia.
    pub fn fechar(&self, id: SessaoId) {
        let Some(sessao) = trava(&self.sessoes).remove(&id) else {
            return;
        };
        let _ = std::thread::Builder::new()
            .name(format!("pty-{id}-fechar"))
            .spawn(move || encerrar_sessao(&sessao, Duration::from_millis(1500), false));
    }

    /// Saída do app: encerra todas e não deixa órfão — quem sobreviver ao
    /// SIGHUP em `prazo` morre com a sessão inteira. Síncrono, sem runtime.
    pub fn encerrar_todas(&self, prazo: Duration) {
        let todas: Vec<Arc<Sessao>> = trava(&self.sessoes).drain().map(|(_, s)| s).collect();
        for s in &todas {
            pedir_para_sair(s);
        }
        esperar_saida(&todas, prazo);
        for s in &todas {
            forcar(s, true);
        }
    }
}

impl Drop for Gerente {
    fn drop(&mut self) {
        self.encerrar_todas(Duration::from_millis(300));
    }
}

fn erro(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

fn resumo(s: &Sessao) -> Resumo {
    let estado = trava(&s.estado);
    Resumo {
        id: s.id,
        tipo: s.tipo.clone(),
        titulo: estado.titulo.clone(),
        pasta: estado.pasta.clone(),
        atencao: estado.atencao,
        viva: estado.viva,
        codigo_saida: estado.codigo_saida,
        pid: s.pid,
        criada_em_ms: s.criada_em_ms,
        fim: trava(&s.saida).anel.fim(),
    }
}

fn ler_ate_o_fim(
    sessao: Arc<Sessao>,
    mut leitor: Box<dyn Read + Send>,
    mut filho: Box<dyn portable_pty::Child + Send + Sync>,
    ouvinte: Ouvinte,
) {
    let mut buf = vec![0u8; 64 * 1024];
    let mut osc = osc::Leitor::default();
    loop {
        let n = match leitor.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            // EIO é como o Linux diz que o lado do filho fechou.
            Err(_) => break,
        };
        let dados = &buf[..n];
        let sinais = osc.ler(dados);
        entregar(&sessao, dados);
        for sinal in sinais {
            if let Some(aviso) = aplicar(&sessao, sinal) {
                ouvinte(sessao.id, aviso);
            }
        }
    }
    let codigo = filho
        .wait()
        .ok()
        .map(|s| i32::try_from(s.exit_code()).unwrap_or(i32::MAX));
    {
        let mut estado = trava(&sessao.estado);
        estado.viva = false;
        estado.codigo_saida = codigo;
    }
    ouvinte(sessao.id, Aviso::Saiu { codigo });
}

/// Guarda no anel, manda ao destino e, se a tela ficou para trás, espera.
fn entregar(sessao: &Sessao, dados: &[u8]) {
    let mut saida = trava(&sessao.saida);
    let offset = saida.anel.guardar(dados);
    if let Some(destino) = &saida.destino {
        let vivo = destino.enviar(Bloco {
            offset,
            dados: dados.to_vec(),
            truncado: false,
        });
        if !vivo {
            saida.destino = None;
        }
    }
    let mut ultimo_avanco = (saida.confirmado, Instant::now());
    while saida.destino.is_some()
        && !saida.fechando
        && saida.anel.fim().saturating_sub(saida.confirmado) > ALTA
    {
        let (guarda, _) = sessao
            .fluxo
            .wait_timeout(saida, Duration::from_millis(500))
            .unwrap_or_else(|e| e.into_inner());
        saida = guarda;
        if saida.confirmado != ultimo_avanco.0 {
            ultimo_avanco = (saida.confirmado, Instant::now());
        } else if ultimo_avanco.1.elapsed() > PACIENCIA {
            log::warn!(
                "pty {}: a tela parou de confirmar; seguindo sem ela",
                sessao.id
            );
            saida.destino = None;
        }
    }
}

fn aplicar(sessao: &Sessao, sinal: Sinal) -> Option<Aviso> {
    let mut estado = trava(&sessao.estado);
    match sinal {
        Sinal::Titulo(t) => {
            estado.titulo = t.clone();
            Some(Aviso::Titulo { titulo: t })
        }
        Sinal::Pasta(p) => {
            estado.pasta = p.clone();
            Some(Aviso::Pasta { pasta: p })
        }
        Sinal::Notificacao(texto) => {
            estado.atencao = true;
            Some(Aviso::Atencao { texto })
        }
        // No shell o sino é a completação que não achou nada: ruído. No
        // agente e nos harnesses, é o pedido de atenção sem OSC 9.
        Sinal::Sino if sessao.tipo == Tipo::Shell => None,
        Sinal::Sino => {
            estado.atencao = true;
            Some(Aviso::Atencao {
                texto: String::new(),
            })
        }
    }
}

fn pedir_para_sair(s: &Sessao) {
    {
        let mut saida = trava(&s.saida);
        saida.fechando = true;
        saida.destino = None;
    }
    s.fluxo.notify_all();
    #[cfg(unix)]
    if let Some(pid) = s.pid {
        encerrar::sinalizar_grupo(pid, libc::SIGHUP);
    }
    #[cfg(windows)]
    {
        // Windows não tem SIGHUP: fechar o terminal é terminar o job.
        let _ = trava(&s.matador).kill();
    }
}

fn esperar_saida(sessoes: &[Arc<Sessao>], prazo: Duration) {
    let limite = Instant::now() + prazo;
    while Instant::now() < limite {
        if sessoes.iter().all(|s| !trava(&s.estado).viva) {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// `sessao_inteira`: mata também quem ignorou o SIGHUP em outro grupo
/// (saída do app). Sem ela, só o líder e o grupo dele (fechar uma aba).
fn forcar(s: &Sessao, sessao_inteira: bool) {
    #[cfg(unix)]
    if let Some(pid) = s.pid {
        if sessao_inteira {
            encerrar::matar_sessao(pid);
        } else if trava(&s.estado).viva {
            encerrar::sinalizar_grupo(pid, libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    let _ = sessao_inteira;
    if trava(&s.estado).viva {
        let _ = trava(&s.matador).kill();
    }
    #[cfg(windows)]
    if let Some(job) = trava(&s.job).take() {
        lr_proc::terminate_job(&job);
    }
}

fn encerrar_sessao(s: &Arc<Sessao>, prazo: Duration, sessao_inteira: bool) {
    pedir_para_sair(s);
    esperar_saida(std::slice::from_ref(s), prazo);
    forcar(s, sessao_inteira);
}

#[cfg(all(test, unix))]
mod tests;
