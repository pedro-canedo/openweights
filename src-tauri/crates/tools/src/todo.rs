//! Plano do run (Focus Chain).
//!
//! Um modelo pequeno esquece o objetivo depois de três ou quatro chamadas de
//! ferramenta. A saída desta ferramenta é reinjetada no prompt a cada passo
//! pelo laço do agente, então ela funciona como memória curta e visível: o
//! modelo escreve o plano, marca o que terminou, e a interface mostra a mesma
//! lista para a pessoa acompanhar.
//!
//! Por isso o formato importa mais que o de costume: a saída é o markdown de
//! checkbox que a UI lê (`- [ ] item` / `- [x] item`). E por isso a entrada é
//! generosa — modelo pequeno manda lista de strings, lista de objetos, ou
//! devolve o próprio markdown que acabou de receber. Todas essas formas são
//! aceitas: recusar por causa do formato desperdiçaria um passo inteiro do run.

use crate::{Tool, ToolContext, ToolError, ToolOutput, ToolResult};
use async_trait::async_trait;
use lr_types::agent::ToolCategory;
use serde_json::{Value, json};
use std::fmt::Write as _;

/// Tarefas guardadas no plano. Acima disso não é plano, é lista de desejos.
const MAX_ITEMS: usize = 30;

/// Teto de cada linha, para o plano não virar um parágrafo.
const MAX_ITEM_CHARS: usize = 200;

/// Mantém o plano do run: as tarefas e o que já foi feito.
pub struct TodoUpdate;

/// Uma tarefa do plano.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TodoItem {
    text: String,
    done: bool,
}

/// Tira o `- [x] ` que o modelo costuma devolver junto quando reescreve o
/// plano a partir do markdown que recebeu.
///
/// Devolve o texto limpo e, quando havia caixa marcada, o estado dela.
fn strip_bullet(raw: &str) -> (String, Option<bool>) {
    let mut text = raw.trim();
    for bullet in ["- ", "* ", "+ "] {
        if let Some(rest) = text.strip_prefix(bullet) {
            text = rest.trim_start();
            break;
        }
    }
    // Também aceita "1. tarefa".
    if let Some(pos) = text.find(". ")
        && pos > 0
        && text[..pos].chars().all(|c| c.is_ascii_digit())
    {
        text = text[pos + 2..].trim_start();
    }

    let done = if let Some(rest) = text
        .strip_prefix("[x]")
        .or_else(|| text.strip_prefix("[X]"))
    {
        text = rest.trim_start();
        Some(true)
    } else if let Some(rest) = text.strip_prefix("[ ]").or_else(|| text.strip_prefix("[]")) {
        text = rest.trim_start();
        Some(false)
    } else {
        None
    };

    (text.trim().to_string(), done)
}

/// Corta a tarefa no limite sem partir UTF-8.
fn clip(text: &str) -> String {
    if text.chars().count() <= MAX_ITEM_CHARS {
        return text.to_string();
    }
    let head: String = text.chars().take(MAX_ITEM_CHARS).collect();
    format!("{head}…")
}

/// Lê o valor `done` de um objeto, aceitando os nomes que os modelos usam.
fn read_done(obj: &Value) -> Option<bool> {
    for key in ["done", "completed", "checked", "finished", "concluido"] {
        match obj.get(key) {
            Some(Value::Bool(b)) => return Some(*b),
            Some(Value::String(s)) => {
                let s = s.trim().to_lowercase();
                return Some(matches!(s.as_str(), "true" | "x" | "sim" | "yes" | "done"));
            }
            _ => {}
        }
    }
    // `status: "done"` / `status: "pending"` também aparece.
    obj.get("status").and_then(Value::as_str).map(|s| {
        matches!(
            s.trim().to_lowercase().as_str(),
            "done" | "completed" | "concluido" | "concluído"
        )
    })
}

/// Converte um item cru (string ou objeto) numa tarefa.
fn parse_item(value: &Value, index: usize) -> ToolResult<TodoItem> {
    match value {
        Value::String(s) => {
            let (text, done) = strip_bullet(s);
            Ok(TodoItem {
                text,
                done: done.unwrap_or(false),
            })
        }
        Value::Object(_) => {
            let raw = ["text", "task", "title", "item", "name", "description"]
                .iter()
                .find_map(|k| value.get(*k).and_then(Value::as_str))
                .ok_or_else(|| {
                    ToolError::InvalidArgs(format!(
                        "o item {} não tem `text`; cada item é {{\"text\": \"...\", \"done\": false}}",
                        index + 1
                    ))
                })?;
            let (text, embedded) = strip_bullet(raw);
            Ok(TodoItem {
                text,
                done: read_done(value).or(embedded).unwrap_or(false),
            })
        }
        other => Err(ToolError::InvalidArgs(format!(
            "o item {} é `{other}`; mande texto ou {{\"text\": \"...\", \"done\": false}}",
            index + 1
        ))),
    }
}

