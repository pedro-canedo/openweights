//! Gerência dos runtimes llama.cpp: escolhe a variante certa para a máquina,
//! baixa da release pinada do GitHub, verifica e extrai.
//!
//! Fatos (verificados em ago/2026):
//! - Releases por build: tag `bNNNNN`, várias por dia — usamos uma tag PINADA
//!   e testada, promovida manualmente.
//! - Assets: `llama-<tag>-bin-win-{cpu|cuda-12.4|cuda-13.3|vulkan|...}-x64.zip`;
//!   macOS: `llama-<tag>-bin-macos-{arm64|x64}.tar.gz`.
//! - CUDA precisa também de `cudart-llama-bin-win-cuda-<ver>-x64.zip`,
//!   extraído NO MESMO diretório dos binários.
//! - Todos os zips embutem as variantes de CPU (`ggml-cpu-*.dll`) com dispatch
//!   em runtime — não há mais escolha por nível de AVX, e builds de GPU têm
//!   fallback de CPU embutido.
//! - CUDA 13.x exige driver >= 580 e GPU Turing+ (CC >= 7.5); CUDA 12.4 roda
//!   com driver >= 527.41 e cobre Maxwell/Pascal.

use lr_types::{GpuVendor, HardwareProfile};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

mod manager;
pub use manager::{RuntimeEvent, RuntimeManager, RuntimeState};

/// Tag da release do llama.cpp homologada para esta versão do app.
/// Atualizada manualmente após testes de contrato (ver plano, risco nº 2).
pub const PINNED_TAG: &str = "b10441";

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

/// Versão de driver NVIDIA no Windows, ex.: "581.42" -> 581.42.
fn parse_driver_version(v: &str) -> Option<f64> {
    v.trim().parse::<f64>().ok()
}

/// Decide a melhor variante para o hardware detectado.
///
/// Ladder: NVIDIA (driver >= 580 e CC >= 7.5) → cuda-13.3; NVIDIA (driver >=
/// 527.41) → cuda-12.4; AMD/Intel dedicada ou integrada → vulkan; sem GPU →
/// cpu. No macOS a arquitetura decide.
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

    match gpu.vendor {
        GpuVendor::Nvidia => {
            let driver = gpu
                .driver_version
                .as_deref()
                .and_then(parse_driver_version)
                .unwrap_or(0.0);
            let cc_ok = gpu
                .cuda_compute
                .is_none_or(|(maj, min)| maj > 7 || (maj == 7 && min >= 5));
            if driver >= 580.0 && cc_ok {
                BackendVariant::Cuda13
            } else if driver >= 527.41 {
                BackendVariant::Cuda12
            } else {
                BackendVariant::Vulkan
            }
        }
        GpuVendor::Amd | GpuVendor::Intel | GpuVendor::Other => BackendVariant::Vulkan,
        GpuVendor::Apple => BackendVariant::MacosArm64,
    }
}

/// Nome do asset principal na release para a variante/tag.
pub fn asset_name(tag: &str, variant: BackendVariant) -> String {
    match variant {
        BackendVariant::Cuda13 => format!("llama-{tag}-bin-win-cuda-13.3-x64.zip"),
        BackendVariant::Cuda12 => format!("llama-{tag}-bin-win-cuda-12.4-x64.zip"),
        BackendVariant::Vulkan => format!("llama-{tag}-bin-win-vulkan-x64.zip"),
        BackendVariant::Cpu => format!("llama-{tag}-bin-win-cpu-x64.zip"),
        BackendVariant::MacosArm64 => format!("llama-{tag}-bin-macos-arm64.tar.gz"),
        BackendVariant::MacosX64 => format!("llama-{tag}-bin-macos-x64.tar.gz"),
    }
}

/// Asset extra com as DLLs do CUDA runtime, quando a variante exige.
pub fn cudart_asset_name(tag: &str, variant: BackendVariant) -> Option<String> {
    match variant {
        BackendVariant::Cuda13 => Some("cudart-llama-bin-win-cuda-13.3-x64.zip".to_string()),
        BackendVariant::Cuda12 => Some("cudart-llama-bin-win-cuda-12.4-x64.zip".to_string()),
        _ => {
            let _ = tag;
            None
        }
    }
}

/// URL de download de um asset da release pinada.
pub fn asset_url(tag: &str, asset: &str) -> String {
    format!("https://github.com/ggml-org/llama.cpp/releases/download/{tag}/{asset}")
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
            asset_name("b10441", BackendVariant::Cuda13),
            "llama-b10441-bin-win-cuda-13.3-x64.zip"
        );
        assert_eq!(
            asset_name("b10441", BackendVariant::MacosArm64),
            "llama-b10441-bin-macos-arm64.tar.gz"
        );
        assert_eq!(
            cudart_asset_name("b10441", BackendVariant::Cuda12).as_deref(),
            Some("cudart-llama-bin-win-cuda-12.4-x64.zip")
        );
        assert_eq!(cudart_asset_name("b10441", BackendVariant::Vulkan), None);
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
