//! Que configuração usar para este modelo, nesta máquina.
//!
//! A pergunta "cabe?" o [`crate::probe`] responde com exatidão. O que falta é
//! decidir **o que perguntar a ele**: sondar todas as combinações levaria
//! meio minuto de ampulheta, e a maioria delas é obviamente ruim. Este módulo
//! é a heurística que estreita a lista antes de gastar sonda — e a explicação
//! que a pessoa lê depois.
//!
//! Tudo aqui é puro: entra hardware e modelo, sai uma lista de candidatos.
//! Quem mede é a casca; quem escolhe entre os medidos é [`pick`].

use crate::{MemoryBudget, ModelMeta};
use lr_types::tuning::{KvType, ModelProfile, ProfileSource};
use serde::{Deserialize, Serialize};

/// Janelas oferecidas, da mais modesta à mais generosa. Passar disso numa
/// máquina de desktop é quase sempre trocar velocidade por contexto que a
/// conversa não usa.
const JANELAS: [u32; 5] = [8192, 16384, 32768, 65536, 131_072];

/// Folga deixada na placa. Encostar no teto é o jeito mais rápido de o driver
/// começar a derramar VRAM para a memória do sistema — o modelo continua
/// "cabendo", e roda várias vezes mais devagar, sem erro nenhum.
pub const MARGEM_VRAM_BYTES: u64 = 768 * 1024 * 1024;

/// Janela mínima para o modo agente funcionar minimamente bem.
///
/// Abaixo disso o harness encolhe por todos os lados: o cardápio cai para 12
/// ferramentas, a compactação dispara cedo e o plano vira etapas minúsculas.
/// 32.768 é onde essas três curvas param de doer — e é o que o app garante ao
/// carregar um modelo local sem perfil escolhido pela pessoa.
pub const AGENT_MIN_CTX: u32 = 32_768;

/// O que a pessoa quer desta máquina. Não é gosto escondido no código: são
/// respostas defensáveis para o mesmo hardware, e a tela mostra todas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Intent {
    /// O equilíbrio: janela suficiente, tudo na placa, sem apertar.
    Balanced,
    /// Mais janela, aceitando comprimir o KV cache para pagar por ela.
    MoreContext,
    /// Velocidade: janela curta e KV leve, porque KV grande custa banda de
    /// memória a cada token gerado.
    Fast,
    /// Pegada mínima, para deixar a placa livre para o resto do computador.
    LowMemory,
}

impl Intent {
    /// Todos, na ordem em que a tela os mostra.
    pub const ALL: [Intent; 4] = [
        Intent::Fast,
        Intent::Balanced,
        Intent::MoreContext,
        Intent::LowMemory,
    ];
}

/// Um candidato antes de ser medido.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub profile: ModelProfile,
    pub intent: Intent,
}

