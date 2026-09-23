//! A seção `llm-pi-ai` que o app entrega ao AgenticOw: as rotas
//! `openweights`, `openrouter` e `ninerouter`, cada uma com os modelos e o que
//! o app sabe deles (janela, teto de saída, níveis de raciocínio que o chat
//! template aceita).
//!
//! Antes isto era escrito à força dentro do `settings.yaml` do dsh; agora vai
//! pelo canal de controle (`catalog`) e o plugin do fork aplica pela API de
//! configurações, com validação de schema — uma mudança do upstream quebra no
//! teste do fork, não na máquina de alguém. As regras do modelo padrão moram no
//! plugin (`apps/openweights-plugins/src/catalog.js`, com os mesmos casos de teste).

use serde_json::{Map, Value, json};

/// Nomes das variáveis de ambiente com as chaves. O VALOR vai só no comando
/// `catalog` (memória do Host); o arquivo guarda o nome.
pub const OPENWEIGHTS_KEY_ENV: &str = "OPENWEIGHTS_API_KEY";
pub const OPENROUTER_KEY_ENV: &str = "OPENROUTER_API_KEY";
pub const NINEROUTER_KEY_ENV: &str = "NINEROUTER_API_KEY";

/// Nível de raciocínio oferecido além de `off` a um template que só liga e
/// desliga. Com `qwen-chat-template` o dispatch distingue LIGADO de DESLIGADO;
/// três botões fazendo a mesma coisa seriam mentira.
const NIVEL_LIGADO: &str = "high";

/// Formato dos modelos locais cujo template lê `enable_thinking`: o pi-ai o
/// traduz em `chat_template_kwargs: { enable_thinking, preserve_thinking }`.
const FORMATO_THINKING: &str = "qwen-chat-template";

/// Os níveis que o harness sabe nomear, em ordem de esforço.
const NIVEIS_DO_HARNESS: [&str; 6] = ["minimal", "low", "medium", "high", "xhigh", "max"];

/// Teto de saída para uma janela: metade dela, entre 2 048 e 65 536 tokens.
///
/// Metade porque a saída divide a janela com o prompt de um agente de código;
/// o piso serve ao modelo pequeno (uma janela de 4k não pode render os 32k que
/// o harness assume sozinho); o teto existe porque quem passa disso está num
/// laço, não escrevendo um arquivo grande.
pub fn teto_de_saida(janela: u32) -> u32 {
    (janela / 2).clamp(2_048, 65_536)
}

/// O nome do harness para um nível do template: igual quando o vocabulário
/// coincide; `high` para o que ele não sabe nomear.
fn apelido_do_nivel(nivel: &str) -> &'static str {
    NIVEIS_DO_HARNESS
        .iter()
        .find(|n| **n == nivel)
        .copied()
        .unwrap_or("high")
}

/// Um modelo de uma rota.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Modelo {
    pub id: String,
    pub name: String,
    /// `None` = deixar o padrão do harness valer.
    pub context_window: Option<u32>,
    /// `None` = deixar o padrão do harness valer (rotas remotas: o teto é do provedor).
    pub max_tokens: Option<u32>,
    /// Níveis de esforço que o chat template ACEITA, na ordem dele. Vazio =
    /// o modelo só liga e desliga.
    pub efforts: Vec<String>,
    /// O raciocínio pode ser ligado e desligado por quem chama. Só então a
    /// rota declara `reasoningEfforts` — e o seletor de esforço aparece.
    pub thinking: bool,
}

/// Uma rota do adaptador `llm-pi-ai`, sempre `openai-completions` (o único dos
/// três formatos que fala com um gateway OpenAI-compatible declarado à mão).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rota {
    pub display_name: String,
    /// Base COM `/v1`.
    pub base_url: String,
    /// NOME da variável de ambiente com a credencial.
    pub api_key_env: String,
    pub models: Vec<Modelo>,
}

