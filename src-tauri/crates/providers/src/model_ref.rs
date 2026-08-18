//! Como o provedor viaja junto do nome do modelo.
//!
//! `chats.model_id`, `runs.model` e `ChatParams.model` são strings livres em
//! todo o app. Em vez de migrar três lugares (e todo `paramsJson` já gravado)
//! para carregar um campo novo, o provedor vira um prefixo do próprio nome:
//! `openrouter:anthropic/claude-sonnet-4.5`. Conversas antigas, que não têm
//! prefixo nenhum, continuam sendo lidas como modelo local — que é o que são.
//!
//! A regra de corte é onde mora a sutileza: cortamos no PRIMEIRO `:` e só
//! aceitamos um prefixo conhecido. Ids do OpenRouter têm `/` e podem ter `:`
//! (`anthropic/claude-sonnet-4.5:beta`), e nomes de GGUF também aparecem com
//! `:` — tratar qualquer coisa antes de um `:` como provedor quebraria os
//! dois casos.

use serde::{Deserialize, Serialize};

/// De onde vem o modelo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Hash)]
#[serde(rename_all = "camelCase")]
pub enum ProviderId {
    /// llama-server em Router mode, na própria máquina.
    #[default]
    Local,
    /// 9router local (instalado e supervisionado pelo app).
    NineRouter,
    /// OpenRouter (remoto, por chave de API).
    OpenRouter,
}

impl ProviderId {
    /// Prefixo usado nas referências de modelo. O local não tem prefixo: é o
    /// caso padrão e manter o nome cru é o que preserva as conversas antigas.
    pub const fn prefix(self) -> Option<&'static str> {
        match self {
            Self::Local => None,
            Self::NineRouter => Some("9router"),
            Self::OpenRouter => Some("openrouter"),
        }
    }

    fn from_prefix(s: &str) -> Option<Self> {
        match s {
            "9router" => Some(Self::NineRouter),
            "openrouter" => Some(Self::OpenRouter),
            _ => None,
        }
    }

    /// Identificador estável usado na configuração e na UI.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::NineRouter => "9router",
            Self::OpenRouter => "openrouter",
        }
    }
}

/// Um modelo, e de onde ele vem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRef {
    pub provider: ProviderId,
    pub model: String,
}

impl ModelRef {
    pub fn local(model: impl Into<String>) -> Self {
        Self {
            provider: ProviderId::Local,
            model: model.into(),
        }
    }

    /// Lê uma referência crua vinda do banco ou da UI.
    ///
    /// Nunca falha: prefixo desconhecido é nome local, porque é exatamente
    /// assim que está gravada toda conversa anterior a esta funcionalidade.
    pub fn parse(raw: &str) -> Self {
        let raw = raw.trim();
        if let Some((prefixo, resto)) = raw.split_once(':')
            && let Some(provider) = ProviderId::from_prefix(prefixo)
            && !resto.is_empty()
        {
            return Self {
                provider,
                model: resto.to_string(),
            };
        }
        Self::local(raw)
    }

    /// Serializa de volta. O local devolve o nome cru — nada no banco muda de
    /// forma por causa desta funcionalidade.
    pub fn to_ref_string(&self) -> String {
        match self.provider.prefix() {
            Some(p) => format!("{p}:{}", self.model),
            None => self.model.clone(),
        }
    }

    pub fn is_local(&self) -> bool {
        self.provider == ProviderId::Local
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_model_name_is_a_local_model() {
        let r = ModelRef::parse("Qwen3-8B-Q4_K_M.gguf");
        assert_eq!(r.provider, ProviderId::Local);
        assert_eq!(r.model, "Qwen3-8B-Q4_K_M.gguf");
    }

    #[test]
    fn an_openrouter_reference_keeps_the_slash_and_the_tag() {
        let r = ModelRef::parse("openrouter:anthropic/claude-sonnet-4.5:beta");
        assert_eq!(r.provider, ProviderId::OpenRouter);
        // Só o PRIMEIRO `:` é separador — o `:beta` pertence ao id do modelo.
        assert_eq!(r.model, "anthropic/claude-sonnet-4.5:beta");
    }

    #[test]
    fn a_nine_router_reference_is_recognised() {
        let r = ModelRef::parse("9router:gpt-5");
        assert_eq!(r.provider, ProviderId::NineRouter);
        assert_eq!(r.model, "gpt-5");
    }

    /// Um nome de arquivo com `:` não pode ser confundido com prefixo.
    #[test]
    fn a_gguf_name_with_a_colon_is_not_mistaken_for_a_prefix() {
        let r = ModelRef::parse("modelo:v2.gguf");
        assert_eq!(r.provider, ProviderId::Local);
        assert_eq!(r.model, "modelo:v2.gguf");
    }

    #[test]
    fn an_unknown_prefix_is_treated_as_a_local_name() {
        let r = ModelRef::parse("ollama:llama3");
        assert_eq!(r.provider, ProviderId::Local);
        assert_eq!(r.model, "ollama:llama3");
    }

    /// Prefixo sem modelo não é referência de provedor.
    #[test]
    fn a_prefix_without_a_model_stays_local() {
        let r = ModelRef::parse("openrouter:");
        assert_eq!(r.provider, ProviderId::Local);
        assert_eq!(r.model, "openrouter:");
    }

    #[test]
    fn round_tripping_a_local_reference_adds_no_prefix() {
        let cru = "Qwen3-8B-Q4_K_M.gguf";
        assert_eq!(ModelRef::parse(cru).to_ref_string(), cru);
    }

    #[test]
    fn round_tripping_a_remote_reference_is_stable() {
        let cru = "openrouter:anthropic/claude-sonnet-4.5:beta";
        assert_eq!(ModelRef::parse(cru).to_ref_string(), cru);
    }

    #[test]
    fn surrounding_whitespace_is_ignored() {
        assert_eq!(
            ModelRef::parse("  openrouter:x/y  ").model,
            "x/y".to_string()
        );
    }
}
