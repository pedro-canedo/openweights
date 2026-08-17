//! Quanta memória uma configuração vai custar — perguntando a quem sabe.
//!
//! O resto do crate estima por aritmética, e precisa disso: antes do download
//! não existe arquivo para inspecionar. Depois do download existe, e aí a
//! resposta certa não é a nossa fórmula — é o `llama-fit-params`, que vem no
//! mesmo pacote do llama-server e responde em menos de dois segundos **sem
//! carregar o modelo**.
//!
//! Ele imprime uma linha por dispositivo, em MiB:
//!
//! ```text
//! CUDA0 5956 562 501
//! Host 545 0 32
//! ```
//!
//! que é, na ordem: pesos do modelo, contexto (KV cache) e buffers de
//! trabalho. É esse número que a tela mostra — e é por isso que ela pode
//! prometer "10,7 de 16 GB" sem estar chutando.
//!
//! Esta é a casca impura do crate, no mesmo desenho de `lr_runtime`: o resto
//! continua sendo funções puras testáveis em microssegundos.

use lr_types::tuning::ModelProfile;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Tempo máximo de uma sonda. Ela lê metadados, não pesos: passar disso
/// significa que algo travou, e travar a tela por causa de uma estimativa
/// seria pior do que não ter a estimativa.
const PROBE_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("o pacote do llama.cpp instalado não traz o `{0}`")]
    Missing(String),
    #[error("falha ao rodar a sonda: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("a sonda demorou demais")]
    Timeout,
    #[error("a sonda não respondeu o que se esperava: {0}")]
    Unreadable(String),
}

/// Memória de um dispositivo, em bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceMemory {
    /// Pesos do modelo.
    pub model_bytes: u64,
    /// KV cache da janela pedida.
    pub context_bytes: u64,
    /// Buffers de trabalho.
    pub compute_bytes: u64,
}

impl DeviceMemory {
    pub fn total_bytes(&self) -> u64 {
        self.model_bytes + self.context_bytes + self.compute_bytes
    }
}

/// O que a sonda respondeu.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeReport {
    /// Por dispositivo, na ordem em que o llama.cpp os enumera (`CUDA0`,
    /// `Host`…). O nome vem dele, não de nós.
    pub devices: Vec<(String, DeviceMemory)>,
}

impl ProbeReport {
    /// Soma de tudo que vai para dispositivos que não são a CPU.
    pub fn gpu_bytes(&self) -> u64 {
        self.devices
            .iter()
            .filter(|(nome, _)| !nome.eq_ignore_ascii_case("host"))
            .map(|(_, m)| m.total_bytes())
            .sum()
    }

    /// O que sobra para a memória do sistema.
    pub fn host_bytes(&self) -> u64 {
        self.devices
            .iter()
            .filter(|(nome, _)| nome.eq_ignore_ascii_case("host"))
            .map(|(_, m)| m.total_bytes())
            .sum()
    }
}

/// Lê a saída do `llama-fit-params -fitp on`.
///
/// Linhas que não têm o formato esperado são ignoradas: o binário também
/// imprime avisos de backend, e um aviso novo não pode derrubar a sonda.
pub fn parse_report(stdout: &str) -> Result<ProbeReport, ProbeError> {
    let mut devices = Vec::new();
    for linha in stdout.lines() {
        let mut campos = linha.split_whitespace();
        let Some(nome) = campos.next() else {
            continue;
        };
        let numeros: Vec<u64> = campos.filter_map(|c| c.parse::<u64>().ok()).collect();
        if numeros.len() != 3 {
            continue;
        }
        const MIB: u64 = 1024 * 1024;
        devices.push((
            nome.to_string(),
            DeviceMemory {
                model_bytes: numeros[0] * MIB,
                context_bytes: numeros[1] * MIB,
                compute_bytes: numeros[2] * MIB,
            },
        ));
    }
    if devices.is_empty() {
        return Err(ProbeError::Unreadable(
            stdout.lines().take(3).collect::<Vec<_>>().join(" | "),
        ));
    }
    Ok(ProbeReport { devices })
}

/// Argumentos que traduzem um perfil para a sonda.
///
/// São os mesmos nomes do llama.cpp, com um detalhe: aqui vão como argumentos
/// de linha de comando, e não como chaves de INI. Só entram os botões que
/// mexem em memória — threads e especulação não mudam o que cabe.
pub fn probe_args(model_path: &Path, profile: &ModelProfile) -> Vec<String> {
    let mut args = vec![
        "-m".to_string(),
        model_path.to_string_lossy().into_owned(),
        "-fitp".to_string(),
        "on".to_string(),
    ];
    let mut push = |k: &str, v: String| {
        args.push(k.to_string());
        args.push(v);
    };
    if let Some(ctx) = profile.ctx {
        push("-c", ctx.to_string());
    }
    if let Some(ngl) = profile.ngl {
        push("-ngl", ngl.to_string());
    }
    if let Some(n) = profile.ncmoe {
        push("-ncmoe", n.to_string());
    }
    if let Some(k) = profile.kv_k {
        push("-ctk", k.as_str().to_string());
    }
    if let Some(v) = profile.kv_v {
        push("-ctv", v.as_str().to_string());
    }
    if let Some(fa) = profile.flash_attn {
        push("-fa", if fa { "on" } else { "off" }.to_string());
    }
    if let Some(b) = profile.batch {
        push("-b", b.to_string());
    }
    if let Some(u) = profile.ubatch {
        push("-ub", u.to_string());
    }
    args
}

