//! Testes das peças puras do laço.
//!
//! O laço inteiro depende de um llama-server de verdade, então aqui ficam as
//! partes determinísticas: montagem do histórico, deduplicação do pedido,
//! rastro de auditoria e recorte de texto para a interface.

use super::*;

fn msg(id: i64, role: &str, content: &str) -> lr_store::MessageRow {
    lr_store::MessageRow {
        id,
        chat_id: 1,
        role: role.to_string(),
        content: content.to_string(),
        created_at: id,
        tokens_per_sec: None,
        gen_tokens: None,
        gen_ms: None,
    }
}

#[test]
fn history_keeps_only_the_conversation() {
    let rows = vec![
        msg(1, "user", "oi"),
        msg(2, "assistant", "olá"),
        // Papéis internos e mensagens vazias não entram no histórico.
        msg(3, "tool", "resultado antigo"),
        msg(4, "assistant", "   "),
        msg(5, "user", "tudo bem?"),
    ];
    let out = history_from_messages(&rows, 40);
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].role, "user");
    assert_eq!(out[1].role, "assistant");
    assert_eq!(out[2].text().trim(), "tudo bem?");
}

#[test]
fn history_keeps_the_most_recent_when_over_the_limit() {
    let rows: Vec<_> = (1..=10)
        .map(|i| {
            msg(
                i,
                if i % 2 == 0 { "assistant" } else { "user" },
                &format!("m{i}"),
            )
        })
        .collect();
    let out = history_from_messages(&rows, 3);
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].text().trim(), "m8");
    assert_eq!(out[2].text().trim(), "m10");
}

#[test]
fn history_of_an_empty_chat_is_empty() {
    assert!(history_from_messages(&[], 10).is_empty());
}

#[test]
fn append_prompt_does_not_duplicate_the_message_the_ui_already_saved() {
    // A interface grava a mensagem da pessoa antes de chamar o agente.
    let mut msgs = vec![ChatMessage::user("faça X".to_string())];
    append_prompt(&mut msgs, "faça X");
    assert_eq!(msgs.len(), 1, "não pode repetir o mesmo pedido");

    // Espaços em volta não fazem dela uma mensagem diferente.
    append_prompt(&mut msgs, "  faça X  ");
    assert_eq!(msgs.len(), 1);
}

#[test]
fn append_prompt_adds_when_it_is_new() {
    let mut msgs = vec![ChatMessage::user("faça X".to_string())];
    append_prompt(&mut msgs, "agora faça Y");
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[1].text().trim(), "agora faça Y");
}

#[test]
fn append_prompt_ignores_empty_text() {
    let mut msgs = Vec::new();
    append_prompt(&mut msgs, "   ");
    assert!(msgs.is_empty());
}

#[test]
fn append_prompt_after_an_assistant_reply_always_adds() {
    let mut msgs = vec![
        ChatMessage::user("oi".to_string()),
        ChatMessage::assistant("olá".to_string()),
    ];
    append_prompt(&mut msgs, "oi");
    assert_eq!(msgs.len(), 3, "repetir o texto numa nova vez é legítimo");
}

#[test]
fn origin_label_distinguishes_builtin_from_connector() {
    assert_eq!(origin_str(&ToolOrigin::Builtin), "builtin");
    assert_eq!(
        origin_str(&ToolOrigin::Mcp {
            server_id: "github".into()
        }),
        "mcp:github"
    );
}

#[test]
fn args_hash_is_stable_and_sensitive() {
    let a = serde_json::json!({"path": "a.txt"});
    let b = serde_json::json!({"path": "a.txt"});
    let c = serde_json::json!({"path": "b.txt"});
    assert_eq!(args_hash(&a), args_hash(&b));
    assert_ne!(args_hash(&a), args_hash(&c));
    assert_eq!(args_hash(&a).len(), 16);
}

#[test]
fn head_chars_cuts_on_character_boundaries() {
    assert_eq!(head_chars("abc", 10), "abc");
    assert_eq!(head_chars("abcdef", 3), "abc…");
    // Acentos contam como um caractere, não como bytes.
    let s = "ação corrigida";
    let cut = head_chars(s, 4);
    assert_eq!(cut, "ação…");
}