/// Lê `items` em qualquer das formas aceitas.
fn parse_items(args: &Value) -> ToolResult<Vec<TodoItem>> {
    let raw = args.get("items").ok_or_else(|| {
        ToolError::InvalidArgs(
            "faltou `items` — mande a lista de tarefas do plano, na ordem".into(),
        )
    })?;

    let values: Vec<Value> = match raw {
        Value::Array(items) => items.clone(),
        // Markdown inteiro numa string só: cada linha vira uma tarefa.
        Value::String(text) => text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|l| Value::String(l.to_string()))
            .collect(),
        other => {
            return Err(ToolError::InvalidArgs(format!(
                "`items` precisa ser uma lista de tarefas, veio `{other}`"
            )));
        }
    };

    let mut out = Vec::new();
    for (i, value) in values.iter().enumerate() {
        let item = parse_item(value, i)?;
        if item.text.is_empty() {
            continue; // linha em branco no meio do markdown
        }
        out.push(TodoItem {
            text: clip(&item.text),
            done: item.done,
        });
        if out.len() == MAX_ITEMS {
            break;
        }
    }

    if out.is_empty() {
        return Err(ToolError::InvalidArgs(
            "o plano está vazio — mande pelo menos uma tarefa em `items`".into(),
        ));
    }
    Ok(out)
}

/// Monta o markdown que a interface lê e o laço reinjeta no prompt.
fn render(items: &[TodoItem], dropped: usize) -> String {
    let mut md = String::new();
    for item in items {
        let mark = if item.done { 'x' } else { ' ' };
        let _ = writeln!(md, "- [{mark}] {}", item.text);
    }

    let done = items.iter().filter(|i| i.done).count();
    let total = items.len();
    match items.iter().find(|i| !i.done) {
        Some(next) => {
            let _ = write!(md, "\nProgresso: {done}/{total}. Agora: {}.", next.text);
        }
        None => {
            let _ = write!(md, "\nProgresso: {done}/{total}. Plano concluído.");
        }
    }
    if dropped > 0 {
        let _ = write!(
            md,
            "\n[{dropped} tarefas a mais foram descartadas; mantenha o plano com até {MAX_ITEMS} itens]"
        );
    }
    md
}

#[async_trait]
impl Tool for TodoUpdate {
    fn name(&self) -> &str {
        "todo_update"
    }