/// Caminho do `llama-fit-params` dentro do runtime instalado.
pub fn probe_exe(runtime_dir: &Path) -> Result<PathBuf, ProbeError> {
    let nome = lr_runtime::exe_name("llama-fit-params");
    let caminho = runtime_dir.join(nome);
    if caminho.exists() {
        Ok(caminho)
    } else {
        Err(ProbeError::Missing(nome.to_string()))
    }
}

/// Pergunta ao llama.cpp quanta memória esta configuração vai custar.
///
/// Não usa o `spawner` das ferramentas do agente de propósito: aquele impõe
/// teto de memória por processo, pensado para comandos de terminal, e aqui o
/// que roda é parte do motor.
pub async fn probe(
    runtime_dir: &Path,
    model_path: &Path,
    profile: &ModelProfile,
) -> Result<ProbeReport, ProbeError> {
    let exe = probe_exe(runtime_dir)?;
    let args = probe_args(model_path, profile);

    let mut cmd = tokio::process::Command::new(&exe);
    cmd.args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // As DLLs do CUDA moram ao lado do executável.
        .current_dir(runtime_dir)
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        // Sem janela de console piscando na cara de quem só abriu um painel.
        // `creation_flags` já vem do `tokio::process::Command` no Windows.
        cmd.creation_flags(0x0800_0000);
    }

    let saida = tokio::time::timeout(PROBE_TIMEOUT, cmd.output())
        .await
        .map_err(|_| ProbeError::Timeout)??;
    parse_report(&String::from_utf8_lossy(&saida.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_types::tuning::KvType;

    /// A saída real da ferramenta, copiada de uma execução de verdade.
    const SAIDA: &str = "0.00.110.388 I llama_fit_params: printing estimated memory in MiB to stdout (device, model, context, compute) ...\nCUDA0 5956 562 501 \nHost 545 0 32 \n";

    #[test]
    fn the_probe_output_becomes_bytes_per_device() {
        let r = parse_report(SAIDA).unwrap();
        assert_eq!(r.devices.len(), 2);
        assert_eq!(r.devices[0].0, "CUDA0");
        assert_eq!(r.devices[0].1.model_bytes, 5956 * 1024 * 1024);
        assert_eq!(r.devices[0].1.context_bytes, 562 * 1024 * 1024);
        assert_eq!(r.gpu_bytes(), (5956 + 562 + 501) * 1024 * 1024);
        assert_eq!(r.host_bytes(), (545 + 32) * 1024 * 1024);
    }

    /// Um aviso novo do backend não pode derrubar a leitura.
    #[test]
    fn noise_around_the_numbers_is_ignored() {
        let com_ruido = format!("ggml_cuda_init: found 1 CUDA devices\n{SAIDA}build: 10441\n");
        assert_eq!(parse_report(&com_ruido).unwrap().devices.len(), 2);
    }

    #[test]
    fn a_silent_probe_is_an_error_and_not_zero_memory() {
        let e = parse_report("").unwrap_err();
        assert!(matches!(e, ProbeError::Unreadable(_)));
        // Zero memória seria a resposta mais perigosa possível: "cabe".
        let e = parse_report("erro: modelo não encontrado").unwrap_err();
        assert!(e.to_string().contains("não respondeu"));
    }

    #[test]
    fn the_profile_becomes_the_flags_the_probe_knows() {
        let p = ModelProfile {
            ctx: Some(16384),
            ngl: Some(10),
            kv_k: Some(KvType::Q8_0),
            kv_v: Some(KvType::Q8_0),
            flash_attn: Some(false),
            // Não muda memória: não deve virar argumento.
            threads: Some(8),
            spec: Some(lr_types::tuning::SpecType::Ngram),
            ..Default::default()
        };
        let args = probe_args(Path::new("/m/a.gguf"), &p).join(" ");
        assert!(args.contains("-m /m/a.gguf"));
        assert!(args.contains("-fitp on"));
        assert!(args.contains("-c 16384"));
        assert!(args.contains("-ngl 10"));
        assert!(args.contains("-ctk q8_0"));
        assert!(args.contains("-ctv q8_0"));
        assert!(args.contains("-fa off"));
        assert!(!args.contains("threads"), "{args}");
        assert!(!args.contains("spec"), "{args}");
    }

    #[test]
    fn a_missing_binary_says_which_one() {
        let dir = tempfile::tempdir().unwrap();
        let e = probe_exe(dir.path()).unwrap_err();
        assert!(e.to_string().contains("llama-fit-params"), "{e}");
    }
}
