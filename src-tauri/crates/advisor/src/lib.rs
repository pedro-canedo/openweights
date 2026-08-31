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
pub mod devices;
pub mod help;
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

    /// Soma a memória anunciada de um worker RPC. Só o host conectado chama.
    pub fn with_extra_vram(mut self, extra: u64) -> Self {
        self.vram_bytes = self.vram_bytes.saturating_add(extra);
        self
    }
}
/// O que se sabe da forma do modelo, campo a campo, com `None` para "não sei".
///
/// Existe para que a mesma conta sirva às duas telas: a Descobrir, que
/// pergunta antes do download (e busca a geometria no `config.json` do
/// repositório base no Hub), e a de ajuste, que pergunta depois (e lê o
/// cabeçalho do GGUF). Antes disso a única fonte era uma tabela por faixa de
/// parâmetros, que descreve modelos densos e não tem o que dizer sobre
/// especialistas.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Geometry {
    pub n_layers: Option<u32>,
    pub n_kv_heads: Option<u32>,
    pub n_heads: Option<u32>,
    /// Declarado (`attention.key_length` / `head_dim`), quando existe.
    pub head_dim: Option<u32>,
    pub d_model: Option<u32>,
    pub n_experts: Option<u32>,
    pub n_experts_used: Option<u32>,
    pub expert_ffn: Option<u32>,
    pub shared_ffn: Option<u32>,
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
    /// Dimensão do embedding. Só entra na conta de MoE, e só quando veio do
    /// arquivo — a tabela de chute não a preenche.
    pub d_model: u32,
    /// Cabeças de atenção (não as de KV).
    pub n_heads: u32,
    /// Geometria de mistura de especialistas, quando o modelo é um MoE **e**
    /// a máquina soube ler o bastante para fazer a conta.
    pub moe: Option<MoeGeometry>,
}

