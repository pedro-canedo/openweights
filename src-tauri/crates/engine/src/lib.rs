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
//! - Sidecars NÃO morrem com o app no Tauri: [`LlamaServer::stop`] precisa ser
//!   chamado em `RunEvent::Exit` (e temos kill-on-drop como cinto de
//!   segurança).

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::{Child, Command};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("falha ao iniciar o processo: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("falha de rede ao falar com o engine: {0}")]
    Network(#[from] reqwest::Error),
    #[error("engine não está rodando")]
    NotRunning,
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

/// Gera o INI do `--models-preset` para o llama-server.
///
/// Cada par `(id, caminho)` vira uma seção: o `id` é o valor de `"model"`
/// nas requisições OpenAI. Caminhos usam `/` para o parser do llama.cpp
/// não tratar `\` do Windows como escape.
pub fn write_models_preset(path: &Path, models: &[(String, PathBuf)]) -> std::io::Result<()> {
    let mut used = HashSet::new();
    let mut body = String::from("; gerado automaticamente — não edite\nversion = 1\n\n");
    for (name, file) in models {
        let id = unique_preset_id(name, &mut used);
        let abs = path_for_ini(file);
        body.push_str(&format!("[{id}]\nmodel = {abs}\n\n"));
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
        }
    }

    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    pub fn is_spawned(&self) -> bool {
        self.child.is_some()
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
            // Cinto de segurança: o handler de RunEvent::Exit chama stop(),
            // mas se o app morrer de forma anormal o SO reaproveita o handle.
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        if let Some(dir) = self.config.exe_path.parent() {
            cmd.current_dir(dir);
        }
        log::info!(
            "iniciando llama-server: {} {}",
            self.config.exe_path.display(),
            self.config.to_args().join(" ")
        );
        self.child = Some(cmd.spawn()?);
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
    /// sozinhos no Tauri).
    pub async fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            log::info!("encerrando llama-server");
            let _ = child.kill().await;
            let _ = child.wait().await;
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

    #[test]
    fn writes_preset_ini_with_unique_ids() {
        let dir = std::env::temp_dir().join(format!("lr-preset-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ini = dir.join("router-models.ini");
        write_models_preset(
            &ini,
            &[
                (
                    "Qwen3.8-27B-UD-Q2_K_XL.gguf".into(),
                    PathBuf::from(r"C:\models\author\repo\Qwen3.8-27B-UD-Q2_K_XL.gguf"),
                ),
                (
                    "Qwen3.8-27B-UD-Q2_K_XL.gguf".into(),
                    PathBuf::from("/other/Qwen3.8-27B-UD-Q2_K_XL.gguf"),
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
}
