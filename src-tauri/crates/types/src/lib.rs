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

    /// Identidade desta máquina para efeito de desempenho.
    ///
    /// Entra tudo que muda o quanto ela rende: sistema, CPU, memória e, de
    /// cada placa, nome, memória e **versão do driver** — uma atualização de
    /// driver muda tok/s em dois dígitos percentuais. Não entra nada que
    /// identifique a pessoa: é uma chave para comparar medições, não para
    /// reconhecer quem está do outro lado.
    pub fn machine_key(&self) -> String {
        let mut partes = vec![
            self.os.clone(),
            self.arch.clone(),
            self.cpu_name.clone(),
            self.cpu_cores.to_string(),
            (self.ram_total_bytes >> 30).to_string(),
        ];
        // Ordenada: a ordem de enumeração das placas não é promessa de nada.
        let mut gpus: Vec<String> = self
            .gpus
            .iter()
            .map(|g| {
                format!(
                    "{}/{}/{}",
                    g.name,
                    g.vram_total_bytes >> 20,
                    g.driver_version.as_deref().unwrap_or("?")
                )
            })
            .collect();
        gpus.sort();
        partes.extend(gpus);

        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for b in partes.join("|").bytes() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100_0000_01b3);
        }
        format!("{hash:016x}")
    }
}

#[cfg(test)]
mod machine_key_tests {
    use super::*;

    fn perfil(driver: &str, vram_gib: u64) -> HardwareProfile {
        HardwareProfile {
            os: "windows".into(),
            arch: "x86_64".into(),
            cpu_name: "Ryzen 5 4600G".into(),
            cpu_cores: 12,
            avx2: true,
            avx512: false,
            ram_total_bytes: 32 << 30,
            gpus: vec![GpuInfo {
                name: "RTX 5060 Ti".into(),
                vendor: GpuVendor::Nvidia,
                vram_total_bytes: vram_gib << 30,
                is_integrated: false,
                driver_version: Some(driver.into()),
                cuda_compute: Some((12, 0)),
            }],
        }
    }

    #[test]
    fn the_same_machine_keeps_the_same_key() {
        assert_eq!(
            perfil("580.1", 16).machine_key(),
            perfil("580.1", 16).machine_key()
        );
    }

    /// Driver novo muda tok/s em dois dígitos: a medição antiga não descreve
    /// mais esta máquina.
    #[test]
    fn a_driver_update_is_a_different_machine() {
        assert_ne!(
            perfil("580.1", 16).machine_key(),
            perfil("581.0", 16).machine_key()
        );
    }

    #[test]
    fn changing_the_card_changes_the_key() {
        assert_ne!(
            perfil("580.1", 16).machine_key(),
            perfil("580.1", 24).machine_key()
        );
    }

    /// A ordem em que as placas foram enumeradas não é promessa de nada.
    #[test]
    fn the_order_of_the_cards_does_not_matter() {
        let mut a = perfil("580.1", 16);
        let segunda = GpuInfo {
            name: "RTX 3060".into(),
            vram_total_bytes: 12 << 30,
            ..a.gpus[0].clone()
        };
        a.gpus.push(segunda.clone());

        let mut b = perfil("580.1", 16);
        b.gpus.insert(0, segunda);

        assert_eq!(a.machine_key(), b.machine_key());
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
