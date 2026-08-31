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

/// Prompt de uma medição comum. Era 256 — **menor que o micro-lote padrão do
/// llama.cpp, que é 512** — e essa era a razão de mexer em `-b`/`-ub` nunca
/// mudar nada no histórico: a medição inteira cabia numa passada, então o
/// tamanho da passada não podia importar. 512 é o mínimo em que o botão
/// começa a existir.
pub const N_PROMPT: u32 = 512;

/// Prompt de uma medição com pesos fora da placa.
///
/// Aqui ler o prompt e escrever a resposta são trabalhos DIFERENTES: gerar é
/// um token por vez, abaixo do limiar do backend, e sai da CPU; um prompt
/// longo passa do limiar e é copiado para a placa. Medir só com prompt curto
/// esconde o segundo caminho — que é justamente onde o ajuste do micro-lote
/// rende.
pub const N_PROMPT_LONGO: u32 = 4096;

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
    n_depth: Option<u32>,
    n_cpu_moe: Option<u32>,
    n_ubatch: Option<u32>,
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
    /// Com que tamanho de prompt o número acima foi medido.
    ///
    /// Existe porque comparar 800 tok/s num prompt de 512 com 300 num de 4096
    /// não é comparar nada — e a tabela do histórico põe os dois na mesma
    /// coluna. Guardar o tamanho é o que permite dizer quando o Δ vale.
    pub n_prompt: u32,
    /// Quantos tokens já havia no contexto durante a geração (`-d`). Zero é
    /// uma conversa que acabou de começar.
    pub n_depth: u32,
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
    // O prompt MAIS LONGO da corrida: quando há mais de um ponto, é o que
    // exercita o caminho que o micro-lote decide.
    let prompt = rows
        .iter()
        .filter(|r| r.n_prompt.unwrap_or(0) > 0 && r.n_gen.unwrap_or(0) == 0)
        .max_by_key(|r| r.n_prompt.unwrap_or(0));

    let Some((gen_tps, gen_stddev, r)) = geracao else {
        return Err(BenchError::Unreadable(head(stdout)));
    };

    Ok(BenchResult {
        gen_tps,
        prompt_tps: prompt.and_then(|r| r.avg_ts).unwrap_or(0.0),
        n_prompt: prompt.and_then(|r| r.n_prompt).unwrap_or(0),
        n_depth: r.n_depth.unwrap_or(0),
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
pub fn bench_args(
    model_path: &Path,
    profile: &ModelProfile,
    cluster: Option<&crate::devices::ClusterArgs>,
) -> Vec<String> {
    bench_args_com_prompt(model_path, profile, cluster, prompt_para(profile))
}

/// O tamanho de prompt que este perfil merece.
///
/// Com especialistas na CPU, o prompt curto mede só metade da máquina: a
/// geração sai da RAM do sistema, mas a leitura do prompt cruza o limiar do
/// backend e vai para a placa. É o caminho onde o micro-lote rende, e ele só
/// aparece com prompt longo.
pub fn prompt_para(profile: &ModelProfile) -> u32 {
    if profile.ncmoe.is_some_and(|n| n > 0) {
        N_PROMPT_LONGO
    } else {
        N_PROMPT
    }
}

pub fn bench_args_com_prompt(
    model_path: &Path,
    profile: &ModelProfile,
    cluster: Option<&crate::devices::ClusterArgs>,
    n_prompt: u32,
) -> Vec<String> {
    let mut args = vec![
        "-m".to_string(),
        model_path.to_string_lossy().into_owned(),
        "-p".to_string(),
        n_prompt.to_string(),
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
    // Como o arquivo entra na memória muda o tempo de carga e, com o modelo
    // maior que a RAM, muda também o que a page cache consegue segurar.
    if let Some(m) = profile.effective_load_mode() {
        push("-lm", m.as_str().to_string());
    }
    // Medir sem o par é medir outra máquina. O `llama-bench` aceita os mesmos
    // três flags do servidor.
    if let Some(c) = cluster {
        args.extend(c.to_args());
    }
    args
}

/// Que botão a varredura gira.
///
/// São os dois cuja resposta certa não dá para calcular: onde parar de
/// empurrar especialista para a CPU, e de que tamanho a passada de prompt
/// compensa. O `llama-bench` aceita lista em qualquer parâmetro, então a
/// curva inteira sai de UMA invocação — um carregamento de modelo, não seis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SweepDim {
    /// Camadas de especialista na CPU (`-ncmoe`).
    Ncmoe,
    /// Tamanho do micro-lote (`-ub`).
    Ubatch,
}

impl SweepDim {
    fn flag(&self) -> &'static str {
        match self {
            SweepDim::Ncmoe => "-ncmoe",
            SweepDim::Ubatch => "-ub",
        }
    }

    fn valor_da_linha(&self, r: &BenchRow) -> Option<u32> {
        match self {
            SweepDim::Ncmoe => r.n_cpu_moe,
            SweepDim::Ubatch => r.n_ubatch,
        }
    }
}

/// Um ponto medido da curva.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepPoint {
    /// O valor testado nesta dimensão.
    pub value: u32,
    pub gen_tps: f64,
    pub prompt_tps: f64,
}

