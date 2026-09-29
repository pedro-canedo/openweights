//! O que o gateway ajusta no pedido antes de mandá-lo a uma fonte.
//!
//! O Codex conversa como a API Responses da OpenAI. Uma fonte que só traduz
//! isso para chat completions (o llama.cpp) recebe o que a OpenAI aceita e o
//! chat template do modelo pode recusar.

use serde_json::Value;

/// Reúne o que o Codex manda como mensagem de sistema numa só, no começo.
///
/// O Codex leva as instruções em `instructions` e, dentro de `input`, mensagens
/// `developer` (permissões, ambiente, troca de modelo…). O llama.cpp converte
/// todas em `system`, e os chat templates de vários modelos (o Ternary Bonsai 2,
/// o Qwen3.8) exigem UMA mensagem de sistema e só na primeira posição:
/// `raise_exception('System message must be at the beginning.')`, que chega ao
/// Codex como HTTP 500 — "We're currently experiencing high demand".
///
/// As mensagens `developer` e `system` de `input` saem de lá e o texto delas
/// vai para o fim de `instructions`, na ordem em que apareciam. Devolve `true`
/// se mudou alguma coisa. O que não é texto (imagens) é ignorado, e um `input`
/// que não é uma lista fica como está.
pub(crate) fn unificar_sistema(corpo: &mut Value) -> bool {
    let Some(itens) = corpo.get_mut("input").and_then(Value::as_array_mut) else {
        return false;
    };
    let mut textos = Vec::new();
    let antes = itens.len();
    itens.retain(|item| {
        if !e_de_sistema(item) {
            return true;
        }
        if let Some(t) = texto_da_mensagem(item)
            && !t.trim().is_empty()
        {
            textos.push(t);
        }
        false
    });
    if itens.len() == antes {
        return false;
    }
    if !textos.is_empty() {
        let mut todos: Vec<String> = corpo
            .get("instructions")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
            .into_iter()
            .collect();
        todos.extend(textos);
        corpo["instructions"] = Value::String(todos.join("\n\n"));
    }
    true
}

fn e_de_sistema(item: &Value) -> bool {
    matches!(
        item.get("role").and_then(Value::as_str),
        Some("developer" | "system")
    ) && item
        .get("type")
        .and_then(Value::as_str)
        .is_none_or(|t| t == "message")
}

/// O texto de uma mensagem: o `content` inteiro se for texto, ou o `text` de
/// cada parte se for uma lista.
fn texto_da_mensagem(item: &Value) -> Option<String> {
    match item.get("content")? {
        Value::String(s) => Some(s.clone()),
        Value::Array(partes) => {
            let textos: Vec<&str> = partes
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect();
            (!textos.is_empty()).then(|| textos.join("\n"))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn as_mensagens_de_desenvolvedor_vao_para_as_instrucoes_na_ordem() {
        let mut c = json!({
            "model": "m",
            "instructions": "Você é o OwCLI.",
            "input": [
                { "type": "message", "role": "developer", "content": [
                    { "type": "input_text", "text": "permissões" },
                    { "type": "input_text", "text": "ambiente" } ] },
                { "type": "message", "role": "user", "content": [
                    { "type": "input_text", "text": "Olá" } ] },
                { "type": "message", "role": "developer", "content": "trocou de modelo" },
                { "type": "message", "role": "assistant", "content": [
                    { "type": "output_text", "text": "Oi" } ] },
            ],
        });
        assert!(unificar_sistema(&mut c));
        assert_eq!(
            c["instructions"],
            "Você é o OwCLI.\n\npermissões\nambiente\n\ntrocou de modelo"
        );
        let papeis: Vec<&str> = c["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["role"].as_str().unwrap())
            .collect();
        assert_eq!(
            papeis,
            ["user", "assistant"],
            "o resto fica, na mesma ordem"
        );
        assert_eq!(c["model"], "m");
    }

    #[test]
    fn sem_instrucoes_a_primeira_mensagem_de_sistema_vira_as_instrucoes() {
        let mut c = json!({ "input": [
            { "role": "system", "content": "regras" },
            { "type": "message", "role": "user", "content": "oi" } ] });
        assert!(unificar_sistema(&mut c));
        assert_eq!(c["instructions"], "regras");
        assert_eq!(c["input"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn pedido_sem_mensagem_de_sistema_nao_muda() {
        let original = json!({
            "instructions": "x",
            "input": [
                { "type": "message", "role": "user", "content": "oi" },
                { "type": "function_call", "name": "f", "arguments": "{}", "call_id": "1" },
                { "type": "function_call_output", "call_id": "1", "output": "ok" } ] });
        let mut c = original.clone();
        assert!(!unificar_sistema(&mut c));
        assert_eq!(c, original);

        let mut texto = json!({ "input": "oi" });
        assert!(!unificar_sistema(&mut texto));
        assert!(!unificar_sistema(&mut json!({ "model": "m" })));
    }

    #[test]
    fn so_mensagens_saem_e_mensagem_vazia_nao_suja_as_instrucoes() {
        let mut c = json!({
            "instructions": "base",
            "input": [
                // Um item que não é mensagem, mesmo com o papel, fica.
                { "type": "reasoning", "role": "developer", "summary": [] },
                { "type": "message", "role": "developer", "content": [
                    { "type": "input_image", "image_url": "data:…" } ] },
                { "type": "message", "role": "developer", "content": "   " },
                { "type": "message", "role": "user", "content": "oi" } ] });
        assert!(unificar_sistema(&mut c));
        assert_eq!(c["instructions"], "base");
        assert_eq!(c["input"].as_array().unwrap().len(), 2);
        assert_eq!(c["input"][0]["type"], "reasoning");
    }
}