fn valor_modelo(m: &Modelo) -> Value {
    let mut v = Map::new();
    v.insert("id".into(), m.id.clone().into());
    v.insert("name".into(), m.name.clone().into());
    if let Some(cw) = m.context_window {
        v.insert("contextWindow".into(), cw.into());
    }
    if let Some(mt) = m.max_tokens {
        v.insert("maxTokens".into(), mt.into());
    }
    if m.thinking {
        // `off` sem valor: só ele pode vir vazio, e é assim que o dispatch
        // sabe "não mandar esforço" — que vira `enable_thinking: false`.
        let mut niveis = Map::new();
        niveis.insert("off".into(), Value::Null);
        let compat = if m.efforts.is_empty() {
            niveis.insert(NIVEL_LIGADO.into(), NIVEL_LIGADO.into());
            json!({ "thinkingFormat": FORMATO_THINKING })
        } else {
            // Os níveis DELE, com o nome que o harness valida; o formato
            // genérico é o único que manda o valor escolhido além do
            // liga/desliga. Desligado não manda esforço: o template levanta
            // exceção com um valor que não conhece.
            for nivel in &m.efforts {
                niveis.insert(apelido_do_nivel(nivel).into(), nivel.clone().into());
            }
            json!({
                "thinkingFormat": "chat-template",
                "chatTemplateKwargs": {
                    "enable_thinking": { "$var": "thinking.enabled" },
                    "reasoning_effort": { "$var": "thinking.effort", "omitWhenOff": true },
                },
            })
        };
        v.insert("reasoningEfforts".into(), Value::Object(niveis));
        v.insert("compat".into(), compat);
    }
    Value::Object(v)
}

/// Os níveis que ESTE modelo aceita, com os nomes do harness. Vazio quando
/// ele não raciocina (o único nível dele é `off`).
fn niveis_do_modelo(m: &Modelo) -> Vec<&'static str> {
    if !m.thinking {
        return Vec::new();
    }
    if m.efforts.is_empty() {
        return vec![NIVEL_LIGADO];
    }
    let mut niveis: Vec<&'static str> = Vec::new();
    for e in &m.efforts {
        let nome = apelido_do_nivel(e);
        if !niveis.contains(&nome) {
            niveis.push(nome);
        }
    }
    niveis
}

/// O nível padrão que a ROTA inteira pode prometer, se existir.
///
/// O adaptador valida o `reasoning` da rota contra o modelo de CADA
/// requisição (`UNSUPPORTED_REASONING_EFFORT`), e os modelos de uma rota local
/// não combinam entre si — um instruct só aceita `off`. O padrão só sai quando
/// TODOS aceitam o mesmo nível: `high` de preferência, senão o mais próximo
/// subindo e só então descendo (a aproximação do `clampThinkingLevel` do pi-ai).
fn nivel_padrao_da_rota(modelos: &[Modelo]) -> Option<&'static str> {
    let mut comuns: Option<Vec<&'static str>> = None;
    for modelo in modelos {
        let niveis = niveis_do_modelo(modelo);
        if niveis.is_empty() {
            return None;
        }
        comuns = Some(match comuns {
            None => niveis,
            Some(anteriores) => anteriores
                .into_iter()
                .filter(|n| niveis.contains(n))
                .collect(),
        });
    }
    let comuns = comuns?;
    let alvo = NIVEIS_DO_HARNESS.iter().position(|n| *n == NIVEL_LIGADO)?;
    NIVEIS_DO_HARNESS[alvo..]
        .iter()
        .chain(NIVEIS_DO_HARNESS[..alvo].iter().rev())
        .find(|n| comuns.contains(n))
        .copied()
}

