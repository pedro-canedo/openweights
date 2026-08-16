//! Curadoria dos fatos: o filtro que separa memória de lixo.
//!
//! Memória boa é curta, específica e pouca. O inimigo aqui não é o disco —
//! é o contexto: cada fato memorizado volta em TODA execução seguinte, então
//! um parágrafo salvo por engano custa tokens para sempre e empurra o pedido
//! da pessoa para longe do fim do prompt (que é onde modelos pequenos olham).
//!
//! Por isso as regras deste módulo são deliberadamente severas:
//! - uma frase, [`MAX_FACT_CHARS`] caracteres — acima disso corta; muito
//!   acima disso recusa, porque texto longo é resumo de conversa, não fato;
//! - nada de duplicata, nem a "quase igual" que só muda a pontuação;
//! - escopo explícito: o que vale para a pessoa é global, o que vale para o
//!   código é do projeto. Na dúvida com uma pasta aberta, o fato é do
//!   projeto — vazar preferência de um projeto para outro é o erro caro.

use lr_store::memory::fact_key;
use serde::{Deserialize, Serialize};

/// Teto de um fato. Acima disso, corta na última palavra que couber.
pub const MAX_FACT_CHARS: usize = 240;

/// Acima deste tamanho não é fato, é texto: recusa em vez de cortar (cortar
/// um parágrafo pela metade guardaria uma frase sem sentido para sempre).
pub const MAX_RAW_CHARS: usize = MAX_FACT_CHARS * 3;

/// Abaixo disso não dá para saber do que se trata ("ok", "sim").
pub const MIN_FACT_CHARS: usize = 6;

/// Onde o fato vale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FactScope {
    /// Vale em qualquer projeto (preferências da pessoa).
    Global,
    /// Vale só na pasta aberta.
    Workspace,
}

/// Fato aprovado, pronto para gravar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuratedFact {
    pub content: String,
    pub scope: FactScope,
    /// Arquivo de assunto em `.openweights/memory/` que recebe o fato.
    pub topic: String,
}

/// Motivos de recusa. Todos viram mensagem para o modelo: ele precisa saber
/// o que fazer diferente, não só que falhou.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CurationError {
    #[error("o fato está vazio")]
    Empty,
    #[error("o fato é curto demais para significar alguma coisa")]
    TooShort,
    #[error("isso é um texto, não um fato")]
    TooLong,
    #[error("já sabemos disso: {0}")]
    Duplicate(String),
}

impl CurationError {
    /// Mensagem acionável para o modelo (mesmo espírito de
    /// `ToolError::to_model_message`).
    pub fn to_model_message(&self) -> String {
        match self {
            CurationError::Empty => {
                "Mande o fato em `fact`, numa frase curta. Ex.: \"este projeto usa pnpm\".".into()
            }
            CurationError::TooShort => {
                "O fato é curto demais. Escreva uma frase completa, ex.: \"os testes rodam com cargo test\"."
                    .into()
            }
            CurationError::TooLong => format!(
                "Isso é um resumo de conversa, não um fato. Guarde só a conclusão durável, em até {MAX_FACT_CHARS} caracteres."
            ),
            CurationError::Duplicate(existing) => {
                format!("Isso já está na memória (\"{existing}\"). Não precisa guardar de novo.")
            }
        }
    }
}

/// Marcas de lista/citação que os modelos colam na frente do fato.
const BULLETS: [&str; 6] = ["- ", "* ", "+ ", "• ", "> ", "#"];

