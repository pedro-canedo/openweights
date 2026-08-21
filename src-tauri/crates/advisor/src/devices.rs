//! Quais dispositivos existem, e quanto de memória cada um tem livre.
//!
//! A regra do projeto vale aqui como no `llama-fit-params`: a resposta certa
//! não é a nossa fórmula, é a do motor. `llama-server --list-devices` imprime
//! o conjunto que o llama.cpp de fato enxerga — com os NOMES que ele usa, na
//! ORDEM que ele usa e com a memória livre medida agora.
//!
//! Com `--rpc` antes, os dispositivos do peer entram na mesma lista: o
//! `--rpc` registra os devices dentro do próprio handler do argumento, e é
//! por isso que a ordem dos flags importa (há um comentário no `arg.cpp` do
//! llama.cpp avisando exatamente disso).
//!
//! O que isso substitui: três chutes nossos — a fração de 75% da VRAM, o
//! nome/índice do device (`CUDA0`, `MTL0`, `Vulkan0`) e a razão do
//! `--tensor-split` tirada de números anunciados.

use std::path::{Path, PathBuf};
use std::time::Duration;

const LIST_TIMEOUT: Duration = Duration::from_secs(45);
const MIB: u64 = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("o pacote do llama.cpp instalado não traz o `{0}`")]
    Missing(String),
    #[error("falha ao listar os dispositivos: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("a listagem de dispositivos demorou demais")]
    Timeout,
    #[error("o motor não listou nenhum dispositivo")]
    Empty,
}

/// Um dispositivo como o llama.cpp o vê.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    /// `CUDA0`, `RPC0`, `Metal0`… O nome que `--device` aceita, sem tradução.
    pub name: String,
    /// Descrição legível ("NVIDIA GeForce RTX 5060 Ti", o endereço do RPC…).
    pub description: String,
    pub total_bytes: u64,
    /// Livre AGORA. Outro programa usando a placa derruba este número — o que
    /// é o comportamento certo (não reservar o que é de outro), mas faz duas
    /// leituras seguidas poderem discordar.
    pub free_bytes: u64,
}

impl Device {
    /// Veio do peer pela rede.
    pub fn is_remote(&self) -> bool {
        self.name.starts_with("RPC")
    }
}

/// Ordena por memória livre, do maior para o menor.
///
/// É a ordem que o `--device` quer: as primeiras camadas (que a entrada
/// atravessa) ficam com quem tem mais espaço.
pub fn by_free_desc(devices: &[Device]) -> Vec<Device> {
    let mut v = devices.to_vec();
    // `sort_by` estável: empate mantém a ordem do motor, que não é aleatória.
    v.sort_by_key(|d| std::cmp::Reverse(d.free_bytes));
    v
}

/// Lê a saída de `--list-devices`.
///
/// Formato, do `common_print_available_devices`:
/// `  NOME: descrição (TOTAL MiB, LIVRE MiB free)`
pub fn parse_devices(saida: &str) -> Vec<Device> {
    let mut out = Vec::new();
    for linha in saida.lines() {
        let linha = linha.trim();
        let Some((nome, resto)) = linha.split_once(':') else {
            continue;
        };
        if nome.is_empty() || nome.contains(' ') {
            continue;
        }
        let Some((desc, nums)) = resto.rsplit_once('(') else {
            continue;
        };
        let Some(nums) = nums.strip_suffix(')') else {
            continue;
        };
        let (total, livre) = match nums.split_once(',') {
            Some((t, l)) => (mib(t), mib(l)),
            None => continue,
        };
        let (Some(total), Some(livre)) = (total, livre) else {
            continue;
        };
        out.push(Device {
            name: nome.to_string(),
            description: desc.trim().to_string(),
            total_bytes: total * MIB,
            free_bytes: livre * MIB,
        });
    }
    out
}

/// `"16311 MiB"` → `16311`.
fn mib(s: &str) -> Option<u64> {
    s.split_whitespace().next()?.parse().ok()
}

