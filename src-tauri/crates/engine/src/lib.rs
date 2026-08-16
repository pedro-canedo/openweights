//! Engines de inferência. O primeiro (e principal) é o llama-server do
//! llama.cpp em **Router mode**: um único processo que carrega/descarrega/
//! troca modelos dinamicamente conforme o campo `model` das requisições.
//!
//! A fronteira do trait é "endpoint OpenAI-compatible + gestão do processo",
//! o que cobre também Ollama e LlamaBarn/"Llama" como adapters futuros.
//!
//! Fatos (ago/2026):
//! - Router mode: subir `llama-server` SEM `-m`; `--models-dir` só vê GGUFs
//!   na raiz (não é recursivo). Modelos em `autor/repo/` entram via
//!   `--models-preset` (INI gerado em [`write_models_preset`]).
//!   `GET /models` (status), `POST /models/load`, `POST /models/unload`,
//!   progresso em `/models/sse`; `--models-max` limita simultâneos;
//!   `--sleep-idle-seconds` descarrega ociosos.
//! - `-ngl` já é `auto` por padrão; `--fit` ajusta `-c`/`-ngl` à VRAM.
//! - `/health`: 503 carregando, 200 pronto — readiness probe.
//! - Sidecars NÃO morrem com o app no Tauri: [`LlamaServer::stop_blocking`]
//!   precisa ser chamado em `ExitRequested`/`Exit`. No Windows o processo
//!   entra num Job Object (`KILL_ON_JOB_CLOSE`) para a árvore morrer mesmo
//!   num crash; `taskkill /T` é o cinto se o job não puder ser criado.

pub mod client;
pub mod props;

pub use client::{
    ChatDelta, ChatMessage, ChatOutcome, ChatRequest, ContentPart, FunctionCallMsg, ImageUrl,
    LlamaClient, MessageContent, NamedFunction, NamedToolChoice, Timings, ToolCallMsg, ToolCallReq,
    ToolChoice, tool_specs_to_api,
};
pub use props::{ChatTemplateCaps, ServerProps, parse_props};

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("falha ao iniciar o processo: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("falha de rede ao falar com o engine: {0}")]
    Network(#[from] reqwest::Error),
    #[error("engine não está rodando")]
    NotRunning,
    /// O servidor respondeu, mas com status de erro (corpo já resumido).
    #[error("HTTP {status} do engine: {body}")]
    Http { status: u16, body: String },
    /// Resposta bem-sucedida porém fora do formato esperado.
    #[error("resposta inesperada do engine: {0}")]
    Protocol(String),
}

/// Configuração de inicialização do llama-server em Router mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    pub exe_path: PathBuf,
    pub models_dir: PathBuf,
    pub host: String,
    pub port: u16,
    pub models_max: u32,
    pub api_key: Option<String>,
    /// Segundos de ociosidade até descarregar um modelo (0 = nunca).
    pub sleep_idle_seconds: u32,
    pub extra_args: Vec<String>,
    /// INI do `--models-preset` (modelos em subpastas; `--models-dir` não recorre).
    #[serde(default)]
    pub models_preset: Option<PathBuf>,
}

impl ServerConfig {
    pub fn new(exe_path: PathBuf, models_dir: PathBuf, port: u16) -> Self {
        Self {
            exe_path,
            models_dir,
            host: "127.0.0.1".to_string(),
            port,
            models_max: 2,
            api_key: None,
            sleep_idle_seconds: 0,
            extra_args: Vec::new(),
            models_preset: None,
        }
    }

    /// Argumentos de linha de comando (sem o executável).
    pub fn to_args(&self) -> Vec<String> {
        let mut args = vec![
            "--models-dir".into(),
            self.models_dir.to_string_lossy().into_owned(),
            "--host".into(),
            self.host.clone(),
            "--port".into(),
            self.port.to_string(),
            "--models-max".into(),
            self.models_max.to_string(),
            // Slots de KV no mesmo processo (mesmo load de pesos).
            "--parallel".into(),
            "4".into(),
        ];
        if let Some(preset) = &self.models_preset {
            args.push("--models-preset".into());
            args.push(preset.to_string_lossy().into_owned());
        }
        if self.sleep_idle_seconds > 0 {
            args.push("--sleep-idle-seconds".into());
            args.push(self.sleep_idle_seconds.to_string());
        }
        if let Some(key) = &self.api_key {
            args.push("--api-key".into());
            args.push(key.clone());
        }
        args.extend(self.extra_args.iter().cloned());
        args
    }

