//! Reescrita do corpo de um `chat/completions` com o nível que o Jev decidiu.
//!
//! Pura e sem rede — é o que os testes cobrem de perto, porque é aqui que
//! um valor errado vira uma exceção do chat template dentro do llama.cpp
//! (o Qwen3.8 recusa `reasoning_effort: high`; o gpt-oss não tem
//! `enable_thinking`). As regras:
//!
//! - `chat_template_kwargs` é MESCLADO, nunca substituído: o dsh manda
//!   `preserve_thinking` junto, e apagá-lo mudaria o comportamento do
//!   modelo por baixo do harness.
//! - `enable_thinking` é escrito quando o template tem o interruptor, ou
//!   quando a chave já veio (quem mandou sabia do que falava). Modelo sem
//!   capacidade conhecida também recebe: templates que não leem a chave a
//!   ignoram.
//! - Nível (`reasoning_effort`) só é escrito com um valor que o template
//!   ACEITA; para modelo só-interruptor, o nível é removido do
//!   `chat_template_kwargs` quando o raciocínio desliga — o mesmo
//!   `omitWhenOff` que o dsh aplica.
//! - `stream`, `model`, `messages` e amostragem não são tocados.

use lr_providers::{CapacidadeModelo, NivelRaciocinio};
use serde_json::Value;

/// Reescreve `corpo` no lugar. Devolve `true` quando algo mudou.
pub fn reescrever(
    corpo: &mut Value,
    nivel: NivelRaciocinio,
    cap: Option<&CapacidadeModelo>,
) -> bool {
    let Some(obj) = corpo.as_object_mut() else {
        return false;
    };
    let antes = obj.clone();
    let ligado = nivel != NivelRaciocinio::Nenhum;
    let nivel_template = cap.and_then(|c| c.nivel_do_template(nivel));
    let so_interruptor = cap.is_some_and(|c| c.thinking_toggle && c.efforts.is_empty());

    // --- chat_template_kwargs -------------------------------------------
    let kwargs_existente = obj
        .get("chat_template_kwargs")
        .and_then(Value::as_object)
        .cloned();
    let mut kwargs = kwargs_existente.clone().unwrap_or_default();
    let tinha_toggle = kwargs.contains_key("enable_thinking");
    let escreve_toggle = tinha_toggle || cap.is_none_or(|c| c.thinking_toggle);
    if escreve_toggle {
        kwargs.insert("enable_thinking".into(), Value::Bool(ligado));
    }
    if kwargs.contains_key("reasoning_effort") {
        // Estilo dsh para templates com níveis.
        match (&nivel_template, ligado || !so_interruptor) {
            (Some(v), true) => {
                kwargs.insert("reasoning_effort".into(), Value::String(v.clone()));
            }
            _ => {
                kwargs.remove("reasoning_effort");
            }
        }
    } else if let Some(v) = &nivel_template {
        // Template com níveis que o chamador não conhecia: informa o nível
        // dentro dos kwargs, que é onde o template o lê.
        if ligado || !so_interruptor {
            kwargs.insert("reasoning_effort".into(), Value::String(v.clone()));
        }
    }
    if !kwargs.is_empty() {
        obj.insert("chat_template_kwargs".into(), Value::Object(kwargs));
    }

    // --- reasoning_effort no topo (estilo chat do app / OpenAI) -----------
    if obj.contains_key("reasoning_effort") {
        let valor = nivel_template
            .clone()
            .unwrap_or_else(|| nivel_openai(nivel).to_string());
        obj.insert("reasoning_effort".into(), Value::String(valor));
    }

    *obj != antes
}

/// O vocabulário OpenAI para os nossos três níveis, usado no
/// `reasoning_effort` do topo quando o template não declarou os seus.
fn nivel_openai(nivel: NivelRaciocinio) -> &'static str {
    match nivel {
        NivelRaciocinio::Nenhum => "low",
        NivelRaciocinio::Medio => "medium",
        NivelRaciocinio::Alto => "high",
    }
}