/// Monta os candidatos a sondar — poucos, e na ordem de preferência.
///
/// A regra é simples e conservadora: começa pela maior janela que a conta
/// grosseira diz caber com folga, e oferece uma alternativa com o dobro de
/// janela e KV comprimido. Flash attention entra sempre que há GPU (é ganho
/// sem custo de memória relevante); o KV comprimido só entra quando a janela
/// pedida o exige.
/// `real_layers` vem do CABEÇALHO do GGUF (`block_count`), quando deu para
/// ler. É a única fonte aceitável para `ngl`: a tabela de chute por faixa de
/// parâmetros escreveu "48" num modelo de 65 camadas, e com `fit = off` as
/// 17 que sobraram foram para a CPU — a geração caiu de 23 para 4 tok/s, sem
/// erro nenhum. Sem o número real, `ngl` fica de fora e o `fit` do llama.cpp
/// continua ligado, que erra sempre para o lado de carregar.
pub fn candidates(
    budget: &MemoryBudget,
    meta: &ModelMeta,
    file_size_bytes: u64,
    real_layers: Option<u32>,
) -> Vec<Candidate> {
    let tem_gpu = budget.vram_bytes > 0;
    let teto = if tem_gpu {
        budget.vram_bytes.saturating_sub(MARGEM_VRAM_BYTES)
    } else {
        (budget.ram_bytes as f64 * crate::RAM_USABLE_FRACTION) as u64
    };

    // Maior janela que cabe com KV em f16.
    let equilibrada = maior_janela(meta, file_size_bytes, teto, KvType::F16);
    // A alternativa dobra a janela e paga com KV comprimido; só existe se
    // couber e se de fato for mais janela do que a equilibrada.
    let ampla = maior_janela(meta, file_size_bytes, teto, KvType::Q8_0);

    let base = |ctx: u32, kv: Option<KvType>, intent: Intent| Candidate {
        profile: ModelProfile {
            ctx: Some(ctx),
            // Nunca o chute: ou o número real do arquivo, ou nada (fit on).
            ngl: real_layers,
            kv_k: kv,
            kv_v: kv,
            flash_attn: tem_gpu.then_some(true),
            source: ProfileSource::Recommended,
            ..Default::default()
        },
        intent,
    };

    let mut out = Vec::new();
    if let Some(ctx) = equilibrada {
        out.push(base(ctx, None, Intent::Balanced));
    }
    match (equilibrada, ampla) {
        (Some(e), Some(a)) if a > e => out.push(base(a, Some(KvType::Q8_0), Intent::MoreContext)),
        // Sem janela nenhuma com f16: a comprimida vira a principal, porque é
        // a única que roda inteira na placa.
        (None, Some(a)) => out.push(base(a, Some(KvType::Q8_0), Intent::Balanced)),
        _ => {}
    }

    // Rápida: a menor janela útil com KV leve. Contexto grande não é de
    // graça nem quando cabe — o KV é lido a cada token gerado.
    if let Some(&menor) = JANELAS.first()
        && equilibrada.is_some_and(|e| e > menor)
    {
        out.push(base(menor, Some(KvType::Q8_0), Intent::Fast));
    }

    // Pouca memória: o mínimo de tudo, para deixar a placa livre para o
    // resto do computador (um jogo, um editor de vídeo, outro modelo).
    if equilibrada.is_some() {
        out.push(base(JANELAS[0], Some(KvType::Q4_0), Intent::LowMemory));
    }

    // Nada coube na placa: o modelo ainda roda partido, e é melhor dizer com
    // que janela do que não dizer nada.
    if out.is_empty() {
        out.push(Candidate {
            profile: ModelProfile {
                ctx: Some(JANELAS[0]),
                kv_k: Some(KvType::Q8_0),
                kv_v: Some(KvType::Q8_0),
                flash_attn: tem_gpu.then_some(true),
                source: ProfileSource::Recommended,
                ..Default::default()
            },
            intent: Intent::Balanced,
        });
    }
    out
}

/// Perfil agêntico para um modelo local sem perfil escolhido.
///
/// A promessa é UMA: janela de [`AGENT_MIN_CTX`] tokens. Sem perfil, o `fit`
/// do llama.cpp decide sozinho — e numa placa apertada ele entrega 8k, que
/// mata o modo agente em silêncio. Aqui a janela é fixada e o resto é
/// deliberadamente conservador:
///
/// - **`ngl` fica de fora.** Sem `ngl`, o `fit` continua LIGADO e divide as
///   camadas entre placa e RAM se for preciso — a janela vale sempre, e não
///   existe o risco de um chute de camadas fazer o modelo nem carregar (este
///   caminho roda no boot, sem a rede de desfazer do `tune_apply`).
/// - **O KV comprime só o necessário**: f16 quando a estimativa diz que o
///   modelo inteiro + KV cabem na placa com folga; q8_0 quando aperta; q4_0
///   quando nem assim — perda de qualidade tem que ser paga por um motivo.
/// - Flash attention entra quando há GPU (ganho sem custo de memória).
///
/// O tamanho do arquivo responde pelo modelo: parâmetros são estimados por
/// `bytes / 0.6` (a densidade típica de um Q4), e o erro dessa conta só muda
/// o TIER de compressão do KV — nunca a janela, nunca a carga.
/// `training_ctx` é a janela de TREINO do arquivo (`context_length`), quando
/// legível: pedir além dela degrada a resposta em silêncio, então a janela
/// agêntica é o mínimo entre as duas.
pub fn agent_profile(
    budget: &MemoryBudget,
    file_size_bytes: u64,
    training_ctx: Option<u32>,
) -> ModelProfile {
    let tem_gpu = budget.vram_bytes > 0;
    let teto = if tem_gpu {
        budget.vram_bytes.saturating_sub(MARGEM_VRAM_BYTES)
    } else {
        (budget.ram_bytes as f64 * crate::RAM_USABLE_FRACTION) as u64
    };
    let janela = AGENT_MIN_CTX.min(training_ctx.unwrap_or(u32::MAX)).max(1);
    let params = (file_size_bytes as f64 / 0.6) as u64;
    let meta = ModelMeta::estimate_from_params(params, janela);

    let cabe = |kv: KvType| {
        let m = ModelMeta {
            kv_bytes_per_elem: kv.bytes_per_elem(),
            ..meta
        };
        file_size_bytes + m.kv_cache_bytes() <= teto
    };
    let kv = if cabe(KvType::F16) {
        None
    } else if cabe(KvType::Q8_0) {
        Some(KvType::Q8_0)
    } else {
        Some(KvType::Q4_0)
    };

    ModelProfile {
        ctx: Some(janela),
        kv_k: kv,
        kv_v: kv,
        flash_attn: tem_gpu.then_some(true),
        source: ProfileSource::Recommended,
        ..Default::default()
    }
}