    fn description(&self) -> &str {
        "Escreve ou atualiza o plano da tarefa. Mande a lista inteira toda vez, marcando `done` \
         no que já terminou. Use no começo, para dividir o trabalho, e depois de cada etapa."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "description": "O plano inteiro, na ordem de execução. Reenvie todas as tarefas a cada atualização, não só as novas.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "text": {
                                "type": "string",
                                "description": "A tarefa em uma frase curta, ex.: ler o README para entender a estrutura."
                            },
                            "done": {
                                "type": "boolean",
                                "description": "true quando a tarefa já foi concluída. Padrão false."
                            }
                        },
                        "required": ["text"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["items"],
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Meta
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let items = parse_items(&args)?;
        let sent = args
            .get("items")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(items.len());
        let dropped = sent.saturating_sub(MAX_ITEMS);
        Ok(ToolOutput::text(render(&items, dropped)).truncated_to(ctx.max_output_bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ToolContext {
        // O plano não toca em arquivo: funciona mesmo sem pasta escolhida.
        ToolContext::new(None, "call-1")
    }

    async fn run(args: Value) -> ToolResult<ToolOutput> {
        TodoUpdate.execute(args, &ctx()).await
    }

    #[tokio::test]
    async fn renders_checkboxes_in_order() {
        let out = run(json!({"items": [
            {"text": "ler o README", "done": true},
            {"text": "escrever o teste"},
            {"text": "rodar a suíte", "done": false}
        ]}))
        .await
        .unwrap();

        let lines: Vec<&str> = out.content.lines().collect();
        assert_eq!(lines[0], "- [x] ler o README");
        assert_eq!(lines[1], "- [ ] escrever o teste");
        assert_eq!(lines[2], "- [ ] rodar a suíte");
    }

    #[tokio::test]
    async fn reports_progress_and_the_current_task() {
        let out = run(json!({"items": [
            {"text": "um", "done": true},
            {"text": "dois"},
            {"text": "três"}
        ]}))
        .await
        .unwrap();
        assert!(out.content.contains("Progresso: 1/3"), "{}", out.content);
        assert!(out.content.contains("Agora: dois."), "{}", out.content);
    }

    #[tokio::test]
    async fn says_when_the_plan_is_finished() {
        let out = run(json!({"items": [{"text": "um", "done": true}]}))
            .await
            .unwrap();
        assert!(out.content.contains("Plano concluído"), "{}", out.content);
    }

    #[tokio::test]
    async fn accepts_a_plain_list_of_strings() {
        let out = run(json!({"items": ["primeira", "segunda"]}))
            .await
            .unwrap();
        assert!(out.content.contains("- [ ] primeira"), "{}", out.content);
        assert!(out.content.contains("- [ ] segunda"), "{}", out.content);
    }

    #[tokio::test]
    async fn accepts_the_markdown_it_produced_before() {
        // O modelo devolve o próprio plano de volta: não pode virar
        // "- [ ] - [x] tarefa".
        let out = run(json!({"items": ["- [x] feito", "- [ ] pendente"]}))
            .await
            .unwrap();
        assert!(out.content.contains("- [x] feito"), "{}", out.content);
        assert!(out.content.contains("- [ ] pendente"), "{}", out.content);
        assert!(!out.content.contains("[ ] - ["), "{}", out.content);
    }

    #[tokio::test]
    async fn accepts_one_markdown_string_for_everything() {
        let out = run(json!({"items": "- [x] um\n- [ ] dois\n"}))
            .await
            .unwrap();
        assert!(out.content.contains("- [x] um"), "{}", out.content);
        assert!(out.content.contains("- [ ] dois"), "{}", out.content);
        assert!(out.content.contains("Progresso: 1/2"), "{}", out.content);
    }

    #[tokio::test]
    async fn accepts_the_other_field_names_models_invent() {
        let out = run(json!({"items": [
            {"task": "com task", "completed": true},
            {"title": "com title", "status": "pending"}
        ]}))
        .await
        .unwrap();
        assert!(out.content.contains("- [x] com task"), "{}", out.content);
        assert!(out.content.contains("- [ ] com title"), "{}", out.content);
    }

    #[tokio::test]
    async fn numbered_lists_lose_the_numbering() {
        let out = run(json!({"items": ["1. primeira", "2. segunda"]}))
            .await
            .unwrap();
        assert!(out.content.contains("- [ ] primeira"), "{}", out.content);
        assert!(out.content.contains("- [ ] segunda"), "{}", out.content);
    }

    #[tokio::test]
    async fn an_empty_plan_is_rejected_with_a_hint() {
        let err = run(json!({"items": []})).await.unwrap_err();
        assert!(err.to_model_message().contains("pelo menos uma tarefa"));

        let err = run(json!({})).await.unwrap_err();
        assert!(err.to_model_message().contains("items"));
    }

    #[tokio::test]
    async fn an_item_without_text_says_the_expected_shape() {
        let err = run(json!({"items": [{"feito": true}]})).await.unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("item 1"), "{msg}");
        assert!(msg.contains("text"), "{msg}");
    }

    #[tokio::test]
    async fn a_very_long_plan_is_cut_with_a_notice() {
        let items: Vec<Value> = (0..MAX_ITEMS + 5)
            .map(|i| json!({"text": format!("tarefa {i}")}))
            .collect();
        let out = run(json!({ "items": items })).await.unwrap();
        let count = out.content.lines().filter(|l| l.starts_with("- [")).count();
        assert_eq!(count, MAX_ITEMS);
        assert!(out.content.contains("descartadas"), "{}", out.content);
    }

    #[tokio::test]
    async fn a_giant_task_text_is_clipped() {
        let long = "a".repeat(MAX_ITEM_CHARS + 50);
        let out = run(json!({"items": [long]})).await.unwrap();
        let line = out.content.lines().next().unwrap();
        assert!(line.ends_with('…'), "{line}");
        assert!(line.chars().count() <= MAX_ITEM_CHARS + 8, "{line}");
    }

    #[tokio::test]
    async fn the_plan_is_a_meta_tool_and_needs_no_workspace() {
        assert_eq!(TodoUpdate.category(), ToolCategory::Meta);
        assert!(TodoUpdate.spec().read_only);
        assert!(TodoUpdate.within_workspace(&json!({}), &ctx()));
    }

    #[test]
    fn the_ui_regex_reads_what_we_write() {
        // Mesmo formato que `parseFocus` no front-end espera.
        let md = render(
            &[
                TodoItem {
                    text: "feito".into(),
                    done: true,
                },
                TodoItem {
                    text: "aberto".into(),
                    done: false,
                },
            ],
            0,
        );
        let parsed: Vec<&str> = md
            .lines()
            .filter(|l| l.starts_with("- [x] ") || l.starts_with("- [ ] "))
            .collect();
        assert_eq!(parsed, vec!["- [x] feito", "- [ ] aberto"]);
    }

    #[test]
    fn strip_bullet_understands_the_usual_shapes() {
        assert_eq!(strip_bullet("tarefa"), ("tarefa".into(), None));
        assert_eq!(strip_bullet("- [x] tarefa"), ("tarefa".into(), Some(true)));
        assert_eq!(strip_bullet("* [ ] tarefa"), ("tarefa".into(), Some(false)));
        assert_eq!(strip_bullet("3. tarefa"), ("tarefa".into(), None));
    }
}
