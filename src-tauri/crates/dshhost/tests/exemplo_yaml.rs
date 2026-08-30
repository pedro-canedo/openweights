//! Gerador temporário do YAML de exemplo para o relatório da entrega.
//! (arquivo descartável — não faz asserções)

use lr_dshhost::settings::{ModeloDsh, ProvedorDsh, merge_settings};

#[test]
fn imprime_exemplo() {
    let provedores = vec![
        (
            "openweights".to_string(),
            ProvedorDsh {
                display_name: "OpenWeights (local)".to_string(),
                base_url: "http://127.0.0.1:11711/v1".to_string(),
                api_key_env: "OPENWEIGHTS_API_KEY".to_string(),
                models: vec![
                    ModeloDsh {
                        id: "Qwen3.6-27B-MTP.gguf".to_string(),
                        name: "Qwen3.6-27B-MTP.gguf".to_string(),
                        context_window: Some(131_072),
                        max_tokens: Some(65_536),
                        thinking: true,
                    },
                    ModeloDsh {
                        id: "Phi-5-mini.gguf".to_string(),
                        name: "Phi-5-mini.gguf".to_string(),
                        context_window: Some(32_768),
                        max_tokens: Some(16_384),
                        thinking: false,
                    },
                ],
            },
        ),
        (
            "openrouter".to_string(),
            ProvedorDsh {
                display_name: "OpenRouter".to_string(),
                base_url: "https://openrouter.ai/api/v1".to_string(),
                api_key_env: "OPENROUTER_API_KEY".to_string(),
                models: vec![ModeloDsh {
                    id: "meta-llama/llama-4-maverick".to_string(),
                    name: "meta-llama/llama-4-maverick".to_string(),
                    context_window: Some(1_048_576),
                    max_tokens: None,
                    thinking: false,
                }],
            },
        ),
        (
            "ninerouter".to_string(),
            ProvedorDsh {
                display_name: "9Router".to_string(),
                base_url: "http://127.0.0.1:20128/v1".to_string(),
                api_key_env: "NINEROUTER_API_KEY".to_string(),
                models: vec![ModeloDsh {
                    id: "gcli/grok-4.6".to_string(),
                    name: "gcli/grok-4.6".to_string(),
                    context_window: Some(256_000),
                    max_tokens: None,
                    thinking: false,
                }],
            },
        ),
    ];
    let existente = "ui-theme:\n  mode: dark\nlocale:\n  language: pt-BR\nagent-default-model:\n  provider: openai\n  model: Qwen3.8-27B-UD-IQ4_XS.gguf\n";
    println!(
        "=== INICIO ===\n{}=== FIM ===",
        merge_settings(existente, &provedores)
    );
}
