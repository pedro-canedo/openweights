//! Processo `ggml-rpc-server` no worker.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::{Child, Command};

/// Porta preferida do relato (e da documentação do llama.cpp).
pub const DEFAULT_RPC_PORT: u16 = 50052;

/// Argumentos do worker. Função pura: o pin `-d` e o cache `-c` não podem
/// sumir sem o teste falhar.
pub fn worker_args(port: u16, device: &str) -> Vec<String> {
    vec![
        "-H".into(),
        "0.0.0.0".into(),
        "-p".into(),
        port.to_string(),
        "-c".into(),
        "-d".into(),
        device.into(),
    ]
}

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error("falha ao iniciar o rpc-server: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("o pacote do llama.cpp instalado não traz o ggml-rpc-server")]
    Missing,
}

/// Sidecar do worker. Mesmo padrão do llama-server: Job Object / grupo.
pub struct RpcWorker {
    child: Option<Child>,
    job: Option<lr_proc::JobGuard>,
    pub port: u16,
    pub exe: PathBuf,
}

impl RpcWorker {
    pub fn spawn(exe: &Path, port: u16, device: &str) -> Result<Self, WorkerError> {
        if !exe.is_file() {
            return Err(WorkerError::Missing);
        }
        let mut cmd = Command::new(exe);
        cmd.args(worker_args(port, device))
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        lr_proc::prepare(&mut cmd);
        if let Some(dir) = exe.parent() {
            cmd.current_dir(dir);
        }
        log::info!(
            "iniciando ggml-rpc-server: {} {}",
            exe.display(),
            worker_args(port, device).join(" ")
        );
        let child = lr_proc::spawn_supervised(&mut cmd)?;
        let job = lr_proc::attach_job(&child);
        Ok(Self {
            child: Some(child),
            job,
            port,
            exe: exe.to_path_buf(),
        })
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().and_then(Child::id)
    }

    pub fn stop_blocking(&mut self) {
        if let Some(job) = self.job.take() {
            lr_proc::terminate_job(&job);
        }
        if let Some(child) = self.child.take()
            && let Some(pid) = child.id()
        {
            lr_proc::kill_process_tree(pid);
        }
    }
}

impl Drop for RpcWorker {
    fn drop(&mut self) {
        self.stop_blocking();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_pins_one_device_and_enables_cache() {
        let args = worker_args(50052, "MTL0");
        let joined = args.join(" ");
        assert!(joined.contains("-H 0.0.0.0"));
        assert!(joined.contains("-p 50052"));
        assert!(joined.contains("-c"));
        assert!(joined.contains("-d MTL0"));
    }
}
