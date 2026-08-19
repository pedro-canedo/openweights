//! Supervisão de processos filhos de longa duração.
//!
//! Extraído de `lr_engine`, que resolveu estes problemas primeiro para o
//! llama-server e agora os divide com o 9router e o Traefik. O que está aqui
//! é exatamente o que impede um sidecar de sobreviver ao app:
//!
//! - **Matar a árvore, não o processo.** Um `node` do Next.js gera netos; um
//!   `kill` no pai deixa a porta ocupada e o app não reabre.
//! - **Job Object no Windows.** É o único mecanismo que garante a morte dos
//!   netos mesmo se o app for encerrado à força.
//! - **Grupo de processos no Unix.** O equivalente: `process_group(0)` no
//!   spawn e `kill(-pid)` no fim.
//!
//! O Tauri não mata sidecars sozinho (issue #3273), então nada disto é
//! opcional — sem esta camada sobra processo órfão segurando VRAM e porta.

use std::time::{Duration, Instant};
use tokio::process::{Child, Command};

/// A flag que impede o Windows de alocar um console para o filho.
///
/// Um app com `windows_subsystem = "windows"` não tem console; sem esta flag,
/// todo filho console-subsystem (`git`, `npm`, `taskkill`) ganha um console
/// novo — a janela preta que pisca na cara de quem só pediu uma tarefa ao
/// agente. Em debug o bug se esconde: aí o app tem console e o filho herda.
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Aplica `CREATE_NO_WINDOW` a um comando síncrono. Fora do Windows é no-op.
///
/// Existe para que nenhum `Command::new` precise saber o número mágico nem
/// lembrar do `#[cfg]`. Chamar sempre é seguro e é o padrão do projeto —
/// há um teste (`no_process_spawn_escapes_the_no_window_rule`) que cobra isso.
pub fn no_window_std(cmd: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// A irmã assíncrona de [`no_window_std`], para `tokio::process::Command`.
pub fn no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Mata o PID e toda a árvore (Windows: `taskkill /T /F`; Unix: grupo).
pub fn kill_process_tree(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(windows)]
    {
        use std::process::Stdio;
        let mut cmd = std::process::Command::new("taskkill");
        let _ = no_window_std(&mut cmd)
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        let pid = pid as i32;
        // O grupo primeiro (pega os netos), depois o processo — se o spawn
        // não pôde criar grupo próprio, o segundo `kill` ainda resolve.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
            libc::kill(pid, libc::SIGKILL);
        }
    }
}

/// Espera o filho sair por até 3 s e desiste matando-o.
///
/// Sem isto o processo vira zumbi no Unix: ninguém colheu o status de saída.
pub fn reap_child(child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => {
                let _ = child.start_kill();
                return;
            }
        }
    }
}

/// Prepara o comando para virar um sidecar supervisionado.
///
/// No Unix cria grupo de processos próprio, para que `kill(-pid)` alcance os
/// netos. Chamar antes do `spawn_supervised`.
pub fn prepare(cmd: &mut Command) -> &mut Command {
    cmd.kill_on_drop(true);
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }
    cmd
}

/// Sobe o processo sem abrir console no Windows.
///
/// `CREATE_BREAKAWAY_FROM_JOB` só é legal se o job pai tiver `BREAKAWAY_OK`.
/// O terminal do Cursor (e o WebView2) costuma colocar o app num job *sem*
/// essa permissão — aí o `CreateProcess` falha com acesso negado (os error 5).
/// Tentamos breakaway só quando dá; se mesmo assim vier 5, repetimos sem.
pub fn spawn_supervised(cmd: &mut Command) -> std::io::Result<Child> {
    #[cfg(not(windows))]
    {
        cmd.spawn()
    }
    #[cfg(windows)]
    {
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        let breakaway = windows_job::parent_allows_breakaway();
        cmd.creation_flags(if breakaway {
            CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB
        } else {
            CREATE_NO_WINDOW
        });
        match cmd.spawn() {
            Ok(child) => Ok(child),
            Err(e) if breakaway && e.raw_os_error() == Some(5) => {
                log::warn!("BREAKAWAY_FROM_JOB negado ({e}); tentando sem breakaway");
                cmd.creation_flags(CREATE_NO_WINDOW);
                cmd.spawn()
            }
            Err(e) => Err(e),
        }
    }
}

