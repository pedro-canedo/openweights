//! Execução de processos com limites.
//!
//! Roda um programa **sem passar por shell** (salvo quando quem chama pede
//! explicitamente), captura stdout/stderr em streaming, corta a saída num
//! teto de bytes e mata tudo quando o tempo estoura.
//!
//! O que torna isso não-trivial é o "mata tudo": um `npm test` abre outros
//! processos, e matar só o filho direto deixa netos rodando para sempre.
//! Por isso:
//! - **Windows**: o filho entra num *Job Object* com `KILL_ON_JOB_CLOSE`,
//!   limite de memória e limite de processos ativos. Matar = encerrar o job,
//!   o que leva junto toda a árvore.
//! - **Unix**: o filho ganha um *process group* próprio e o timeout manda
//!   `SIGKILL` para o grupo inteiro.
//!
//! ## Janela de corrida na associação ao Job (Windows)
//!
//! O ideal seria criar o processo suspenso (`CREATE_SUSPENDED`), associá-lo
//! ao job e só então retomá-lo — mas retomar exige `ResumeThread`, e nem
//! `std::process` nem `tokio::process` devolvem o handle da *thread* inicial.
//! Então fazemos o que dá: `spawn` seguido de `AssignProcessToJobObject`
//! imediato. Entre um e outro existe uma janela de microssegundos em que o
//! filho já roda fora do job; se ele criar um neto exatamente aí, esse neto
//! escapa do controle. Na prática isso não acontece (processos gastam bem
//! mais que isso só inicializando), e a alternativa — reimplementar
//! `CreateProcessW` na mão — traria muito mais risco do que resolve.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::mpsc;

/// Teto padrão de saída devolvida (o restante é descartado com aviso).
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 30_000;

/// Tempo padrão de execução, em segundos.
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Teto absoluto: nenhum comando do agente roda mais que isso.
pub const MAX_TIMEOUT_SECS: u64 = 3_600;

/// Tamanho de cada leitura dos canos de saída.
const READ_CHUNK: usize = 8 * 1024;

/// Depois que o processo termina, esperamos só isto para os canos fecharem.
/// Protege contra um neto sobrevivente que herdou o cano e nunca o fecha.
const DRAIN_GRACE_MS: u64 = 500;

/// Memória máxima por processo do job (Windows).
#[cfg(windows)]
const JOB_MEMORY_LIMIT_BYTES: usize = 2 * 1024 * 1024 * 1024;

/// Processos simultâneos permitidos dentro do job (Windows).
#[cfg(windows)]
const JOB_ACTIVE_PROCESS_LIMIT: u32 = 64;

/// `CREATE_NO_WINDOW`: nada de janela preta piscando na cara do usuário.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// O que rodar e sob quais limites.
#[derive(Debug, Clone)]
pub struct SpawnRequest {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub timeout_secs: u64,
    pub max_output_bytes: usize,
    /// Linha original quando a execução é via shell.
    ///
    /// Só tem efeito no Windows: as regras de aspas do `std` estragariam um
    /// `cmd /C` cuja linha já traz aspas (`git commit -m "oi"`), então
    /// entregamos a linha exata ao `cmd.exe`. Em Unix o `sh -c` recebe a
    /// linha como argumento normal e nada disso é necessário.
    pub shell_line: Option<String>,
}

impl SpawnRequest {
    pub fn new(program: impl Into<String>, args: Vec<String>, cwd: PathBuf) -> Self {
        Self {
            program: program.into(),
            args,
            cwd,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            shell_line: None,
        }
    }

    /// Roda a linha pelo shell do sistema (`cmd /C` no Windows, `sh -c` fora).
    ///
    /// Use só quando a linha depende de recursos de shell (pipe, `&&`,
    /// redireção) — que é justamente o caso em que a política já exigiu
    /// confirmação humana.
    pub fn shell(command_line: impl Into<String>, cwd: PathBuf) -> Self {
        let line = command_line.into();
        #[cfg(windows)]
        {
            Self {
                program: "cmd".into(),
                args: vec!["/C".into(), line.clone()],
                cwd,
                timeout_secs: DEFAULT_TIMEOUT_SECS,
                max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
                shell_line: Some(line),
            }
        }
        #[cfg(not(windows))]
        {
            Self {
                program: "sh".into(),
                args: vec!["-c".into(), line],
                cwd,
                timeout_secs: DEFAULT_TIMEOUT_SECS,
                max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
                shell_line: None,
            }
        }
    }

    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    pub fn with_max_output(mut self, bytes: usize) -> Self {
        self.max_output_bytes = bytes;
        self
    }
}

/// Como o processo terminou.
#[derive(Debug, Clone)]
pub struct SpawnOutcome {
    /// `None` quando o processo morreu por sinal (Unix) ou foi encerrado.
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    /// A saída passou do teto e foi cortada.
    pub truncated: bool,
    /// O tempo estourou e nós matamos o processo.
    pub timed_out: bool,
}

