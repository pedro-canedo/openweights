//! Consolidação: a arrumação da memória, feita em ocioso.
//!
//! Durante o run o agente vai deixando **episódios** — "corrigi o build",
//! "rodei os testes com pnpm test". Episódio não é memória: é matéria-prima
//! barulhenta, específica de um dia. O que interessa guardar é o que
//! sobrevive à semana ("os testes rodam com pnpm test").
//!
//! Separar uma coisa da outra é justamente o que um modelo de linguagem faz
//! bem — e é caro. Por isso a consolidação:
//! - **nunca roda com run ativo** (quem chama garante): usaria o mesmo
//!   modelo, roubando contexto e tempo no meio do trabalho;
//! - manda um prompt curto e força JSON por schema, porque a saída aqui não
//!   é lida por gente e um modelo pequeno inventa formato se deixarem;
//! - trata resposta ruim como **nada extraído**, nunca como erro fatal: o
//!   pior desfecho aceitável é a memória não crescer nesta rodada.
//!
//! A parte que decide o que vira fato ([`plan`]) é pura: recebe o texto do
//! modelo e devolve fatos curados. Ela é testável sem servidor, e é ela que
//! contém a regra de negócio — [`run`] só faz E/S ao redor.

use crate::facts::{self, CuratedFact};
use crate::{MemoryError, MemoryResult, MemoryStore};
use lr_engine::{ChatMessage, ChatRequest, LlamaClient};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Episódios lidos por rodada. Mais que isso vira um prompt longo — e o
/// resto continua pendente para a próxima.
pub const MAX_EPISODES: u32 = 20;

/// Fatos que uma rodada pode acrescentar. Consolidação que despeja vinte
/// fatos por vez não está resumindo, está copiando.
pub const MAX_NEW_FACTS: usize = 5;

/// Resultado de uma rodada — o que a interface mostra depois de "organizar".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsolidateReport {
    /// Episódios lidos (e marcados como consolidados).
    pub episodes: usize,
    /// Fatos que entraram para a memória.
    pub added: Vec<String>,
    /// Candidatos descartados pela curadoria (repetidos, vazios, longos).
    pub skipped: usize,
}

const SYSTEM: &str = "Você organiza a memória de longo prazo de um agente de programação.\n\
Receba o que aconteceu nas últimas execuções e extraia APENAS fatos duráveis \
sobre o projeto ou sobre a pessoa: ferramentas usadas, comandos que funcionam, \
convenções, decisões e preferências.\n\
Descarte o que é de um dia só: o que foi feito, arquivos tocados, erros já \
corrigidos, números de execução.\n\
Cada fato é uma frase curta, no idioma dos episódios, com até 200 caracteres. \
Sem nada durável, devolva uma lista vazia.";

/// Schema que obriga o formato da resposta. O llama-server aceita
/// `response_format` com `json_schema` e passa a gramática ao sampler, então
/// isto não é sugestão: é o único formato que o modelo consegue emitir.
fn response_format() -> Value {
    json!({
        "type": "json_schema",
        "json_schema": {
            "name": "memoria_consolidada",
            "strict": true,
            "schema": {
                "type": "object",
                "properties": {
                    "facts": {
                        "type": "array",
                        "maxItems": MAX_NEW_FACTS,
                        "items": { "type": "string" }
                    }
                },
                "required": ["facts"],
                "additionalProperties": false
            }
        }
    })
}

/// Monta a requisição da consolidação.
///
/// Os fatos que já sabemos entram no prompt de propósito: pedir ao modelo que
/// não repita é mais barato que deduplicar depois — e o que escapar ainda
/// cai no filtro de [`facts::duplicate_of`].
pub fn consolidation_request(model: &str, episodes: &[String], existing: &[String]) -> ChatRequest {
    let mut user = String::from("Episódios recentes:\n");
    for episode in episodes {
        user.push_str("- ");
        user.push_str(episode.trim());
        user.push('\n');
    }
    if !existing.is_empty() {
        user.push_str("\nJá está na memória (não repita):\n");
        for fact in existing.iter().take(30) {
            user.push_str("- ");
            user.push_str(fact.trim());
            user.push('\n');
        }
    }
    user.push_str("\nResponda com o JSON de fatos duráveis.");

    let mut req = ChatRequest::new(
        model,
        vec![ChatMessage::system(SYSTEM), ChatMessage::user(user)],
    );
    // Extração, não criação: temperatura baixa e resposta curta.
    req.temperature = Some(0.2);
    req.max_tokens = Some(400);
    req.with_extra("response_format", response_format())
}

/// Tira cercas de código (` ```json ... ``` `) que o modelo insiste em pôr
/// mesmo com schema forçado.
fn strip_fences(raw: &str) -> String {
    let text = raw.trim();
    let Some(rest) = text.strip_prefix("```") else {
        return text.to_string();
    };
    // A primeira linha depois da cerca pode ser a linguagem ("json").
    let body = match rest.split_once('\n') {
        Some((_lang, body)) => body,
        None => rest,
    };
    body.trim_end().trim_end_matches("```").trim().to_string()
}