fn valor_rota(r: &Rota) -> Value {
    let mut v = Map::new();
    v.insert("displayName".into(), r.display_name.clone().into());
    v.insert("api".into(), "openai-completions".into());
    v.insert("baseURL".into(), r.base_url.clone().into());
    v.insert("apiKeyEnv".into(), r.api_key_env.clone().into());
    v.insert(
        "models".into(),
        Value::Array(r.models.iter().map(valor_modelo).collect()),
    );
    if let Some(nivel) = nivel_padrao_da_rota(&r.models) {
        v.insert("reasoning".into(), nivel.into());
    }
    Value::Object(v)
}

/// A seção `llm-pi-ai` inteira: `providers` é um DICT keyed pela rota.
pub fn secao_llm_pi_ai(rotas: &[(String, Rota)]) -> Value {
    let providers: Map<String, Value> = rotas
        .iter()
        .map(|(id, r)| (id.clone(), valor_rota(r)))
        .collect();
    json!({ "providers": providers })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modelo(id: &str, ctx: Option<u32>) -> Modelo {
        Modelo {
            id: id.into(),
            name: id.into(),
            context_window: ctx,
            max_tokens: ctx.map(teto_de_saida),
            efforts: Vec::new(),
            thinking: false,
        }
    }

    fn pensante(id: &str, ctx: u32) -> Modelo {
        Modelo {
            thinking: true,
            ..modelo(id, Some(ctx))
        }
    }

    fn local(models: Vec<Modelo>) -> (String, Rota) {
        (
            "openweights".into(),
            Rota {
                display_name: "OpenWeights (local)".into(),
                base_url: "http://127.0.0.1:11711/v1".into(),
                api_key_env: OPENWEIGHTS_KEY_ENV.into(),
                models,
            },
        )
    }

    fn rota<'a>(secao: &'a Value, id: &str) -> &'a Value {
        &secao["providers"][id]
    }

    #[test]
    fn o_teto_de_saida_acompanha_a_janela_entre_piso_e_teto() {
        assert_eq!(teto_de_saida(131_072), 65_536);
        assert_eq!(teto_de_saida(32_768), 16_384);
        assert_eq!(teto_de_saida(4_096), 2_048);
        assert_eq!(teto_de_saida(1_024), 2_048);
        assert_eq!(teto_de_saida(1_000_000), 65_536);
    }

    #[test]
    fn modelo_que_raciocina_declara_niveis_e_formato() {
        let s = secao_llm_pi_ai(&[local(vec![
            pensante("Qwen3.8-27B.gguf", 131_072),
            modelo("Phi-5.gguf", Some(32_768)),
        ])]);
        let r = rota(&s, "openweights");
        assert_eq!(r["api"], "openai-completions");
        assert_eq!(r["apiKeyEnv"], OPENWEIGHTS_KEY_ENV);
        let m0 = &r["models"][0];
        assert_eq!(m0["reasoningEfforts"]["off"], Value::Null);
        assert_eq!(m0["reasoningEfforts"]["high"], "high");
        assert_eq!(m0["compat"]["thinkingFormat"], FORMATO_THINKING);
        assert_eq!(m0["maxTokens"], 65_536);
        // Rota mista: o Phi só aceita `off`; nenhum padrão serve a todos.
        assert!(r.get("reasoning").is_none());
        // Sem interruptor, sem seletor — só o teto.
        let m1 = &r["models"][1];
        assert!(m1.get("reasoningEfforts").is_none() && m1.get("compat").is_none());
        assert_eq!(m1["maxTokens"], 16_384);
    }

    #[test]
    fn modelo_com_niveis_oferece_exatamente_os_do_template() {
        let s = secao_llm_pi_ai(&[local(vec![Modelo {
            efforts: vec!["xhigh".into(), "medium".into(), "low".into()],
            ..pensante("Qwen3.8-27B.gguf", 131_072)
        }])]);
        let m = &rota(&s, "openweights")["models"][0];
        assert_eq!(
            m["reasoningEfforts"],
            json!({ "off": null, "xhigh": "xhigh", "medium": "medium", "low": "low" })
        );
        assert_eq!(m["compat"]["thinkingFormat"], "chat-template");
        assert_eq!(
            m["compat"]["chatTemplateKwargs"]["enable_thinking"]["$var"],
            "thinking.enabled"
        );
        assert_eq!(
            m["compat"]["chatTemplateKwargs"]["reasoning_effort"]["$var"],
            "thinking.effort"
        );
        assert_eq!(
            m["compat"]["chatTemplateKwargs"]["reasoning_effort"]["omitWhenOff"],
            true
        );
    }

    #[test]
    fn template_de_liga_desliga_mantem_o_interruptor_simples() {
        let s = secao_llm_pi_ai(&[local(vec![pensante("Qwen3-8B.gguf", 32_768)])]);
        let m = &rota(&s, "openweights")["models"][0];
        assert_eq!(m["compat"]["thinkingFormat"], FORMATO_THINKING);
        assert!(m["compat"].get("chatTemplateKwargs").is_none());
        assert_eq!(m["reasoningEfforts"]["high"], "high");
    }

    #[test]
    fn rota_com_instruct_nao_promete_nivel_padrao() {
        let s = secao_llm_pi_ai(&[local(vec![
            pensante("Ornith-1.5-9B.gguf", 131_072),
            modelo("Qwen3-Coder-30B-Instruct.gguf", Some(131_072)),
        ])]);
        let r = rota(&s, "openweights");
        assert!(r.get("reasoning").is_none());
        assert!(
            r["models"][0].get("reasoningEfforts").is_some(),
            "o que raciocina perde só o padrão"
        );
    }

    #[test]
    fn rota_de_raciocinadores_fica_no_nivel_que_todos_aceitam() {
        let s = secao_llm_pi_ai(&[local(vec![
            Modelo {
                efforts: vec!["xhigh".into(), "medium".into(), "low".into()],
                ..pensante("Qwen3.8-27B.gguf", 131_072)
            },
            Modelo {
                efforts: vec!["medium".into(), "low".into()],
                ..pensante("Outro.gguf", 32_768)
            },
        ])]);
        assert_eq!(rota(&s, "openweights")["reasoning"], "medium");
    }

    #[test]
    fn rota_toda_de_liga_desliga_mantem_high() {
        let s = secao_llm_pi_ai(&[local(vec![
            pensante("Qwen3-8B.gguf", 32_768),
            pensante("Ornith.gguf", 131_072),
        ])]);
        assert_eq!(rota(&s, "openweights")["reasoning"], "high");
    }

    #[test]
    fn rotas_remotas_ficam_com_os_padroes_do_harness_e_o_dict_e_por_rota() {
        let remoto = |id: &str| Modelo {
            max_tokens: None,
            ..modelo(id, Some(256_000))
        };
        let s = secao_llm_pi_ai(&[
            (
                "openrouter".into(),
                Rota {
                    display_name: "OpenRouter".into(),
                    base_url: "https://openrouter.ai/api/v1".into(),
                    api_key_env: OPENROUTER_KEY_ENV.into(),
                    models: vec![remoto("meta-llama/llama-4-maverick")],
                },
            ),
            (
                "ninerouter".into(),
                Rota {
                    display_name: "9Router".into(),
                    base_url: "http://127.0.0.1:20128/v1".into(),
                    api_key_env: NINEROUTER_KEY_ENV.into(),
                    models: vec![remoto("gcli/grok-4.6")],
                },
            ),
        ]);
        let texto = s.to_string();
        assert!(!texto.contains("maxTokens") && !texto.contains("reasoningEfforts"));
        assert_eq!(
            rota(&s, "openrouter")["baseURL"],
            "https://openrouter.ai/api/v1"
        );
        assert_eq!(rota(&s, "ninerouter")["apiKeyEnv"], NINEROUTER_KEY_ENV);
        assert_eq!(s["providers"].as_object().unwrap().len(), 2);
    }
}