/// Caminho do executável que sabe listar (o próprio servidor).
pub fn list_exe(runtime_dir: &Path) -> Result<PathBuf, DeviceError> {
    let nome = lr_runtime::server_exe_name();
    let caminho = runtime_dir.join(nome);
    if caminho.exists() {
        Ok(caminho)
    } else {
        Err(DeviceError::Missing(nome.to_string()))
    }
}

/// Argumentos da listagem. `--rpc` vem ANTES de `--list-devices` de propósito.
pub fn list_args(rpc_addr: Option<&str>) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(addr) = rpc_addr {
        args.push("--rpc".into());
        args.push(addr.to_string());
    }
    args.push("--list-devices".into());
    args
}

/// Pergunta ao motor quais dispositivos existem.
pub async fn list_devices(
    runtime_dir: &Path,
    rpc_addr: Option<&str>,
) -> Result<Vec<Device>, DeviceError> {
    let exe = list_exe(runtime_dir)?;
    let mut cmd = tokio::process::Command::new(&exe);
    cmd.args(list_args(rpc_addr))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // As DLLs do CUDA moram ao lado do executável.
        .current_dir(runtime_dir)
        .kill_on_drop(true);
    lr_proc::no_window(&mut cmd);

    let saida = tokio::time::timeout(LIST_TIMEOUT, cmd.output())
        .await
        .map_err(|_| DeviceError::Timeout)??;
    let devices = parse_devices(&String::from_utf8_lossy(&saida.stdout));
    if devices.is_empty() {
        return Err(DeviceError::Empty);
    }
    Ok(devices)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Saída real, com um device local e um do peer pela rede.
    const SAIDA: &str = "\
Available devices:
  CUDA0: NVIDIA GeForce RTX 5060 Ti (16311 MiB, 15402 MiB free)
  RPC0: 192.168.1.8:50052 (16384 MiB, 12043 MiB free)
";

    #[test]
    fn the_engine_names_the_devices_not_us() {
        let d = parse_devices(SAIDA);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].name, "CUDA0");
        assert_eq!(d[0].description, "NVIDIA GeForce RTX 5060 Ti");
        assert_eq!(d[0].total_bytes, 16311 * MIB);
        assert_eq!(d[0].free_bytes, 15402 * MIB);
        assert!(!d[0].is_remote());
        assert_eq!(d[1].name, "RPC0");
        assert!(d[1].is_remote());
    }

    #[test]
    fn ruido_do_backend_nao_derruba_a_leitura() {
        let com_ruido = format!("ggml_cuda_init: found 1 CUDA devices\n{SAIDA}build: 10441\n");
        assert_eq!(parse_devices(&com_ruido).len(), 2);
    }

    #[test]
    fn sem_placa_a_lista_vem_vazia() {
        assert!(parse_devices("Available devices:\n  (none)\n").is_empty());
        assert!(parse_devices("").is_empty());
    }

    #[test]
    fn quem_tem_mais_espaco_entra_primeiro() {
        let d = by_free_desc(&parse_devices(SAIDA));
        assert_eq!(d[0].name, "CUDA0", "15402 livre > 12043 livre");
        let invertido = by_free_desc(&parse_devices(
            "  RPC0: peer (16384 MiB, 15000 MiB free)\n  CUDA0: local (16311 MiB, 9000 MiB free)\n",
        ));
        assert_eq!(invertido[0].name, "RPC0");
    }

    #[test]
    fn o_rpc_entra_antes_do_list_devices() {
        // A ordem não é estética: o `--rpc` registra os devices no handler do
        // próprio argumento, então listar antes não enxergaria o peer.
        let args = list_args(Some("192.168.1.8:50052"));
        assert_eq!(args, ["--rpc", "192.168.1.8:50052", "--list-devices"]);
        assert_eq!(list_args(None), ["--list-devices"]);
    }
}
