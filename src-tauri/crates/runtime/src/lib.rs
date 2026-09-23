//! Gerência dos runtimes llama.cpp: escolhe a variante certa para a máquina,
//! baixa da release pinada do GitHub, verifica e extrai.
//!
//! Fatos (verificados em ago/2026):
//! - Releases por build: tag `bNNNNN`, várias por dia — usamos uma tag PINADA
//!   e testada, promovida manualmente.
//! - Assets: `llama-<tag>-bin-win-{cpu|cuda-12.4|cuda-13.3|vulkan|...}-x64.zip`;
//!   Linux: `llama-<tag>-bin-ubuntu-{x64|vulkan-x64}.tar.gz` — a release
//!   pinada não publica CUDA para Linux, então lá as placas vão pelo Vulkan;
//!   macOS: `llama-<tag>-bin-macos-{arm64|x64}.tar.gz`.
//! - Os pacotes de Linux trazem `.so` ao lado dos executáveis, ligados por
//!   `RUNPATH=$ORIGIN` e por symlinks de versão (`libllama.so.0 -> …`) — a
//!   extração tem de preservar os links.
//! - CUDA precisa também de `cudart-llama-bin-win-cuda-<ver>-x64.zip`,
//!   extraído NO MESMO diretório dos binários.
//! - Todos os zips embutem as variantes de CPU (`ggml-cpu-*.dll`) com dispatch
//!   em runtime — não há mais escolha por nível de AVX, e builds de GPU têm
//!   fallback de CPU embutido.
//! - CUDA 13.x exige driver >= 580 e GPU Turing+ (CC >= 7.5); CUDA 12.4 roda
//!   com driver >= 527.41 e cobre Maxwell/Pascal.

use lr_types::{GpuInfo, GpuVendor, HardwareProfile};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

mod check;
pub mod decision;
pub mod experimental;
mod manager;
pub mod prism;
pub use check::{
    EngineCheck, InstalledRuntime, PruneResult, Verdict, build_number, check, is_prism_tag, prune,
    scan_installed,
};
pub use manager::{RuntimeError, RuntimeEvent, RuntimeManager, RuntimeState};

/// Tag da release do llama.cpp homologada para esta versão do app.
/// Atualizada manualmente após testes de contrato (ver plano, risco nº 2).
pub const PINNED_TAG: &str = "b10441";

/// Repositório de onde vêm os binários oficiais.
pub const OFFICIAL_REPO: &str = "ggml-org/llama.cpp";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendVariant {
    /// CUDA 13.3 — NVIDIA Turing+ (CC >= 7.5) com driver >= 580.
    Cuda13,
    /// CUDA 12.4 — NVIDIA com driver >= 527.41 (inclui Maxwell/Pascal).
    Cuda12,
    /// Vulkan — AMD/Intel (e NVIDIA como fallback).
    Vulkan,
    /// CPU puro (dispatch de microarquitetura em runtime).
    Cpu,
    /// macOS arm64 com Metal.
    MacosArm64,
    /// macOS x64 (somente CPU).
    MacosX64,
}

impl BackendVariant {
    /// Ordem de fallback caso o download/health-check da variante falhe.
    pub fn fallback(self) -> Option<BackendVariant> {
        match self {
            BackendVariant::Cuda13 => Some(BackendVariant::Cuda12),
            BackendVariant::Cuda12 => Some(BackendVariant::Vulkan),
            BackendVariant::Vulkan => Some(BackendVariant::Cpu),
            _ => None,
        }
    }
}

/// Versão de driver NVIDIA como número comparável: "581.42" (Windows) ->
/// 581.42; "610.43.03" (Linux, três partes) -> 610.43. Só as duas primeiras
/// partes decidem — é assim que a NVIDIA numera os ramos de driver.
fn parse_driver_version(v: &str) -> Option<f64> {
    let mut partes = v.trim().split('.');
    let major = partes.next()?;
    let minor = partes.next().unwrap_or("0");
    format!("{major}.{minor}").parse::<f64>().ok()
}