impl SpawnOutcome {
    pub fn success(&self) -> bool {
        self.exit_code == Some(0) && !self.timed_out
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stream {
    Out,
    Err,
}

/// Executa o processo, transmitindo cada pedaço de saída para `on_output`.
///
/// Devolve `Err` só quando nem foi possível iniciar o programa (não existe,
/// sem permissão, pasta inválida). Programa que roda e falha é sucesso aqui:
/// o código de saída vai no [`SpawnOutcome`].
pub async fn run(
    req: SpawnRequest,
    mut on_output: impl FnMut(&str) + Send,
) -> std::io::Result<SpawnOutcome> {
    let mut cmd = Command::new(&req.program);
    cmd.current_dir(&req.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
        match &req.shell_line {
            Some(line) => {
                // Linha crua: o `cmd.exe` faz a própria análise de aspas.
                use std::os::windows::process::CommandExt as _;
                cmd.as_std_mut().raw_arg(format!("/C {line}"));
            }
            None => {
                cmd.args(&req.args);
            }
        }
    }
    #[cfg(not(windows))]
    {
        cmd.args(&req.args);
        // Grupo próprio: o timeout mata o grupo inteiro, netos incluídos.
        cmd.process_group(0);
    }

    #[cfg(windows)]
    let job = windows_job::create();

    let mut child = cmd.spawn()?;

    #[cfg(windows)]
    let job = {
        // Associação imediata — ver a nota de corrida no topo do módulo.
        let assigned = match (&job, child.raw_handle()) {
            (Some(job), Some(handle)) => windows_job::assign(job, handle),
            _ => false,
        };
        if !assigned {
            log::warn!(
                "não foi possível colocar `{}` num Job Object; netos podem sobreviver ao timeout",
                req.program
            );
        }
        job.filter(|_| assigned)
    };

    let pid = child.id();
    let (tx, mut rx) = mpsc::unbounded_channel::<(Stream, String)>();
    tokio::spawn(pump(child.stdout.take(), Stream::Out, tx.clone()));
    tokio::spawn(pump(child.stderr.take(), Stream::Err, tx));

    let limit = Duration::from_secs(req.timeout_secs.clamp(1, MAX_TIMEOUT_SECS));
    let deadline = tokio::time::sleep(limit);
    tokio::pin!(deadline);
    // Só é armado quando o processo termina; até lá fica num futuro distante.
    let grace = tokio::time::sleep(Duration::from_secs(MAX_TIMEOUT_SECS));
    tokio::pin!(grace);

    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut kept = 0usize;
    let mut truncated = false;
    let mut timed_out = false;
    let mut grace_armed = false;
    let mut draining = true;
    let mut status = None;
    let mut wait_error = None;

    while draining || status.is_none() {
        tokio::select! {
            biased;
            chunk = rx.recv(), if draining => match chunk {
                // Ao estourar o teto paramos de acumular (e de avisar a
                // interface), mas seguimos drenando o cano: parar de ler
                // travaria o processo num pipe cheio.
                Some((which, text)) if !truncated => {
                    let room = req.max_output_bytes.saturating_sub(kept);
                    let piece = if text.len() <= room {
                        text.as_str()
                    } else {
                        truncated = true;
                        let mut cut = room;
                        while cut > 0 && !text.is_char_boundary(cut) {
                            cut -= 1;
                        }
                        &text[..cut]
                    };
                    if !piece.is_empty() {
                        kept += piece.len();
                        on_output(piece);
                        match which {
                            Stream::Out => stdout.push_str(piece),
                            Stream::Err => stderr.push_str(piece),
                        }
                    }
                }
                Some(_) => {}
                None => draining = false,
            },
            res = child.wait(), if status.is_none() => {
                match res {
                    Ok(st) => status = Some(st),
                    Err(e) => {
                        wait_error = Some(e);
                        break;
                    }
                }
                grace.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(DRAIN_GRACE_MS));
                grace_armed = true;
            },
            _ = &mut deadline, if !timed_out => {
                timed_out = true;
                #[cfg(windows)]
                kill_tree(job.as_ref(), pid);
                #[cfg(not(windows))]
                kill_tree(pid);
            },
            _ = &mut grace, if grace_armed && draining => {
                // Alguém ficou segurando o cano; não vale esperar mais.
                draining = false;
            },
        }
    }

    if let Some(e) = wait_error {
        return Err(e);
    }

    Ok(SpawnOutcome {
        exit_code: status.and_then(|s| s.code()),
        stdout,
        stderr,
        truncated,
        timed_out,
    })
}

/// Lê um cano até o fim, mandando texto decodificado pelo canal.
async fn pump<R>(reader: Option<R>, which: Stream, tx: mpsc::UnboundedSender<(Stream, String)>)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let Some(mut reader) = reader else {
        return;
    };
    let mut buf = vec![0u8; READ_CHUNK];
    let mut pending: Vec<u8> = Vec::new();
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                pending.extend_from_slice(&buf[..n]);
                let text = take_valid_utf8(&mut pending);
                if !text.is_empty() && tx.send((which, text)).is_err() {
                    return;
                }
            }
        }
    }
    if !pending.is_empty() {
        let _ = tx.send((which, String::from_utf8_lossy(&pending).into_owned()));
    }
}

