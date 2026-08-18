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
        run_id: None,
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

/// O deslize que fazia o agente "terminar" com a pasta vazia.
///
/// Um 9B respondeu "Vou criar os três arquivos. Começando com o `app.py`:" e
/// parou. Como texto sem ferramenta significa "acabei", o run encerrou como
/// concluído sem ter escrito nada — o pior desfecho possível, porque anuncia
/// sucesso. O detector precisa pegar a promessa e deixar passar a entrega.
#[test]
fn announcing_an_action_is_not_the_same_as_finishing() {
    for promessa in [
        "Vou criar os três arquivos. Começando com o `app.py`:",
        "Primeiro, vou ler o arquivo de configuração.",
        "Deixa eu conferir a estrutura do projeto",
        "I'll start by creating the server file:",
        "Perfeito. Agora vou escrever o README",
    ] {
        assert!(
            super::anuncio_sem_acao(promessa),
            "devia cutucar: {promessa}"
        );
    }
}

#[test]
fn a_finished_answer_is_left_alone() {
    for entrega in [
        "Criei os três arquivos. O servidor sobe com `python3 app.py`.",
        "Não encontrei nenhum uso dessa função no projeto.",
        "Pronto: o teste passa e o build está limpo.",
        // Termina em bloco de código: mostrou algo, não prometeu.
        "Ficou assim:\n\n```py\nprint(1)\n```",
        // Dois-pontos no MEIO não é promessa.
        "O erro é este: faltava fechar o parêntese na linha 12.",
        "",
    ] {
        assert!(
            !super::anuncio_sem_acao(entrega),
            "não devia cutucar: {entrega}"
        );
    }
}

/// A espiral que o harness respondia "ok" doze vezes.
#[test]
fn a_rewrite_that_keeps_shrinking_is_worth_a_warning() {
    // Primeira escrita nunca é retrocesso — não há com o que comparar.
    assert!(!super::reescrita_encolhendo(1, 0, 1_260));
    // Encolheu para 27% do maior: o conteúdo está sendo cortado.
    assert!(super::reescrita_encolhendo(2, 1_260, 343));
    // Limpeza legítima (85% do tamanho) não vira alarme.
    assert!(!super::reescrita_encolhendo(3, 1_000, 850));
    // Cresceu: é progresso.
    assert!(!super::reescrita_encolhendo(4, 1_000, 1_400));
}

/// A chamada escrita como texto — vista com o qwen2.5-coder-14b, que imprimiu
/// o JSON da ferramenta num bloco de código e encerrou o run no passo 1.
#[test]
fn a_tool_call_typed_as_text_is_not_an_answer() {
    let bloco = "```json\n{\n  \"name\": \"fs_glob\",\n  \"arguments\": {\"pattern\": \"**/*.html\"}\n}\n```";
    assert!(super::chamada_em_texto(bloco));
    assert!(super::chamada_em_texto(
        "<tool_call>{\"name\":\"fs_read\"}</tool_call>"
    ));
    // Sem cerca nenhuma, que foi como o 14B mandou na segunda tentativa.
    assert!(super::chamada_em_texto(
        "{\n  \"name\": \"fs_write\",\n  \"arguments\": {\"path\": \"README.md\"}\n}"
    ));

    // Explicar uma ferramenta em prosa não é chamá-la.
    assert!(!super::chamada_em_texto(
        "A ferramenta `fs_glob` recebe um `pattern` e devolve os caminhos."
    ));
    // Bloco de código comum também não.
    assert!(!super::chamada_em_texto(
        "Ficou assim:\n```py\nprint(1)\n```"
    ));
}

/// Os três empurrões, e o silêncio quando a resposta está pronta.
#[test]
fn only_an_unfinished_turn_gets_pushed() {
    assert_eq!(
        super::cutucada_para("   ", false),
        Some(super::CUTUCADA_VAZIA)
    );
    assert_eq!(
        super::cutucada_para("Vou criar o arquivo agora:", false),
        Some(super::CUTUCADA_ANUNCIO)
    );
    assert_eq!(
        super::cutucada_para("```json\n{\"name\":\"x\",\"arguments\":{}}\n```", false),
        Some(super::CUTUCADA_TEXTO)
    );
    assert_eq!(
        super::cutucada_para("Pronto: criei os três arquivos.", false),
        None
    );
}

/// No Code Mode o empurrão é outro: pedir "use tool call" a um modelo que não
/// consegue emitir tool call o mantém escrevendo o mesmo JSON quebrado. O que
/// destrava é pedir o programa em bloco de código — que o texto sempre
/// aguenta.
#[test]
fn code_mode_asks_for_a_code_block_not_for_a_tool_call() {
    let escrita = "```json\n{\"name\":\"run_code\",\"arguments\":{\"code\":\"say(1)\"}}\n```";
    assert_eq!(
        super::cutucada_para(escrita, true),
        Some(super::CUTUCADA_PROGRAMA)
    );
    // As outras duas continuam iguais: elas não falam de formato.
    assert_eq!(
        super::cutucada_para("   ", true),
        Some(super::CUTUCADA_VAZIA)
    );
}

/// O fallback de arquivo em texto: o formato que o modelo usa quando o JSON
/// da chamada não aguenta o conteúdo (a saída do mercado — patch do Codex,
/// XML do Cline — adaptada ao que o llama.cpp permite).
#[test]
fn a_file_delivered_as_text_is_parsed_whole() {
    let texto = "Aqui está:\n\nARQUIVO: jogo.html\n```html\n<!DOCTYPE html>\n<html lang=\"pt-BR\">\n<body onload=\"init()\">\n</html>\n```\n";
    let (caminho, conteudo) = super::arquivo_em_texto(texto).expect("parse");
    assert_eq!(caminho, "jogo.html");
    // Aspas e atributos chegam INTEIROS — é o ponto do formato.
    assert!(conteudo.contains("lang=\"pt-BR\""));
    assert!(conteudo.starts_with("<!DOCTYPE html>"));
    assert!(conteudo.ends_with("</html>\n"));
}

/// Uma cerca DENTRO do conteúdo não corta o arquivo: vale a última.
#[test]
fn an_inner_fence_does_not_truncate_the_file() {
    let texto = "ARQUIVO: LEIA.md\n```\n# Doc\n\n```js\ncodigo();\n```\n\nfim\n```";
    let (_, conteudo) = super::arquivo_em_texto(texto).expect("parse");
    assert!(conteudo.contains("codigo();"));
    assert!(conteudo.trim_end().ends_with("fim"));
}

/// O que não é o formato não vira escrita: prosa, caminho com espaço,
/// bloco vazio.
#[test]
fn prose_and_malformed_headers_are_not_files() {
    assert!(super::arquivo_em_texto("O ARQUIVO: é importante para o projeto").is_none());
    assert!(super::arquivo_em_texto("ARQUIVO: dois nomes\n```\nx\n```").is_none());
    assert!(super::arquivo_em_texto("ARQUIVO: a.md\n```\n\n```").is_none());
    assert!(super::arquivo_em_texto("sem nada").is_none());
}
