//! Detecção de hardware (na inicialização) e telemetria em tempo real (1–2 Hz).
//!
//! Fontes por plataforma:
//! - RAM/CPU: `sysinfo` (instância persistente; CPU% exige duas leituras
//!   separadas por >= `MINIMUM_CPU_UPDATE_INTERVAL`).
//! - AVX2/AVX-512: `is_x86_feature_detected!`.
//! - NVIDIA: NVML via `nvml-wrapper` (carregado dinamicamente — falha graciosa
//!   sem driver). VRAM/nome/driver/compute capability + utilização.
//! - Windows (qualquer vendor): DXGI `EnumAdapters1` para enumeração e VRAM
//!   total; classificação iGPU/dGPU via `D3D12_FEATURE_ARCHITECTURE1` (UMA) —
//!   nunca confiar apenas em `DedicatedVideoMemory > 0`. Telemetria: VRAM
//!   usada via `QueryVideoMemoryInfo` e utilização AMD/Intel via contadores
//!   PDH (`\GPU Engine(*)\Utilization Percentage`) — ver `windows_gpu`.
//! - macOS: memória unificada (`recommendedMaxWorkingSetSize` ≈ 75% da RAM).

use lr_types::{GpuInfo, GpuTelemetry, HardwareProfile, Telemetry};
use sysinfo::System;

#[cfg(windows)]
mod windows_gpu;

/// Detecta o perfil completo da máquina. Chamado uma vez na inicialização
/// (e sob demanda pelo botão "redetectar" nas configurações).
pub fn detect() -> HardwareProfile {
    let mut sys = System::new();
    sys.refresh_memory();
    sys.refresh_cpu_all();

    let cpu_name = sys
        .cpus()
        .first()
        .map(|c| c.brand().trim().to_string())
        .unwrap_or_else(|| "CPU desconhecida".to_string());

    HardwareProfile {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        cpu_name,
        cpu_cores: sys.cpus().len(),
        avx2: detect_avx2(),
        avx512: detect_avx512(),
        ram_total_bytes: sys.total_memory(),
        gpus: detect_gpus(),
    }
}

fn detect_avx2() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx2")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

fn detect_avx512() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx512f")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

fn detect_gpus() -> Vec<GpuInfo> {
    #[cfg(windows)]
    {
        windows_gpu::detect()
    }
    #[cfg(target_os = "macos")]
    {
        macos_gpu_stub()
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "macos")]
fn macos_gpu_stub() -> Vec<GpuInfo> {
    // Apple Silicon: memória unificada. O teto prático para Metal é
    // ~75% da RAM (recommendedMaxWorkingSetSize); usamos essa fração até a
    // integração com Metal ser implementada (M1).
    let mut sys = System::new();
    sys.refresh_memory();
    vec![GpuInfo {
        name: "Apple GPU (memória unificada)".to_string(),
        vendor: lr_types::GpuVendor::Apple,
        vram_total_bytes: sys.total_memory() * 3 / 4,
        is_integrated: true,
        driver_version: None,
        cuda_compute: None,
    }]
}

/// Amostrador de telemetria. Mantenha UMA instância viva e chame
/// [`Monitor::sample`] no máximo a cada ~500 ms.
pub struct Monitor {
    sys: System,
    profile_gpus: Vec<GpuInfo>,
    /// Estado persistente de GPU no Windows (NVML + adaptadores DXGI + query
    /// PDH) — handles abertos uma vez e reutilizados a cada amostra.
    #[cfg(windows)]
    win_gpu: windows_gpu::WinGpuMonitor,
}

impl Monitor {
    pub fn new(profile: &HardwareProfile) -> Self {
        let mut sys = System::new();
        // Primeira leitura de CPU é descartável (diferencial).
        sys.refresh_cpu_usage();
        Self {
            sys,
            profile_gpus: profile.gpus.clone(),
            #[cfg(windows)]
            win_gpu: windows_gpu::WinGpuMonitor::new(&profile.gpus),
        }
    }

    pub fn sample(&mut self) -> Telemetry {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();

        let cpu_percent = self.sys.global_cpu_usage();

        Telemetry {
            cpu_percent,
            ram_used_bytes: self.sys.used_memory(),
            ram_total_bytes: self.sys.total_memory(),
            gpus: self.sample_gpus(),
            ts_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    fn sample_gpus(&mut self) -> Vec<GpuTelemetry> {
        #[cfg(windows)]
        {
            self.win_gpu.sample(&self.profile_gpus)
        }
        #[cfg(not(windows))]
        {
            self.profile_gpus
                .iter()
                .map(|g| GpuTelemetry {
                    util_percent: None,
                    vram_used_bytes: None,
                    vram_total_bytes: g.vram_total_bytes,
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_returns_sane_profile() {
        let p = detect();
        assert!(p.cpu_cores > 0);
        assert!(p.ram_total_bytes > 1 << 28, "RAM deveria ser > 256 MiB");
        assert!(!p.cpu_name.is_empty());
    }

    #[test]
    fn monitor_samples() {
        let p = detect();
        let mut m = Monitor::new(&p);
        std::thread::sleep(std::time::Duration::from_millis(250));
        let t = m.sample();
        assert!(t.ram_used_bytes > 0);
        assert!(t.cpu_percent >= 0.0 && t.cpu_percent <= 100.0);
    }
}