/// O pacote CUDA que a placa NVIDIA aguenta, pelo driver e pela compute
/// capability: driver >= 580 e Turing+ (CC >= 7.5) levam o 13.3; driver >=
/// 527.41, o 12.4. `None` = driver velho demais para qualquer um dos dois.
fn cuda_variant(gpu: &GpuInfo) -> Option<BackendVariant> {
    let driver = gpu
        .driver_version
        .as_deref()
        .and_then(parse_driver_version)
        .unwrap_or(0.0);
    let cc_ok = gpu
        .cuda_compute
        .is_none_or(|(maj, min)| maj > 7 || (maj == 7 && min >= 5));
    if driver >= 580.0 && cc_ok {
        Some(BackendVariant::Cuda13)
    } else if driver >= 527.41 {
        Some(BackendVariant::Cuda12)
    } else {
        None
    }
}

/// A placa principal roda um pacote CUDA 13 (NVIDIA Turing+ com driver >=
/// 580), em qualquer sistema.
///
/// Não é o mesmo que `select_variant(..) == Cuda13`: no Linux o motor
/// oficial vai pelo Vulkan (a release pinada não tem CUDA para Linux), mas os
/// pacotes que o projeto compila — o MoE-cache e o motor de decisão — trazem
/// CUDA 13 para Linux também. É esta a pergunta que eles fazem.
pub fn cuda13_capable(profile: &HardwareProfile) -> bool {
    profile.best_gpu().is_some_and(|gpu| {
        gpu.vendor == GpuVendor::Nvidia && cuda_variant(gpu) == Some(BackendVariant::Cuda13)
    })
}

/// Decide a melhor variante para o hardware detectado.
///
/// Ladder: NVIDIA (driver >= 580 e CC >= 7.5) → cuda-13.3; NVIDIA (driver >=
/// 527.41) → cuda-12.4; AMD/Intel dedicada ou integrada → vulkan; sem GPU →
/// cpu. No macOS a arquitetura decide. No Linux, qualquer placa → vulkan: a
/// release pinada só publica CPU e Vulkan para Linux, e o Vulkan cobre as
/// três marcas (a NVIDIA pelo ICD que o driver proprietário instala).
pub fn select_variant(profile: &HardwareProfile) -> BackendVariant {
    if profile.os == "macos" {
        return if profile.arch == "aarch64" {
            BackendVariant::MacosArm64
        } else {
            BackendVariant::MacosX64
        };
    }

    let Some(gpu) = profile.best_gpu() else {
        return BackendVariant::Cpu;
    };
    if profile.os == "linux" {
        return BackendVariant::Vulkan;
    }

    match gpu.vendor {
        GpuVendor::Nvidia => cuda_variant(gpu).unwrap_or(BackendVariant::Vulkan),
        GpuVendor::Amd | GpuVendor::Intel | GpuVendor::Other => BackendVariant::Vulkan,
        GpuVendor::Apple => BackendVariant::MacosArm64,
    }
}

/// Nome do asset principal na release para a variante/tag, neste sistema.
///
/// `None` quando a release não publica essa variante para este sistema (CUDA
/// no Linux, por exemplo): quem pede recebe um erro claro, em vez de um nome
/// que daria 404 — ou, pior, o pacote de outro sistema, que foi o que o app
/// baixava no Linux até a 0.23: o zip do Windows, sem `llama-server` dentro.
pub fn asset_name(tag: &str, variant: BackendVariant) -> Option<String> {
    asset_name_for(std::env::consts::OS, tag, variant)
}