/// Retira do buffer o maior prefixo já decodificável, guardando um eventual
/// caractere partido no meio para o próximo pedaço.
fn take_valid_utf8(buf: &mut Vec<u8>) -> String {
    let valid = match std::str::from_utf8(buf) {
        Ok(_) => buf.len(),
        Err(e) => e.valid_up_to(),
    };
    // Um caractere UTF-8 tem no máximo 4 bytes: sobra maior que isso não é
    // caractere partido, é byte inválido mesmo (saída binária).
    if buf.len() - valid >= 4 {
        let text = String::from_utf8_lossy(buf).into_owned();
        buf.clear();
        return text;
    }
    let text = String::from_utf8_lossy(&buf[..valid]).into_owned();
    buf.drain(..valid);
    text
}

/// Mata o processo e tudo que ele criou.
#[cfg(not(windows))]
fn kill_tree(pid: Option<u32>) {
    let Some(pid) = pid else {
        return;
    };
    let pid = pid as i32;
    // Grupo primeiro (pega os netos), depois o próprio, caso o grupo não
    // tenha sido criado.
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
        libc::kill(pid, libc::SIGKILL);
    }
}

/// Mata o job inteiro; sem job, cai para o `taskkill` com a árvore.
#[cfg(windows)]
fn kill_tree(job: Option<&windows_job::JobHandle>, pid: Option<u32>) {
    if let Some(job) = job {
        windows_job::terminate(job);
        return;
    }
    if let Some(pid) = pid {
        // Sem job não temos como encerrar a árvore por API sem abrir handles
        // um a um; o `taskkill /T` faz exatamente isso e existe em todo
        // Windows suportado.
        use std::os::windows::process::CommandExt as _;
        let _ = std::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

/// Job Object: caixa que segura o processo e todos os descendentes dele.
#[cfg(windows)]
mod windows_job {
    use std::os::windows::io::RawHandle;
    use win32job::Job;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
    };

    pub type JobHandle = Job;

    fn handle(job: &Job) -> HANDLE {
        HANDLE(job.handle() as *mut core::ffi::c_void)
    }

    /// Cria o job já com os limites. `None` quando o sistema recusa —
    /// seguimos sem job (e sem controle dos netos), nunca sem rodar.
    pub fn create() -> Option<Job> {
        let job = Job::create().ok()?;

        // `win32job` 2.x não expõe `ProcessMemoryLimit` nem
        // `ActiveProcessLimit` (o campo interno é privado), então montamos a
        // estrutura e chamamos a API direto.
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | JOB_OBJECT_LIMIT_PROCESS_MEMORY
            | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        info.ProcessMemoryLimit = super::JOB_MEMORY_LIMIT_BYTES;
        info.BasicLimitInformation.ActiveProcessLimit = super::JOB_ACTIVE_PROCESS_LIMIT;

        let ok = unsafe {
            SetInformationJobObject(
                handle(&job),
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok.is_err() {
            log::warn!("não foi possível aplicar limites ao Job Object: {ok:?}");
            return None;
        }
        Some(job)
    }

    pub fn assign(job: &Job, child: RawHandle) -> bool {
        job.assign_process(child as isize).is_ok()
    }

    pub fn terminate(job: &Job) {
        // Código 1: o processo foi encerrado por nós, não terminou sozinho.
        let _ = unsafe { TerminateJobObject(handle(job), 1) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cwd() -> PathBuf {
        std::env::temp_dir()
    }

    /// Comando de shell portátil o suficiente para os testes.
    fn sh(line: &str) -> SpawnRequest {
        SpawnRequest::shell(line, cwd()).with_timeout(20)
    }

    /// Escolhe a linha equivalente para cada shell (`sh` fora, `cmd` dentro
    /// do Windows). Mantém a suíte útil nas duas plataformas.
    fn line(unix: &str, windows: &str) -> String {
        if cfg!(windows) {
            windows.to_string()
        } else {
            unix.to_string()
        }
    }

    #[tokio::test]
    async fn captures_stdout_of_a_simple_command() {
        let out = run(sh("echo ola-mundo"), |_| {}).await.unwrap();
        assert_eq!(out.exit_code, Some(0));
        assert!(out.stdout.contains("ola-mundo"), "stdout: {:?}", out.stdout);
        assert!(!out.truncated);
        assert!(!out.timed_out);
        assert!(out.success());
    }

    #[tokio::test]
    async fn streams_output_to_the_callback() {
        let mut seen = String::new();
        let out = run(sh("echo pedaco"), |chunk| seen.push_str(chunk))
            .await
            .unwrap();
        assert!(seen.contains("pedaco"), "callback recebeu: {seen:?}");
        assert_eq!(seen, out.stdout);
    }

    #[tokio::test]
    async fn failing_command_reports_the_exit_code() {
        let out = run(sh("exit 3"), |_| {}).await.unwrap();
        assert_eq!(out.exit_code, Some(3));
        assert!(!out.success());
    }

    #[tokio::test]
    async fn stderr_is_captured_apart_from_stdout() {
        let cmd = line(
            "echo saida; echo problema 1>&2",
            "echo saida & echo problema 1>&2",
        );
        let out = run(sh(&cmd), |_| {}).await.unwrap();
        assert!(out.stdout.contains("saida"), "{:?}", out.stdout);
        assert!(out.stderr.contains("problema"), "{:?}", out.stderr);
    }

    #[tokio::test]
    async fn timeout_kills_the_process() {
        let started = std::time::Instant::now();
        let cmd = line("sleep 30", "ping -n 30 127.0.0.1 >nul");
        let out = run(sh(&cmd), |_| {}).await.unwrap();
        assert!(out.timed_out, "deveria estourar o tempo");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "demorou demais: {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn huge_output_is_truncated_but_the_process_finishes() {
        let cmd = line(
            "i=0; while [ $i -lt 20000 ]; do echo linha-de-saida-$i; i=$((i+1)); done",
            "for /L %i in (1,1,20000) do @echo linha-de-saida-%i",
        );
        let req = sh(&cmd).with_max_output(1_000).with_timeout(60);
        let out = run(req, |_| {}).await.unwrap();
        assert!(out.truncated, "deveria marcar corte");
        assert!(
            out.stdout.len() <= 1_000,
            "guardou {} bytes",
            out.stdout.len()
        );
        // O processo não pode ter travado num cano cheio.
        assert_eq!(out.exit_code, Some(0));
        assert!(!out.timed_out);
    }

    #[tokio::test]
    async fn missing_program_is_an_io_error() {
        let req = SpawnRequest::new("programa-que-nao-existe-xyz", vec![], cwd());
        assert!(run(req, |_| {}).await.is_err());
    }

    #[tokio::test]
    async fn runs_a_program_without_shell() {
        // Sem shell: `program` + `args` vão direto para o sistema.
        #[cfg(windows)]
        let req = SpawnRequest::new("cmd", vec!["/C".into(), "echo direto".into()], cwd());
        #[cfg(not(windows))]
        let req = SpawnRequest::new("/bin/echo", vec!["direto".into()], cwd());
        let out = run(req.with_timeout(20), |_| {}).await.unwrap();
        assert!(out.stdout.contains("direto"), "{:?}", out.stdout);
    }

    /// `printf` com escapes octais só existe no shell Unix; no Windows a
    /// mesma garantia é coberta por `split_utf8_is_held_until_complete`.
    #[cfg(unix)]
    #[tokio::test]
    async fn utf8_output_is_decoded_correctly() {
        let out = run(sh("printf '\\303\\251 ok'"), |_| {}).await.unwrap();
        assert!(out.stdout.contains("é ok"), "{:?}", out.stdout);
    }

    #[test]
    fn split_utf8_is_held_until_complete() {
        // "é" = 0xC3 0xA9 chegando em pedaços separados.
        let mut buf = vec![b'a', 0xC3];
        assert_eq!(take_valid_utf8(&mut buf), "a");
        buf.push(0xA9);
        assert_eq!(take_valid_utf8(&mut buf), "é");
        assert!(buf.is_empty());
    }

    #[test]
    fn invalid_bytes_are_flushed_instead_of_piling_up() {
        let mut buf = vec![0xFF, 0xFE, 0xFD, 0xFC, 0xFB];
        let text = take_valid_utf8(&mut buf);
        assert!(!text.is_empty());
        assert!(buf.is_empty(), "não pode segurar lixo para sempre");
    }

    /// Prova que o timeout leva junto os netos (o `sleep` em segundo plano).
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn timeout_also_kills_grandchildren() {
        let req = sh("sleep 300 & echo $!; wait").with_timeout(1);
        let out = run(req, |_| {}).await.unwrap();
        assert!(out.timed_out);
        let pid: u32 = out
            .stdout
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("esperava o pid do neto, veio {:?}", out.stdout));
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !std::path::Path::new(&format!("/proc/{pid}")).exists(),
            "o neto {pid} sobreviveu ao timeout"
        );
    }
}
