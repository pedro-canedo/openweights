//! Cliente REST do OpenRouter — catálogo, saldo e teste de chave.
//!
//! O streaming da conversa NÃO passa por aqui: o webview fala direto com o
//! endpoint, como já faz com o llama-server local (`src/lib/llama.ts`). Este
//! módulo cobre só o que a tela de provedores precisa saber antes de conversar.

use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::config::OPENROUTER_BASE_URL;

const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("falha de rede: {0}")]
    Network(#[from] reqwest::Error),
    #[error("chave de API recusada")]
    Unauthorized,
    #[error("{0}")]
    Other(String),
}

/// Um modelo do catálogo, reduzido ao que a UI mostra.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenRouterModel {
    pub id: String,
    pub name: String,
    pub context_length: Option<u32>,
    /// Preço por token (não por milhão) — é como o OpenRouter publica.
    pub prompt_price: Option<f64>,
    pub completion_price: Option<f64>,
    pub is_free: bool,
    pub supports_tools: bool,
}

/// Saldo e limites da chave.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KeyInfo {
    pub label: String,
    pub usage: f64,
    /// `None` = sem teto definido na chave.
    pub limit: Option<f64>,
    pub is_free_tier: bool,
}

// --- espelhos do JSON da API ---------------------------------------------

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<RawModel>,
}

#[derive(Deserialize)]
struct RawModel {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    context_length: Option<u32>,
    #[serde(default)]
    pricing: Option<RawPricing>,
    #[serde(default)]
    supported_parameters: Vec<String>,
}

#[derive(Deserialize)]
struct RawPricing {
    /// Vem como string ("0.0000045") para não perder precisão em float.
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    completion: Option<String>,
}

#[derive(Deserialize)]
struct KeyResponse {
    data: RawKey,
}

#[derive(Deserialize, Default)]
struct RawKey {
    #[serde(default)]
    label: String,
    #[serde(default)]
    usage: f64,
    #[serde(default)]
    limit: Option<f64>,
    #[serde(default)]
    is_free_tier: bool,
}

// --- conversões puras (é o que os testes cobrem) --------------------------

/// Um preço "0" (em qualquer grafia decimal) significa modelo gratuito.
fn preco(raw: Option<&str>) -> Option<f64> {
    raw.and_then(|s| s.trim().parse::<f64>().ok())
}

impl RawModel {
    fn into_model(self) -> OpenRouterModel {
        let (p, c) = match &self.pricing {
            Some(pr) => (preco(pr.prompt.as_deref()), preco(pr.completion.as_deref())),
            None => (None, None),
        };
        // Grátis é o par de preços zerado. Um preço ausente NÃO conta como
        // zero: sem informação, o honesto é não anunciar "grátis".
        let is_free = p == Some(0.0) && c == Some(0.0);
        let supports_tools = self.supported_parameters.iter().any(|s| s == "tools");
        let name = if self.name.is_empty() {
            self.id.clone()
        } else {
            self.name.clone()
        };
        OpenRouterModel {
            id: self.id,
            name,
            context_length: self.context_length,
            prompt_price: p,
            completion_price: c,
            is_free,
            supports_tools,
        }
    }
}

// --- chamadas -------------------------------------------------------------

fn cliente() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(concat!("OpenWeights/", env!("CARGO_PKG_VERSION")))
        .build()
}

/// Catálogo completo. **Não exige chave** — dá para mostrar os modelos e os
/// preços antes de a pessoa se cadastrar.
pub async fn list_models() -> Result<Vec<OpenRouterModel>, ProviderError> {
    let resp = cliente()?
        .get(format!("{OPENROUTER_BASE_URL}/models"))
        .send()
        .await?
        .error_for_status()?
        .json::<ModelsResponse>()
        .await?;
    let mut modelos: Vec<_> = resp.data.into_iter().map(RawModel::into_model).collect();
    // Ordem estável e útil: os gratuitos primeiro, depois por nome.
    modelos.sort_by(|a, b| b.is_free.cmp(&a.is_free).then_with(|| a.name.cmp(&b.name)));
    Ok(modelos)
}

/// Saldo e limite da chave. Serve de "testar conexão": é o endpoint mais
/// barato que exige autenticação.
pub async fn key_info(api_key: &str) -> Result<KeyInfo, ProviderError> {
    let resp = cliente()?
        .get(format!("{OPENROUTER_BASE_URL}/key"))
        .bearer_auth(api_key)
        .send()
        .await?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(ProviderError::Unauthorized);
    }
    let raw = resp.error_for_status()?.json::<KeyResponse>().await?.data;
    Ok(KeyInfo {
        label: raw.label,
        usage: raw.usage,
        limit: raw.limit,
        is_free_tier: raw.is_free_tier,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(id: &str, prompt: Option<&str>, completion: Option<&str>, params: &[&str]) -> RawModel {
        RawModel {
            id: id.to_string(),
            name: String::new(),
            context_length: Some(8192),
            pricing: Some(RawPricing {
                prompt: prompt.map(str::to_string),
                completion: completion.map(str::to_string),
            }),
            supported_parameters: params.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn a_model_priced_at_zero_is_reported_as_free() {
        let m = raw("a/b:free", Some("0"), Some("0"), &[]).into_model();
        assert!(m.is_free);
    }

    #[test]
    fn a_paid_model_is_not_reported_as_free() {
        let m = raw("a/b", Some("0.0000045"), Some("0.000032"), &[]).into_model();
        assert!(!m.is_free);
        assert_eq!(m.prompt_price, Some(0.0000045));
    }

    /// Sem informação de preço, não anunciamos "grátis".
    #[test]
    fn a_model_without_pricing_is_not_reported_as_free() {
        let m = RawModel {
            id: "a/b".to_string(),
            name: String::new(),
            context_length: None,
            pricing: None,
            supported_parameters: vec![],
        }
        .into_model();
        assert!(!m.is_free);
        assert_eq!(m.prompt_price, None);
    }

    #[test]
    fn tool_support_is_read_from_the_supported_parameters() {
        assert!(
            raw("a/b", None, None, &["tools", "temperature"])
                .into_model()
                .supports_tools
        );
        assert!(
            !raw("a/b", None, None, &["temperature"])
                .into_model()
                .supports_tools
        );
    }

    /// Sem `name`, o seletor mostraria uma linha vazia.
    #[test]
    fn a_model_without_a_name_falls_back_to_its_id() {
        let m = raw("vendor/modelo", None, None, &[]).into_model();
        assert_eq!(m.name, "vendor/modelo");
    }

    #[test]
    fn a_price_in_scientific_notation_is_parsed() {
        assert_eq!(preco(Some("4.5e-7")), Some(0.00000045));
    }

    #[test]
    fn a_malformed_price_is_ignored() {
        assert_eq!(preco(Some("grátis")), None);
    }

    /// Teste live — roda só com `--ignored`, como o do runtime.
    #[tokio::test]
    #[ignore = "rede: consulta o catálogo público do OpenRouter"]
    async fn live_catalogue_is_not_empty_and_has_free_models_first() {
        let modelos = list_models().await.expect("catálogo indisponível");
        assert!(modelos.len() > 50);
        if let Some(primeiro) = modelos.first() {
            assert!(primeiro.is_free, "gratuitos devem vir primeiro");
        }
    }
}