    /// URL de BIND (pode ser 0.0.0.0 em modo LAN) — para logs/exibição.
    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }

    /// URL CONECTÁVEL a partir desta máquina: bind em wildcard (0.0.0.0/::)
    /// não é roteável como destino no Windows e é bloqueado pelo CSP — a UI e
    /// o health-check devem sempre usar esta.
    pub fn connect_url(&self) -> String {
        let host = if self.host == "0.0.0.0" || self.host == "::" {
            "127.0.0.1"
        } else {
            self.host.as_str()
        };
        format!("http://{host}:{}", self.port)
    }
}

/// Uma seção do INI do `--models-preset`.
///
/// `extras` são pares `chave = valor` gravados junto do `model` na mesma
/// seção — é assim que um modelo de embedding recebe `pooling = mean` e um
/// reranker recebe `reranking = true` sem afetar os modelos de chat, que
/// dividem o mesmo processo em Router mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetEntry {
    /// Valor que a UI/API manda no campo `model`.
    pub id: String,
    pub path: PathBuf,
    pub extras: Vec<(String, String)>,
}

impl PresetEntry {
    pub fn new(id: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            id: id.into(),
            path: path.into(),
            extras: Vec::new(),
        }
    }

    pub fn with_extra(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extras.push((key.into(), value.into()));
        self
    }
}

impl From<(String, PathBuf)> for PresetEntry {
    fn from((id, path): (String, PathBuf)) -> Self {
        Self::new(id, path)
    }
}

/// Gera o INI do `--models-preset` para o llama-server.
///
/// Cada entrada vira uma seção: o `id` é o valor de `"model"` nas
/// requisições OpenAI. Caminhos usam `/` para o parser do llama.cpp não
/// tratar `\` do Windows como escape.
pub fn write_models_preset(path: &Path, entries: &[PresetEntry]) -> std::io::Result<()> {
    let mut used = HashSet::new();
    let mut body = String::from("; gerado automaticamente — não edite\nversion = 1\n\n");
    for entry in entries {
        let id = unique_preset_id(&entry.id, &mut used);
        let abs = path_for_ini(&entry.path);
        body.push_str(&format!("[{id}]\nmodel = {abs}\n"));
        for (key, value) in &entry.extras {
            body.push_str(&format!("{key} = {value}\n"));
        }
        body.push('\n');
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, body)
}

fn unique_preset_id(name: &str, used: &mut HashSet<String>) -> String {
    let base = name.replace([']', '\n', '\r'], "_");
    let base = if base.is_empty() {
        "model".to_string()
    } else {
        base
    };
    if used.insert(base.clone()) {
        return base;
    }
    for i in 2.. {
        let cand = format!("{base}-{i}");
        if used.insert(cand.clone()) {
            return cand;
        }
    }
    base
}

