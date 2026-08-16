//! Capacidades do servidor (`GET /props`).
//!
//! Regra de ouro: **nunca** decidir se um modelo suporta ferramentas pelo
//! nome do arquivo. Quem manda é o `chat_template_caps` que o llama-server
//! deriva do template Jinja do GGUF carregado.
//!
//! O payload de `/props` muda entre builds do llama.cpp (campos entram, saem
//! e mudam de lugar), então o parse aqui é **defensivo por projeto**: tudo é
//! opcional, formatos alternativos são aceitos e nada dá erro — na pior das
//! hipóteses o resultado é conservador (tudo `false`), o que só faz o harness
//! desligar ferramentas, nunca habilitá-las por engano.
//!
//! Formatos já vistos em campo:
//! - `modalities` como objeto (`{"vision": true, "audio": false}`) e como
//!   array de strings (`["vision"]`);
//! - `n_ctx` dentro de `default_generation_settings` (mais comum), dentro de
//!   `default_generation_settings.params` ou solto na raiz.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// O que o template de chat do modelo carregado sabe fazer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatTemplateCaps {
    /// Aceita o campo `tools` do OpenAI (pré-requisito do modo agente).
    pub supports_tools: bool,
    /// Aceita mais de uma tool call por resposta.
    pub supports_parallel_tool_calls: bool,
    /// Aceita mensagens com `role: "system"` (senão o prompt vai no primeiro
    /// turno de usuário).
    pub supports_system_role: bool,
}

/// Recorte de `GET /props` que interessa ao harness.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServerProps {
    /// Caminho do GGUF carregado (útil para casar com a biblioteca local).
    pub model_path: Option<String>,
    pub chat_template_caps: ChatTemplateCaps,
    /// Janela de contexto efetiva do slot — base do orçamento de contexto.
    pub n_ctx: Option<u32>,
    /// Modalidades extras do modelo (`vision`, `audio`), sempre ordenadas.
    pub modalities: Vec<String>,
}

impl ServerProps {
    /// Atalho de legibilidade nos call sites do harness.
    pub fn supports_tools(&self) -> bool {
        self.chat_template_caps.supports_tools
    }

    pub fn has_modality(&self, name: &str) -> bool {
        self.modalities.iter().any(|m| m == name)
    }
}

/// Lê um booleano tolerando `true`/`false`, `1`/`0` e `"true"`/`"false"`.
fn bool_at(v: &Value, key: &str) -> bool {
    match &v[key] {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_i64().is_some_and(|i| i != 0),
        Value::String(s) => s.eq_ignore_ascii_case("true") || s == "1",
        _ => false,
    }
}