/// Handle do Job Object que mata a árvore quando fechado. No Unix é vazio —
/// lá o grupo de processos cumpre o mesmo papel.
#[cfg(windows)]
pub struct JobGuard(windows_job::JobHandle);
#[cfg(not(windows))]
pub struct JobGuard;

/// Coloca o filho num Job Object com `KILL_ON_JOB_CLOSE`.
///
/// Devolve `None` quando não foi possível — não é fatal: o `kill_process_tree`
/// no shutdown continua sendo a rede de segurança.
#[cfg(windows)]
pub fn attach_job(child: &Child) -> Option<JobGuard> {
    let job = windows_job::create()?;
    let handle = child.raw_handle()?;
    if windows_job::assign(&job, handle) {
        Some(JobGuard(job))
    } else {
        log::warn!("não foi possível colocar o processo num Job Object; o exit usará taskkill /T");
        None
    }
}

#[cfg(not(windows))]
pub fn attach_job(_child: &Child) -> Option<JobGuard> {
    None
}

/// Mata tudo que está no job. No Unix é no-op (quem mata é o grupo).
#[cfg(windows)]
pub fn terminate_job(job: &JobGuard) {
    windows_job::terminate(&job.0);
}

#[cfg(not(windows))]
pub fn terminate_job(_job: &JobGuard) {}

// ---------------------------------------------------------------------------
// Portas
// ---------------------------------------------------------------------------

/// A porta está ocupada agora?
pub fn port_in_use(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_err()
}

/// Devolve `preferida` se estiver livre; senão pede uma efêmera ao SO.
///
/// Há uma corrida inerente entre soltar o listener e o filho subir — por isso
/// preferimos sempre a porta fixa, que também é o que o usuário vê
/// documentado, e só caímos na efêmera quando ela está de fato ocupada (o
/// caso real: a pessoa já roda a própria instância do serviço). Devolve
/// `preferida` também quando o SO nega a efêmera: aí o erro aparece no start,
/// com mensagem do serviço, em vez de virar um zero silencioso.
pub fn free_port(preferida: u16) -> u16 {
    if preferida != 0 && !port_in_use(preferida) {
        return preferida;
    }
    match std::net::TcpListener::bind(("127.0.0.1", 0)) {
        Ok(l) => l.local_addr().map(|a| a.port()).unwrap_or(preferida),
        Err(_) => preferida,
    }
}