/// Deixa o fato numa linha só, sem enfeite de markdown e sem aspas soltas.
///
/// Não mexe no conteúdo em si (nem em maiúsculas, nem em pontuação): o texto
/// é mostrado para a pessoa no painel e escrito no `MEMORY.md`, então tem que
/// continuar sendo a frase que alguém escreveu.
pub fn normalize(raw: &str) -> String {
    let mut text = raw.trim();

    // Marcas de lista podem vir empilhadas ("- - fato", "> - fato").
    let mut changed = true;
    while changed {
        changed = false;
        for bullet in BULLETS {
            if let Some(rest) = text.strip_prefix(bullet) {
                text = rest.trim_start();
                changed = true;
            }
        }
        // "1. fato" / "2) fato"
        if let Some(pos) = text.find([')', '.'])
            && pos > 0
            && pos < 3
            && text[..pos].chars().all(|c| c.is_ascii_digit())
        {
            text = text[pos + 1..].trim_start();
            changed = true;
        }
    }

    // Aspas envolvendo a frase inteira (o modelo copia do JSON dele mesmo).
    for (open, close) in [('"', '"'), ('\'', '\''), ('“', '”'), ('«', '»')] {
        if text.starts_with(open) && text.ends_with(close) && text.chars().count() > 1 {
            let inner = &text[open.len_utf8()..text.len() - close.len_utf8()];
            if !inner.contains(open) {
                text = inner.trim();
            }
        }
    }

    // Uma linha só: quebras viram espaço e espaço repetido some.
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    // Sobrou só pontuação ("-", "•", "..."): não é fato, é enfeite órfão.
    if !joined.chars().any(char::is_alphanumeric) {
        return String::new();
    }
    joined
}

/// Corta no teto, na última palavra inteira que couber.
pub fn clip(text: &str) -> String {
    if text.chars().count() <= MAX_FACT_CHARS {
        return text.to_string();
    }
    let head: String = text.chars().take(MAX_FACT_CHARS).collect();
    let cut = match head.rfind(' ') {
        // Só respeita a fronteira de palavra se ela não jogar metade fora.
        Some(pos) if pos > MAX_FACT_CHARS / 2 => pos,
        _ => head.len(),
    };
    format!("{}…", head[..cut].trim_end())
}

/// Trechos que denunciam um fato sobre a PESSOA (vale em todo projeto).
const GLOBAL_HINTS: [&str; 14] = [
    "prefiro",
    "prefere",
    "não gosto",
    "sempre responda",
    "responda sempre",
    "me chame",
    "meu nome",
    "eu uso",
    "costumo",
    "i prefer",
    "always answer",
    "always reply",
    "call me",
    "my name",
];

/// Trechos que prendem o fato à pasta aberta.
const WORKSPACE_HINTS: [&str; 10] = [
    "este projeto",
    "neste projeto",
    "esse projeto",
    "este repo",
    "neste repo",
    "aqui usamos",
    "o projeto usa",
    "this project",
    "this repo",
    "the codebase",
];

/// Decide o escopo pelo texto. Só é consultada quando quem chamou não disse
/// explicitamente (o parâmetro `scope` da ferramenta manda mais que isto).
pub fn classify_scope(content: &str, has_workspace: bool) -> FactScope {
    if !has_workspace {
        // Sem pasta aberta não existe "deste projeto": guardar como do
        // projeto criaria um fato órfão que nunca mais apareceria.
        return FactScope::Global;
    }
    let lower = content.to_lowercase();
    if WORKSPACE_HINTS.iter().any(|h| lower.contains(h)) {
        return FactScope::Workspace;
    }
    if GLOBAL_HINTS.iter().any(|h| lower.contains(h)) {
        return FactScope::Global;
    }
    FactScope::Workspace
}

/// Quanto duas chaves precisam se sobrepor para serem "o mesmo fato".
const NEAR_DUPLICATE_RATIO: f32 = 0.6;

/// Acha um fato já conhecido equivalente a `content`.
///
/// Vai além da igualdade porque o modelo reescreve o que já sabe: "usa pnpm"
/// e "usa pnpm, não npm" são a mesma informação, e guardar as duas enche o
/// prompt com a mesma frase duas vezes.
pub fn duplicate_of<'a>(content: &str, existing: &'a [String]) -> Option<&'a str> {
    let key = fact_key(content);
    if key.is_empty() {
        return None;
    }
    existing.iter().find_map(|candidate| {
        let other = fact_key(candidate);
        if other.is_empty() {
            return None;
        }
        if other == key {
            return Some(candidate.as_str());
        }
        let (short, long) = if key.len() <= other.len() {
            (&key, &other)
        } else {
            (&other, &key)
        };
        // Contido E de tamanho parecido: "usa pnpm" dentro de "usa pnpm não
        // npm" é duplicata; dentro de um parágrafo inteiro, não é.
        let ratio = short.len() as f32 / long.len() as f32;
        (long.contains(short.as_str()) && ratio >= NEAR_DUPLICATE_RATIO)
            .then_some(candidate.as_str())
    })
}