/// Lê um item de fato, aceitando string ou objeto (modelo pequeno embrulha).
fn item_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Object(_) => ["fact", "fato", "content", "text", "summary"]
            .iter()
            .find_map(|k| value.get(*k).and_then(Value::as_str))
            .map(str::to_string),
        _ => None,
    }
}

/// Extrai a lista de fatos de um JSON já parseado.
fn facts_from_json(value: &Value) -> Option<Vec<String>> {
    let array = match value {
        Value::Array(items) => items.clone(),
        Value::Object(_) => ["facts", "fatos", "memory", "memoria", "items"]
            .iter()
            .find_map(|k| value.get(*k).and_then(Value::as_array))
            .cloned()?,
        _ => return None,
    };
    Some(array.iter().filter_map(item_text).collect())
}

/// Tenta parsear o maior trecho JSON dentro do texto.
fn first_json(text: &str) -> Option<Value> {
    for (open, close) in [('{', '}'), ('[', ']')] {
        if let (Some(start), Some(end)) = (text.find(open), text.rfind(close))
            && end > start
            && let Ok(v) = serde_json::from_str::<Value>(&text[start..=end])
        {
            return Some(v);
        }
    }
    None
}

/// Lê a resposta do modelo em qualquer das formas plausíveis.
///
/// Nunca falha: resposta ilegível vira lista vazia. Um erro aqui pararia a
/// arrumação para sempre por causa de uma geração ruim.
pub fn parse_reply(raw: &str) -> Vec<String> {
    let text = strip_fences(raw);
    if let Some(value) = first_json(&text)
        && let Some(facts) = facts_from_json(&value)
    {
        return facts.into_iter().take(MAX_NEW_FACTS).collect();
    }
    // Último recurso: o modelo respondeu uma lista em markdown.
    text.lines()
        .map(str::trim)
        .filter_map(|l| l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")))
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .take(MAX_NEW_FACTS)
        .collect()
}

/// Parte pura: resposta do modelo + o que já sabemos → fatos aprovados.
///
/// Devolve também quantos candidatos morreram no caminho, que é o número que
/// a interface mostra como "descartados".
pub fn plan(reply: &str, existing: &[String], has_workspace: bool) -> (Vec<CuratedFact>, usize) {
    let candidates = parse_reply(reply);
    let mut known: Vec<String> = existing.to_vec();
    let mut approved = Vec::new();
    let mut skipped = 0usize;

    for candidate in candidates {
        match facts::curate(&candidate, &known, None, has_workspace) {
            Ok(fact) => {
                known.push(fact.content.clone());
                approved.push(fact);
            }
            Err(e) => {
                log::debug!("fato descartado na consolidação ({e}): {candidate}");
                skipped += 1;
            }
        }
    }
    (approved, skipped)
}

/// Roda uma arrumação completa: lê episódios pendentes, pergunta ao modelo,
/// grava o que sobreviveu e marca os episódios como consolidados.
///
/// Só marca os episódios quando o modelo respondeu — falha de rede deixa tudo
/// pendente para a próxima tentativa. Resposta vazia ou ilegível marca do
/// mesmo jeito: os episódios foram vistos e não havia nada durável neles.
pub async fn run(
    memory: &MemoryStore,
    client: &LlamaClient,
    model: &str,
) -> MemoryResult<ConsolidateReport> {
    let workspace_key = memory.workspace().map(|p| p.to_string_lossy().into_owned());
    let episodes = memory
        .store()
        .pending_episodes(workspace_key.as_deref(), MAX_EPISODES)?;
    if episodes.is_empty() {
        return Ok(ConsolidateReport::default());
    }

    let summaries: Vec<String> = episodes.iter().map(|e| e.summary.clone()).collect();
    let existing = memory.fact_texts()?;
    let request = consolidation_request(model, &summaries, &existing);

    let outcome = client
        .complete_once(&request)
        .await
        .map_err(MemoryError::Engine)?;

    let (approved, skipped) = plan(&outcome.content, &existing, memory.workspace().is_some());
    let mut added = Vec::new();
    for fact in &approved {
        match memory.save_curated(fact, None) {
            Ok(saved) => added.push(saved.content),
            // Um fato que não grava não pode abortar a rodada inteira.
            Err(e) => log::warn!("fato não gravado na consolidação: {e}"),
        }
    }

    let ids: Vec<i64> = episodes.iter().map(|e| e.id).collect();
    memory.store().mark_episodes_consolidated(&ids)?;

    Ok(ConsolidateReport {
        episodes: ids.len(),
        added,
        skipped,
    })
}

#[cfg(test)]
mod tests;
