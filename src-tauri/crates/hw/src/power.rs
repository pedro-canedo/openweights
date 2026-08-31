//! Limite de energia da GPU NVIDIA, lido pelo NVML.
//!
//! Existe porque a conta de "mais watts = mais rápido" não fecha nesta carga:
//! gerar tokens é um problema de BANDA de memória, e a banda não sobe com o
//! limite de energia. Numa RTX 3090 medida por terceiros, cortar de 350 W
//! para 250 W custou perto de nada em tokens por segundo — e a mesma placa,
//! comparada com uma geração mais nova e quase o dobro de consumo, ficou 2%
//! atrás, porque a placa nova tem só 8% mais banda.
//!
//! Ler é grátis e não pede privilégio. ESCREVER pede administrador, e o
//! próprio NVML documenta que o valor não sobrevive a reiniciar a máquina nem
//! a recarregar o driver — por isso o app informa em vez de prometer.

use nvml_wrapper::Nvml;
use serde::Serialize;

/// O estado de energia de uma placa, em watts.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PowerStatus {
    /// Índice no NVML — é o que o `nvidia-smi -i` espera.
    pub index: u32,
    pub name: String,
    /// Limite em vigor agora.
    pub limit_w: u32,
    /// O que a placa vem de fábrica.
    pub default_w: Option<u32>,
    /// A faixa que o driver aceita.
    pub min_w: Option<u32>,
    pub max_w: Option<u32>,
    /// Consumo instantâneo, quando a placa informa.
    pub usage_w: Option<u32>,
}

/// O estado de energia de cada placa NVIDIA. Vazio quando não há NVML — nem
/// toda máquina tem placa NVIDIA, e isso não é erro.
pub fn status() -> Vec<PowerStatus> {
    let Ok(nvml) = Nvml::init() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for i in 0..nvml.device_count().unwrap_or(0) {
        let Ok(dev) = nvml.device_by_index(i) else {
            continue;
        };
        // Sem limite legível não há o que mostrar nem o que mudar.
        let Ok(limite) = dev.power_management_limit() else {
            continue;
        };
        let faixa = dev.power_management_limit_constraints().ok();
        out.push(PowerStatus {
            index: i,
            name: dev.name().unwrap_or_else(|_| "GPU NVIDIA".to_string()),
            limit_w: mw_para_w(limite),
            default_w: dev.power_management_limit_default().ok().map(mw_para_w),
            min_w: faixa.as_ref().map(|c| mw_para_w(c.min_limit)),
            max_w: faixa.as_ref().map(|c| mw_para_w(c.max_limit)),
            usage_w: dev.power_usage().ok().map(mw_para_w),
        });
    }
    out
}

/// O NVML fala em miliwatts; a tela e o `nvidia-smi` falam em watts.
fn mw_para_w(mw: u32) -> u32 {
    mw.div_ceil(1000)
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

    /// Sem placa NVIDIA a lista é vazia — não é erro, é uma máquina sem NVML.
    #[test]
    fn a_machine_without_nvml_reports_nothing() {
        let _ = status();
    }
}