/// Palavras que apontam o assunto, na ordem em que são testadas (a primeira
/// que casar decide). Ordem = especificidade: "cargo test" é teste, não build.
const TOPIC_HINTS: [(&str, &[&str]); 5] = [
    (
        "testes",
        &[
            "teste",
            "testes",
            "test ",
            "tests",
            "vitest",
            "jest",
            "pytest",
            "cobertura",
            "coverage",
        ],
    ),
    (
        "estilo",
        &[
            "estilo",
            "style",
            "lint",
            "eslint",
            "prettier",
            "clippy",
            "rustfmt",
            "formata",
            "format",
            "convenç",
            "convention",
            "nomea",
            "naming",
        ],
    ),
    (
        "build",
        &[
            "build",
            "compil",
            "cargo",
            "npm",
            "pnpm",
            "yarn",
            "vite",
            "webpack",
            "make",
            "docker",
            "deploy",
            "dependênc",
            "dependenc",
        ],
    ),
    (
        "arquitetura",
        &[
            "arquitet",
            "architect",
            "módulo",
            "modulo",
            "module",
            "crate",
            "pasta",
            "folder",
            "estrutura",
            "structure",
            "banco",
            "database",
            "api ",
            "schema",
        ],
    ),
    (
        "preferencias",
        &[
            "prefiro", "prefere", "prefer", "idioma", "language", "responda", "sempre", "always",
            "me chame", "call me",
        ],
    ),
];

/// Assunto do fato = nome do arquivo em `.openweights/memory/`.
///
/// Agrupar por assunto é o que mantém os arquivos legíveis: um `build.md` com
/// cinco linhas é útil para a pessoa; um `memoria.md` com trinta, não.
pub fn topic_for(content: &str) -> String {
    let lower = content.to_lowercase();
    for (topic, hints) in TOPIC_HINTS {
        if hints.iter().any(|h| lower.contains(h)) {
            return topic.to_string();
        }
    }
    "geral".to_string()
}

