//! Recomendação de quantização: estima a memória necessária para rodar cada
//! arquivo GGUF e classifica contra o hardware do usuário, como o widget
//! "Hardware compatibility" do Hugging Face.
//!
//! Modelo de memória (consenso verificado ago/2026):
//!   necessário = tamanho_do_arquivo
//!              + KV cache (2 · camadas · kv_heads · head_dim · ctx · bytes/elem)
//!              + reserva de runtime (~1 GiB: buffers de compute + contexto CUDA)
//!
//! Em runtime o `--fit` do llama-server faz o ajuste fino; aqui o objetivo é
//! um veredito honesto ANTES do download.

use serde::{Deserialize, Serialize};

pub mod bench;
pub mod probe;
pub mod quant;
pub mod tune;

/// Reserva de VRAM para driver/compositor/buffers (pesquisa: 577 MB–1 GiB).
pub const VRAM_RESERVE_BYTES: u64 = 1 << 30;
/// Fração da RAM utilizável para inferência em CPU (deixa folga pro SO).
pub const RAM_USABLE_FRACTION: f64 = 0.85;

/// Orçamento de memória da máquina, derivado do `HardwareProfile`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBudget {
    /// VRAM da melhor GPU dedicada (0 se não houver).
    pub vram_bytes: u64,
    pub ram_bytes: u64,
    /// Memória unificada (Apple Silicon): VRAM e RAM são o mesmo pool.
    pub unified: bool,
}

impl MemoryBudget {
    pub fn from_profile(p: &lr_types::HardwareProfile) -> Self {
        let gpu = p.best_gpu();
        let unified = gpu
            .map(|g| g.vendor == lr_types::GpuVendor::Apple)
            .unwrap_or(false);
        Self {
            vram_bytes: gpu.map(|g| g.vram_total_bytes).unwrap_or(0),
            ram_bytes: p.ram_total_bytes,
            unified,
        }
    }
}

/// Metadados do modelo necessários para estimar o KV cache. Vêm do
/// `expand[]=gguf` da API do HF ou da leitura do header GGUF; quando ausentes,
/// [`ModelMeta::estimate_from_params`] aproxima pelos nº de parâmetros.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelMeta {
    pub n_layers: u32,
    pub n_kv_heads: u32,
    pub head_dim: u32,
    /// Contexto em que avaliamos o fit (não o máximo do modelo).
    pub ctx_len: u32,
    /// Bytes por elemento do KV cache (2.0 = f16, 1.0 = q8_0).
    pub kv_bytes_per_elem: f32,
}

impl ModelMeta {
    /// Aproximação quando o header não está disponível: arquiteturas densas
    /// típicas (Llama/Qwen/Gemma) por faixa de parâmetros.
    pub fn estimate_from_params(params: u64, ctx_len: u32) -> Self {
        let (n_layers, n_kv_heads, head_dim) = match params {
            0..=2_000_000_000 => (28, 8, 64),
            2_000_000_001..=5_000_000_000 => (32, 8, 128),
            5_000_000_001..=10_000_000_000 => (36, 8, 128),
            10_000_000_001..=20_000_000_000 => (40, 8, 128),
            20_000_000_001..=40_000_000_000 => (48, 8, 128),
            _ => (80, 8, 128),
        };
        Self {
            n_layers,
            n_kv_heads,
            head_dim,
            ctx_len,
            kv_bytes_per_elem: 2.0,
        }
    }

    /// KV cache total em bytes: K e V por camada.
    pub fn kv_cache_bytes(&self) -> u64 {
        let per_elem = self.kv_bytes_per_elem as f64;
        (2.0 * self.n_layers as f64
            * self.n_kv_heads as f64
            * self.head_dim as f64
            * self.ctx_len as f64
            * per_elem) as u64
    }
}

/// Veredito de compatibilidade de um arquivo GGUF com a máquina.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum FitVerdict {
    /// Cabe inteiro na GPU: a experiência boa (badge verde).
    FullGpu { ngl: u32 },
    /// Offload parcial: roda, mas com queda brusca de velocidade (amarelo).
    Partial { ngl: u32, layers_total: u32 },
    /// Só CPU: funciona, lento (cinza).
    CpuOnly,
    /// Não cabe nem na RAM (vermelho).
    WontFit,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FitReport {
    pub verdict: FitVerdict,
    pub est_total_bytes: u64,
    pub kv_cache_bytes: u64,
}