/// Repetições da varredura. UMA, e de propósito: aqui o que se procura é o
/// FORMATO da curva — onde ela dobra —, não o número final. O ponto escolhido
/// depois passa pelo bench normal, com três repetições e desvio.
const REPETICOES_VARREDURA: u32 = 1;

/// Roda a curva de um botão numa invocação só.
pub async fn sweep(
    runtime_dir: &Path,
    model_path: &Path,
    profile: &ModelProfile,
    cluster: Option<&crate::devices::ClusterArgs>,
    dim: SweepDim,
    valores: &[u32],
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Vec<SweepPoint>, BenchError> {
    use std::sync::atomic::Ordering;
    if cancel.load(Ordering::SeqCst) || valores.is_empty() {
        return Err(BenchError::Cancelled);
    }
    let exe = bench_exe(runtime_dir)?;

    // Prompt longo sempre: a varredura existe para enxergar o caminho que o
    // prompt curto esconde.
    let mut args = bench_args_com_prompt(model_path, profile, cluster, N_PROMPT_LONGO);
    // A lista vence o valor único que o perfil possa ter posto: a última
    // ocorrência é a que o llama-bench usa.
    args.push(dim.flag().to_string());
    args.push(
        valores
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(","),
    );
    if let Some(i) = args.iter().position(|a| a == "-r") {
        args[i + 1] = REPETICOES_VARREDURA.to_string();
    }

    let mut cmd = tokio::process::Command::new(&exe);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
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
    parse_sweep(&String::from_utf8_lossy(&saida.stdout), dim)
}

/// Junta as linhas do JSON num ponto por valor testado.
///
/// Cada valor rende duas linhas — a do prompt e a da geração — e o ponto só
/// existe quando as duas apareceram: metade da medição não descreve a
/// escolha, porque é exatamente entre as duas metades que o custo se move.
pub fn parse_sweep(stdout: &str, dim: SweepDim) -> Result<Vec<SweepPoint>, BenchError> {
    let rows: Vec<BenchRow> = serde_json::from_str(stdout)
        .map_err(|e| BenchError::Unreadable(format!("{e}: {}", head(stdout))))?;

    let mut pontos: Vec<SweepPoint> = Vec::new();
    for r in &rows {
        let Some(v) = dim.valor_da_linha(r) else {
            continue;
        };
        let Some(ts) = r.avg_ts else { continue };
        let ponto = match pontos.iter_mut().find(|p| p.value == v) {
            Some(p) => p,
            None => {
                pontos.push(SweepPoint {
                    value: v,
                    gen_tps: 0.0,
                    prompt_tps: 0.0,
                });
                pontos.last_mut().expect("acabou de ser inserido")
            }
        };
        if r.n_gen.unwrap_or(0) > 0 {
            ponto.gen_tps = ts;
        } else if r.n_prompt.unwrap_or(0) > 0 {
            ponto.prompt_tps = ts;
        }
    }
    pontos.retain(|p| p.gen_tps > 0.0 || p.prompt_tps > 0.0);
    if pontos.is_empty() {
        return Err(BenchError::Unreadable(head(stdout)));
    }
    pontos.sort_by_key(|p| p.value);
    Ok(pontos)
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
    cluster: Option<&crate::devices::ClusterArgs>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<BenchResult, BenchError> {
    use std::sync::atomic::Ordering;
    if cancel.load(Ordering::SeqCst) {
        return Err(BenchError::Cancelled);
    }
    let exe = bench_exe(runtime_dir)?;

    let mut cmd = tokio::process::Command::new(&exe);
    cmd.args(bench_args(model_path, profile, cluster))
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
            spec: Some(lr_types::tuning::SpecType::NgramMod.into()),
            mmproj: Some("/m/mmproj.gguf".into()),
            ..Default::default()
        };
        let args = bench_args(Path::new("/m/a.gguf"), &p, None).join(" ");
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

    #[test]
    fn com_o_par_ligado_o_bench_mede_os_dois() {
        let c = crate::devices::ClusterArgs {
            rpc_addr: "192.168.1.8:50052".into(),
            devices: "CUDA0,RPC0".into(),
            tensor_split: "4,3".into(),
        };
        let args = bench_args(Path::new("/m/a.gguf"), &ModelProfile::default(), Some(&c));
        let joined = args.join(" ");
        assert!(joined.contains("--rpc 192.168.1.8:50052"));
        assert!(joined.contains("-dev CUDA0,RPC0"));
        assert!(joined.contains("-ts 4,3"));
    }

    #[tokio::test]
    async fn a_cancelled_run_never_starts_the_process() {
        let cancel = std::sync::atomic::AtomicBool::new(true);
        let e = bench(
            Path::new("/nao/existe"),
            Path::new("/m/a.gguf"),
            &ModelProfile::default(),
            None,
            &cancel,
        )
        .await
        .unwrap_err();
        assert!(matches!(e, BenchError::Cancelled));
    }
}