/// Resumo de um `messages` do OpenAI para o Jev: papel + texto, imagens
/// viram `[imagem]`, partes desconhecidas somem.
pub fn resumir_mensagens(corpo: &Value) -> Vec<lr_providers::MensagemResumida> {
    let Some(msgs) = corpo.get("messages").and_then(Value::as_array) else {
        return Vec::new();
    };
    msgs.iter()
        .filter_map(|m| {
            let papel = m.get("role")?.as_str()?.to_string();
            let texto = match m.get("content") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Array(partes)) => partes
                    .iter()
                    .filter_map(|p| match p.get("type").and_then(Value::as_str) {
                        Some("text") => p.get("text").and_then(Value::as_str).map(str::to_string),
                        Some("image_url") | Some("input_image") => Some("[imagem]".to_string()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => String::new(),
            };
            Some(lr_providers::MensagemResumida::nova(papel, texto))
        })
        .collect()
}

/// Uma chave estável para "esta conversa": o primeiro e o último texto do
/// usuário. Continuações de um laço de agente (a mesma conversa com mais
/// turnos `assistant`/`tool` no fim) batem na mesma chave e reaproveitam a
/// decisão, em vez de pagar o Jev a cada ida ao modelo.
pub fn chave_da_conversa(mensagens: &[lr_providers::MensagemResumida]) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let primeiro = mensagens.iter().find(|m| m.papel == "user")?;
    let ultimo = mensagens
        .iter()
        .rev()
        .find(|m| m.papel == "user")
        .unwrap_or(primeiro);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    primeiro.texto.hash(&mut h);
    ultimo.texto.hash(&mut h);
    Some(h.finish())
}