/// Avalia um único arquivo GGUF contra o orçamento de memória.
pub fn evaluate(budget: &MemoryBudget, meta: &ModelMeta, file_size_bytes: u64) -> FitReport {
    let kv = meta.kv_cache_bytes();
    let total = file_size_bytes + kv + VRAM_RESERVE_BYTES;

    // Memória unificada (Apple Silicon): um único pool; `vram_bytes` já vem
    // com o teto do Metal (~75% da RAM) aplicado pelo crate hw.
    if budget.unified {
        let verdict = if total <= budget.vram_bytes {
            FitVerdict::FullGpu { ngl: meta.n_layers }
        } else if file_size_bytes + kv <= usable_ram(budget.ram_bytes) {
            FitVerdict::CpuOnly
        } else {
            FitVerdict::WontFit
        };
        return FitReport {
            verdict,
            est_total_bytes: total,
            kv_cache_bytes: kv,
        };
    }

    let vram_avail = budget.vram_bytes.saturating_sub(VRAM_RESERVE_BYTES);
    let ram_avail = usable_ram(budget.ram_bytes);

    let verdict = if budget.vram_bytes > 0 && file_size_bytes + kv <= vram_avail {
        FitVerdict::FullGpu { ngl: meta.n_layers }
    } else if budget.vram_bytes > 0 {
        // Offload parcial: peso por camada ≈ (arquivo + KV) / camadas.
        let per_layer = (file_size_bytes + kv) / meta.n_layers.max(1) as u64;
        let layers_fit = vram_avail.checked_div(per_layer).unwrap_or(0) as u32;
        let cpu_part = (file_size_bytes + kv).saturating_sub(vram_avail);
        if layers_fit >= 1 && cpu_part <= ram_avail {
            FitVerdict::Partial {
                ngl: layers_fit.min(meta.n_layers),
                layers_total: meta.n_layers,
            }
        } else if file_size_bytes + kv <= ram_avail {
            FitVerdict::CpuOnly
        } else {
            FitVerdict::WontFit
        }
    } else if file_size_bytes + kv <= ram_avail {
        FitVerdict::CpuOnly
    } else {
        FitVerdict::WontFit
    };

    FitReport {
        verdict,
        est_total_bytes: total,
        kv_cache_bytes: kv,
    }
}

fn usable_ram(ram_bytes: u64) -> u64 {
    (ram_bytes as f64 * RAM_USABLE_FRACTION) as u64
}

/// Um arquivo candidato de um repositório (uma quantização).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuantFile {
    pub filename: String,
    pub size_bytes: u64,
}

/// Resultado por quantização, pronto para a UI (badge + recomendação).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuantOption {
    pub filename: String,
    pub label: String,
    pub size_bytes: u64,
    pub bits: Option<f32>,
    pub recommended: bool,
    pub verdict: FitVerdict,
}