#[cfg(windows)]
mod windows_job {
    //! Job Object só com `KILL_ON_JOB_CLOSE` — sem teto de memória (um modelo
    //! precisa de dezenas de GiB de VRAM/RAM).
    use std::os::windows::io::RawHandle;
    use win32job::Job;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
    };

    pub type JobHandle = Job;

    fn handle(job: &Job) -> HANDLE {
        HANDLE(job.handle() as *mut core::ffi::c_void)
    }

    pub fn create() -> Option<Job> {
        let job = Job::create().ok()?;
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = unsafe {
            SetInformationJobObject(
                handle(&job),
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok.is_err() {
            log::warn!("não foi possível aplicar KILL_ON_JOB_CLOSE ao Job Object: {ok:?}");
            return None;
        }
        Some(job)
    }

    pub fn assign(job: &Job, child: RawHandle) -> bool {
        job.assign_process(child as isize).is_ok()
    }

    pub fn terminate(job: &Job) {
        let _ = unsafe { TerminateJobObject(handle(job), 1) };
    }

    /// `CREATE_BREAKAWAY_FROM_JOB` exige `JOB_OBJECT_LIMIT_BREAKAWAY_OK`
    /// no job do processo atual. Sem isso o Windows devolve ACCESS_DENIED.
    pub fn parent_allows_breakaway() -> bool {
        use windows::Win32::System::JobObjects::{
            IsProcessInJob, JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectExtendedLimitInformation, QueryInformationJobObject,
        };
        use windows::Win32::System::Threading::GetCurrentProcess;

        unsafe {
            let mut in_job = windows::core::BOOL::default();
            if IsProcessInJob(GetCurrentProcess(), None, &mut in_job).is_err() || !in_job.as_bool()
            {
                return false;
            }
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            if QueryInformationJobObject(
                None,
                JobObjectExtendedLimitInformation,
                &mut info as *mut _ as *mut core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                None,
            )
            .is_err()
            {
                return false;
            }
            info.BasicLimitInformation
                .LimitFlags
                .contains(JOB_OBJECT_LIMIT_BREAKAWAY_OK)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::Stdio;

    /// Nenhum arquivo do app pode criar processo sem passar pela flag.
    ///
    /// A regra é por ARQUIVO, não por chamada: analisar Rust com busca de
    /// texto é ilusão, e a granularidade grossa é justamente o que a mantém
    /// sem falso-positivo. Hoje o repo inteiro passa; se um `Command::new`
    /// novo aparecer num arquivo que não conhece nenhuma das âncoras, este
    /// teste falha antes de virar janela preta na máquina de alguém.
    #[test]
    fn no_process_spawn_escapes_the_no_window_rule() {
        const ANCORAS: [&str; 3] = ["no_window", "creation_flags", "spawn_supervised"];

        let Some(raiz) = raiz_do_src_tauri() else {
            return; // crate vendorizado, fora da árvore: nada a varrer.
        };

        let mut fontes = Vec::new();
        colher_rs(&raiz.join("src"), &mut fontes);
        if let Ok(crates) = std::fs::read_dir(raiz.join("crates")) {
            for entrada in crates.flatten() {
                colher_rs(&entrada.path().join("src"), &mut fontes);
            }
        }
        assert!(
            fontes.len() > 20,
            "a varredura não achou as fontes ({} arquivos) — o caminho da raiz mudou?",
            fontes.len()
        );

        let mut faltando = Vec::new();
        for arquivo in fontes {
            let Ok(conteudo) = std::fs::read_to_string(&arquivo) else {
                continue;
            };
            if !conteudo.contains("Command::new(") {
                continue;
            }
            if ANCORAS.iter().any(|a| conteudo.contains(a)) {
                continue;
            }
            let linha = conteudo
                .lines()
                .position(|l| l.contains("Command::new("))
                .map(|i| i + 1)
                .unwrap_or(0);
            faltando.push(format!("{}:{linha}", arquivo.display()));
        }

        assert!(
            faltando.is_empty(),
            "estes arquivos criam processo sem suprimir o console do Windows:\n  {}\n\
             Passe o comando por `lr_proc::no_window` (tokio) ou `lr_proc::no_window_std` \
             (std) antes do spawn — fora do Windows é no-op, então aplicar nunca é errado.",
            faltando.join("\n  ")
        );
    }

    /// `crates/proc` → `src-tauri`. `None` se a árvore não estiver por perto.
    fn raiz_do_src_tauri() -> Option<PathBuf> {
        let raiz = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
        raiz.join("crates").is_dir().then_some(raiz)
    }

    fn colher_rs(dir: &Path, saida: &mut Vec<PathBuf>) {
        let Ok(entradas) = std::fs::read_dir(dir) else {
            return;
        };
        for entrada in entradas.flatten() {
            let caminho = entrada.path();
            if caminho.is_dir() {
                colher_rs(&caminho, saida);
            } else if caminho.extension().is_some_and(|e| e == "rs") {
                saida.push(caminho);
            }
        }
    }

    #[test]
    fn killing_pid_zero_is_a_no_op() {
        // PID 0 é o próprio grupo no Unix: mandar SIGKILL para ele mataria o
        // app. A guarda existe para isso.
        kill_process_tree(0);
    }

    #[test]
    fn a_free_port_is_bindable() {
        let porta = free_port(0);
        assert_ne!(porta, 0);
        assert!(!port_in_use(porta));
    }

    #[test]
    fn an_occupied_preferred_port_falls_back_to_another_one() {
        let ocupada = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let numero = ocupada.local_addr().unwrap().port();
        assert!(port_in_use(numero));

        let escolhida = free_port(numero);
        assert_ne!(escolhida, numero, "não pode devolver uma porta ocupada");
    }

    #[test]
    fn a_free_preferred_port_is_kept() {
        // Solta a porta antes de perguntar: ela volta a ficar livre e a
        // preferência tem de ser respeitada.
        let numero = {
            let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            l.local_addr().unwrap().port()
        };
        assert_eq!(free_port(numero), numero);
    }

    #[tokio::test]
    async fn a_supervised_child_can_be_reaped() {
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", "exit", "0"]);
            c
        } else {
            let mut c = Command::new("true");
            c.stdout(Stdio::null());
            c
        };
        prepare(&mut cmd);
        let mut child = spawn_supervised(&mut cmd).expect("spawn");
        reap_child(&mut child);
    }
}
