//! Especulação medida: MTP e n-grama valem a pena nesta máquina?
//!
//! Este é o eixo que motivou o recurso — a pergunta do vídeo sobre o Ollama,
//! "a variante MTP é mais rápida?". A resposta honesta é: **depende da
//! máquina e do texto**, e por isso ela não é uma regra no código, é uma
//! medição.
//!
//! Diferente do resto, aqui o `llama-bench` não serve: especulação é do
//! servidor, não do motor por baixo. Medir exige subir o llama-server com
//! cada configuração e **gerar de verdade** — o que, sendo configuração de
//! boot, obriga a reiniciar entre os braços.
//!
//! Dois prompts, e não um: n-grama prevê repetição, então ganha muito em
//! código e reescrita e atrapalha em prosa. Uma média entre os dois esconderia
//! exatamente a informação que interessa, então cada um é reportado separado.

use crate::commands::restart_engine;
use crate::state::AppState;
use lr_engine::{ChatMessage, ChatRequest, LlamaClient};
use lr_types::tuning::{ModelProfile, SpecSet};
use serde::Serialize;
use std::time::Instant;
use tauri::AppHandle;

/// Tokens gerados por prompt. Curto o bastante para a bateria inteira caber
/// em minutos, longo o bastante para a diferença aparecer.
const N_TOKENS: u32 = 96;

/// Os dois tipos de texto onde a especulação se comporta de forma oposta.
const PROMPTS: [(&str, &str); 2] = [
    (
        "code",
        "Reescreva esta função em Rust trocando o laço por iterador, mantendo \
         o mesmo comportamento:\n\nfn soma(v: &[i32]) -> i32 { let mut t = 0; \
         for x in v { t += x; } t }",
    ),
    (
        "prose",
        "Explique, em um parágrafo corrido e sem listas, por que rodar um \
         modelo de linguagem no próprio computador muda a relação da pessoa \
         com a ferramenta.",
    ),
];

/// O veredito de QUALIDADE de um braço.
///
/// Existe por causa da lição mais cara do assunto: um benchmark não sabe
/// dizer que a saída virou lixo. Um kernel de 8 bits chegou a render números
/// lindos por uma hora enquanto servia texto sem sentido, porque metade dos
/// fatores de escala era lida com o sinal trocado. Aqui o teste é barato e
/// exato: decodificação especulativa é *lossless* — o rascunho é conferido
/// pelo modelo grande e o que ele recusa é jogado fora —, então com
/// temperatura zero a resposta tem de ser a MESMA de quando não há
/// especulação nenhuma. Se mudou, não é otimização: é defeito.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SpecQuality {
    /// Byte a byte igual à referência.
    Match,
    /// Um texto é começo do outro, e o mais curto parou por limite de tokens.
    /// Benigno: com especulação o servidor gera em blocos e ultrapassa o teto.
    Truncated,
    /// Disse outra coisa. O braço é RECUSADO.
    Diverged,
    /// A própria máquina não repete a si mesma — ver [`QualityGate`].
    Unverifiable,
}

/// Onde os dois textos passaram a discordar, com o suficiente para a tela
/// mostrar o problema em vez de apenas afirmá-lo.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Divergence {
    pub prompt: String,
    pub at_char: usize,
    pub expected: String,
    pub got: String,
}

/// Trecho guardado de cada lado da divergência.
const TRECHO: usize = 120;

/// A medição inteira é confiável?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum QualityGate {
    /// A referência repetiu a si mesma: comparar os outros braços tem sentido.
    Ok,
    /// A referência divergiu DE SI MESMA entre duas execuções idênticas. O
    /// não-determinismo é do kernel, não da especulação — e sem esta saída o
    /// app reprovaria todos os braços numa máquina perfeitamente saudável.
    Unverifiable,
}

/// O que um braço da medição rendeu.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecArm {
    pub spec: SpecSet,
    /// Tokens por segundo em cada tipo de texto, na ordem de [`PROMPTS`].
    pub by_prompt: Vec<(String, f64)>,
    /// Média simples — serve para ordenar, não para decidir sozinha.
    pub avg_tps: f64,
    /// A resposta continuou a mesma?
    pub quality: SpecQuality,
    /// Preenchido quando `quality == Diverged`.
    pub divergence: Option<Divergence>,
}

