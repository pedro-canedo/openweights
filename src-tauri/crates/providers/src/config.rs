//! Configuração dos provedores, gravada como JSON único no setting
//! `providers.config`.
//!
//! Setting e não tabela: são três provedores fixos, não um CRUD de N
//! conectores arbitrários. O precedente é o `web.config` do `lr_webtools` —
//! mesma forma, mesmo parse defensivo, mesma escrita atômica. Quando (e se) o
//! usuário puder cadastrar endpoints OpenAI-compatíveis quaisquer, aí sim a
//! tabela se paga.

use crate::model_ref::ProviderId;
use serde::{Deserialize, Serialize};

/// Onde o OpenRouter atende. Constante e não configurável: apontar isto para
/// outro host seria outro provedor, não uma opção deste.
pub const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";

/// Raiz da API do OpenRouter, sem o `/v1`.
///
/// É esta que vai no `ResolvedEndpoint`: quem fala com o modelo (o
/// `streamChat` da UI e o `lr_engine`) monta `{base}/v1/chat/completions`
/// sozinho, como já fazia com o llama-server. Devolver a URL com `/v1` aqui
/// produzia `/v1/v1/chat/completions` — 404 em todo provedor remoto.
pub const OPENROUTER_API_ROOT: &str = "https://openrouter.ai/api";