/// [`asset_name`] para um sistema explícito — os nomes de cada plataforma
/// ficam testáveis em qualquer uma.
pub fn asset_name_for(os: &str, tag: &str, variant: BackendVariant) -> Option<String> {
    let sufixo = match (os, variant) {
        ("windows", BackendVariant::Cuda13) => "win-cuda-13.3-x64.zip",
        ("windows", BackendVariant::Cuda12) => "win-cuda-12.4-x64.zip",
        ("windows", BackendVariant::Vulkan) => "win-vulkan-x64.zip",
        ("windows", BackendVariant::Cpu) => "win-cpu-x64.zip",
        ("linux", BackendVariant::Vulkan) => "ubuntu-vulkan-x64.tar.gz",
        ("linux", BackendVariant::Cpu) => "ubuntu-x64.tar.gz",
        ("macos", BackendVariant::MacosArm64) => "macos-arm64.tar.gz",
        ("macos", BackendVariant::MacosX64) => "macos-x64.tar.gz",
        _ => return None,
    };
    Some(format!("llama-{tag}-bin-{sufixo}"))
}

/// Asset extra com as DLLs do CUDA runtime, quando a variante exige. Só o
/// Windows tem pacote CUDA — e é lá que o cudart vem à parte.
pub fn cudart_asset_name(tag: &str, variant: BackendVariant) -> Option<String> {
    cudart_asset_name_for(std::env::consts::OS, tag, variant)
}

/// [`cudart_asset_name`] para um sistema explícito.
pub fn cudart_asset_name_for(os: &str, tag: &str, variant: BackendVariant) -> Option<String> {
    let _ = tag;
    match (os, variant) {
        ("windows", BackendVariant::Cuda13) => {
            Some("cudart-llama-bin-win-cuda-13.3-x64.zip".to_string())
        }
        ("windows", BackendVariant::Cuda12) => {
            Some("cudart-llama-bin-win-cuda-12.4-x64.zip".to_string())
        }
        _ => None,
    }
}

/// URL de download de um asset de uma release do GitHub.
///
/// O repositório é parâmetro porque há dois: o oficial e o fork da PrismML
/// ([`prism::REPOSITORY`]), que publica com o mesmo padrão de nomes.
pub fn asset_url(repo: &str, tag: &str, asset: &str) -> String {
    format!("https://github.com/{repo}/releases/download/{tag}/{asset}")
}

/// Nome do executável do servidor dentro do pacote extraído.
pub fn server_exe_name() -> &'static str {
    exe_name("llama-server")
}

/// Worker RPC, irmão do servidor dentro do mesmo pacote.
///
/// O release do llama.cpp compila com `GGML_RPC=ON` (está no `CMAKE_ARGS` do
/// workflow deles), então `ggml-rpc-server` vem junto — conferido no índice
/// dos pacotes de b10441, nas três variantes que interessam. Não há nada a
/// baixar à parte: se este arquivo existe, o cluster tem motor.
pub fn rpc_exe_name() -> &'static str {
    exe_name("ggml-rpc-server")
}

/// Nome de um executável do pacote do llama.cpp neste sistema.
///
/// O pacote traz irmãos além do servidor — `llama-fit-params` responde
/// quanta memória uma configuração vai custar, `llama-bench` mede tokens por
/// segundo. Eles são extraídos na mesma pasta.
pub fn exe_name(stem: &str) -> &'static str {
    // O nome é montado uma vez e vazado de propósito: são dois ou três por
    // processo, e devolver `&'static str` mantém a API igual à de antes.
    let full = if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    };
    Box::leak(full.into_boxed_str())
}

