//! Busca híbrida: BM25 (FTS5) + vizinhos mais próximos (vec0), fundidos por
//! Reciprocal Rank Fusion.
//!
//! Por que fundir por POSIÇÃO e não por nota: BM25 devolve um número negativo
//! sem escala fixa (depende do tamanho do acervo e da raridade dos termos) e o
//! vec0 devolve distância L2 em [0, 2]. Somar ou ponderar essas duas notas é
//! comparar régua com balança — o resultado muda de sentido a cada acervo. O
//! RRF joga a nota fora e usa só a ordem:
//!
//! ```text
//! score(d) = Σ  1 / (k + posição de d na lista i)      k = 60
//! ```
//!
//! O `k = 60` é o valor do artigo original (Cormack et al., 2009) e continua
//! sendo o padrão de fato: alto o bastante para o 1º lugar não atropelar o
//! resto, baixo o bastante para a cauda longa não pesar.
//!
//! Cada lista contribui de forma independente, então um trecho que aparece nas
//! duas sobe naturalmente — sem precisar de peso ajustado à mão.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::{EmbedConfig, RagError, schema, workspace_key};

/// Constante do RRF. Ver o artigo original; não é palpite.
pub const RRF_K: f64 = 60.0;

/// Quantos candidatos cada lista traz, em múltiplos do limite pedido. A fusão
/// precisa de folga: o melhor resultado do vetor pode estar em 12º no texto.
const OVER_FETCH: usize = 5;

/// Teto de candidatos por lista — evita varrer o índice inteiro num `limit`
/// grande.
const MAX_CANDIDATES: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HitSource {
    /// Só o casamento textual achou.
    Text,
    /// Só o vetor achou (o caso que justifica o RAG existir).
    Vector,
    /// As duas listas concordaram.
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub chunk_id: i64,
    /// Caminho relativo ao projeto, com `/`.
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    /// Texto do trecho (o chamador corta no tamanho que couber).
    pub snippet: String,
    /// Nota do RRF. Serve para ordenar, não para interpretar.
    pub score: f64,
    pub source: HitSource,
}

impl SearchHit {
    /// `caminho:linha` — o formato que o modelo deve citar.
    pub fn citation(&self) -> String {
        if self.start_line == self.end_line {
            format!("{}:{}", self.path, self.start_line)
        } else {
            format!("{}:{}-{}", self.path, self.start_line, self.end_line)
        }
    }
}

/// Funde listas ordenadas (melhor primeiro) por Reciprocal Rank Fusion.
///
/// Empate é resolvido pelo id, para a mesma consulta devolver sempre a mesma
/// ordem — resultado que dança entre execuções destrói a confiança do usuário.
pub fn rrf_fuse(rankings: &[Vec<i64>], k: f64, limit: usize) -> Vec<(i64, f64)> {
    let mut scores: HashMap<i64, f64> = HashMap::new();
    for list in rankings {
        for (pos, id) in list.iter().enumerate() {
            *scores.entry(*id).or_insert(0.0) += 1.0 / (k + (pos as f64 + 1.0));
        }
    }
    let mut out: Vec<(i64, f64)> = scores.into_iter().collect();
    out.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    out.truncate(limit);
    out
}

/// Palavras vazias de pt-BR e inglês. Elas só atrapalham na alternativa `OR`:
/// "prazo OR de OR duracao" casa com qualquer arquivo que tenha um "de" —
/// ou seja, com o projeto inteiro. Na alternativa `AND` seriam inofensivas,
/// mas tirar dos dois lados mantém as duas listas comparáveis.
const STOPWORDS: &[&str] = &[
    // Português
    "as", "os", "um", "uma", "uns", "umas", "de", "do", "da", "dos", "das", "em", "no", "na", "nos",
    "nas", "por", "pelo", "pela", "para", "com", "sem", "que", "ou", "se", "ao", "aos", "como",
    "onde", "quando", "qual", "quais", "isso", "isto", "este", "esta", "esse", "essa", "ser",
    "sao", "foi", "tem", "mais", "menos", "muito", "meu", "minha", "seu", "sua", "nao",
    // Inglês
    "the", "an", "of", "in", "on", "at", "to", "for", "and", "or", "is", "are", "was", "were", "be",
    "it", "this", "that", "with", "from", "by", "as", "we", "you", "do", "does", "how", "what",
    "where", "when", "why", "which", "not",
];

