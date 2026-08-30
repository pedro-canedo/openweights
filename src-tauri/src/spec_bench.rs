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
use lr_types::tuning::{ModelProfile, SpecType};
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

/// O que um braço da medição rendeu.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecArm {
    pub spec: SpecType,
    /// Tokens por segundo em cada tipo de texto, na ordem de [`PROMPTS`].
    pub by_prompt: Vec<(String, f64)>,
    /// Média simples — serve para ordenar, não para decidir sozinha.
    pub avg_tps: f64,
}

/// O resultado da bateria.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecOutcome {
    pub model: String,
    pub arms: Vec<SpecArm>,
    /// Índice do braço que rendeu mais na média.
    pub best: Option<usize>,
    /// `true` quando a diferença entre o melhor e o "sem especulação" é
    /// pequena demais para valer uma mudança de configuração.
    pub inconclusive: bool,
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
    candidates: &[SpecType],
) -> Result<SpecOutcome, String> {
    let mut arms: Vec<SpecArm> = Vec::new();

    for spec in candidates {
        let perfil = ModelProfile {
            spec: Some(*spec),
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
        for (rotulo, prompt) in PROMPTS {
            let tps = generate_tps(state, model, prompt).await?;
            by_prompt.push((rotulo.to_string(), tps));
        }
        let avg_tps = by_prompt.iter().map(|(_, t)| t).sum::<f64>() / by_prompt.len() as f64;
        arms.push(SpecArm {
            spec: *spec,
            by_prompt,
            avg_tps,
        });
    }

    let best = arms
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.avg_tps.total_cmp(&b.1.avg_tps))
        .map(|(i, _)| i);

    // Ganho pequeno não justifica mudar configuração: a tela precisa poder
    // dizer "não muda nada aqui" em vez de recomendar ruído.
    let sem_spec = arms
        .iter()
        .find(|a| a.spec == SpecType::None)
        .map(|a| a.avg_tps);
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
    })
}

/// Gera um punhado de tokens e mede a taxa.
async fn generate_tps(state: &AppState, model: &str, prompt: &str) -> Result<f64, String> {
    let endpoint = state.llama_endpoint().await?;
    let client =
        LlamaClient::new(&endpoint.base_url).with_optional_api_key(endpoint.api_key.clone());

    let mut req = ChatRequest::new(model.trim(), vec![ChatMessage::user(prompt.to_string())]);
    req.max_tokens = Some(N_TOKENS);
    req.temperature = Some(0.0);

    // A primeira chamada carrega o modelo; medir isso mediria o disco.
    let _ = client
        .complete_once(&req)
        .await
        .map_err(|e| e.to_string())?;

    let comeco = Instant::now();
    let saida = client
        .complete_once(&req)
        .await
        .map_err(|e| e.to_string())?;
    let relogio = comeco.elapsed().as_secs_f64().max(0.001);

    // O próprio servidor informa a taxa de geração, já sem o custo de HTTP e
    // sem o tempo do prompt. Quando ele não informa, o relógio serve.
    Ok(saida
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
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arm(spec: SpecType, code: f64, prose: f64) -> SpecArm {
        let by_prompt = vec![("code".into(), code), ("prose".into(), prose)];
        let avg_tps = (code + prose) / 2.0;
        SpecArm {
            spec,
            by_prompt,
            avg_tps,
        }
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
            .find(|a| a.spec == SpecType::None)
            .map(|a| a.avg_tps)
            .unwrap();
        (best.avg_tps - base) / base < DIFERENCA_MINIMA
    }

    #[test]
    fn a_small_gain_is_not_worth_changing_the_setup() {
        let arms = [
            arm(SpecType::None, 40.0, 38.0),
            arm(SpecType::Ngram, 41.0, 38.5),
        ];
        assert!(inconclusivo(&arms), "3% cabe no ruído de duas execuções");
    }

    #[test]
    fn a_real_gain_is_reported_as_such() {
        let arms = [
            arm(SpecType::None, 40.0, 38.0),
            arm(SpecType::Mtp, 62.0, 55.0),
        ];
        assert!(!inconclusivo(&arms));
    }

    /// O caso que o vídeo do Ollama mostrou: MTP pior que sem especulação.
    /// O app precisa saber concluir isso, em vez de assumir que ligar ajuda.
    #[test]
    fn speculation_that_makes_it_slower_is_a_valid_conclusion() {
        let arms = [
            arm(SpecType::None, 40.0, 38.0),
            arm(SpecType::Mtp, 20.0, 19.0),
        ];
        let melhor = arms
            .iter()
            .max_by(|a, b| a.avg_tps.total_cmp(&b.avg_tps))
            .unwrap();
        assert_eq!(melhor.spec, SpecType::None);
        assert!(inconclusivo(&arms));
    }

    /// N-grama ganha em repetição e perde em prosa: reportar só a média
    /// esconderia a única informação que interessa para decidir.
    #[test]
    fn each_kind_of_text_is_reported_on_its_own() {
        let a = arm(SpecType::Ngram, 80.0, 30.0);
        assert_eq!(a.by_prompt.len(), 2);
        assert_eq!(a.by_prompt[0].0, "code");
        assert!(a.by_prompt[0].1 > a.by_prompt[1].1);
    }
}