/// Passa o fato bruto pelo filtro inteiro.
///
/// `scope_hint` vem de quem chamou (parâmetro da ferramenta ou escolha na
/// interface); `None` deixa a heurística decidir.
pub fn curate(
    raw: &str,
    existing: &[String],
    scope_hint: Option<FactScope>,
    has_workspace: bool,
) -> Result<CuratedFact, CurationError> {
    if raw.chars().count() > MAX_RAW_CHARS {
        return Err(CurationError::TooLong);
    }
    let normalized = normalize(raw);
    if normalized.is_empty() {
        return Err(CurationError::Empty);
    }
    if normalized.chars().count() < MIN_FACT_CHARS {
        return Err(CurationError::TooShort);
    }
    let content = clip(&normalized);
    if let Some(existing) = duplicate_of(&content, existing) {
        return Err(CurationError::Duplicate(existing.to_string()));
    }
    let scope = match scope_hint {
        // Pedir escopo de projeto sem pasta aberta não pode virar fato
        // perdido: cai para global.
        Some(FactScope::Workspace) if !has_workspace => FactScope::Global,
        Some(scope) => scope,
        None => classify_scope(&content, has_workspace),
    };
    Ok(CuratedFact {
        topic: topic_for(&content),
        content,
        scope,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_markup_and_joins_lines() {
        assert_eq!(normalize("  - usa pnpm  "), "usa pnpm");
        assert_eq!(normalize("* [x] usa pnpm"), "[x] usa pnpm");
        assert_eq!(normalize("1. usa pnpm"), "usa pnpm");
        assert_eq!(normalize("\"usa pnpm\""), "usa pnpm");
        assert_eq!(normalize("usa\n  pnpm\tsempre"), "usa pnpm sempre");
        assert_eq!(normalize("> - “usa pnpm”"), "usa pnpm");
        // Aspas internas não podem ser comidas.
        assert_eq!(
            normalize("chama de \"app\" o pacote"),
            "chama de \"app\" o pacote"
        );
    }

    #[test]
    fn empty_and_tiny_facts_are_refused_with_a_hint() {
        let err = curate("   \n  ", &[], None, true).unwrap_err();
        assert_eq!(err, CurationError::Empty);
        assert!(err.to_model_message().contains("fact"));

        let err = curate("- ", &[], None, true).unwrap_err();
        assert_eq!(err, CurationError::Empty);

        let err = curate("ok", &[], None, true).unwrap_err();
        assert_eq!(err, CurationError::TooShort);
        assert!(err.to_model_message().contains("frase completa"));
    }

    #[test]
    fn long_facts_are_clipped_and_paragraphs_are_refused() {
        let long = format!("o projeto {}", "muito longo ".repeat(30));
        let fact = curate(&long, &[], None, true).unwrap();
        assert!(fact.content.chars().count() <= MAX_FACT_CHARS + 1);
        assert!(fact.content.ends_with('…'), "{}", fact.content);
        // Cortou em palavra inteira, não no meio de uma.
        assert!(!fact.content.contains("mui…"));

        let paragraph = "a".repeat(MAX_RAW_CHARS + 1);
        let err = curate(&paragraph, &[], None, true).unwrap_err();
        assert_eq!(err, CurationError::TooLong);
        assert!(err.to_model_message().contains("240"));
    }

    #[test]
    fn duplicates_and_near_duplicates_are_refused() {
        let existing = vec!["Usa pnpm, não npm.".to_string()];
        // Igual a menos de caixa e pontuação.
        let err = curate("usa pnpm não npm", &existing, None, true).unwrap_err();
        assert!(matches!(err, CurationError::Duplicate(_)));
        // Reescrita mais curta do mesmo fato.
        let err = curate("usa pnpm não", &existing, None, true).unwrap_err();
        assert!(matches!(err, CurationError::Duplicate(_)));
        assert!(err.to_model_message().contains("já está na memória"));
        // Fato de verdade diferente passa.
        assert!(curate("os testes rodam com vitest", &existing, None, true).is_ok());
        // Substring curta demais não conta como duplicata.
        assert!(duplicate_of("usa", &existing).is_none());
    }

    #[test]
    fn scope_defaults_to_the_project_but_follows_the_text() {
        assert_eq!(
            classify_scope("este projeto usa pnpm", true),
            FactScope::Workspace
        );
        assert_eq!(
            classify_scope("prefiro respostas curtas", true),
            FactScope::Global
        );
        assert_eq!(
            classify_scope("o build usa vite", true),
            FactScope::Workspace
        );
        // Sem pasta aberta tudo é global, senão o fato ficaria órfão.
        assert_eq!(
            classify_scope("este projeto usa pnpm", false),
            FactScope::Global
        );
        let fact = curate(
            "este projeto usa pnpm",
            &[],
            Some(FactScope::Workspace),
            false,
        )
        .unwrap();
        assert_eq!(fact.scope, FactScope::Global);
        // Pedido explícito manda mais que a heurística.
        let fact = curate("este projeto usa pnpm", &[], Some(FactScope::Global), true).unwrap();
        assert_eq!(fact.scope, FactScope::Global);
    }

    #[test]
    fn topics_group_facts_by_subject() {
        assert_eq!(topic_for("os testes rodam com cargo test"), "testes");
        assert_eq!(topic_for("o build usa vite e pnpm"), "build");
        assert_eq!(topic_for("segue o padrão do clippy"), "estilo");
        assert_eq!(
            topic_for("os crates ficam em src-tauri/crates"),
            "arquitetura"
        );
        assert_eq!(topic_for("prefiro respostas curtas"), "preferencias");
        assert_eq!(topic_for("o cliente se chama Maria"), "geral");
    }
}