/// Traduz o que o usuário digitou para uma expressão MATCH do FTS5.
///
/// A entrada é texto livre e pode ter `"`, `*`, `NEAR`, parênteses — tudo isso
/// é sintaxe do FTS5 e um erro de sintaxe derrubaria a busca inteira. Então
/// nada é interpretado: os termos são extraídos e cada um vai entre aspas.
pub fn fts_query(user: &str, conjunctive: bool) -> Option<String> {
    let mut terms: Vec<String> = Vec::new();
    let mut all: Vec<String> = Vec::new();
    for raw in user.split(|c: char| !c.is_alphanumeric()) {
        let t = raw.trim().to_lowercase();
        if t.len() < 2 || all.contains(&t) {
            continue;
        }
        all.push(t.clone());
        if !STOPWORDS.contains(&t.as_str()) {
            terms.push(t);
        }
        if all.len() >= 12 {
            break;
        }
    }
    // Consulta feita só de palavras vazias ("o que é isso"): melhor procurar
    // por elas do que não procurar por nada.
    if terms.is_empty() {
        terms = all;
    }
    if terms.is_empty() {
        return None;
    }
    let joined = terms
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(if conjunctive { " AND " } else { " OR " });
    Some(joined)
}

/// Lista do BM25 (melhor primeiro).
fn fts_search(
    conn: &Connection,
    ws: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<i64>, RagError> {
    // Primeiro com AND (precisão). Sem resultado, cai para OR (cobertura) —
    // é melhor devolver algo parcialmente relevante do que nada.
    for conjunctive in [true, false] {
        let Some(expr) = fts_query(query, conjunctive) else {
            return Ok(Vec::new());
        };
        let ids = run_fts(conn, ws, &expr, limit)?;
        if !ids.is_empty() {
            return Ok(ids);
        }
    }
    Ok(Vec::new())
}

fn run_fts(conn: &Connection, ws: &str, expr: &str, limit: usize) -> Result<Vec<i64>, RagError> {
    let mut stmt = conn.prepare(
        "SELECT c.id
         FROM rag_chunks_fts f
         JOIN rag_chunks c ON c.id = f.rowid
         WHERE rag_chunks_fts MATCH ?1 AND c.workspace = ?2
         ORDER BY bm25(rag_chunks_fts, 1.0, 0.4)
         LIMIT ?3",
    )?;
    // Expressão malformada não pode derrubar a busca: o vetor ainda responde.
    let rows = match stmt.query_map(params![expr, ws, limit as i64], |r| r.get::<_, i64>(0)) {
        Ok(rows) => rows,
        Err(e) => {
            log::debug!("consulta FTS5 recusada ({e}); seguindo sem a parte textual");
            return Ok(Vec::new());
        }
    };
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Lista dos vizinhos mais próximos (mais parecido primeiro).
fn vec_search(
    conn: &Connection,
    ws: &str,
    query_vec: &[f32],
    limit: usize,
) -> Result<Vec<i64>, RagError> {
    let mut stmt = conn.prepare(
        "SELECT chunk_id FROM rag_vec
         WHERE embedding MATCH ?1 AND workspace = ?2 AND k = ?3
         ORDER BY distance",
    )?;
    let blob = crate::vec_blob(query_vec);
    let rows = stmt.query_map(params![blob, ws, limit as i64], |r| r.get::<_, i64>(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Busca híbrida no índice do projeto.
///
/// Sem modelo de embedding (ou com o servidor fora do ar) a parte vetorial
/// simplesmente não entra na fusão: o RRF de uma lista só devolve a ordem do
/// BM25. É o degrade gracioso — pior, mas útil.
pub async fn search(
    db_path: &Path,
    workspace: &Path,
    query: &str,
    limit: usize,
    embed: EmbedConfig,
) -> Result<Vec<SearchHit>, RagError> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let limit = limit.clamp(1, 50);
    let candidates = (limit * OVER_FETCH).min(MAX_CANDIDATES);
    let ws = workspace_key(workspace);

    let conn = crate::vec_init::open_rag_connection(db_path)?;
    schema::ensure_schema(&conn)?;

    let text_ids = fts_search(&conn, &ws, query, candidates)?;

    // O vetor da consulta só serve se vier do MESMO modelo dos vetores
    // gravados — misturar espaços vetoriais devolve lixo com cara de acerto.
    //
    // O resultado sai para uma variável ANTES do `match`: um `&Connection`
    // temporário vivo dentro do `match` atravessaria o `await` e tornaria este
    // futuro não-`Send` (o `Connection` do rusqlite não é `Sync`).
    let embedder = embeddable_model(&conn, &embed);
    let query_vec = match embedder {
        Some((client, model)) => match client.embeddings(&model, &[query.to_string()]).await {
            Ok(mut v) if !v.is_empty() => {
                let mut first = v.swap_remove(0);
                crate::normalize(&mut first);
                Some(first)
            }
            Ok(_) => None,
            Err(e) => {
                log::warn!("embedding da consulta falhou ({e}); busca só textual");
                None
            }
        },
        None => None,
    };

    let vector_ids = match &query_vec {
        Some(v) if schema::table_exists(&conn, "rag_vec")? => {
            vec_search(&conn, &ws, v, candidates).unwrap_or_default()
        }
        _ => Vec::new(),
    };

    let fused = rrf_fuse(&[text_ids.clone(), vector_ids.clone()], RRF_K, limit);
    if fused.is_empty() {
        return Ok(Vec::new());
    }

    let in_text: std::collections::HashSet<i64> = text_ids.into_iter().collect();
    let in_vector: std::collections::HashSet<i64> = vector_ids.into_iter().collect();
    let rows = load_chunks(&conn, fused.iter().map(|(id, _)| *id))?;

    let mut hits = Vec::with_capacity(fused.len());
    for (id, score) in fused {
        let Some((path, start, end, content)) = rows.get(&id) else {
            continue;
        };
        hits.push(SearchHit {
            chunk_id: id,
            path: path.clone(),
            start_line: *start,
            end_line: *end,
            snippet: content.clone(),
            score,
            source: match (in_text.contains(&id), in_vector.contains(&id)) {
                (true, true) => HitSource::Both,
                (false, true) => HitSource::Vector,
                _ => HitSource::Text,
            },
        });
    }
    Ok(hits)
}

/// Cliente de embedding, mas só se o modelo configurado for o mesmo que gerou
/// os vetores guardados.
fn embeddable_model(
    conn: &Connection,
    embed: &EmbedConfig,
) -> Option<(lr_engine::LlamaClient, String)> {
    let stored = schema::meta_get(conn, "embed_model")?;
    let (client, model) = embed.client()?;
    if stored != model {
        log::debug!("índice gravado com `{stored}`, configurado `{model}`: busca só textual");
        return None;
    }
    Some((client, model))
}

type ChunkRow = (String, u32, u32, String);

fn load_chunks(
    conn: &Connection,
    ids: impl Iterator<Item = i64>,
) -> Result<HashMap<i64, ChunkRow>, RagError> {
    let mut stmt =
        conn.prepare("SELECT path, start_line, end_line, content FROM rag_chunks WHERE id = ?1")?;
    let mut out = HashMap::new();
    for id in ids {
        if let Ok(row) = stmt.query_row([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        {
            out.insert(id, row);
        }
    }
    Ok(out)
}

/// Corta o trecho para caber numa lista, sem partir caractere no meio.
pub fn preview(content: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, ch) in content.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{IndexOptions, index_workspace};
    use std::fs;

    #[test]
    fn rrf_puts_the_consensus_first() {
        // `2` é 2º no texto e 1º no vetor; `1` é 1º no texto e nem aparece no
        // vetor. A concordância das duas listas tem que vencer.
        let text = vec![1, 2, 3];
        let vector = vec![2, 4, 5];
        let fused = rrf_fuse(&[text, vector], RRF_K, 5);
        assert_eq!(fused[0].0, 2, "o consenso deveria liderar");
        assert!(fused[0].1 > fused[1].1);
        assert_eq!(fused.len(), 5);
    }

    #[test]
    fn rrf_with_a_single_list_keeps_its_order() {
        let fused = rrf_fuse(&[vec![7, 8, 9]], RRF_K, 10);
        assert_eq!(
            fused.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![7, 8, 9]
        );
    }

    #[test]
    fn rrf_is_deterministic_on_ties() {
        // Listas espelhadas: `2` e `3` empatam (1º numa, 3º na outra). A ordem
        // das listas não pode mudar o resultado, e o empate cai no menor id.
        let a = rrf_fuse(&[vec![3, 1, 2], vec![2, 1, 3]], RRF_K, 3);
        let b = rrf_fuse(&[vec![2, 1, 3], vec![3, 1, 2]], RRF_K, 3);
        assert_eq!(a, b, "a ordem das listas não pode mudar a fusão");
        assert_eq!(a[0].0, 2, "empate resolvido pelo menor id");
        assert!((a[0].1 - a[1].1).abs() < 1e-12, "{a:?} deveria empatar");
    }

    #[test]
    fn rrf_scores_follow_the_formula() {
        let fused = rrf_fuse(&[vec![10]], 60.0, 1);
        assert!((fused[0].1 - 1.0 / 61.0).abs() < 1e-12);
    }

    #[test]
    fn fts_query_neutralizes_operators() {
        let q = fts_query("token de \"sessão\" OR (algo*)", true).unwrap();
        assert!(q.contains("\"token\""));
        assert!(q.contains("\"sessão\""));
        assert!(q.contains(" AND "));
        assert!(!q.contains('*'), "asterisco não pode virar sintaxe");
        assert!(!q.contains('('), "parêntese não pode virar sintaxe");

        let q = fts_query("token sessão", false).unwrap();
        assert!(q.contains(" OR "));

        assert_eq!(fts_query("   ", true), None);
        assert_eq!(fts_query("a + b", true), None, "termos de 1 letra saem");
    }

    #[test]
    fn fts_query_drops_stopwords_but_never_everything() {
        let q = fts_query("prazo de duracao", true).unwrap();
        assert!(!q.contains("\"de\""), "palavra vazia sobrou: {q}");
        assert!(q.contains("\"prazo\"") && q.contains("\"duracao\""));

        // Só palavras vazias: procurar por elas é melhor que não procurar.
        let q = fts_query("de que", true).unwrap();
        assert!(q.contains("\"de\"") && q.contains("\"que\""));
    }

    #[test]
    fn preview_truncates_without_breaking_chars() {
        let s = preview("çãéíõ ção", 3);
        assert_eq!(s, "çãé…");
        assert_eq!(preview("curto", 50), "curto");
    }

    /// Sem modelo de embedding o índice tem que continuar respondendo — só que
    /// pelo caminho textual. É o degrade gracioso que a interface anuncia com
    /// `agent.rag.vectorOff`.
    #[tokio::test]
    async fn search_falls_back_to_text_only_without_vectors() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("proj");
        fs::create_dir_all(proj.join("src")).unwrap();
        fs::write(
            proj.join("src/session.rs"),
            "// valida o token de sessao do usuario\npub fn validate_session_token() -> bool { true }\n",
        )
        .unwrap();
        fs::write(
            proj.join("src/math.rs"),
            "pub fn soma(a: i32, b: i32) -> i32 { a + b }\n",
        )
        .unwrap();
        let db = dir.path().join("idx.db");

        let report = index_workspace(&db, &proj, IndexOptions::default())
            .await
            .unwrap();
        assert!(!report.vector, "sem modelo, nada de vetor");

        let hits = search(&db, &proj, "token de sessao", 5, EmbedConfig::default())
            .await
            .unwrap();
        assert!(!hits.is_empty(), "FTS5 sozinho deveria achar");
        assert_eq!(hits[0].path, "src/session.rs");
        assert_eq!(hits[0].source, HitSource::Text);
        assert!(hits[0].start_line >= 1);
        assert!(hits[0].citation().starts_with("src/session.rs:"));
    }

    #[tokio::test]
    async fn search_is_scoped_to_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("um.rs"), "palavra rarissima aqui").unwrap();
        fs::write(b.join("dois.rs"), "palavra rarissima ali").unwrap();
        let db = dir.path().join("idx.db");
        index_workspace(&db, &a, IndexOptions::default())
            .await
            .unwrap();
        index_workspace(&db, &b, IndexOptions::default())
            .await
            .unwrap();

        let hits = search(&db, &a, "rarissima", 10, EmbedConfig::default())
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "um.rs");
    }

    #[tokio::test]
    async fn search_on_empty_index_returns_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("proj");
        fs::create_dir_all(&proj).unwrap();
        let db = dir.path().join("idx.db");
        let hits = search(&db, &proj, "qualquer coisa", 5, EmbedConfig::default())
            .await
            .unwrap();
        assert!(hits.is_empty());
        // Consulta em branco também não pode explodir.
        assert!(
            search(&db, &proj, "   ", 5, EmbedConfig::default())
                .await
                .unwrap()
                .is_empty()
        );
    }
}