pub fn tem_ferramentas(corpo: &Value) -> bool {
    corpo
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|t| !t.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn qwen3() -> CapacidadeModelo {
        CapacidadeModelo {
            thinking_toggle: true,
            efforts: vec![],
        }
    }

    fn qwen38() -> CapacidadeModelo {
        CapacidadeModelo {
            thinking_toggle: true,
            efforts: vec!["xhigh".into(), "medium".into(), "low".into()],
        }
    }

    fn gpt_oss() -> CapacidadeModelo {
        CapacidadeModelo {
            thinking_toggle: false,
            efforts: vec!["low".into(), "medium".into(), "high".into()],
        }
    }

    /// O corpo que o chat do app manda (`applyEffort` em llama.ts).
    fn corpo_do_chat() -> Value {
        json!({
            "model": "qwen3", "stream": true, "temperature": 0.8,
            "messages": [{"role": "user", "content": "oi"}],
            "reasoning_effort": "high",
            "chat_template_kwargs": {"enable_thinking": true}
        })
    }

    #[test]
    fn the_chat_body_is_rewritten_on_both_fields() {
        let mut c = corpo_do_chat();
        assert!(reescrever(&mut c, NivelRaciocinio::Nenhum, Some(&qwen3())));
        assert_eq!(c["reasoning_effort"], "low");
        assert_eq!(c["chat_template_kwargs"]["enable_thinking"], false);
        // O resto fica exatamente como veio.
        assert_eq!(c["stream"], true);
        assert_eq!(c["temperature"], 0.8);
        assert_eq!(c["messages"][0]["content"], "oi");
    }

    /// dsh + Qwen3 (só interruptor): `{enable_thinking, preserve_thinking}`.
    #[test]
    fn a_dsh_toggle_only_body_keeps_preserve_thinking_and_gets_no_level() {
        let mut c = json!({
            "model": "qwen3", "messages": [],
            "chat_template_kwargs": {"enable_thinking": false, "preserve_thinking": true}
        });
        assert!(reescrever(&mut c, NivelRaciocinio::Alto, Some(&qwen3())));
        assert_eq!(c["chat_template_kwargs"]["enable_thinking"], true);
        assert_eq!(c["chat_template_kwargs"]["preserve_thinking"], true);
        assert!(c["chat_template_kwargs"].get("reasoning_effort").is_none());
        assert!(
            c.get("reasoning_effort").is_none(),
            "não inventa o campo do topo"
        );
    }

    /// dsh + gpt-oss (níveis, sem interruptor): `{reasoning_effort}` nos
    /// kwargs. `Nenhum` vira o MENOR nível, nunca "off".
    #[test]
    fn a_dsh_levels_body_only_receives_values_the_template_accepts() {
        let mut c = json!({
            "model": "gpt-oss", "messages": [],
            "chat_template_kwargs": {"reasoning_effort": "medium"}
        });
        assert!(reescrever(&mut c, NivelRaciocinio::Alto, Some(&gpt_oss())));
        assert_eq!(c["chat_template_kwargs"]["reasoning_effort"], "high");
        assert!(
            c["chat_template_kwargs"].get("enable_thinking").is_none(),
            "sem interruptor no template"
        );

        assert!(reescrever(
            &mut c,
            NivelRaciocinio::Nenhum,
            Some(&gpt_oss())
        ));
        assert_eq!(c["chat_template_kwargs"]["reasoning_effort"], "low");
    }

    /// Qwen3.8 declara xhigh/medium/low e NENHUM `high`: um `high` aqui
    /// seria exceção no template. `Alto` tem de virar `xhigh`.
    #[test]
    fn a_template_without_high_gets_its_own_top_level() {
        let mut c = json!({
            "model": "qwen3.8", "messages": [],
            "reasoning_effort": "high",
            "chat_template_kwargs": {"enable_thinking": true, "reasoning_effort": "medium"}
        });
        assert!(reescrever(&mut c, NivelRaciocinio::Alto, Some(&qwen38())));
        assert_eq!(c["reasoning_effort"], "xhigh");
        assert_eq!(c["chat_template_kwargs"]["reasoning_effort"], "xhigh");
        assert_eq!(c["chat_template_kwargs"]["enable_thinking"], true);
    }

    /// Só-interruptor com um nível que o chamador colocou por engano:
    /// desligar remove o nível (o `omitWhenOff` do dsh).
    #[test]
    fn turning_off_a_toggle_only_model_removes_a_stray_level() {
        let mut c = json!({
            "model": "qwen3", "messages": [],
            "chat_template_kwargs": {"enable_thinking": true, "reasoning_effort": "high"}
        });
        assert!(reescrever(&mut c, NivelRaciocinio::Nenhum, Some(&qwen3())));
        assert_eq!(c["chat_template_kwargs"]["enable_thinking"], false);
        assert!(c["chat_template_kwargs"].get("reasoning_effort").is_none());
    }

    /// Sem capacidade conhecida: escreve o interruptor (inofensivo para quem
    /// não o lê) e traduz o nível do topo se ele já existia.
    #[test]
    fn an_unknown_model_gets_the_toggle_and_a_translated_top_level() {
        let mut c = json!({"model": "x", "messages": [], "reasoning_effort": "high"});
        assert!(reescrever(&mut c, NivelRaciocinio::Medio, None));
        assert_eq!(c["reasoning_effort"], "medium");
        assert_eq!(c["chat_template_kwargs"]["enable_thinking"], true);
    }

    #[test]
    fn a_body_without_any_reasoning_field_is_still_given_the_toggle() {
        let mut c = json!({"model": "qwen3", "messages": [{"role":"user","content":"oi"}]});
        assert!(reescrever(&mut c, NivelRaciocinio::Nenhum, Some(&qwen3())));
        assert_eq!(c["chat_template_kwargs"]["enable_thinking"], false);
        assert!(c.get("reasoning_effort").is_none());
    }

    #[test]
    fn a_non_object_body_is_left_alone() {
        let mut c = json!([1, 2]);
        assert!(!reescrever(&mut c, NivelRaciocinio::Alto, None));
        assert_eq!(c, json!([1, 2]));
    }

    #[test]
    fn a_rewrite_that_changes_nothing_says_so() {
        let mut c = json!({"chat_template_kwargs": {"enable_thinking": true}});
        assert!(!reescrever(&mut c, NivelRaciocinio::Alto, Some(&qwen3())));
    }

    // ------------------------------------------------------------ resumo ---

    #[test]
    fn messages_are_summarised_with_images_replaced() {
        let c = json!({"messages": [
            {"role": "system", "content": "regras"},
            {"role": "user", "content": [{"type": "text", "text": "o que é?"}, {"type": "image_url", "image_url": {"url": "data:..."}}]},
            {"role": "assistant", "content": null, "tool_calls": [{}]},
            {"role": "tool", "content": "saída"}
        ]});
        let r = resumir_mensagens(&c);
        assert_eq!(r.len(), 4);
        assert_eq!(r[1].papel, "user");
        assert_eq!(r[1].texto, "o que é?\n[imagem]");
        assert_eq!(r[2].texto, "");
        assert_eq!(r[3].papel, "tool");
    }

    #[test]
    fn the_conversation_key_survives_agent_continuations() {
        let base = resumir_mensagens(&json!({"messages": [
            {"role": "user", "content": "conserte o bug"},
        ]}));
        let continuacao = resumir_mensagens(&json!({"messages": [
            {"role": "user", "content": "conserte o bug"},
            {"role": "assistant", "content": "lendo"},
            {"role": "tool", "content": "conteúdo"},
        ]}));
        let outra = resumir_mensagens(&json!({"messages": [
            {"role": "user", "content": "conserte o bug"},
            {"role": "assistant", "content": "pronto"},
            {"role": "user", "content": "agora os testes"},
        ]}));
        assert_eq!(chave_da_conversa(&base), chave_da_conversa(&continuacao));
        assert_ne!(chave_da_conversa(&base), chave_da_conversa(&outra));
        assert_eq!(chave_da_conversa(&[]), None);
    }

    #[test]
    fn tools_are_detected_only_when_non_empty() {
        assert!(tem_ferramentas(&json!({"tools": [{"type": "function"}]})));
        assert!(!tem_ferramentas(&json!({"tools": []})));
        assert!(!tem_ferramentas(&json!({})));
    }
}
