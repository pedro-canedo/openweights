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

/// Quantos caracteres do começo de uma chamada cortada seguem para o modelo.
const INICIO_MAXIMO: usize = 2000;

/// Garante que todo `function_call` do histórico tenha `arguments` = objeto JSON.
///
/// Quando a geração para no meio de uma chamada de ferramenta (o modelo
/// escrevendo um arquivo grande num heredoc, o limite de saída, o contexto),
/// o llama.cpp devolve o `function_call` com os `arguments` cortados e
/// `status: "completed"`: nenhum sinal de que faltou pedaço. O Codex grava o
/// item e o reenvia a cada requisição; o llama.cpp, ao montar o prompt, faz
/// parse dos `arguments` e responde HTTP 500 (`Failed to parse tool call
/// arguments as JSON`) — o agente mostra "high demand" e a sessão nunca mais
/// anda. Aqui a chamada cortada vira um objeto válido que diz ao modelo o que
/// houve e guarda o começo do que ele escreveu; a ferramenta já falhou no
/// Codex (`failed to parse function arguments`), então nada é executado.
///
/// O texto é sempre o mesmo para o mesmo item (nada de contador ou hora): o
/// histórico continua igual de uma requisição para a outra e o prefixo segue
/// aproveitável no cache do servidor. Devolve quantos itens mudaram.
pub(crate) fn reparar_chamadas(corpo: &mut Value) -> usize {
    let Some(itens) = corpo.get_mut("input").and_then(Value::as_array_mut) else {
        return 0;
    };
    let mut reparados = 0;
    for item in itens.iter_mut() {
        if item.get("type").and_then(Value::as_str) != Some("function_call") {
            continue;
        }
        if let Some(novo) = argumentos_reparados(item.get("arguments")) {
            item["arguments"] = Value::String(novo);
            reparados += 1;
        }
    }
    reparados
}

/// `None`: já serve. `Some(texto)`: o que vai no lugar.
fn argumentos_reparados(atual: Option<&Value>) -> Option<String> {
    match atual {
        None | Some(Value::Null) => Some("{}".into()),
        Some(Value::String(s)) => {
            if serde_json::from_str::<Value>(s).is_ok_and(|v| v.is_object()) {
                None
            } else if s.trim().is_empty() || s.trim() == "null" {
                Some("{}".into())
            } else {
                Some(aviso_de_corte(s))
            }
        }
        // O item da Responses leva uma string; o llama.cpp responde 400 a um
        // objeto. Não deveria acontecer, mas não custa serializar.
        Some(Value::Object(o)) => Some(Value::Object(o.clone()).to_string()),
        Some(outro) => Some(aviso_de_corte(&outro.to_string())),
    }
}

fn aviso_de_corte(bruto: &str) -> String {
    let inicio: String = bruto.chars().take(INICIO_MAXIMO).collect();
    serde_json::json!({
        "_ow_notice": format!(
            "The arguments of this tool call were cut off before they finished ({} characters) and the call was not run. Redo it in smaller pieces (for example, write a large file in several parts).",
            bruto.chars().count()
        ),
        "_ow_start": inicio,
    })
    .to_string()
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

    fn chamada(argumentos: Value) -> Value {
        serde_json::json!({
            "input": [
                { "type": "message", "role": "user", "content": "oi" },
                { "type": "function_call", "call_id": "c1", "name": "shell", "arguments": argumentos },
                { "type": "function_call_output", "call_id": "c1", "output": "falhou" },
            ]
        })
    }

    #[test]
    fn chamada_cortada_vira_objeto_valido_com_o_comeco_e_o_aviso() {
        let cortado = "{\"command\":\"cd /x && cat > js/main.js <<'JSEOF'\\n// DOOM";
        let mut c = chamada(Value::String(cortado.into()));
        assert_eq!(reparar_chamadas(&mut c), 1);
        let novo = c["input"][1]["arguments"].as_str().unwrap();
        let obj: Value = serde_json::from_str(novo).expect("objeto JSON válido");
        assert!(obj["_ow_notice"].as_str().unwrap().contains("cut off"));
        assert_eq!(obj["_ow_start"], cortado);
        // O resto do histórico não é tocado.
        assert_eq!(c["input"][2]["output"], "falhou");
        assert_eq!(c["input"][0]["content"], "oi");
    }

    #[test]
    fn o_que_ja_serve_fica_identico_e_o_vazio_vira_objeto_vazio() {
        for bom in [r#"{"command":"ls"}"#, "{}", r#"{"a":{"b":[1,2]}}"#] {
            let mut c = chamada(Value::String(bom.into()));
            let antes = c.clone();
            assert_eq!(reparar_chamadas(&mut c), 0, "{bom}");
            assert_eq!(c, antes);
        }
        for vazio in ["", "   ", "null"] {
            let mut c = chamada(Value::String(vazio.into()));
            assert_eq!(reparar_chamadas(&mut c), 1, "{vazio:?}");
            assert_eq!(c["input"][1]["arguments"], "{}");
        }
        let mut sem = serde_json::json!({"input":[{"type":"function_call","name":"x"}]});
        assert_eq!(reparar_chamadas(&mut sem), 1);
        assert_eq!(sem["input"][0]["arguments"], "{}");
    }

    #[test]
    fn json_valido_que_nao_e_objeto_tambem_e_reparado() {
        // O llama.cpp aceita o parse e quebra no template (`|items`).
        for nao_objeto in ["[1,2]", "12", "true", "\"abc\""] {
            let mut c = chamada(Value::String(nao_objeto.into()));
            assert_eq!(reparar_chamadas(&mut c), 1, "{nao_objeto}");
            let obj: Value =
                serde_json::from_str(c["input"][1]["arguments"].as_str().unwrap()).unwrap();
            assert!(obj.is_object());
        }
        // Objeto nativo (o llama.cpp dá 400) vira a string dele.
        let mut c = chamada(serde_json::json!({"command": "ls"}));
        assert_eq!(reparar_chamadas(&mut c), 1);
        assert_eq!(c["input"][1]["arguments"], r#"{"command":"ls"}"#);
    }

    #[test]
    fn o_comeco_e_cortado_em_fronteira_de_caractere_e_o_reparo_e_idempotente() {
        let longo = format!("{{\"x\":\"{}", "é".repeat(INICIO_MAXIMO + 500));
        let mut c = chamada(Value::String(longo));
        assert_eq!(reparar_chamadas(&mut c), 1);
        let obj: Value =
            serde_json::from_str(c["input"][1]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(
            obj["_ow_start"].as_str().unwrap().chars().count(),
            INICIO_MAXIMO
        );
        // Reaplicar não muda nada: o prefixo do histórico segue estável.
        let depois = c.clone();
        assert_eq!(reparar_chamadas(&mut c), 0);
        assert_eq!(c, depois);
    }

    #[test]
    fn sem_input_ou_com_outros_tipos_nao_faz_nada() {
        let mut a = serde_json::json!({"model": "m"});
        assert_eq!(reparar_chamadas(&mut a), 0);
        let mut b = serde_json::json!({"input": "texto solto"});
        assert_eq!(reparar_chamadas(&mut b), 0);
        let mut c = serde_json::json!({"input":[{"type":"custom_tool_call","input":"não é json"}]});
        assert_eq!(reparar_chamadas(&mut c), 0);
    }
}