/// Uma geração completa: o número E o texto, que é o que faltava.
#[derive(Debug, Clone)]
struct Sample {
    tps: f64,
    content: String,
    reasoning: String,
    finish_reason: Option<String>,
}

impl Sample {
    /// Tudo o que o modelo disse — raciocínio incluído. Uma divergência que
    /// aconteça só no raciocínio é divergência do mesmo jeito.
    fn texto(&self) -> String {
        format!("{}\u{1}{}", self.reasoning, self.content)
    }
}

/// Compara um braço com a referência para UM prompt.
fn comparar(
    rotulo: &str,
    referencia: &Sample,
    braco: &Sample,
) -> (SpecQuality, Option<Divergence>) {
    let a = referencia.texto();
    let b = braco.texto();
    if a == b {
        return (SpecQuality::Match, None);
    }

    // Prefixo: um continuou onde o outro parou. Só é benigno se o mais curto
    // parou por LIMITE — se ele parou sozinho ("stop"), a diferença de
    // tamanho é o modelo dizendo outra coisa.
    let (curto, longo, fim_do_curto) = if a.len() < b.len() {
        (&a, &b, referencia.finish_reason.as_deref())
    } else {
        (&b, &a, braco.finish_reason.as_deref())
    };
    if longo.starts_with(curto.as_str()) && fim_do_curto == Some("length") {
        return (SpecQuality::Truncated, None);
    }

    let at_char = a
        .chars()
        .zip(b.chars())
        .position(|(x, y)| x != y)
        .unwrap_or_else(|| a.chars().count().min(b.chars().count()));
    let recorte = |t: &str| t.chars().skip(at_char).take(TRECHO).collect::<String>();
    (
        SpecQuality::Diverged,
        Some(Divergence {
            prompt: rotulo.to_string(),
            at_char,
            expected: recorte(&a),
            got: recorte(&b),
        }),
    )
}

/// O pior de dois vereditos — um prompt que divergiu condena o braço.
fn pior(a: SpecQuality, b: SpecQuality) -> SpecQuality {
    use SpecQuality::*;
    match (a, b) {
        (Diverged, _) | (_, Diverged) => Diverged,
        (Unverifiable, _) | (_, Unverifiable) => Unverifiable,
        (Truncated, _) | (_, Truncated) => Truncated,
        _ => Match,
    }
}

/// O resultado da bateria.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecOutcome {
    pub model: String,
    pub arms: Vec<SpecArm>,
    /// Índice do braço que rendeu mais na média ENTRE OS APROVADOS.
    pub best: Option<usize>,
    /// `true` quando a diferença entre o melhor e o "sem especulação" é
    /// pequena demais para valer uma mudança de configuração.
    pub inconclusive: bool,
    /// Índice do braço sem especulação — a régua de tudo.
    pub reference: usize,
    /// A máquina se repete? Sem isso, nenhum veredito de qualidade vale.
    pub quality_gate: QualityGate,
    /// Braços recusados por mudarem a resposta.
    pub rejected: Vec<usize>,
}

/// Abaixo disto a diferença cabe no ruído de duas execuções seguidas.
const DIFERENCA_MINIMA: f64 = 0.08;