/// O que faz um MoE caber onde não caberia.
///
/// Um modelo de 35 bilhões de parâmetros com 3 ativos não lê 35 bilhões de
/// pesos por token: lê os da atenção, os do especialista compartilhado e os
/// dos poucos especialistas que o roteador acordou. Os outros ficam parados —
/// e peso parado não precisa da memória rápida.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoeGeometry {
    /// Especialistas roteados por camada.
    pub n_experts: u32,
    /// Quantos disparam por token.
    pub n_experts_used: u32,
    /// Dimensão interna de um especialista roteado.
    pub expert_ffn: u32,
    /// Dimensão interna do especialista compartilhado (0 = não tem).
    pub shared_ffn: u32,
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
            // A tabela de chute não sabe nada disto, e inventar aqui seria
            // pior que não responder: quem não tem geometria não ganha o
            // veredito de MoE, e continua sendo avaliado como denso.
            d_model: 0,
            n_heads: 0,
            moe: None,
        }
    }

    /// A geometria do arquivo, quando ela existe, caindo na tabela de chute
    /// só no que faltar.
    ///
    /// É o caminho honesto: o cabeçalho do GGUF (depois do download) e o
    /// `config.json` do repositório base no Hub (antes dele) dizem camadas,
    /// cabeças e especialistas de verdade. A estimativa por faixa de
    /// parâmetros continua existindo para o que nenhum dos dois responder —
    /// e um campo chutado nunca habilita o veredito de MoE, que exige a
    /// geometria completa.
    pub fn from_geometry(params: u64, ctx_len: u32, g: &Geometry) -> Self {
        let base = Self::estimate_from_params(params, ctx_len);
        let n_heads = g.n_heads.unwrap_or(0);
        // `head_dim` declarado vence; senão sai de `d_model / cabeças`; e só
        // então cai no chute.
        let head_dim = g
            .head_dim
            .or_else(|| match (g.d_model, n_heads) {
                (Some(d), h) if h > 0 && d % h == 0 => Some(d / h),
                _ => None,
            })
            .unwrap_or(base.head_dim);
        Self {
            n_layers: g.n_layers.unwrap_or(base.n_layers),
            n_kv_heads: g.n_kv_heads.unwrap_or(base.n_kv_heads),
            head_dim,
            ctx_len,
            kv_bytes_per_elem: base.kv_bytes_per_elem,
            d_model: g.d_model.unwrap_or(0),
            n_heads,
            moe: match (g.n_experts, g.expert_ffn) {
                (Some(n), Some(ffn)) if n > 0 && ffn > 0 => Some(MoeGeometry {
                    n_experts: n,
                    n_experts_used: g.n_experts_used.unwrap_or(0),
                    expert_ffn: ffn,
                    shared_ffn: g.shared_ffn.unwrap_or(0),
                }),
                _ => None,
            },
        }
    }

    /// Que fatia dos pesos são especialistas ROTEADOS — a fatia que pode
    /// morar na RAM do sistema sem tirar a placa do caminho crítico.
    ///
    /// Sai da geometria, nunca de constante: por camada, os especialistas
    /// roteados são `n_experts × 3 × d_model × expert_ffn` (as três matrizes
    /// de um FFN com porta), contra a atenção e o especialista compartilhado,
    /// que disparam em todo token e por isso ficam na placa. Sem a geometria
    /// completa a resposta é `None`, e o veredito segue o caminho denso.
    pub fn routed_expert_fraction(&self) -> Option<f32> {
        let m = self.moe?;
        if m.n_experts == 0 || m.expert_ffn == 0 || self.d_model == 0 || self.n_heads == 0 {
            return None;
        }
        let d = self.d_model as f64;
        let roteados = m.n_experts as f64 * 3.0 * d * m.expert_ffn as f64;
        let compartilhado = 3.0 * d * m.shared_ffn as f64;
        // Q, K, V e O. Q e O andam com as cabeças completas; K e V, com as
        // de KV — é daí que vem a economia de GQA.
        let atencao =
            d * self.head_dim as f64 * (2.0 * self.n_heads as f64 + 2.0 * self.n_kv_heads as f64);
        let total = roteados + compartilhado + atencao;
        (total > 0.0).then(|| (roteados / total) as f32)
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
    /// MoE com os especialistas roteados na RAM do sistema.
    ///
    /// É um veredito próprio, e não um "parcial", porque a divisão é outra:
    /// no parcial, camadas inteiras — atenção incluída — vão para a CPU, e a
    /// atenção roda em TODO token, então cada camada fora da placa custa
    /// caro. Aqui só saem os especialistas, que ficam parados quase sempre.
    /// O mesmo arquivo, dividido do jeito certo, roda numa placa que a conta
    /// densa diria não caber.
    MoeOffload { ncmoe: u32, layers_total: u32 },
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
    } else if let Some(v) = moe_verdict(budget, meta, file_size_bytes, kv, vram_avail, ram_avail) {
        v
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

/// O veredito quando o modelo é um MoE e a máquina soube ler a geometria.
///
/// A pergunta muda de "quantas camadas cabem?" para "de quantas camadas
/// preciso tirar os especialistas?". A conta é direta: cada camada cujos
/// especialistas roteados saem da placa devolve
/// `arquivo × fração_roteada / camadas` bytes de VRAM. Sobe-se esse número
/// até o modelo caber — e ele tem de caber DUAS vezes: na placa o que fica, e
/// na RAM o que sai.
///
/// É estimativa, e assumida como tal: depois do download o
/// `llama-fit-params` mede a mesma configuração de verdade, e é o número dele
/// que a tela de ajuste mostra. Esta conta serve para a tela Descobrir não
/// pintar de cinza um arquivo que a máquina roda bem.
fn moe_verdict(
    budget: &MemoryBudget,
    meta: &ModelMeta,
    file_size_bytes: u64,
    kv: u64,
    vram_avail: u64,
    ram_avail: u64,
) -> Option<FitVerdict> {
    if budget.vram_bytes == 0 || budget.unified {
        return None;
    }
    let fracao = meta.routed_expert_fraction()? as f64;
    let camadas = meta.n_layers.max(1);
    let por_camada = (file_size_bytes as f64 * fracao) / camadas as f64;
    if por_camada <= 0.0 {
        return None;
    }

    let precisa = (file_size_bytes + kv).saturating_sub(vram_avail) as f64;
    let ncmoe = (precisa / por_camada).ceil() as u32;
    if ncmoe == 0 || ncmoe > camadas {
        // Nem com todos os especialistas fora a placa dá conta: quem responde
        // é o caminho denso, que sabe dizer "só CPU" e "não cabe".
        return None;
    }
    // O que sai da placa tem de caber na RAM — junto com o resto do que o
    // sistema já usa.
    if (por_camada * ncmoe as f64) as u64 > ram_avail {
        return None;
    }
    Some(FitVerdict::MoeOffload {
        ncmoe,
        layers_total: camadas,
    })
}

/// A RAM que dá para contar com ela: o total menos a fatia que o sistema e o
/// resto do computador já ocupam.
pub fn usable_ram(ram_bytes: u64) -> u64 {
    (ram_bytes as f64 * RAM_USABLE_FRACTION) as u64
}

/// Quanto a nossa aritmética costuma errar nesta máquina.
///
/// Antes do download não existe arquivo para o `llama-fit-params` inspecionar,
/// então a estimativa continua sendo conta nossa. Mas depois de cada download
/// a sonda mede o mesmo modelo de verdade — e comparar as duas dá o fator que
/// corrige as próximas estimativas.
///
/// Sem histórico, o fator é 1.0: melhor uma estimativa crua e dita como tal
/// do que uma correção inventada.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Calibration {
    pub factor: f64,
    /// Quantas comparações sustentam o fator.
    pub samples: u32,
}

