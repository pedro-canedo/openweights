//! Medir de verdade: quantos tokens por segundo esta configuração rende.
//!
//! A sonda ([`crate::probe`]) responde "cabe?" em dois segundos. Esta camada
//! responde "rende quanto?", e cobra o preço: cada configuração exige carregar
//! o modelo e gerar, o que leva de dezenas de segundos a alguns minutos e
//! ocupa a placa inteira. Por isso só roda quando a pessoa pede.
//!
//! Quem mede é o `llama-bench`, do mesmo pacote do motor. Ele devolve JSON com
//! tudo que identifica a medição — build do llama.cpp, CPU, GPU, o arquivo, e
//! cada botão da configuração — mais `avg_ts` (tokens por segundo) e o desvio.
//! Esses campos são a impressão digital que diz quando o número caduca.
//!
//! Dois cuidados que o desenho assume:
//!
//! - **O `llama-bench` e o `llama-server` não dividem a placa.** Medir obriga
//!   a derrubar o motor por minutos; é operação modal, com progresso e
//!   cancelamento, nunca tarefa de fundo.
//! - **Aquecimento enviesa a ordem.** A placa esquenta, e uma varredura
//!   sequencial concluiria que "a última configuração é a pior". Por isso o
//!   primeiro braço é medido de novo no fim: se a diferença passar de
//!   [`DERIVA_MAX`], a corrida inteira é suspeita.

use lr_types::tuning::ModelProfile;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Prompt e geração de cada medição. Curto de propósito: o que interessa é
/// comparar configurações, não bater recorde.
const N_PROMPT: u32 = 256;
const N_GEN: u32 = 64;
/// Repetições por configuração. Três dá desvio utilizável sem triplicar a
/// espera (o custo real é carregar o modelo, que acontece uma vez).
const REPETICOES: u32 = 3;

/// Deriva aceitável entre a primeira e a última medição do mesmo braço.
/// Acima disso a placa esquentou o bastante para contaminar a comparação.
pub const DERIVA_MAX: f64 = 0.05;

#[derive(Debug, thiserror::Error)]
pub enum BenchError {
    #[error("o pacote do llama.cpp instalado não traz o `{0}`")]
    Missing(String),
    #[error("falha ao rodar a medição: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("a medição foi cancelada")]
    Cancelled,
    #[error("a medição não devolveu número nenhum: {0}")]
    Unreadable(String),
}

/// Uma linha do JSON do `llama-bench` — só o que nos interessa.
#[derive(Debug, Clone, Deserialize)]
struct BenchRow {
    build_number: Option<u64>,
    cpu_info: Option<String>,
    gpu_info: Option<String>,
    backends: Option<String>,
    model_size: Option<u64>,
    n_prompt: Option<u32>,
    n_gen: Option<u32>,
    avg_ts: Option<f64>,
    stddev_ts: Option<f64>,
}

/// O que uma configuração rendeu.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchResult {
    /// Tokens por segundo gerando (o número que a pessoa sente).
    pub gen_tps: f64,
    /// Tokens por segundo processando o prompt.
    pub prompt_tps: f64,
    /// Desvio da geração — mede quão confiável é a média.
    pub gen_stddev: f64,
    /// Build do llama.cpp, para saber quando o número caduca.
    pub build_number: u64,
    pub cpu_info: String,
    pub gpu_info: String,
    pub backends: String,
    pub model_size: u64,
}

/// Lê a saída JSON do `llama-bench`.
///
/// Ele emite uma linha por teste: a de `n_prompt > 0` é o processamento do
/// prompt, a de `n_gen > 0` é a geração. Uma corrida sem nenhuma das duas é
/// erro — devolver zero seria dizer "esta configuração é péssima" quando o
/// que houve foi uma falha de leitura.
pub fn parse_results(stdout: &str) -> Result<BenchResult, BenchError> {
    let rows: Vec<BenchRow> = serde_json::from_str(stdout)
        .map_err(|e| BenchError::Unreadable(format!("{e}: {}", head(stdout))))?;

    let geracao = rows
        .iter()
        .find(|r| r.n_gen.unwrap_or(0) > 0)
        .and_then(|r| r.avg_ts.map(|ts| (ts, r.stddev_ts.unwrap_or(0.0), r)));
    let prompt = rows
        .iter()
        .find(|r| r.n_prompt.unwrap_or(0) > 0)
        .and_then(|r| r.avg_ts);

    let Some((gen_tps, gen_stddev, r)) = geracao else {
        return Err(BenchError::Unreadable(head(stdout)));
    };

    Ok(BenchResult {
        gen_tps,
        prompt_tps: prompt.unwrap_or(0.0),
        gen_stddev,
        build_number: r.build_number.unwrap_or(0),
        cpu_info: r.cpu_info.clone().unwrap_or_default(),
        gpu_info: r.gpu_info.clone().unwrap_or_default(),
        backends: r.backends.clone().unwrap_or_default(),
        model_size: r.model_size.unwrap_or(0),
    })
}

fn head(s: &str) -> String {
    s.lines().take(2).collect::<Vec<_>>().join(" | ")
}