/// Lê um inteiro positivo tolerando número ou string numérica.
fn u32_at(v: &Value, key: &str) -> Option<u32> {
    match &v[key] {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_i64().and_then(|i| u64::try_from(i).ok()))
            .and_then(|u| u32::try_from(u).ok()),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn string_at(v: &Value, key: &str) -> Option<String> {
    v[key]
        .as_str()
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

/// Aceita `["vision"]`, `{"vision":true,"audio":false}` e ausência total.
fn parse_modalities(v: &Value) -> Vec<String> {
    let mut out: Vec<String> = match v {
        Value::Array(items) => items
            .iter()
            .filter_map(|i| i.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        Value::Object(map) => map
            .iter()
            .filter(|(_, val)| match val {
                Value::Bool(b) => *b,
                Value::Number(n) => n.as_i64().is_some_and(|i| i != 0),
                Value::Null => false,
                // Objeto/array/string não-vazios contam como "presente".
                other => !other.as_str().is_some_and(str::is_empty),
            })
            .map(|(k, _)| k.clone())
            .collect(),
        _ => Vec::new(),
    };
    out.sort();
    out.dedup();
    out
}

/// Converte o JSON cru de `/props` no recorte que o harness usa.
///
/// Nunca falha: campo ausente ou em formato inesperado vira o padrão
/// conservador (`false`/`None`/vazio).
pub fn parse_props(v: &Value) -> ServerProps {
    let caps_raw = &v["chat_template_caps"];
    let chat_template_caps = ChatTemplateCaps {
        supports_tools: bool_at(caps_raw, "supports_tools"),
        supports_parallel_tool_calls: bool_at(caps_raw, "supports_parallel_tool_calls"),
        supports_system_role: bool_at(caps_raw, "supports_system_role"),
    };

    let defaults = &v["default_generation_settings"];
    let n_ctx = u32_at(defaults, "n_ctx")
        .or_else(|| u32_at(&defaults["params"], "n_ctx"))
        .or_else(|| u32_at(v, "n_ctx"))
        .filter(|n| *n > 0);

    let model_path = string_at(v, "model_path")
        .or_else(|| string_at(v, "model"))
        .or_else(|| string_at(defaults, "model_path"))
        .or_else(|| string_at(defaults, "model"));

    let mut modalities = parse_modalities(&v["modalities"]);
    if modalities.is_empty() {
        modalities = parse_modalities(&v["modality"]);
    }

    ServerProps {
        model_path,
        chat_template_caps,
        n_ctx,
        modalities,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Payload completo de um build recente (modalities como objeto).
    #[test]
    fn parses_full_payload() {
        let v = json!({
            "model_path": "C:/models/qwen/Qwen3-8B-Q4_K_M.gguf",
            "chat_template": "{% for message in messages %}...",
            "chat_template_caps": {
                "supports_tools": true,
                "supports_tool_calls": true,
                "supports_parallel_tool_calls": true,
                "supports_system_role": true,
                "supports_tool_responses": true
            },
            "modalities": { "vision": true, "audio": false },
            "default_generation_settings": { "n_ctx": 8192, "n_predict": -1 },
            "total_slots": 4
        });
        let p = parse_props(&v);
        assert_eq!(
            p.model_path.as_deref(),
            Some("C:/models/qwen/Qwen3-8B-Q4_K_M.gguf")
        );
        assert!(p.supports_tools());
        assert!(p.chat_template_caps.supports_parallel_tool_calls);
        assert!(p.chat_template_caps.supports_system_role);
        assert_eq!(p.n_ctx, Some(8192));
        assert_eq!(p.modalities, vec!["vision".to_string()]);
        assert!(p.has_modality("vision"));
        assert!(!p.has_modality("audio"));
    }

    /// Sem `chat_template_caps`: tudo `false` (conservador — desliga tools).
    #[test]
    fn missing_caps_is_all_false() {
        let v = json!({ "model_path": "/m/model.gguf", "total_slots": 1 });
        let p = parse_props(&v);
        assert!(!p.supports_tools());
        assert!(!p.chat_template_caps.supports_parallel_tool_calls);
        assert!(!p.chat_template_caps.supports_system_role);
        assert_eq!(p.n_ctx, None);
        assert!(p.modalities.is_empty());
    }

    /// `modalities` como array de strings + `n_ctx` na raiz.
    #[test]
    fn parses_modalities_array_and_root_n_ctx() {
        let v = json!({
            "modalities": ["vision", "audio"],
            "n_ctx": 4096,
            "chat_template_caps": { "supports_tools": true }
        });
        let p = parse_props(&v);
        assert_eq!(
            p.modalities,
            vec!["audio".to_string(), "vision".to_string()]
        );
        assert_eq!(p.n_ctx, Some(4096));
        assert!(p.supports_tools());
        // Ausentes dentro de um caps presente continuam false.
        assert!(!p.chat_template_caps.supports_system_role);
    }

    /// Objeto vazio e tipos errados não podem explodir nem inventar caps.
    #[test]
    fn tolerates_garbage() {
        assert_eq!(parse_props(&json!({})), ServerProps::default());
        assert_eq!(parse_props(&json!(null)), ServerProps::default());
        let weird = json!({
            "chat_template_caps": "sim",
            "modalities": 7,
            "default_generation_settings": { "n_ctx": "16384" },
            "model_path": ""
        });
        let p = parse_props(&weird);
        assert!(!p.supports_tools());
        assert!(p.modalities.is_empty());
        assert_eq!(p.n_ctx, Some(16384));
        assert_eq!(p.model_path, None);
    }

    /// `n_ctx` aninhado em `default_generation_settings.params`.
    #[test]
    fn reads_n_ctx_from_nested_params() {
        let v = json!({ "default_generation_settings": { "params": { "n_ctx": 2048 } } });
        assert_eq!(parse_props(&v).n_ctx, Some(2048));
    }

    /// O tipo vai para o frontend em camelCase (espelho em types.ts).
    #[test]
    fn serializes_camel_case_for_the_ui() {
        let p = ServerProps {
            model_path: Some("/m.gguf".into()),
            chat_template_caps: ChatTemplateCaps {
                supports_tools: true,
                ..Default::default()
            },
            n_ctx: Some(1024),
            modalities: vec!["vision".into()],
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["modelPath"], "/m.gguf");
        assert_eq!(v["chatTemplateCaps"]["supportsTools"], true);
        assert_eq!(v["chatTemplateCaps"]["supportsSystemRole"], false);
        assert_eq!(v["nCtx"], 1024);
        assert_eq!(v["modalities"][0], "vision");
    }
}