fn path_for_ini(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Processo llama-server gerenciado.
pub struct LlamaServer {
    config: ServerConfig,
    child: Option<Child>,
    http: reqwest::Client,
    /// Job Object Windows: fecha o handle → o kernel mata a árvore inteira.
    #[cfg(windows)]
    job: Option<windows_job::JobHandle>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Health {
    Ready,
    Loading,
    Down,
}

impl LlamaServer {
    pub fn new(config: ServerConfig) -> Self {
        Self {
            config,
            child: None,
            http: reqwest::Client::new(),
            #[cfg(windows)]
            job: None,
        }
    }

    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    pub fn is_spawned(&self) -> bool {
        self.child.is_some()
    }

    /// PID do llama-server, para matar a árvore no exit se o mutex estiver ocupado.
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().and_then(Child::id)
    }

    /// Inicia o processo. stdout/stderr ficam disponíveis para streaming de
    /// logs à UI (ligação feita pelo chamador).
    pub fn spawn(&mut self) -> Result<(), EngineError> {
        if self.child.is_some() {
            return Ok(());
        }
        let mut cmd = Command::new(&self.config.exe_path);
        cmd.args(self.config.to_args())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        {
            cmd.process_group(0);
        }
        if let Some(dir) = self.config.exe_path.parent() {
            cmd.current_dir(dir);
        }
        log::info!(
            "iniciando llama-server: {} {}",
            self.config.exe_path.display(),
            self.config.to_args().join(" ")
        );
        let child = spawn_llama(&mut cmd)?;
        #[cfg(windows)]
        {
            self.job = attach_job(&child);
        }
        self.child = Some(child);
        Ok(())
    }

    /// Toma os pipes de stdout/stderr do processo (uma única vez) para
    /// streaming de logs.
    pub fn take_output(
        &mut self,
    ) -> (
        Option<tokio::process::ChildStdout>,
        Option<tokio::process::ChildStderr>,
    ) {
        match &mut self.child {
            Some(c) => (c.stdout.take(), c.stderr.take()),
            None => (None, None),
        }
    }

    /// Readiness probe: `GET /health` (200 pronto, 503 carregando).
    pub async fn health(&self) -> Health {
        let url = format!("{}/health", self.config.connect_url());
        match self.http.get(&url).send().await {
            Ok(r) if r.status().is_success() => Health::Ready,
            Ok(r) if r.status().as_u16() == 503 => Health::Loading,
            _ => Health::Down,
        }
    }

    /// Espera o servidor ficar pronto, com timeout.
    pub async fn wait_ready(&self, timeout: std::time::Duration) -> Result<(), EngineError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.health().await == Health::Ready {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(EngineError::NotRunning);
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
    }

    /// Encerra o processo. SEMPRE chamar no exit do app (sidecars não morrem
    /// sozinhos no Tauri). Versão async para os comandos; o exit usa
    /// [`Self::stop_blocking`] para não depender do runtime do Tokio.
    pub async fn stop(&mut self) {
        self.stop_blocking();
    }

    /// Mata o llama-server e os netos de forma síncrona (VRAM libera aqui).
    pub fn stop_blocking(&mut self) {
        let pid = self.pid();
        if pid.is_some() {
            log::info!("encerrando llama-server pid={pid:?}");
        }
        #[cfg(windows)]
        if let Some(job) = self.job.take() {
            windows_job::terminate(&job);
        } else if let Some(pid) = pid {
            kill_process_tree(pid);
        }
        #[cfg(not(windows))]
        if let Some(pid) = pid {
            kill_process_tree(pid);
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            reap_child(&mut child);
        }
    }
}

impl Drop for LlamaServer {
    fn drop(&mut self) {
        self.stop_blocking();
    }
}

/// Mata o PID e toda a árvore (Windows: `taskkill /T /F`; Unix: grupo).
pub fn kill_process_tree(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        let pid = pid as i32;
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
            libc::kill(pid, libc::SIGKILL);
        }
    }
}

fn reap_child(child: &mut Child) {
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

/// Sobe o llama-server. No Windows, `CREATE_BREAKAWAY_FROM_JOB` só é
/// legal se o job pai tiver `BREAKAWAY_OK`. O terminal do Cursor (e o
/// WebView2) costuma colocar o app num job *sem* essa permissão — aí o
/// `CreateProcess` falha com acesso negado (os error 5). Tentamos
/// breakaway só quando dá; se mesmo assim vier 5, repetimos sem a flag.
fn spawn_llama(cmd: &mut Command) -> Result<Child, EngineError> {
    #[cfg(not(windows))]
    {
        Ok(cmd.spawn()?)
    }
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
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
                log::warn!(
                    "llama-server: BREAKAWAY_FROM_JOB negado ({e}); tentando sem breakaway"
                );
                cmd.creation_flags(CREATE_NO_WINDOW);
                Ok(cmd.spawn()?)
            }
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(windows)]
fn attach_job(child: &Child) -> Option<windows_job::JobHandle> {
    let job = windows_job::create()?;
    let handle = child.raw_handle()?;
    if windows_job::assign(&job, handle) {
        Some(job)
    } else {
        log::warn!(
            "não foi possível colocar o llama-server num Job Object; o exit usará taskkill /T"
        );
        None
    }
}