/// Porta padrão do 9router. Cai numa efêmera se estiver ocupada.
pub const NINEROUTER_DEFAULT_PORT: u16 = 20128;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProvidersConfig {
    pub open_router: OpenRouterConfig,
    pub nine_router: NineRouterConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OpenRouterConfig {
    pub enabled: bool,
    pub api_key: String,
    /// Modelos fixados pelo usuário. O catálogo tem centenas de entradas;
    /// despejar tudo no seletor do chat o tornaria inútil, então lá aparecem
    /// só estes.
    pub favorites: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NineRouterConfig {
    pub installed: bool,
    /// Versão instalada, para a UI saber quando o pino do app avançou.
    pub version: String,
    pub port: u16,
    /// Senha do primeiro boot do painel. Só vale enquanto o 9router não
    /// gravou o hash dela; depois disso trocá-la aqui não muda nada.
    pub password: String,
    pub jwt_secret: String,
    /// Chave de API do 9router que o app usa para falar com o `/v1` dele.
    ///
    /// O painel tem um interruptor "Require API key"; com ele ligado, uma
    /// requisição sem `Authorization` responde 401 e a conversa morre com
    /// "Missing API key". Guardar a chave aqui faz esse interruptor deixar de
    /// afetar o app — quem a obtém é o `ninerouter_start`, pela API interna
    /// do próprio 9router.
    pub api_key: String,
}

impl Default for NineRouterConfig {
    fn default() -> Self {
        Self {
            installed: false,
            version: String::new(),
            port: NINEROUTER_DEFAULT_PORT,
            password: String::new(),
            jwt_secret: String::new(),
            api_key: String::new(),
        }
    }
}

/// Endereço pronto para uso: para onde mandar e o que anexar no cabeçalho.
///
/// `base_url` é a RAIZ do serviço, sem `/v1` e sem barra no fim — mesmo
/// contrato do endereço do llama-server local. Quem chama acrescenta o
/// caminho (`/v1/chat/completions`, `/v1/models`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedEndpoint {
    pub provider: ProviderId,
    pub base_url: String,
    pub api_key: Option<String>,
    pub headers: Vec<(String, String)>,
}

/// Por que um provedor não pode ser usado agora.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EndpointError {
    #[error("o provedor {0} está desligado")]
    Disabled(&'static str),
    #[error("falta a chave de API do {0}")]
    MissingKey(&'static str),
    #[error("o 9router ainda não está instalado")]
    NotInstalled,
    #[error("o servidor local não está no ar")]
    LocalDown,
}

impl ProvidersConfig {
    /// Lê o JSON gravado. Conteúdo estragado cai nos padrões, em silêncio —
    /// igual ao `WebConfig`: o setting pode ter sido editado à mão, e derrubar
    /// o app por causa disso seria pior que ignorar.
    pub fn from_json_or_default(raw: Option<&str>) -> Self {
        raw.and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// Resolve para onde mandar a conversa.
    ///
    /// `local_base` é a URL do llama-server quando ele já está no ar; `None`
    /// significa que ninguém o subiu ainda — quem chama decide se sobe.
    pub fn resolve(
        &self,
        provider: ProviderId,
        local_base: Option<&str>,
    ) -> Result<ResolvedEndpoint, EndpointError> {
        match provider {
            ProviderId::Local => Ok(ResolvedEndpoint {
                provider,
                base_url: local_base.ok_or(EndpointError::LocalDown)?.to_string(),
                api_key: None,
                headers: Vec::new(),
            }),

            ProviderId::OpenRouter => {
                let cfg = &self.open_router;
                if !cfg.enabled {
                    return Err(EndpointError::Disabled("OpenRouter"));
                }
                let chave = cfg.api_key.trim();
                if chave.is_empty() {
                    return Err(EndpointError::MissingKey("OpenRouter"));
                }
                Ok(ResolvedEndpoint {
                    provider,
                    base_url: OPENROUTER_API_ROOT.to_string(),
                    api_key: Some(chave.to_string()),
                    // O OpenRouter usa estes dois para atribuir o tráfego ao
                    // app nos rankings públicos. São opcionais para ele e
                    // identificam o projeto, não a pessoa.
                    headers: vec![
                        (
                            "HTTP-Referer".to_string(),
                            "https://github.com/pedro-canedo/openweights".to_string(),
                        ),
                        ("X-Title".to_string(), "OpenWeights".to_string()),
                    ],
                })
            }

            ProviderId::NineRouter => {
                let cfg = &self.nine_router;
                if !cfg.installed {
                    return Err(EndpointError::NotInstalled);
                }
                let chave = cfg.api_key.trim();
                Ok(ResolvedEndpoint {
                    provider,
                    // Sempre loopback: o 9router guarda credenciais OAuth de
                    // contas de terceiros e não pode escutar na rede.
                    base_url: format!("http://127.0.0.1:{}", cfg.port),
                    // Vazia é legítimo: com "Require API key" desligado o
                    // 9router atende sem `Authorization`, e mandar cabeçalho
                    // vazio seria pior que não mandar nada.
                    api_key: (!chave.is_empty()).then(|| chave.to_string()),
                    headers: Vec::new(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broken_json_falls_back_to_the_defaults() {
        let cfg = ProvidersConfig::from_json_or_default(Some("{ isto não é json"));
        assert!(!cfg.open_router.enabled);
        assert_eq!(cfg.nine_router.port, NINEROUTER_DEFAULT_PORT);
    }

    #[test]
    fn an_absent_setting_falls_back_to_the_defaults() {
        let cfg = ProvidersConfig::from_json_or_default(None);
        assert!(cfg.open_router.api_key.is_empty());
    }

    /// Um JSON gravado por uma versão anterior não pode perder os campos que
    /// ela não conhecia.
    #[test]
    fn a_config_without_the_openrouter_block_uses_the_defaults() {
        let cfg = ProvidersConfig::from_json_or_default(Some(r#"{"nineRouter":{"port":30000}}"#));
        assert_eq!(cfg.nine_router.port, 30000);
        assert!(!cfg.open_router.enabled);
    }

    #[test]
    fn the_config_round_trips_through_json() {
        let mut cfg = ProvidersConfig::default();
        cfg.open_router.enabled = true;
        cfg.open_router.api_key = "sk-or-v1-abc".to_string();
        cfg.open_router.favorites = vec!["x/y".to_string()];
        let lido = ProvidersConfig::from_json_or_default(Some(&cfg.to_json()));
        assert!(lido.open_router.enabled);
        assert_eq!(lido.open_router.api_key, "sk-or-v1-abc");
        assert_eq!(lido.open_router.favorites, vec!["x/y".to_string()]);
    }

    #[test]
    fn the_openrouter_endpoint_carries_the_referer_and_title_headers() {
        let mut cfg = ProvidersConfig::default();
        cfg.open_router.enabled = true;
        cfg.open_router.api_key = "sk-or-v1-abc".to_string();

        let ep = cfg.resolve(ProviderId::OpenRouter, None).unwrap();
        assert_eq!(ep.base_url, OPENROUTER_API_ROOT);
        assert_eq!(ep.api_key.as_deref(), Some("sk-or-v1-abc"));
        assert!(ep.headers.iter().any(|(k, _)| k == "HTTP-Referer"));
        assert!(
            ep.headers
                .iter()
                .any(|(k, v)| k == "X-Title" && v == "OpenWeights")
        );
    }

    #[test]
    fn a_provider_without_a_key_does_not_resolve() {
        let mut cfg = ProvidersConfig::default();
        cfg.open_router.enabled = true;
        assert_eq!(
            cfg.resolve(ProviderId::OpenRouter, None),
            Err(EndpointError::MissingKey("OpenRouter"))
        );
    }

    /// Espaço em branco colado junto da chave não conta como chave.
    #[test]
    fn a_whitespace_only_key_counts_as_missing() {
        let mut cfg = ProvidersConfig::default();
        cfg.open_router.enabled = true;
        cfg.open_router.api_key = "   ".to_string();
        assert!(cfg.resolve(ProviderId::OpenRouter, None).is_err());
    }

    #[test]
    fn a_disabled_provider_does_not_resolve() {
        let mut cfg = ProvidersConfig::default();
        cfg.open_router.api_key = "sk-or-v1-abc".to_string();
        assert_eq!(
            cfg.resolve(ProviderId::OpenRouter, None),
            Err(EndpointError::Disabled("OpenRouter"))
        );
    }

    #[test]
    fn the_nine_router_endpoint_always_binds_to_loopback() {
        let mut cfg = ProvidersConfig::default();
        cfg.nine_router.installed = true;
        cfg.nine_router.port = 20500;
        let ep = cfg.resolve(ProviderId::NineRouter, None).unwrap();
        assert_eq!(ep.base_url, "http://127.0.0.1:20500");
    }

    /// O `/v1` é de quem chama, não do endereço. Já custou um 404 em todo
    /// provedor remoto: o `streamChat` monta `{base}/v1/chat/completions`.
    #[test]
    fn no_resolved_endpoint_carries_the_version_segment() {
        let mut cfg = ProvidersConfig::default();
        cfg.open_router.enabled = true;
        cfg.open_router.api_key = "sk-or-v1-abc".to_string();
        cfg.nine_router.installed = true;
        for id in [ProviderId::OpenRouter, ProviderId::NineRouter] {
            let ep = cfg.resolve(id, None).unwrap();
            assert!(
                !ep.base_url.ends_with("/v1") && !ep.base_url.ends_with('/'),
                "{id:?} devolveu {}",
                ep.base_url
            );
        }
        assert_eq!(OPENROUTER_BASE_URL, format!("{OPENROUTER_API_ROOT}/v1"));
    }

    #[test]
    fn an_uninstalled_nine_router_does_not_resolve() {
        let cfg = ProvidersConfig::default();
        assert_eq!(
            cfg.resolve(ProviderId::NineRouter, None),
            Err(EndpointError::NotInstalled)
        );
    }

    #[test]
    fn the_local_provider_uses_the_running_server_url() {
        let cfg = ProvidersConfig::default();
        let ep = cfg
            .resolve(ProviderId::Local, Some("http://127.0.0.1:11711"))
            .unwrap();
        assert_eq!(ep.base_url, "http://127.0.0.1:11711");
        assert!(ep.api_key.is_none());
    }

    #[test]
    fn the_local_provider_reports_when_the_server_is_down() {
        let cfg = ProvidersConfig::default();
        assert_eq!(
            cfg.resolve(ProviderId::Local, None),
            Err(EndpointError::LocalDown)
        );
    }
}