/// Mede os braços de especulação e devolve o que rendeu mais.
///
/// Não grava nem aplica nada: quem decide o que fazer com o número é a tela,
/// e quem aplica é o `tune_apply`, com a rede de desfazer que ele já tem.
pub async fn measure(
    app: &AppHandle,
    state: &AppState,
    model: &str,
    base: &ModelProfile,
    candidates: &[SpecSet],
) -> Result<SpecOutcome, String> {
    // A referência vai PRIMEIRO e é obrigatória: sem ela não há régua de
    // qualidade, e uma bateria sem juiz é exatamente o que se quer evitar.
    let mut ordem: Vec<SpecSet> = candidates.to_vec();
    let Some(pos) = ordem.iter().position(|s| s.is_off()) else {
        return Err("a medição precisa do braço sem especulação como referência".into());
    };
    ordem.swap(0, pos);

    let mut arms: Vec<SpecArm> = Vec::new();
    // Textos da referência, por prompt, para comparar os demais.
    let mut referencia: Vec<Sample> = Vec::new();
    let mut quality_gate = QualityGate::Ok;

    for (indice, spec) in ordem.iter().enumerate() {
        let perfil = ModelProfile {
            spec: Some(spec.clone()),
            ..base.clone()
        };
        // Especulação é argumento de boot: cada braço exige o motor de pé com
        // aquela configuração.
        state
            .store
            .set_model_profile(model.trim(), &perfil)
            .map_err(|e| e.to_string())?;
        restart_engine(app, state, true).await?;

        let mut by_prompt = Vec::new();
        let mut qualidade = SpecQuality::Match;
        let mut divergencia: Option<Divergence> = None;

        for (i, (rotulo, prompt)) in PROMPTS.iter().enumerate() {
            let (aquecimento, medida) = generate_twice(state, model, prompt).await?;
            by_prompt.push((rotulo.to_string(), medida.tps));

            if indice == 0 {
                // O aquecimento da referência já existia e era jogado fora.
                // Ele é a evidência grátis de que ESTA máquina repete a si
                // mesma: se as duas execuções do mesmo braço com o mesmo
                // prompt já discordam, o não-determinismo é do kernel CUDA e
                // nenhum veredito de qualidade pode condenar ninguém.
                if aquecimento.texto() != medida.texto() {
                    quality_gate = QualityGate::Unverifiable;
                }
                referencia.push(medida.clone());
            } else if let Some(r) = referencia.get(i) {
                let (q, d) = comparar(rotulo, r, &medida);
                qualidade = pior(qualidade, q);
                divergencia = divergencia.or(d);
            }
        }

        if quality_gate == QualityGate::Unverifiable {
            qualidade = SpecQuality::Unverifiable;
            divergencia = None;
        }

        let avg_tps = by_prompt.iter().map(|(_, t)| t).sum::<f64>() / by_prompt.len() as f64;
        arms.push(SpecArm {
            spec: spec.clone(),
            by_prompt,
            avg_tps,
            quality: qualidade,
            divergence: divergencia,
        });
    }

    // Um braço que mudou a resposta não concorre, por mais rápido que seja.
    // Antes disto o `max_by` corria sobre todos, e o mais rápido vencia mesmo
    // servindo outra coisa.
    let rejected: Vec<usize> = arms
        .iter()
        .enumerate()
        .filter(|(_, a)| a.quality == SpecQuality::Diverged)
        .map(|(i, _)| i)
        .collect();
    let best = arms
        .iter()
        .enumerate()
        .filter(|(i, _)| !rejected.contains(i))
        .max_by(|a, b| a.1.avg_tps.total_cmp(&b.1.avg_tps))
        .map(|(i, _)| i);

    // Ganho pequeno não justifica mudar configuração: a tela precisa poder
    // dizer "não muda nada aqui" em vez de recomendar ruído.
    let sem_spec = arms.first().map(|a| a.avg_tps);
    let inconclusive = match (best, sem_spec) {
        (Some(i), Some(base_tps)) if base_tps > 0.0 => {
            (arms[i].avg_tps - base_tps) / base_tps < DIFERENCA_MINIMA
        }
        _ => false,
    };

    Ok(SpecOutcome {
        model: model.to_string(),
        arms,
        best,
        inconclusive,
        reference: 0,
        quality_gate,
        rejected,
    })
}

/// Gera o mesmo prompt DUAS vezes e devolve as duas amostras.
///
/// A primeira sempre existiu (carregar o modelo mediria o disco); o que muda
/// é que ela deixou de ser descartada — comparar as duas é o auto-teste de
/// determinismo da máquina.
async fn generate_twice(
    state: &AppState,
    model: &str,
    prompt: &str,
) -> Result<(Sample, Sample), String> {
    let endpoint = state.llama_endpoint().await?;
    let client =
        LlamaClient::new(&endpoint.base_url).with_optional_api_key(endpoint.api_key.clone());

    let mut req = ChatRequest::new(model.trim(), vec![ChatMessage::user(prompt.to_string())]);
    req.max_tokens = Some(N_TOKENS);
    req.temperature = Some(0.0);
    // Temperatura zero não basta para comparar texto: sem `top_k = 1` e uma
    // semente fixa sobra empate para o sampler desfazer, e sem desligar o
    // cache de prompt a segunda chamada percorre outro caminho que a
    // primeira. Aí uma diferença legítima viraria "divergência".
    req.top_k = Some(1);
    let req = req
        .with_extra("seed", serde_json::json!(0))
        .with_extra("cache_prompt", serde_json::json!(false));

    let primeira = uma_geracao(&client, &req).await?;
    let segunda = uma_geracao(&client, &req).await?;
    Ok((primeira, segunda))
}