/// Avalia todos os arquivos de um repositório e marca o recomendado:
/// a quantização de melhor qualidade que roda 100% na GPU; se nenhuma roda,
/// a melhor que roda de qualquer forma.
pub fn evaluate_files(
    budget: &MemoryBudget,
    meta: &ModelMeta,
    files: &[QuantFile],
) -> Vec<QuantOption> {
    let mut opts: Vec<QuantOption> = files
        .iter()
        .map(|f| {
            let label = quant::parse_label(&f.filename);
            let report = evaluate(budget, meta, f.size_bytes);
            QuantOption {
                filename: f.filename.clone(),
                bits: quant::bits_per_weight(&label),
                label,
                size_bytes: f.size_bytes,
                recommended: false,
                verdict: report.verdict,
            }
        })
        .collect();

    let pick = |opts: &[QuantOption], pred: fn(&FitVerdict) -> bool| {
        opts.iter()
            .enumerate()
            .filter(|(_, o)| pred(&o.verdict))
            .max_by_key(|(_, o)| quant::quality_rank(&o.label))
            .map(|(i, _)| i)
    };

    let full_gpu = pick(&opts, |v| matches!(v, FitVerdict::FullGpu { .. }));
    let any_fit = pick(&opts, |v| !matches!(v, FitVerdict::WontFit));
    if let Some(i) = full_gpu.or(any_fit) {
        opts[i].recommended = true;
    }
    opts
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Máquina da referência do usuário: RTX 5060 Ti 16 GB + 32 GB RAM.
    fn rtx_5060ti() -> MemoryBudget {
        MemoryBudget {
            vram_bytes: 16 << 30,
            ram_bytes: 32 << 30,
            unified: false,
        }
    }

    /// Modelo ~27B denso (como o Qwen3-27B do screenshot de referência).
    fn meta_27b() -> ModelMeta {
        ModelMeta {
            n_layers: 64,
            n_kv_heads: 8,
            head_dim: 128,
            ctx_len: 8192,
            kv_bytes_per_elem: 2.0,
        }
    }

    #[test]
    fn small_quant_fits_fully_on_16gb_gpu() {
        // 2-bit ~10 GB: verde no screenshot de referência.
        let r = evaluate(&rtx_5060ti(), &meta_27b(), 10 << 30);
        assert!(matches!(r.verdict, FitVerdict::FullGpu { ngl: 64 }));
    }

    #[test]
    fn q4_of_27b_is_partial_on_16gb_gpu() {
        // 4-bit ~16-17 GB: não cabe em 16 GB de VRAM (vermelho/parcial no HF).
        let r = evaluate(&rtx_5060ti(), &meta_27b(), 17 << 30);
        match r.verdict {
            FitVerdict::Partial { ngl, layers_total } => {
                assert_eq!(layers_total, 64);
                assert!(ngl > 0 && ngl < 64, "ngl parcial fora do esperado: {ngl}");
            }
            other => panic!("esperava Partial, veio {other:?}"),
        }
    }

    #[test]
    fn huge_file_wont_fit() {
        // BF16 de 55 GB: 32 GB RAM + 16 VRAM não seguram (com margens).
        let r = evaluate(&rtx_5060ti(), &meta_27b(), 55 << 30);
        assert_eq!(r.verdict, FitVerdict::WontFit);
    }

    #[test]
    fn no_gpu_machine_is_cpu_only() {
        let budget = MemoryBudget {
            vram_bytes: 0,
            ram_bytes: 16 << 30,
            unified: false,
        };
        let meta = ModelMeta::estimate_from_params(8_000_000_000, 4096);
        let r = evaluate(&budget, &meta, 5 << 30);
        assert_eq!(r.verdict, FitVerdict::CpuOnly);
    }

    #[test]
    fn kv_cache_formula_matches_hand_calc() {
        // 2 · 32 camadas · 8 kv_heads · 128 dim · 4096 ctx · 2 bytes = 512 MiB
        let meta = ModelMeta {
            n_layers: 32,
            n_kv_heads: 8,
            head_dim: 128,
            ctx_len: 4096,
            kv_bytes_per_elem: 2.0,
        };
        assert_eq!(meta.kv_cache_bytes(), 512 << 20);
    }

    #[test]
    fn recommends_best_full_gpu_quant() {
        let budget = rtx_5060ti();
        let meta = ModelMeta::estimate_from_params(8_000_000_000, 8192);
        let files = vec![
            QuantFile {
                filename: "m-Q2_K.gguf".into(),
                size_bytes: 3 << 30,
            },
            QuantFile {
                filename: "m-Q4_K_M.gguf".into(),
                size_bytes: 5 << 30,
            },
            QuantFile {
                filename: "m-Q8_0.gguf".into(),
                size_bytes: 9 << 30,
            },
            QuantFile {
                filename: "m-BF16.gguf".into(),
                size_bytes: 16 << 30,
            },
        ];
        let opts = evaluate_files(&budget, &meta, &files);
        // Q8_0 (9 GB) ainda cabe em 16 GB - reserva; é a melhor qualidade verde.
        let rec: Vec<_> = opts.iter().filter(|o| o.recommended).collect();
        assert_eq!(rec.len(), 1);
        assert_eq!(rec[0].label, "Q8_0");
    }

    #[test]
    fn ud_quant_preferred_over_static_at_same_fit() {
        let budget = rtx_5060ti();
        let meta = ModelMeta::estimate_from_params(8_000_000_000, 8192);
        let files = vec![
            QuantFile {
                filename: "m-UD-Q4_K_XL.gguf".into(),
                size_bytes: 5 << 30,
            },
            QuantFile {
                filename: "m-Q4_K_M.gguf".into(),
                size_bytes: 5 << 30,
            },
        ];
        let opts = evaluate_files(&budget, &meta, &files);
        let rec = opts.iter().find(|o| o.recommended).unwrap();
        assert_eq!(rec.label, "UD-Q4_K_XL");
    }
}