/// Argumentos do `llama-bench` para um perfil.
///
/// Só entram os botões que o bench conhece: ele mede o motor por baixo, não o
/// servidor, então especulação e visão não têm lugar aqui — quem mede esses é
/// uma geração real, com o servidor de pé.
pub fn bench_args(model_path: &Path, profile: &ModelProfile) -> Vec<String> {
    let mut args = vec![
        "-m".to_string(),
        model_path.to_string_lossy().into_owned(),
        "-p".to_string(),
        N_PROMPT.to_string(),
        "-n".to_string(),
        N_GEN.to_string(),
        "-r".to_string(),
        REPETICOES.to_string(),
        "-o".to_string(),
        "json".to_string(),
    ];
    let mut push = |k: &str, v: String| {
        args.push(k.to_string());
        args.push(v);
    };
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
    if let Some(t) = profile.threads {
        push("-t", t.to_string());
    }
    args
}

pub fn bench_exe(runtime_dir: &Path) -> Result<PathBuf, BenchError> {
    let nome = lr_runtime::exe_name("llama-bench");
    let caminho = runtime_dir.join(nome);
    if caminho.exists() {
        Ok(caminho)
    } else {
        Err(BenchError::Missing(nome.to_string()))
    }
}

/// A placa esquentou o bastante para invalidar a comparação?
pub fn drifted(primeiro: f64, repetido: f64) -> bool {
    if primeiro <= 0.0 {
        return false;
    }
    ((primeiro - repetido).abs() / primeiro) > DERIVA_MAX
}

/// Mede uma configuração. `cancel` é consultado antes de começar — uma
/// medição em curso não é interrompida no meio porque o resultado parcial não
/// serve para nada.
pub async fn bench(
    runtime_dir: &Path,
    model_path: &Path,
    profile: &ModelProfile,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<BenchResult, BenchError> {
    use std::sync::atomic::Ordering;
    if cancel.load(Ordering::SeqCst) {
        return Err(BenchError::Cancelled);
    }
    let exe = bench_exe(runtime_dir)?;

    let mut cmd = tokio::process::Command::new(&exe);
    cmd.args(bench_args(model_path, profile))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        // As DLLs do CUDA moram ao lado do executável.
        .current_dir(runtime_dir)
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        cmd.creation_flags(0x0800_0000);
    }

    let saida = cmd.output().await?;
    if cancel.load(Ordering::SeqCst) {
        return Err(BenchError::Cancelled);
    }
    parse_results(&String::from_utf8_lossy(&saida.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_types::tuning::KvType;

    /// Saída real do `llama-bench -o json`, recortada.
    const JSON: &str = r#"[
      {
        "build_number": 10441,
        "cpu_info": "AMD Ryzen 5 4600G",
        "gpu_info": "NVIDIA GeForce RTX 5060 Ti",
        "backends": "CUDA",
        "model_size": 6969004032,
        "n_prompt": 256,
        "n_gen": 0,
        "avg_ts": 812.5,
        "stddev_ts": 4.1
      },
      {
        "build_number": 10441,
        "cpu_info": "AMD Ryzen 5 4600G",
        "gpu_info": "NVIDIA GeForce RTX 5060 Ti",
        "backends": "CUDA",
        "model_size": 6969004032,
        "n_prompt": 0,
        "n_gen": 64,
        "avg_ts": 53.98,
        "stddev_ts": 0.42
      }
    ]"#;

    #[test]
    fn the_two_numbers_that_matter_come_out_separated() {
        let r = parse_results(JSON).unwrap();
        assert!((r.gen_tps - 53.98).abs() < 0.01);
        assert!((r.prompt_tps - 812.5).abs() < 0.01);
        assert!((r.gen_stddev - 0.42).abs() < 0.01);
        assert_eq!(r.build_number, 10441);
        assert_eq!(r.backends, "CUDA");
        assert_eq!(r.model_size, 6_969_004_032);
    }

    /// Zero tokens por segundo seria lido como "esta configuração é péssima".
    /// Falha de leitura tem de ser erro.
    #[test]
    fn an_unreadable_run_is_an_error_and_not_zero() {
        assert!(parse_results("").is_err());
        assert!(parse_results("[]").is_err());
        assert!(parse_results("não é json").is_err());
    }

    #[test]
    fn speculation_and_vision_are_not_the_benchs_business() {
        let p = ModelProfile {
            ngl: Some(30),
            kv_k: Some(KvType::Q8_0),
            flash_attn: Some(true),
            spec: Some(lr_types::tuning::SpecType::Ngram),
            mmproj: Some("/m/mmproj.gguf".into()),
            ..Default::default()
        };
        let args = bench_args(Path::new("/m/a.gguf"), &p).join(" ");
        assert!(args.contains("-ngl 30"));
        assert!(args.contains("-ctk q8_0"));
        assert!(args.contains("-fa on"));
        assert!(args.contains("-o json"));
        assert!(!args.contains("spec"), "{args}");
        assert!(!args.contains("mmproj"), "{args}");
    }

    #[test]
    fn a_hot_card_makes_the_run_suspect() {
        assert!(!drifted(50.0, 49.0));
        assert!(drifted(50.0, 40.0));
        // Sem primeira medição não há do que suspeitar.
        assert!(!drifted(0.0, 30.0));
    }

    #[tokio::test]
    async fn a_cancelled_run_never_starts_the_process() {
        let cancel = std::sync::atomic::AtomicBool::new(true);
        let e = bench(
            Path::new("/nao/existe"),
            Path::new("/m/a.gguf"),
            &ModelProfile::default(),
            &cancel,
        )
        .await
        .unwrap_err();
        assert!(matches!(e, BenchError::Cancelled));
    }
}