/// Job Object só com `KILL_ON_JOB_CLOSE` — sem teto de memória (o modelo
/// precisa de dezenas de GiB de VRAM/RAM).
#[cfg(windows)]
mod windows_job {
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
            IsProcessInJob, JobObjectExtendedLimitInformation, QueryInformationJobObject,
            JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        };
        use windows::Win32::System::Threading::GetCurrentProcess;

        unsafe {
            let mut in_job = windows::core::BOOL::default();
            if IsProcessInJob(GetCurrentProcess(), None, &mut in_job).is_err()
                || !in_job.as_bool()
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

    #[test]
    fn args_include_router_mode_essentials() {
        let cfg = ServerConfig::new(
            PathBuf::from("/tmp/llama-server"),
            PathBuf::from("/tmp/models"),
            8080,
        );
        let args = cfg.to_args();
        let joined = args.join(" ");
        assert!(joined.contains("--models-dir"));
        assert!(joined.contains("--host 127.0.0.1"));
        assert!(joined.contains("--port 8080"));
        assert!(joined.contains("--models-max 2"));
        assert!(joined.contains("--parallel 4"));
        // Router mode = SEM -m.
        assert!(!joined.contains(" -m "));
        assert!(!args.contains(&"-m".to_string()));
    }

    #[test]
    fn optional_args_appear_when_set() {
        let mut cfg = ServerConfig::new(PathBuf::from("srv"), PathBuf::from("models"), 9000);
        cfg.api_key = Some("secreta".into());
        cfg.sleep_idle_seconds = 300;
        let joined = cfg.to_args().join(" ");
        assert!(joined.contains("--api-key secreta"));
        assert!(joined.contains("--sleep-idle-seconds 300"));
    }

    #[test]
    fn base_url_formats() {
        let cfg = ServerConfig::new(PathBuf::from("srv"), PathBuf::from("m"), 8081);
        assert_eq!(cfg.base_url(), "http://127.0.0.1:8081");
    }

    #[test]
    fn args_include_models_preset() {
        let mut cfg = ServerConfig::new(PathBuf::from("srv"), PathBuf::from("models"), 9000);
        cfg.models_preset = Some(PathBuf::from("/tmp/router-models.ini"));
        let joined = cfg.to_args().join(" ");
        assert!(joined.contains("--models-preset /tmp/router-models.ini"));
    }

    /// Pasta temporária isolada por teste (o id do processo é o mesmo para
    /// todos os testes do binário).
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lr-preset-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn writes_preset_ini_with_unique_ids() {
        let dir = temp_dir("ids");
        let ini = dir.join("router-models.ini");
        write_models_preset(
            &ini,
            &[
                PresetEntry::new(
                    "Qwen3.8-27B-UD-Q2_K_XL.gguf",
                    r"C:\models\author\repo\Qwen3.8-27B-UD-Q2_K_XL.gguf",
                ),
                PresetEntry::new(
                    "Qwen3.8-27B-UD-Q2_K_XL.gguf",
                    "/other/Qwen3.8-27B-UD-Q2_K_XL.gguf",
                ),
            ],
        )
        .unwrap();
        let text = std::fs::read_to_string(&ini).unwrap();
        assert!(text.contains("[Qwen3.8-27B-UD-Q2_K_XL.gguf]"));
        assert!(text.contains("[Qwen3.8-27B-UD-Q2_K_XL.gguf-2]"));
        assert!(text.contains("model = C:/models/author/repo/Qwen3.8-27B-UD-Q2_K_XL.gguf"));
        assert!(!text.contains('\\'));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Extras viram linhas `chave = valor` DENTRO da seção do modelo —
    /// é o que permite subir chat + embedding no mesmo processo.
    #[test]
    fn writes_preset_extras_inside_the_section() {
        let dir = temp_dir("extras");
        let ini = dir.join("router-models.ini");
        write_models_preset(
            &ini,
            &[
                PresetEntry::new("chat.gguf", "/m/chat.gguf"),
                PresetEntry::new("bge-m3.gguf", "/m/bge-m3.gguf")
                    .with_extra("embedding", "true")
                    .with_extra("pooling", "mean"),
            ],
        )
        .unwrap();
        let text = std::fs::read_to_string(&ini).unwrap();
        assert_eq!(
            text,
            "; gerado automaticamente — não edite\nversion = 1\n\n\
             [chat.gguf]\nmodel = /m/chat.gguf\n\n\
             [bge-m3.gguf]\nmodel = /m/bge-m3.gguf\nembedding = true\npooling = mean\n\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Janela de contexto fixa: `ctx-size` + `fit = off` na seção do modelo.
    #[test]
    fn writes_ctx_size_and_disables_fit() {
        let dir = temp_dir("ctx");
        let ini = dir.join("router-models.ini");
        write_models_preset(
            &ini,
            &[PresetEntry::new("qwen.gguf", "/m/qwen.gguf")
                .with_extra("ctx-size", "32768")
                .with_extra("fit", "off")],
        )
        .unwrap();
        let text = std::fs::read_to_string(&ini).unwrap();
        assert!(text.contains("ctx-size = 32768"));
        assert!(text.contains("fit = off"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// O formato antigo `(id, caminho)` ainda converte em uma linha.
    #[test]
    fn preset_entry_from_tuple() {
        let e: PresetEntry = ("m.gguf".to_string(), PathBuf::from("/m/m.gguf")).into();
        assert_eq!(e, PresetEntry::new("m.gguf", "/m/m.gguf"));
        assert!(e.extras.is_empty());
    }
}