async fn uma_geracao(client: &LlamaClient, req: &ChatRequest) -> Result<Sample, String> {
    let comeco = Instant::now();
    let saida = client.complete_once(req).await.map_err(|e| e.to_string())?;
    let relogio = comeco.elapsed().as_secs_f64().max(0.001);

    // O próprio servidor informa a taxa de geração, já sem o custo de HTTP e
    // sem o tempo do prompt. Quando ele não informa, o relógio serve.
    let tps = saida
        .timings
        .as_ref()
        .and_then(|t| t.predicted_per_second)
        .filter(|v| *v > 0.0)
        .unwrap_or_else(|| {
            let tokens = saida
                .timings
                .as_ref()
                .and_then(|t| t.predicted_n)
                .unwrap_or(N_TOKENS) as f64;
            tokens / relogio
        });
    Ok(Sample {
        tps,
        content: saida.content,
        reasoning: saida.reasoning,
        finish_reason: saida.finish_reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_types::tuning::SpecType;

    fn arm(spec: SpecSet, code: f64, prose: f64) -> SpecArm {
        let by_prompt = vec![("code".into(), code), ("prose".into(), prose)];
        let avg_tps = (code + prose) / 2.0;
        SpecArm {
            spec,
            by_prompt,
            avg_tps,
            quality: SpecQuality::Match,
            divergence: None,
        }
    }

    fn amostra(texto: &str, fim: &str) -> Sample {
        Sample {
            tps: 50.0,
            content: texto.to_string(),
            reasoning: String::new(),
            finish_reason: Some(fim.to_string()),
        }
    }

    fn desligado() -> SpecSet {
        SpecSet::new([SpecType::None])
    }

    /// A conta de "vale a pena?" é a única lógica pura aqui, e é a que decide
    /// se o app vai sugerir mexer na configuração.
    fn inconclusivo(arms: &[SpecArm]) -> bool {
        let best = arms
            .iter()
            .max_by(|a, b| a.avg_tps.total_cmp(&b.avg_tps))
            .unwrap();
        let base = arms
            .iter()
            .find(|a| a.spec.is_off())
            .map(|a| a.avg_tps)
            .unwrap();
        (best.avg_tps - base) / base < DIFERENCA_MINIMA
    }

    #[test]
    fn a_small_gain_is_not_worth_changing_the_setup() {
        let arms = [
            arm(desligado(), 40.0, 38.0),
            arm(SpecType::NgramMod.into(), 41.0, 38.5),
        ];
        assert!(inconclusivo(&arms), "3% cabe no ruído de duas execuções");
    }

    #[test]
    fn a_real_gain_is_reported_as_such() {
        let arms = [
            arm(desligado(), 40.0, 38.0),
            arm(SpecType::DraftMtp.into(), 62.0, 55.0),
        ];
        assert!(!inconclusivo(&arms));
    }

    /// Especulação que atrasa é conclusão válida — o app precisa saber dizer
    /// isso, em vez de assumir que ligar ajuda.
    #[test]
    fn speculation_that_makes_it_slower_is_a_valid_conclusion() {
        let arms = [
            arm(desligado(), 40.0, 38.0),
            arm(SpecType::DraftMtp.into(), 20.0, 19.0),
        ];
        let melhor = arms
            .iter()
            .max_by(|a, b| a.avg_tps.total_cmp(&b.avg_tps))
            .unwrap();
        assert!(melhor.spec.is_off());
        assert!(inconclusivo(&arms));
    }

    /// N-grama ganha em repetição e perde em prosa: reportar só a média
    /// esconderia a única informação que interessa para decidir.
    #[test]
    fn each_kind_of_text_is_reported_on_its_own() {
        let a = arm(SpecType::NgramMod.into(), 80.0, 30.0);
        assert_eq!(a.by_prompt.len(), 2);
        assert_eq!(a.by_prompt[0].0, "code");
        assert!(a.by_prompt[0].1 > a.by_prompt[1].1);
    }

    // ------------------------------------------------------- qualidade ---

    /// Resposta igual: o que a especulação promete e o que ela tem de
    /// entregar, já que o modelo grande confere cada rascunho.
    #[test]
    fn an_identical_answer_passes() {
        let r = amostra("a raposa salta", "stop");
        let b = amostra("a raposa salta", "stop");
        assert_eq!(comparar("code", &r, &b).0, SpecQuality::Match);
    }

    /// Parou no teto de tokens em pontos diferentes: benigno. Com
    /// especulação o servidor gera em blocos e ultrapassa o limite.
    #[test]
    fn stopping_at_the_token_limit_in_different_places_is_benign() {
        let r = amostra("a raposa salta sobre", "length");
        let b = amostra("a raposa salta sobre o cão", "length");
        assert_eq!(comparar("code", &r, &b).0, SpecQuality::Truncated);
    }

    /// O MESMO prefixo, mas o curto terminou sozinho: aí a diferença de
    /// tamanho é o modelo dizendo outra coisa, não o teto cortando.
    #[test]
    fn a_short_answer_that_stopped_on_its_own_is_a_divergence() {
        let r = amostra("a raposa salta", "stop");
        let b = amostra("a raposa salta sobre o cão", "length");
        assert_eq!(comparar("code", &r, &b).0, SpecQuality::Diverged);
    }

    /// Divergência no meio guarda ONDE e O QUÊ — a tela mostra o trecho, em
    /// vez de pedir que se acredite nela.
    #[test]
    fn a_divergence_records_where_it_happened() {
        let r = amostra("resultado: 42", "stop");
        let b = amostra("resultado: 37", "stop");
        let (q, d) = comparar("code", &r, &b);
        assert_eq!(q, SpecQuality::Diverged);
        let d = d.expect("divergência tem de vir com o trecho");
        assert_eq!(d.prompt, "code");
        assert!(d.expected.starts_with("42"), "{d:?}");
        assert!(d.got.starts_with("37"), "{d:?}");
    }

    /// Divergir só no raciocínio é divergir. O Qwen3 emite raciocínio
    /// separado, e ignorá-lo deixaria passar exatamente o tipo de defeito que
    /// este portão existe para pegar.
    #[test]
    fn a_divergence_only_in_the_reasoning_still_counts() {
        let mut r = amostra("42", "stop");
        let mut b = amostra("42", "stop");
        r.reasoning = "somando 40 e 2".into();
        b.reasoning = "somando 20 e 22".into();
        assert_eq!(comparar("code", &r, &b).0, SpecQuality::Diverged);
    }

    /// O caso que motiva tudo: rápido E errado não pode vencer.
    #[test]
    fn a_fast_but_wrong_arm_never_wins() {
        let mut rapido = arm(SpecType::DraftMtp.into(), 200.0, 190.0);
        rapido.quality = SpecQuality::Diverged;
        let arms = [arm(desligado(), 40.0, 38.0), rapido];

        let rejected: Vec<usize> = arms
            .iter()
            .enumerate()
            .filter(|(_, a)| a.quality == SpecQuality::Diverged)
            .map(|(i, _)| i)
            .collect();
        let best = arms
            .iter()
            .enumerate()
            .filter(|(i, _)| !rejected.contains(i))
            .max_by(|a, b| a.1.avg_tps.total_cmp(&b.1.avg_tps))
            .map(|(i, _)| i);

        assert_eq!(rejected, vec![1]);
        assert_eq!(best, Some(0), "o lento e correto vence o rápido e errado");
    }

    /// O pior veredito manda: um prompt que divergiu condena o braço, mesmo
    /// que o outro tenha batido.
    #[test]
    fn the_worst_verdict_decides_the_arm() {
        assert_eq!(
            pior(SpecQuality::Match, SpecQuality::Diverged),
            SpecQuality::Diverged
        );
        assert_eq!(
            pior(SpecQuality::Truncated, SpecQuality::Match),
            SpecQuality::Truncated
        );
        assert_eq!(
            pior(SpecQuality::Match, SpecQuality::Match),
            SpecQuality::Match
        );
    }
}