/// Diretório onde os runtimes ficam instalados: `<data_dir>/runtimes/<tag>/<variante>`.
pub fn runtime_dir(data_dir: &std::path::Path, tag: &str, variant: BackendVariant) -> PathBuf {
    let v = match variant {
        BackendVariant::Cuda13 => "cuda-13.3",
        BackendVariant::Cuda12 => "cuda-12.4",
        BackendVariant::Vulkan => "vulkan",
        BackendVariant::Cpu => "cpu",
        BackendVariant::MacosArm64 => "macos-arm64",
        BackendVariant::MacosX64 => "macos-x64",
    };
    data_dir.join("runtimes").join(tag).join(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_types::{GpuInfo, HardwareProfile};

    fn profile(gpus: Vec<GpuInfo>) -> HardwareProfile {
        HardwareProfile {
            os: "windows".into(),
            arch: "x86_64".into(),
            cpu_name: "test".into(),
            cpu_cores: 8,
            avx2: true,
            avx512: false,
            ram_total_bytes: 32 << 30,
            ram_speed_mts: None,
            ram_channels: None,
            ram_bandwidth_bytes_s: None,
            gpus,
        }
    }

    fn nvidia(driver: &str, cc: (u32, u32), vram_gb: u64) -> GpuInfo {
        GpuInfo {
            name: "NVIDIA test".into(),
            vendor: GpuVendor::Nvidia,
            vram_total_bytes: vram_gb << 30,
            is_integrated: false,
            driver_version: Some(driver.into()),
            cuda_compute: Some(cc),
            bandwidth_bytes_s: None,
        }
    }

    #[test]
    fn modern_nvidia_gets_cuda13() {
        let p = profile(vec![nvidia("581.42", (12, 0), 16)]);
        assert_eq!(select_variant(&p), BackendVariant::Cuda13);
    }

    #[test]
    fn pascal_gets_cuda12_even_with_new_driver() {
        // GTX 1080: CC 6.1 — CUDA 13 não suporta, mesmo com driver novo.
        let p = profile(vec![nvidia("581.42", (6, 1), 8)]);
        assert_eq!(select_variant(&p), BackendVariant::Cuda12);
    }

    #[test]
    fn old_driver_gets_cuda12() {
        let p = profile(vec![nvidia("545.00", (8, 9), 12)]);
        assert_eq!(select_variant(&p), BackendVariant::Cuda12);
    }

    #[test]
    fn ancient_driver_falls_to_vulkan() {
        let p = profile(vec![nvidia("470.00", (7, 5), 8)]);
        assert_eq!(select_variant(&p), BackendVariant::Vulkan);
    }

    #[test]
    fn amd_gets_vulkan() {
        let p = profile(vec![GpuInfo {
            name: "Radeon RX 7800 XT".into(),
            vendor: GpuVendor::Amd,
            vram_total_bytes: 16 << 30,
            is_integrated: false,
            driver_version: None,
            cuda_compute: None,
            bandwidth_bytes_s: None,
        }]);
        assert_eq!(select_variant(&p), BackendVariant::Vulkan);
    }

    #[test]
    fn no_gpu_gets_cpu() {
        let p = profile(vec![]);
        assert_eq!(select_variant(&p), BackendVariant::Cpu);
    }

    #[test]
    fn macos_arm_gets_metal_build() {
        let mut p = profile(vec![]);
        p.os = "macos".into();
        p.arch = "aarch64".into();
        assert_eq!(select_variant(&p), BackendVariant::MacosArm64);
    }

    #[test]
    fn asset_names_match_release_layout() {
        assert_eq!(
            asset_name_for("windows", "b10441", BackendVariant::Cuda13).as_deref(),
            Some("llama-b10441-bin-win-cuda-13.3-x64.zip")
        );
        assert_eq!(
            asset_name_for("macos", "b10441", BackendVariant::MacosArm64).as_deref(),
            Some("llama-b10441-bin-macos-arm64.tar.gz")
        );
        assert_eq!(
            cudart_asset_name_for("windows", "b10441", BackendVariant::Cuda12).as_deref(),
            Some("cudart-llama-bin-win-cuda-12.4-x64.zip")
        );
        assert_eq!(
            cudart_asset_name_for("windows", "b10441", BackendVariant::Vulkan),
            None
        );
    }

    /// Os nomes do Linux são os da release real (conferidos em b10441):
    /// `ubuntu-x64` e `ubuntu-vulkan-x64`, em `.tar.gz`.
    #[test]
    fn linux_asset_names_match_the_release() {
        assert_eq!(
            asset_name_for("linux", "b10441", BackendVariant::Vulkan).as_deref(),
            Some("llama-b10441-bin-ubuntu-vulkan-x64.tar.gz")
        );
        assert_eq!(
            asset_name_for("linux", "b10441", BackendVariant::Cpu).as_deref(),
            Some("llama-b10441-bin-ubuntu-x64.tar.gz")
        );
        assert_eq!(
            cudart_asset_name_for("linux", "b10441", BackendVariant::Vulkan),
            None
        );
    }

    /// Regressão da 0.23 no Linux: sem nome próprio, o app baixava o zip do
    /// Windows e falhava com "llama-server não encontrado". Variante que o
    /// sistema não tem é `None`, nunca o pacote de outro sistema.
    #[test]
    fn a_variant_the_system_does_not_have_has_no_asset() {
        for v in [BackendVariant::Cuda13, BackendVariant::Cuda12] {
            assert_eq!(asset_name_for("linux", "b10441", v), None);
            assert_eq!(cudart_asset_name_for("linux", "b10441", v), None);
            assert_eq!(asset_name_for("macos", "b10441", v), None);
        }
        assert_eq!(
            asset_name_for("windows", "b10441", BackendVariant::MacosArm64),
            None
        );
        assert_eq!(
            asset_name_for("linux", "b10441", BackendVariant::MacosX64),
            None
        );
    }

    /// Todo sistema acha pacote para a variante que o `select_variant` dá a
    /// ele, com ou sem placa — a cadeia de fallback inteira também.
    #[test]
    fn every_selected_variant_has_an_asset_on_its_system() {
        let perfis = [
            ("windows", vec![nvidia("581.42", (12, 0), 16)]),
            ("windows", vec![]),
            ("linux", vec![nvidia("610.43.03", (8, 6), 24)]),
            ("linux", vec![]),
        ];
        for (os, gpus) in perfis {
            let mut p = profile(gpus);
            p.os = os.into();
            let mut v = Some(select_variant(&p));
            while let Some(variante) = v {
                assert!(
                    asset_name_for(os, "b10441", variante).is_some(),
                    "{os}/{variante:?} sem pacote"
                );
                v = variante.fallback();
            }
        }
    }

    /// No Linux não há CUDA na release pinada: a NVIDIA vai pelo Vulkan, e
    /// sem placa, CPU.
    #[test]
    fn linux_nvidia_gets_vulkan() {
        let mut p = profile(vec![nvidia("610.43.03", (8, 6), 24)]);
        p.os = "linux".into();
        assert_eq!(select_variant(&p), BackendVariant::Vulkan);

        let mut sem_placa = profile(vec![]);
        sem_placa.os = "linux".into();
        assert_eq!(select_variant(&sem_placa), BackendVariant::Cpu);
    }

    /// O driver do Linux tem três partes ("610.43.03"); o `parse::<f64>`
    /// antigo devolvia `None` e a placa virava "driver velho".
    #[test]
    fn linux_driver_versions_parse() {
        assert_eq!(parse_driver_version("610.43.03"), Some(610.43));
        assert_eq!(parse_driver_version("581.42"), Some(581.42));
        assert_eq!(parse_driver_version("580"), Some(580.0));
        assert_eq!(parse_driver_version("abc"), None);
    }

    /// Os pacotes CUDA que o projeto compila (MoE-cache, decisão) perguntam
    /// pela placa, não pela variante do motor oficial — que no Linux é Vulkan.
    #[test]
    fn cuda13_capability_does_not_depend_on_the_official_variant() {
        let mut linux = profile(vec![nvidia("610.43.03", (8, 6), 24)]);
        linux.os = "linux".into();
        assert!(cuda13_capable(&linux));
        assert_eq!(select_variant(&linux), BackendVariant::Vulkan);

        assert!(cuda13_capable(&profile(vec![nvidia(
            "581.42",
            (12, 0),
            16
        )])));
        // Pascal com driver novo: CUDA 12, não 13.
        assert!(!cuda13_capable(&profile(vec![nvidia("581.42", (6, 1), 8)])));
        assert!(!cuda13_capable(&profile(vec![])));
    }

    #[test]
    fn fallback_chain_ends_at_cpu() {
        let mut v = BackendVariant::Cuda13;
        let mut steps = 0;
        while let Some(next) = v.fallback() {
            v = next;
            steps += 1;
            assert!(steps < 10);
        }
        assert_eq!(v, BackendVariant::Cpu);
    }
}
