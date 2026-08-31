//! Memória anunciada e pin de device — a auto-configuração do cluster.
//!
//! O worker diz ao host quanto pode emprestar. Não é a VRAM cheia: uma
//! fatia fica de fora para KV cache e buffers. No Metal a Apple já recusa
//! ir além de ~75% da RAM unificada, então a conta é 75% de 75%.

use lr_types::{GpuVendor, HardwareProfile};

/// Fração da VRAM (ou do teto Metal) reservada aos pesos. O resto é KV e
/// compute — anunciar 100% é o jeito clássico de OOM no primeiro prompt.
pub const WEIGHTS_FRACTION: f64 = 0.75;

/// Teto prático do Metal (`recommendedMaxWorkingSetSize`).
pub const METAL_UNIFIED_FRACTION: f64 = 0.75;

/// Quanto esta máquina anuncia para o cluster, em bytes.
pub fn advertised_bytes(profile: &HardwareProfile) -> u64 {
    let Some(gpu) = profile.best_gpu() else {
        return 0;
    };
    let pool = if gpu.vendor == GpuVendor::Apple {
        ((profile.ram_total_bytes as f64) * METAL_UNIFIED_FRACTION) as u64
    } else {
        gpu.vram_total_bytes
    };
    ((pool as f64) * WEIGHTS_FRACTION) as u64
}

/// Device que o `ggml-rpc-server` deve pinar com `-d`.
///
/// Sem pin, o Mac anuncia Metal **e** o BLAS da Apple; o scheduler quebra.
/// O índice segue a enumeração do `hw`: iGPU+dGPU não pode virar sempre
/// `Vulkan0` se a discreta (a que anunciamos) é a segunda da lista.
pub fn device_pin(profile: &HardwareProfile) -> Option<String> {
    let gpu = profile.best_gpu()?;
    Some(match gpu.vendor {
        GpuVendor::Apple => "MTL0".into(),
        GpuVendor::Nvidia => {
            let idx = profile
                .gpus
                .iter()
                .filter(|g| g.vendor == GpuVendor::Nvidia)
                .position(|g| std::ptr::eq(g, gpu))
                .unwrap_or(0);
            format!("CUDA{idx}")
        }
        GpuVendor::Amd | GpuVendor::Intel | GpuVendor::Other => {
            let idx = profile
                .gpus
                .iter()
                .position(|g| std::ptr::eq(g, gpu))
                .unwrap_or(0);
            format!("Vulkan{idx}")
        }
    })
}

/// Nome curto da GPU para o anúncio na rede.
pub fn gpu_label(profile: &HardwareProfile) -> String {
    profile
        .best_gpu()
        .map(|g| truncate(&g.name, 80))
        .unwrap_or_else(|| "CPU".into())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_types::{GpuInfo, GpuVendor, HardwareProfile};

    fn profile(os: &str, ram_gb: u64, gpu: Option<GpuInfo>) -> HardwareProfile {
        HardwareProfile {
            os: os.into(),
            arch: "x86_64".into(),
            cpu_name: "test".into(),
            cpu_cores: 8,
            avx2: true,
            avx512: false,
            ram_total_bytes: ram_gb << 30,
            ram_speed_mts: None,
            ram_channels: None,
            ram_bandwidth_bytes_s: None,
            gpus: gpu.into_iter().collect(),
        }
    }

    fn nvidia(vram_gb: u64) -> GpuInfo {
        GpuInfo {
            name: "NVIDIA GeForce RTX 3060".into(),
            vendor: GpuVendor::Nvidia,
            vram_total_bytes: vram_gb << 30,
            is_integrated: false,
            driver_version: None,
            cuda_compute: Some((8, 6)),
            bandwidth_bytes_s: None,
        }
    }

    fn apple() -> GpuInfo {
        GpuInfo {
            name: "Apple GPU (memória unificada)".into(),
            vendor: GpuVendor::Apple,
            vram_total_bytes: 18 << 30,
            is_integrated: true,
            driver_version: None,
            cuda_compute: None,
            bandwidth_bytes_s: None,
        }
    }

    #[test]
    fn nvidia_advertises_three_quarters_of_vram() {
        let p = profile("windows", 61, Some(nvidia(12)));
        assert_eq!(advertised_bytes(&p), ((12u64 << 30) as f64 * 0.75) as u64);
        assert_eq!(device_pin(&p).as_deref(), Some("CUDA0"));
    }

    #[test]
    fn metal_is_seventy_five_of_seventy_five_of_ram() {
        // Mac 24 GB: teto Metal 18 GB, pesos 13.5 GB.
        let p = profile("macos", 24, Some(apple()));
        let metal_cap = ((24u64 << 30) as f64 * 0.75) as u64;
        let pesos = (metal_cap as f64 * 0.75) as u64;
        assert_eq!(advertised_bytes(&p), pesos);
        assert_eq!(device_pin(&p).as_deref(), Some("MTL0"));
    }

    #[test]
    fn no_gpu_advertises_nothing() {
        let p = profile("linux", 16, None);
        assert_eq!(advertised_bytes(&p), 0);
        assert_eq!(device_pin(&p), None);
    }

    #[test]
    fn amd_pins_vulkan() {
        let gpu = GpuInfo {
            name: "Radeon".into(),
            vendor: GpuVendor::Amd,
            vram_total_bytes: 16 << 30,
            is_integrated: false,
            driver_version: None,
            cuda_compute: None,
            bandwidth_bytes_s: None,
        };
        let p = profile("windows", 32, Some(gpu));
        assert_eq!(device_pin(&p).as_deref(), Some("Vulkan0"));
    }

    #[test]
    fn vulkan_pin_follows_best_gpu_index() {
        let igpu = GpuInfo {
            name: "Intel Graphics".into(),
            vendor: GpuVendor::Intel,
            vram_total_bytes: 1 << 30,
            is_integrated: true,
            driver_version: None,
            cuda_compute: None,
            bandwidth_bytes_s: None,
        };
        let dgpu = GpuInfo {
            name: "Radeon".into(),
            vendor: GpuVendor::Amd,
            vram_total_bytes: 16 << 30,
            is_integrated: false,
            driver_version: None,
            cuda_compute: None,
            bandwidth_bytes_s: None,
        };
        let p = profile("windows", 32, None);
        let mut p = p;
        p.gpus = vec![igpu, dgpu];
        assert_eq!(device_pin(&p).as_deref(), Some("Vulkan1"));
    }
}