impl Default for Calibration {
    fn default() -> Self {
        Self {
            factor: 1.0,
            samples: 0,
        }
    }
}

impl Calibration {
    /// Calcula o fator a partir de pares (estimado, medido).
    ///
    /// Usa a mediana, não a média: um modelo com geometria fora do comum
    /// (MoE, atenção incomum) erraria muito e puxaria a média sozinho.
    pub fn from_pairs(pairs: &[(u64, u64)]) -> Self {
        let mut razoes: Vec<f64> = pairs
            .iter()
            .filter(|(e, m)| *e > 0 && *m > 0)
            .map(|(e, m)| *m as f64 / *e as f64)
            .collect();
        if razoes.is_empty() {
            return Self::default();
        }
        razoes.sort_by(|a, b| a.total_cmp(b));
        let meio = razoes.len() / 2;
        let mediana = if razoes.len().is_multiple_of(2) {
            (razoes[meio - 1] + razoes[meio]) / 2.0
        } else {
            razoes[meio]
        };
        Self {
            // Um fator absurdo é sinal de dado ruim, não de máquina estranha.
            factor: mediana.clamp(0.5, 2.0),
            samples: razoes.len() as u32,
        }
    }

    /// Vale a pena mostrar à pessoa? Duas amostras não são padrão.
    pub fn is_meaningful(&self) -> bool {
        self.samples >= 3 && (self.factor - 1.0).abs() >= 0.03
    }

    pub fn apply(&self, bytes: u64) -> u64 {
        (bytes as f64 * self.factor) as u64
    }
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
    /// Memória estimada com a janela avaliada — arquivo + KV cache + reserva.
    ///
    /// Era calculada e descartada antes de sair do Rust, e a tela só mostrava
    /// o tamanho do arquivo. É a diferença entre "18 GB" e "18 GB agora, 21
    /// GB com a janela que você usa".
    pub est_total_bytes: u64,
    pub kv_cache_bytes: u64,
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
                est_total_bytes: report.est_total_bytes,
                kv_cache_bytes: report.kv_cache_bytes,
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
    // Especialistas na RAM vem ANTES do offload parcial: no parcial saem
    // camadas inteiras, e a atenção — que roda em todo token — sai junto.
    let moe = pick(&opts, |v| matches!(v, FitVerdict::MoeOffload { .. }));
    let any_fit = pick(&opts, |v| !matches!(v, FitVerdict::WontFit));
    if let Some(i) = full_gpu.or(moe).or(any_fit) {
        opts[i].recommended = true;
    }
    opts
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A estimativa que a tela mostra antes do download vinha sendo jogada
    /// fora aqui dentro: só o tamanho do arquivo chegava à pessoa.
    #[test]
    fn the_estimate_reaches_whoever_asked() {
        let budget = MemoryBudget {
            vram_bytes: 16 << 30,
            ram_bytes: 32 << 30,
            unified: false,
        };
        let meta = ModelMeta::estimate_from_params(8_000_000_000, 8192);
        let opts = evaluate_files(
            &budget,
            &meta,
            &[QuantFile {
                filename: "m-Q4_K_M.gguf".into(),
                size_bytes: 5 << 30,
            }],
        );
        assert!(opts[0].est_total_bytes > opts[0].size_bytes);
        assert!(opts[0].kv_cache_bytes > 0);
    }

    #[test]
    fn without_history_the_estimate_is_left_alone() {
        let c = Calibration::default();
        assert_eq!(c.apply(1000), 1000);
        assert!(!c.is_meaningful());
        assert_eq!(Calibration::from_pairs(&[]), Calibration::default());
    }

    /// Uma medição fora da curva não pode reescrever a régua sozinha.
    #[test]
    fn the_correction_uses_the_median_and_not_the_average() {
        let pares = [(100u64, 106u64), (200, 214), (300, 318), (400, 800)];
        let c = Calibration::from_pairs(&pares);
        assert!(
            c.factor < 1.10,
            "o par fora da curva puxaria a média: {c:?}"
        );
        assert!(c.factor > 1.04);
        assert_eq!(c.samples, 4);
    }

