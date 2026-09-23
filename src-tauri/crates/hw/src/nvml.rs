//! O que o NVML responde, igual no Windows e no Linux.
//!
//! O NVML vem com o driver proprietário da NVIDIA (`nvml.dll` no Windows,
//! `libnvidia-ml.so.1` no Linux) e é carregado em tempo de execução: sem o
//! driver, só a inicialização falha, e o app segue sem esses números. Por
//! isso nada aqui devolve erro — o que o driver não responde vira `None`.

use lr_types::{GpuInfo, GpuVendor};
use nvml_wrapper::Nvml;
use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};

/// O NVML fala em miliwatts; a tela e o `nvidia-smi` falam em watts.
pub(crate) fn mw_para_w(mw: u32) -> u32 {
    mw.div_ceil(1000)
}

/// Banda de memória da placa, em bytes por segundo.
///
/// `relógio de memória × 2 × largura do barramento / 8` — o ×2 é o "double
/// data rate" das memórias GDDR, e é o que faz a conta bater com a ficha
/// técnica: uma RTX 3090 reporta 9751 MHz em 384 bits, que dão os 936 GB/s
/// anunciados. Qualquer dos dois números faltando devolve `None`: metade da
/// conta não é uma banda.
fn banda(dev: &nvml_wrapper::Device<'_>) -> Option<u64> {
    let mhz = dev.max_clock_info(Clock::Memory).ok()? as u64;
    let bits = dev.memory_bus_width().ok()? as u64;
    (mhz > 0 && bits > 0).then(|| mhz * 1_000_000 * 2 * bits / 8)
}

/// Uma placa NVIDIA como o NVML a descreve.
///
/// O Windows só usa `info` (casa a telemetria pelo nome, contra o DXGI); o
/// índice e o endereço PCI são do caminho do Linux.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) struct Placa {
    /// Índice no NVML — o que a telemetria e o `nvidia-smi -i` usam.
    pub index: u32,
    /// Endereço PCI no formato do sysfs (`0000:05:00.0`). É a chave exata
    /// que casa a placa com o `/sys/class/drm` no Linux.
    pub pci: Option<String>,
    pub info: GpuInfo,
}

/// As placas NVIDIA: nome, driver, compute capability, VRAM e banda.
pub(crate) fn placas(nvml: &Nvml) -> Vec<Placa> {
    let driver = nvml.sys_driver_version().ok();
    let mut out = Vec::new();
    for index in 0..nvml.device_count().unwrap_or(0) {
        let Ok(dev) = nvml.device_by_index(index) else {
            continue;
        };
        let cc = dev
            .cuda_compute_capability()
            .ok()
            .map(|c| (c.major as u32, c.minor as u32));
        out.push(Placa {
            index,
            pci: dev.pci_info().ok().map(|p| pci_do_sysfs(&p.bus_id)),
            info: GpuInfo {
                name: dev.name().unwrap_or_else(|_| "GPU NVIDIA".to_string()),
                vendor: GpuVendor::Nvidia,
                vram_total_bytes: dev.memory_info().map(|m| m.total).unwrap_or(0),
                is_integrated: false,
                driver_version: driver.clone(),
                cuda_compute: cc,
                bandwidth_bytes_s: banda(&dev),
            },
        });
    }
    out
}

/// O NVML escreve o domínio com oito dígitos (`00000000:05:00.0`); o sysfs,
/// com quatro (`0000:05:00.0`). Comparar sem normalizar nunca casaria.
pub(crate) fn pci_do_sysfs(bus_id: &str) -> String {
    let id = bus_id.trim().to_ascii_lowercase();
    match id.split_once(':') {
        Some((dominio, resto)) if resto.contains(':') => match u32::from_str_radix(dominio, 16) {
            Ok(d) => format!("{d:04x}:{resto}"),
            Err(_) => id,
        },
        _ => id,
    }
}

/// Uma leitura de telemetria de uma placa. O formato também serve ao
/// amdgpu no Linux, que lê os mesmos números de arquivos do sysfs.
pub(crate) struct Amostra {
    pub util_percent: Option<f32>,
    pub vram_used_bytes: Option<u64>,
    /// O NVML é mais preciso que o DXGI para a VRAM de uma NVIDIA — quem
    /// tiver este número deve preferi-lo ao do perfil.
    pub vram_total_bytes: Option<u64>,
    pub temp_c: Option<f32>,
    pub power_w: Option<u32>,
    pub power_limit_w: Option<u32>,
}

/// Utilização, VRAM, temperatura e watts de uma placa, numa passada só: o
/// NVML já está aberto e o device já está em mãos.
pub(crate) fn amostra(dev: &nvml_wrapper::Device<'_>) -> Amostra {
    let mem = dev.memory_info().ok();
    Amostra {
        util_percent: dev.utilization_rates().ok().map(|u| u.gpu as f32),
        vram_used_bytes: mem.as_ref().map(|m| m.used),
        vram_total_bytes: mem.as_ref().map(|m| m.total),
        temp_c: dev
            .temperature(TemperatureSensor::Gpu)
            .ok()
            .map(|t| t as f32),
        power_w: dev.power_usage().ok().map(mw_para_w),
        power_limit_w: dev.power_management_limit().ok().map(mw_para_w),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Arredondar para cima evita o "349 W" de um limite de 350 000 mW que
    /// perdeu um milésimo no caminho.
    #[test]
    fn milliwatts_become_whole_watts() {
        assert_eq!(mw_para_w(350_000), 350);
        assert_eq!(mw_para_w(250_000), 250);
        assert_eq!(mw_para_w(349_999), 350);
        assert_eq!(mw_para_w(0), 0);
    }

    #[test]
    fn the_nvml_bus_id_becomes_the_sysfs_address() {
        assert_eq!(pci_do_sysfs("00000000:05:00.0"), "0000:05:00.0");
        assert_eq!(pci_do_sysfs("00000000:0A:00.0"), "0000:0a:00.0");
        assert_eq!(pci_do_sysfs("00000001:65:00.0"), "0001:65:00.0");
        // Já no formato do sysfs: continua igual.
        assert_eq!(pci_do_sysfs("0000:05:00.0"), "0000:05:00.0");
    }
}
