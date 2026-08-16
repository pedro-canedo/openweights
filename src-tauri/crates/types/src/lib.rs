//! Tipos compartilhados entre os crates do OpenWeights.
//!
//! Tudo aqui é serializado para o frontend com `camelCase` — mantenha em
//! sincronia com `src/lib/types.ts`.

use serde::{Deserialize, Serialize};

pub mod agent;
pub mod automation;
pub mod scout;
pub mod tuning;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
    Apple,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuInfo {
    pub name: String,
    pub vendor: GpuVendor,
    pub vram_total_bytes: u64,
    pub is_integrated: bool,
    pub driver_version: Option<String>,
    /// Compute capability CUDA (major, minor), se NVIDIA com driver ativo.
    pub cuda_compute: Option<(u32, u32)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareProfile {
    pub os: String,
    pub arch: String,
    pub cpu_name: String,
    pub cpu_cores: usize,
    pub avx2: bool,
    pub avx512: bool,
    pub ram_total_bytes: u64,
    pub gpus: Vec<GpuInfo>,
}

impl HardwareProfile {
    /// Melhor GPU dedicada para inferência, se houver.
    pub fn best_gpu(&self) -> Option<&GpuInfo> {
        self.gpus
            .iter()
            .filter(|g| !g.is_integrated)
            .max_by_key(|g| g.vram_total_bytes)
            .or_else(|| self.gpus.iter().max_by_key(|g| g.vram_total_bytes))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuTelemetry {
    /// Utilização 0–100; `None` quando o SO/driver não expõe.
    pub util_percent: Option<f32>,
    pub vram_used_bytes: Option<u64>,
    pub vram_total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Telemetry {
    pub cpu_percent: f32,
    pub ram_used_bytes: u64,
    pub ram_total_bytes: u64,
    pub gpus: Vec<GpuTelemetry>,
    pub ts_ms: u64,
}