/// Maior janela da tabela cujo custo estimado cabe no teto.
fn maior_janela(meta: &ModelMeta, file_size_bytes: u64, teto: u64, kv: KvType) -> Option<u32> {
    JANELAS.iter().rev().copied().find(|&ctx| {
        let m = ModelMeta {
            ctx_len: ctx,
            kv_bytes_per_elem: kv.bytes_per_elem(),
            ..*meta
        };
        file_size_bytes + m.kv_cache_bytes() <= teto
    })
}

/// Um candidato depois de medido.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Measured {
    pub profile: ModelProfile,
    pub intent: Intent,
    /// Memória por dispositivo, vinda da sonda.
    pub report: crate::probe::ProbeReport,
    /// Cabe na placa com a folga que deixamos?
    pub fits_gpu: bool,
}

/// Escolhe entre os medidos: o maior contexto que ainda cabe com folga.
///
/// Quando nenhum cabe, devolve o de menor pegada — a tela dirá que vai rodar
/// partido, o que é honesto e melhor do que não oferecer nada.
pub fn pick(medidos: &[Measured]) -> Option<&Measured> {
    medidos
        .iter()
        .filter(|m| m.fits_gpu)
        .max_by_key(|m| m.profile.ctx.unwrap_or(0))
        .or_else(|| medidos.iter().min_by_key(|m| m.report.gpu_bytes()))
}

/// Um motivo, em linguagem de gente, para uma escolha.
///
/// Chave e valores separados para a interface traduzir; o backend não sabe o
/// idioma da pessoa.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reason {
    /// Chave em `tune.reason.*` no i18n.
    pub key: String,
    /// Valores já formatados (números em bytes, contagens).
    pub values: Vec<(String, String)>,
}

impl Reason {
    fn new(key: &str, values: &[(&str, String)]) -> Self {
        Self {
            key: key.to_string(),
            values: values
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        }
    }
}

