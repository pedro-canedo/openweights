//! O decisor local: um segundo `llama-server`, compilado do fork
//! `parallel-decision`, que serve só `POST /v1/decision` com um modelo
//! pequeno dedicado.
//!
//! Não é o roteador: sobe com `-m` (um modelo), sem `--models-dir`, na
//! porta 11713 (a seguinte à do proxy do Jev), e com `--decision-seqs`, a
//! flag que só o fork tem — é ela que reserva as sequências das decisões e
//! muda o KV cache para unificado, para as perguntas de um mesmo contexto
//! compartilharem as células. Enquanto carrega o modelo responde 503, e é
//! assim que a cadeia de decisores sabe cair no remoto.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::process::Child;

use crate::{EngineError, Health, spawn_llama, stop_llama};

/// Porta preferida. O motor está em 11711 e o proxy do Jev em 11712.
pub const DECISION_PORTA_PADRAO: u16 = 11713;

/// Sequências reservadas às decisões. A bateria do app tem duas perguntas
/// (cinco ramos em modo `tree`); dezesseis deixa folga para quem usa o
/// endpoint por fora, e num modelo de atenção pura custa quase nada.
pub const DECISION_SEQS_PADRAO: u32 = 16;

/// Janela: o estado que o app manda tem no máximo 12 000 caracteres, mais o
/// prefixo (instruções + esquema) que fica em cache.
pub const CTX_PADRAO: u32 = 8192;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionServerConfig {
    pub exe_path: PathBuf,
    pub model_path: PathBuf,
    pub port: u16,
    pub decision_seqs: u32,
    pub ctx_size: u32,
    pub gpu_layers: u32,
}

impl DecisionServerConfig {
    pub fn new(exe_path: PathBuf, model_path: PathBuf, port: u16) -> Self {
        Self {
            exe_path,
            model_path,
            port,
            decision_seqs: DECISION_SEQS_PADRAO,
            ctx_size: CTX_PADRAO,
            gpu_layers: 99,
        }
    }

    /// Argumentos de linha de comando (sem o executável). Só loopback: o
    /// endpoint chega a quem é de fora pelo proxy do Jev, nunca direto.
    pub fn to_args(&self) -> Vec<String> {
        vec![
            "-m".into(),
            self.model_path.to_string_lossy().into_owned(),
            "--host".into(),
            "127.0.0.1".into(),
            "--port".into(),
            self.port.to_string(),
            "--decision-seqs".into(),
            self.decision_seqs.max(3).to_string(),
            // Um slot de chat basta (e é o mínimo): as decisões usam as
            // sequências reservadas acima.
            "--parallel".into(),
            "1".into(),
            "-ngl".into(),
            self.gpu_layers.to_string(),
            "-fa".into(),
            "on".into(),
            "-c".into(),
            self.ctx_size.to_string(),
            "--no-webui".into(),
        ]
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

/// O processo do decisor. Mesma disciplina do [`crate::LlamaServer`]: Job
/// Object no Windows, grupo de processos no Unix, e `stop` explícito no exit.
pub struct DecisionServer {
    config: DecisionServerConfig,
    child: Option<Child>,
    job: Option<lr_proc::JobGuard>,
    http: reqwest::Client,
}

impl DecisionServer {
    pub fn new(config: DecisionServerConfig) -> Self {
        Self {
            config,
            child: None,
            job: None,
            http: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn config(&self) -> &DecisionServerConfig {
        &self.config
    }

    pub fn base_url(&self) -> String {
        self.config.base_url()
    }

    pub fn is_spawned(&self) -> bool {
        self.child.is_some()
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().and_then(Child::id)
    }

    pub fn spawn(&mut self) -> Result<(), EngineError> {
        if self.child.is_some() {
            return Ok(());
        }
        log::info!(
            "iniciando decisor local: {} {}",
            self.config.exe_path.display(),
            self.config.to_args().join(" ")
        );
        let (child, job) = spawn_llama(&self.config.exe_path, &self.config.to_args(), &[])?;
        self.child = Some(child);
        self.job = job;
        Ok(())
    }

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

    /// `GET /health`: 200 pronto, 503 carregando.
    pub async fn health(&self) -> Health {
        let url = format!("{}/health", self.config.base_url());
        match self.http.get(&url).send().await {
            Ok(r) if r.status().is_success() => Health::Ready,
            Ok(r) if r.status().as_u16() == 503 => Health::Loading,
            _ => Health::Down,
        }
    }

    pub async fn wait_ready(&self, timeout: Duration) -> Result<(), EngineError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.health().await == Health::Ready {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(EngineError::NotRunning);
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }

    /// Uma decisão de amostra logo depois do `/health`: paga o prefill do
    /// prefixo e o aquecimento dos kernels agora, e não na primeira mensagem
    /// da pessoa (que tem um prazo de um segundo para decidir).
    pub async fn aquecer(&self, corpo: &serde_json::Value) -> bool {
        let url = format!("{}/v1/decision", self.config.base_url());
        match self.http.post(&url).json(corpo).send().await {
            Ok(r) => r.status().is_success(),
            Err(_) => false,
        }
    }

    pub async fn stop(&mut self) {
        self.stop_blocking();
    }

    pub fn stop_blocking(&mut self) {
        if let Some(pid) = self.pid() {
            log::info!("encerrando decisor local pid={pid}");
        }
        stop_llama(&mut self.child, &mut self.job);
    }
}

impl Drop for DecisionServer {
    fn drop(&mut self) {
        self.stop_blocking();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_args_have_no_router_flags() {
        let cfg = DecisionServerConfig::new(
            PathBuf::from("/rt/llama-server"),
            PathBuf::from("/models/q.gguf"),
            11713,
        );
        let args = cfg.to_args();
        let linha = args.join(" ");
        assert!(
            linha.starts_with(
                "-m /models/q.gguf --host 127.0.0.1 --port 11713 --decision-seqs 16 --parallel 1"
            ),
            "{linha}"
        );
        assert!(
            linha.contains("-ngl 99 -fa on -c 8192 --no-webui"),
            "{linha}"
        );
        assert!(!linha.contains("--models-dir"));
        assert!(!linha.contains("--models-preset"));
        assert!(!linha.contains("--metrics"));
        assert_eq!(cfg.base_url(), "http://127.0.0.1:11713");
    }

    #[test]
    fn decision_seqs_never_go_below_the_forks_minimum() {
        let mut cfg = DecisionServerConfig::new("a".into(), "b".into(), 1);
        cfg.decision_seqs = 1;
        let args = cfg.to_args();
        let i = args.iter().position(|a| a == "--decision-seqs").unwrap();
        assert_eq!(args[i + 1], "3");
    }
}