    #[test]
    fn a_correction_of_two_percent_is_not_worth_saying() {
        let c = Calibration::from_pairs(&[(100, 102), (200, 204), (300, 306)]);
        assert!(!c.is_meaningful(), "2% é ruído: {c:?}");

        let vale = Calibration::from_pairs(&[(100, 112), (200, 226), (300, 335)]);
        assert!(vale.is_meaningful());
    }

    #[test]
    fn an_absurd_ratio_is_clamped_instead_of_believed() {
        let c = Calibration::from_pairs(&[(100, 10_000), (200, 20_000), (300, 30_000)]);
        assert_eq!(c.factor, 2.0);
    }

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
            d_model: 0,
            n_heads: 0,
            moe: None,
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

    /// A geometria de um Qwen3-30B-A3B: 128 especialistas, 8 disparando.
    fn moe_30b(ctx: u32) -> ModelMeta {
        ModelMeta {
            n_layers: 48,
            n_kv_heads: 4,
            head_dim: 128,
            ctx_len: ctx,
            kv_bytes_per_elem: 2.0,
            d_model: 2048,
            n_heads: 32,
            moe: Some(MoeGeometry {
                n_experts: 128,
                n_experts_used: 8,
                expert_ffn: 768,
                shared_ffn: 0,
            }),
        }
    }

    /// O caso do vídeo: um arquivo que a conta densa condena, e que roda com
    /// os especialistas na RAM do sistema.
    #[test]
    fn a_moe_that_does_not_fit_gets_its_own_verdict() {
        let budget = MemoryBudget {
            vram_bytes: 6 << 30,
            ram_bytes: 32 << 30,
            unified: false,
        };
        let r = evaluate(&budget, &moe_30b(8192), 22 << 30);
        match r.verdict {
            FitVerdict::MoeOffload {
                ncmoe,
                layers_total,
            } => {
                assert_eq!(layers_total, 48);
                assert!(ncmoe > 0 && ncmoe <= 48, "ncmoe fora da faixa: {ncmoe}");
            }
            outro => panic!("esperava MoeOffload, veio {outro:?}"),
        }
    }

    /// Sem RAM para receber o que sai da placa, a divisão não existe — e
    /// prometer que existe seria pior do que dizer que não cabe.
    #[test]
    fn without_ram_for_the_experts_there_is_no_offload() {
        let budget = MemoryBudget {
            vram_bytes: 6 << 30,
            ram_bytes: 4 << 30,
            unified: false,
        };
        let r = evaluate(&budget, &moe_30b(8192), 22 << 30);
        assert!(
            !matches!(r.verdict, FitVerdict::MoeOffload { .. }),
            "veio {:?}",
            r.verdict
        );
    }

    /// Modelo denso não ganha o veredito de MoE, nem quando aperta.
    #[test]
    fn a_dense_model_is_never_a_moe_offload() {
        let budget = MemoryBudget {
            vram_bytes: 6 << 30,
            ram_bytes: 32 << 30,
            unified: false,
        };
        let denso = ModelMeta::estimate_from_params(27_000_000_000, 8192);
        let r = evaluate(&budget, &denso, 14 << 30);
        assert!(!matches!(r.verdict, FitVerdict::MoeOffload { .. }));
    }

    /// A fração roteada sai da geometria: com 128 especialistas de 768 contra
    /// uma atenção pequena, a esmagadora maioria dos pesos pode sair da placa.
    #[test]
    fn the_routed_fraction_comes_from_the_geometry() {
        let f = moe_30b(8192).routed_expert_fraction().unwrap();
        assert!(f > 0.9, "fração roteada baixa demais: {f}");
        // Sem geometria não há fração — e sem fração, nada de veredito MoE.
        assert_eq!(
            ModelMeta::estimate_from_params(30_000_000_000, 8192).routed_expert_fraction(),
            None
        );
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
    fn extra_vram_from_a_peer_is_added_to_the_budget() {
        let sozinho = MemoryBudget {
            vram_bytes: 8 << 30,
            ram_bytes: 32 << 30,
            unified: false,
        };
        let junto = sozinho.with_extra_vram(9 << 30);
        assert_eq!(junto.vram_bytes, 17 << 30);
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
            d_model: 0,
            n_heads: 0,
            moe: None,
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