/// Por que esta configuração — com os números que a sustentam.
pub fn explain(
    escolhido: &Measured,
    alternativa: Option<&Measured>,
    vram_bytes: u64,
) -> Vec<Reason> {
    let mut out = Vec::new();
    let p = &escolhido.profile;

    if let Some(ctx) = p.ctx {
        out.push(Reason::new(
            "ctx",
            &[
                ("ctx", ctx.to_string()),
                (
                    "kv",
                    escolhido
                        .report
                        .devices
                        .iter()
                        .map(|(_, m)| m.context_bytes)
                        .sum::<u64>()
                        .to_string(),
                ),
            ],
        ));
    }
    if p.kv_k == Some(KvType::Q8_0) {
        out.push(Reason::new("kvCompressed", &[]));
    }
    if p.flash_attn == Some(true) {
        out.push(Reason::new("flashAttn", &[]));
    }
    if escolhido.fits_gpu {
        out.push(Reason::new(
            "fitsGpu",
            &[
                ("used", escolhido.report.gpu_bytes().to_string()),
                ("total", vram_bytes.to_string()),
            ],
        ));
    } else {
        out.push(Reason::new(
            "partial",
            &[("host", escolhido.report.host_bytes().to_string())],
        ));
    }
    if let Some(alt) = alternativa
        && let (Some(a), Some(e)) = (alt.profile.ctx, p.ctx)
        && a != e
    {
        out.push(Reason::new(
            "alternative",
            &[
                ("ctx", a.to_string()),
                ("used", alt.report.gpu_bytes().to_string()),
            ],
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{DeviceMemory, ProbeReport};

    const GIB: u64 = 1 << 30;

    fn placa(vram_gib: u64) -> MemoryBudget {
        MemoryBudget {
            vram_bytes: vram_gib * GIB,
            ram_bytes: 32 * GIB,
            unified: false,
        }
    }

    fn modelo_8b() -> ModelMeta {
        ModelMeta::estimate_from_params(8_000_000_000, 8192)
    }

    #[test]
    fn a_model_that_fits_gets_the_biggest_window_that_still_fits() {
        let c = candidates(&placa(16), &modelo_8b(), 5 * GIB, Some(36));
        assert!(!c.is_empty());
        let escolhido = &c[0];
        assert_eq!(escolhido.intent, Intent::Balanced);
        assert!(escolhido.profile.ctx.unwrap() >= 32768, "{:?}", escolhido);
        // Com GPU, flash attention entra: é ganho sem custo de memória.
        assert_eq!(escolhido.profile.flash_attn, Some(true));
        assert_eq!(escolhido.profile.ngl, Some(36));
    }

    /// A alternativa existe para a explicação ser verificável: "e se eu
    /// quiser mais contexto?" tem resposta com número, não com opinião.
    #[test]
    fn there_is_an_alternative_with_more_window() {
        let c = candidates(&placa(16), &modelo_8b(), 5 * GIB, Some(36));
        let ampla = c.iter().find(|x| x.intent == Intent::MoreContext);
        if let Some(a) = ampla {
            assert!(a.profile.ctx.unwrap() > c[0].profile.ctx.unwrap());
            assert_eq!(a.profile.kv_k, Some(KvType::Q8_0));
        }
    }

    /// Placa pequena com modelo grande: ainda assim sai uma proposta, porque
    /// "não sei dizer" é pior do que "vai rodar partido, com esta janela".
    #[test]
    fn a_model_too_big_still_gets_a_proposal() {
        let c = candidates(&placa(8), &modelo_8b(), 40 * GIB, Some(36));
        assert_eq!(c.len(), 1);
        assert!(c[0].profile.ctx.is_some());
    }

    #[test]
    fn without_a_gpu_flash_attention_is_left_alone() {
        let budget = MemoryBudget {
            vram_bytes: 0,
            ram_bytes: 32 * GIB,
            unified: false,
        };
        let c = candidates(&budget, &modelo_8b(), 5 * GIB, Some(36));
        assert_eq!(c[0].profile.flash_attn, None);
    }

    /// Quatro perfis não são quatro gostos: são quatro perguntas diferentes
    /// sobre a mesma placa, e cada uma tem resposta com número.
    #[test]
    fn the_four_profiles_answer_four_different_questions() {
        let c = candidates(&placa(16), &modelo_8b(), 5 * GIB, Some(36));
        let por_intencao = |i: Intent| c.iter().find(|x| x.intent == i);

        let rapida = por_intencao(Intent::Fast).expect("rápida");
        let equilibrada = por_intencao(Intent::Balanced).expect("equilibrada");
        let pouca = por_intencao(Intent::LowMemory).expect("pouca memória");

        // A rápida nunca pede mais janela que a equilibrada.
        assert!(rapida.profile.ctx.unwrap() <= equilibrada.profile.ctx.unwrap());
        // A de pouca memória comprime mais que todas.
        assert_eq!(pouca.profile.kv_k, Some(KvType::Q4_0));
    }

    /// Numa máquina onde nada cabe, oferecer quatro perfis seria teatro.
    #[test]
    fn a_machine_that_cannot_hold_the_model_gets_one_honest_proposal() {
        let c = candidates(&placa(8), &modelo_8b(), 40 * GIB, Some(36));
        assert_eq!(c.len(), 1);
    }

    fn medido(ctx: u32, gpu_gib: u64, cabe: bool) -> Measured {
        Measured {
            profile: ModelProfile {
                ctx: Some(ctx),
                ..Default::default()
            },
            intent: Intent::Balanced,
            report: ProbeReport {
                devices: vec![(
                    "CUDA0".into(),
                    DeviceMemory {
                        model_bytes: gpu_gib * GIB,
                        context_bytes: 0,
                        compute_bytes: 0,
                    },
                )],
            },
            fits_gpu: cabe,
        }
    }

    /// O bug que derrubou um 27B de 23 para 4 tok/s: a tabela de chute dizia
    /// 48 camadas, o arquivo tinha 65, e com `fit = off` as 17 restantes
    /// foram para a CPU. O contrato agora: SEM o número real do cabeçalho,
    /// `ngl` fica de fora (e o fit continua ligado); com ele, é ele.
    #[test]
    fn a_layer_guess_never_becomes_ngl() {
        let sem_leitura = candidates(&placa(16), &modelo_8b(), 5 * GIB, None);
        assert!(
            sem_leitura.iter().all(|c| c.profile.ngl.is_none()),
            "chute de tabela virou ngl: {sem_leitura:?}"
        );
        let com_leitura = candidates(&placa(16), &modelo_8b(), 5 * GIB, Some(65));
        assert!(com_leitura.iter().all(|c| c.profile.ngl == Some(65)));
    }

    /// A janela agêntica respeita o TREINO do modelo: pedir 32k a um modelo
    /// treinado em 16k degrada a resposta em silêncio.
    #[test]
    fn the_agent_window_respects_the_training_context() {
        let curto = agent_profile(&placa(16), 5_500_000_000, Some(16_384));
        assert_eq!(curto.ctx, Some(16_384));
        let longo = agent_profile(&placa(16), 5_500_000_000, Some(262_144));
        assert_eq!(longo.ctx, Some(AGENT_MIN_CTX));
    }

    /// A promessa do perfil agêntico: a janela é SEMPRE 32.768; o que muda
    /// conforme a placa é só quanto o KV comprime — e `ngl` nunca entra,
    /// porque este perfil é gravado no boot, sem rede de desfazer.
    #[test]
    fn the_agent_profile_always_promises_the_agent_window() {
        // 9B Q4 (~5.5 GiB) numa placa de 16 GB: cabe folgado, KV em f16.
        let folgado = agent_profile(&placa(16), 5_500_000_000, None);
        assert_eq!(folgado.ctx, Some(AGENT_MIN_CTX));
        assert_eq!(folgado.kv_k, None, "com folga não se paga compressão");
        assert_eq!(folgado.ngl, None, "o fit continua ligado");
        assert_eq!(folgado.flash_attn, Some(true));
        assert_eq!(folgado.source, ProfileSource::Recommended);

        // Um Q4 de ~20B (12 GB) na mesma placa: aperta, o KV comprime.
        let apertado = agent_profile(&placa(16), 12_000_000_000, None);
        assert_eq!(apertado.ctx, Some(AGENT_MIN_CTX));
        assert!(apertado.kv_k.is_some(), "apertou: comprime");

        // Modelo maior que a placa: a janela NÃO cede — o fit divide as
        // camadas com a RAM, mais lento porém agêntico.
        let transborda = agent_profile(&placa(8), 40_000_000_000, None);
        assert_eq!(transborda.ctx, Some(AGENT_MIN_CTX));
        assert_eq!(transborda.kv_k, Some(KvType::Q4_0));
        assert_eq!(transborda.ngl, None);

        // Sem GPU: sem flash attention, e o teto vem da RAM.
        let cpu = agent_profile(
            &MemoryBudget {
                vram_bytes: 0,
                ram_bytes: 32 * GIB,
                unified: false,
            },
            5_500_000_000,
            None,
        );
        assert_eq!(cpu.ctx, Some(AGENT_MIN_CTX));
        assert_eq!(cpu.flash_attn, None);
    }

    #[test]
    fn the_pick_is_the_largest_window_that_still_fits() {
        let m = [
            medido(8192, 6, true),
            medido(32768, 9, true),
            medido(131_072, 15, false),
        ];
        assert_eq!(pick(&m).unwrap().profile.ctx, Some(32768));
    }

    #[test]
    fn when_nothing_fits_the_lightest_one_is_offered() {
        let m = [medido(32768, 20, false), medido(8192, 12, false)];
        assert_eq!(pick(&m).unwrap().profile.ctx, Some(8192));
        assert!(pick(&[]).is_none());
    }

    #[test]
    fn the_explanation_carries_the_numbers_that_justify_it() {
        let escolhido = Measured {
            profile: ModelProfile {
                ctx: Some(16384),
                kv_k: Some(KvType::Q8_0),
                kv_v: Some(KvType::Q8_0),
                flash_attn: Some(true),
                ..Default::default()
            },
            ..medido(16384, 10, true)
        };
        let chaves: Vec<String> = explain(&escolhido, None, 16 * GIB)
            .into_iter()
            .map(|r| r.key)
            .collect();
        assert!(chaves.contains(&"ctx".to_string()));
        assert!(chaves.contains(&"kvCompressed".to_string()));
        assert!(chaves.contains(&"flashAttn".to_string()));
        assert!(chaves.contains(&"fitsGpu".to_string()));
    }

    #[test]
    fn not_fitting_is_said_out_loud() {
        let chaves: Vec<String> = explain(&medido(8192, 20, false), None, 16 * GIB)
            .into_iter()
            .map(|r| r.key)
            .collect();
        assert!(chaves.contains(&"partial".to_string()));
        assert!(!chaves.contains(&"fitsGpu".to_string()));
    }
}
