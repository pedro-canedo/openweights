//! A face legível da memória: `.openweights/memory/` dentro do projeto.
//!
//! O banco é onde a memória é consultada; esta pasta é onde ela é
//! **auditada**. Quem abre o projeto vê um `MEMORY.md` com o índice e um
//! arquivo por assunto — dá para ler no editor, versionar no git, corrigir
//! um fato errado à mão e mandar para um colega. Memória que a pessoa não
//! consegue inspecionar é memória em que ela não confia.
//!
//! Daí as duas regras que moldam o módulo:
//!
//! 1. **A pasta só nasce quando há o que guardar.** Nada de sujar um projeto
//!    com estrutura vazia por ter aberto o app uma vez.
//! 2. **O arquivo é do usuário também.** Nunca reescrevemos o que ele
//!    escreveu: fatos novos são *acrescentados* ao fim do arquivo do assunto,
//!    e o índice tem um bloco delimitado por marcadores HTML — só o que está
//!    entre eles é gerado, o resto do `MEMORY.md` é dele. Se nada mudou, o
//!    arquivo nem é tocado (mtime intacto, git limpo).

use lr_store::memory::fact_key;
use std::io;
use std::path::{Path, PathBuf};

/// Pasta da memória, relativa à raiz do projeto.
pub const MEMORY_SUBDIR: &str = ".openweights/memory";

/// Índice legível, dentro da pasta da memória.
pub const INDEX_FILE: &str = "MEMORY.md";

/// Início do trecho gerado do índice. Tudo fora dele é da pessoa.
const BLOCK_BEGIN: &str = "<!-- openweights:index -->";
const BLOCK_END: &str = "<!-- /openweights:index -->";

/// Cabeçalho de um `MEMORY.md` novo (só quando o arquivo ainda não existe).
const INDEX_HEADER: &str = "# Memória do projeto\n\n\
Fatos que o agente do OpenWeights aprendeu aqui. Pode editar à mão: o app só \
reescreve o índice entre os marcadores, e nunca apaga o que você escreveu.\n\n";

/// Teto de arquivos de assunto listados no índice (defesa contra pasta suja).
const MAX_TOPICS: usize = 50;

pub fn memory_dir(workspace: &Path) -> PathBuf {
    workspace.join(".openweights").join("memory")
}

pub fn index_path(workspace: &Path) -> PathBuf {
    memory_dir(workspace).join(INDEX_FILE)
}

pub fn topic_path(workspace: &Path, topic: &str) -> PathBuf {
    memory_dir(workspace).join(format!("{}.md", slug(topic)))
}

/// Tira o acento de uma letra latina comum. O nome do arquivo tem que
/// sobreviver a `git`, a zip e ao Explorer do Windows.
fn deaccent(c: char) -> char {
    match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        other => other,
    }
}

/// Nome de arquivo seguro para um assunto.
///
/// O assunto pode vir do modelo, então isto é fronteira de segurança: só
/// `[a-z0-9-]` sai daqui, o que torna impossível `../` ou `C:` escaparem da
/// pasta da memória.
pub fn slug(topic: &str) -> String {
    let mut out = String::new();
    for c in topic.trim().to_lowercase().chars().map(deaccent) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let cleaned = out.trim_matches('-');
    let cut: String = cleaned.chars().take(40).collect();
    let cut = cut.trim_end_matches('-').to_string();
    if cut.is_empty() {
        "geral".to_string()
    } else {
        cut
    }
}

/// Título humano do arquivo de assunto.
fn title_for(topic: &str) -> String {
    match topic {
        "testes" => "Testes".to_string(),
        "build" => "Build e dependências".to_string(),
        "estilo" => "Estilo e convenções".to_string(),
        "arquitetura" => "Arquitetura".to_string(),
        "preferencias" => "Preferências".to_string(),
        "geral" => "Geral".to_string(),
        other => {
            let words = other.replace('-', " ");
            let mut chars = words.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => words,
            }
        }
    }
}

/// Um arquivo de assunto já lido.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicFile {
    pub topic: String,
    pub title: String,
    /// Só as linhas de fato (`- ...`); o texto que a pessoa escreveu fica
    /// no arquivo, mas não entra aqui.
    pub facts: Vec<String>,
    pub path: PathBuf,
}

/// Lê os fatos (linhas `- ...`) de um markdown.
fn parse_facts(body: &str) -> Vec<String> {
    body.lines()
        .map(str::trim)
        .filter_map(|line| {
            line.strip_prefix("- ")
                .or_else(|| line.strip_prefix("* "))
                .map(|f| f.trim().to_string())
        })
        .filter(|f| !f.is_empty())
        .collect()
}

/// Primeiro título `# ...` do arquivo, se houver.
fn parse_title(body: &str, fallback: &str) -> String {
    body.lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("# "))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

/// Arquivos de assunto existentes, em ordem alfabética.
///
/// Pasta inexistente devolve lista vazia — ausência de memória não é erro.
pub fn list_topics(workspace: &Path) -> Vec<TopicFile> {
    let dir = memory_dir(workspace);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<TopicFile> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                return None;
            }
            let name = path.file_name()?.to_str()?;
            if name == INDEX_FILE {
                return None;
            }
            let topic = name.trim_end_matches(".md").to_string();
            let body = std::fs::read_to_string(&path).ok()?;
            Some(TopicFile {
                title: parse_title(&body, &title_for(&topic)),
                facts: parse_facts(&body),
                topic,
                path,
            })
        })
        .collect();
    out.sort_by(|a, b| a.topic.cmp(&b.topic));
    out.truncate(MAX_TOPICS);
    out
}

/// Fatos já escritos em qualquer arquivo de assunto.
pub fn all_facts(workspace: &Path) -> Vec<String> {
    list_topics(workspace)
        .into_iter()
        .flat_map(|t| t.facts)
        .collect()
}

/// Escreve `content` em `path` só se for diferente do que já está lá.
///
/// Devolve `true` quando escreveu. Evitar a escrita idêntica é o que mantém
/// o `git status` limpo e não invalida o mtime que o editor observa.
fn write_if_changed(path: &Path, content: &str) -> io::Result<bool> {
    if let Ok(current) = std::fs::read_to_string(path)
        && current == content
    {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(true)
}

/// Acrescenta fatos ao arquivo do assunto e devolve os que entraram.
///
/// O que já está lá (por [`fact_key`]) é ignorado, e quando nada é novo o
/// arquivo não é tocado — nem criado. Tudo que a pessoa escreveu no arquivo
/// permanece: os fatos novos entram no fim.
pub fn append_facts(workspace: &Path, topic: &str, facts: &[String]) -> io::Result<Vec<String>> {
    let path = topic_path(workspace, topic);
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    let known: Vec<String> = parse_facts(&current).iter().map(|f| fact_key(f)).collect();

    let mut fresh: Vec<String> = Vec::new();
    for fact in facts {
        let fact = fact.trim();
        let key = fact_key(fact);
        if fact.is_empty() || key.is_empty() {
            continue;
        }
        if known.contains(&key) || fresh.iter().any(|f| fact_key(f) == key) {
            continue;
        }
        fresh.push(fact.to_string());
    }
    if fresh.is_empty() {
        return Ok(fresh);
    }

    let mut body = if current.trim().is_empty() {
        format!("# {}\n\n", title_for(&slug(topic)))
    } else {
        let mut b = current;
        if !b.ends_with('\n') {
            b.push('\n');
        }
        b
    };
    for fact in &fresh {
        body.push_str("- ");
        body.push_str(fact);
        body.push('\n');
    }
    write_if_changed(&path, &body)?;
    Ok(fresh)
}

/// Apaga um fato dos arquivos de assunto (a pessoa mandou esquecer).
///
/// Só a linha do fato sai; o resto do arquivo — inclusive o que ela escreveu
/// — fica exatamente como estava. Devolve `true` se algo saiu.
pub fn remove_fact(workspace: &Path, content: &str) -> io::Result<bool> {
    let key = fact_key(content);
    if key.is_empty() {
        return Ok(false);
    }
    let mut removed = false;
    for topic in list_topics(workspace) {
        let Ok(body) = std::fs::read_to_string(&topic.path) else {
            continue;
        };
        let kept: Vec<&str> = body
            .lines()
            .filter(|line| {
                let trimmed = line.trim();
                match trimmed
                    .strip_prefix("- ")
                    .or_else(|| trimmed.strip_prefix("* "))
                {
                    Some(fact) => fact_key(fact) != key,
                    None => true,
                }
            })
            .collect();
        if kept.len() == body.lines().count() {
            continue;
        }
        let mut next = kept.join("\n");
        if !next.ends_with('\n') {
            next.push('\n');
        }
        write_if_changed(&topic.path, &next)?;
        removed = true;
    }
    Ok(removed)
}

/// Monta o trecho gerado do índice.
fn index_block(topics: &[TopicFile]) -> String {
    let mut out = String::from(BLOCK_BEGIN);
    out.push('\n');
    if topics.is_empty() {
        out.push_str("\n_Nada memorizado ainda._\n\n");
    } else {
        out.push('\n');
        for topic in topics {
            out.push_str(&format!(
                "- [{}]({}.md) — {} fato(s)\n",
                topic.title,
                topic.topic,
                topic.facts.len()
            ));
        }
        out.push('\n');
    }
    out.push_str(BLOCK_END);
    out.push('\n');
    out
}

/// Regenera o `MEMORY.md`.
///
/// - Sem assunto nenhum e sem índice: não cria nada (regra 1).
/// - Índice já existente com os marcadores: troca só o trecho entre eles.
/// - Índice existente sem marcadores (a pessoa escreveu o dela): o bloco é
///   acrescentado no fim, sem tocar no que estava escrito.
///
/// Devolve `true` quando o arquivo mudou.
pub fn write_index(workspace: &Path) -> io::Result<bool> {
    let path = index_path(workspace);
    let topics = list_topics(workspace);
    let current = std::fs::read_to_string(&path).ok();
    if topics.is_empty() && current.is_none() {
        return Ok(false);
    }

    let block = index_block(&topics);
    let next = match current {
        Some(body) => match (body.find(BLOCK_BEGIN), body.find(BLOCK_END)) {
            (Some(start), Some(end)) if end > start => {
                let tail = &body[end + BLOCK_END.len()..];
                format!("{}{}{}", &body[..start], block.trim_end(), tail)
            }
            _ => {
                let mut merged = body;
                if !merged.ends_with('\n') {
                    merged.push('\n');
                }
                merged.push('\n');
                merged.push_str(&block);
                merged
            }
        },
        None => format!("{INDEX_HEADER}{block}"),
    };
    write_if_changed(&path, &next)
}

/// Conteúdo do índice, se existir.
pub fn read_index(workspace: &Path) -> Option<String> {
    std::fs::read_to_string(index_path(workspace)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn ws() -> TempDir {
        TempDir::new().unwrap()
    }

    #[test]
    fn nothing_is_created_until_there_is_something_to_keep() {
        let dir = ws();
        assert!(list_topics(dir.path()).is_empty());
        assert!(!write_index(dir.path()).unwrap());
        assert!(
            !memory_dir(dir.path()).exists(),
            "pasta de memória não pode nascer vazia"
        );

        append_facts(dir.path(), "build", &["usa pnpm".into()]).unwrap();
        assert!(topic_path(dir.path(), "build").exists());
        assert!(write_index(dir.path()).unwrap());
        assert!(index_path(dir.path()).exists());
    }

    #[test]
    fn the_index_lists_the_topics_with_their_counts() {
        let dir = ws();
        append_facts(dir.path(), "build", &["usa pnpm".into(), "vite 6".into()]).unwrap();
        append_facts(dir.path(), "testes", &["roda vitest".into()]).unwrap();
        write_index(dir.path()).unwrap();

        let index = read_index(dir.path()).unwrap();
        assert!(index.contains("# Memória do projeto"));
        assert!(index.contains("[Build e dependências](build.md) — 2 fato(s)"));
        assert!(index.contains("[Testes](testes.md) — 1 fato(s)"));
        assert!(index.contains(BLOCK_BEGIN) && index.contains(BLOCK_END));

        // Regerar com um assunto novo atualiza a contagem, não duplica bloco.
        append_facts(dir.path(), "testes", &["cobertura mínima 80%".into()]).unwrap();
        write_index(dir.path()).unwrap();
        let index = read_index(dir.path()).unwrap();
        assert_eq!(index.matches(BLOCK_BEGIN).count(), 1);
        assert!(index.contains("[Testes](testes.md) — 2 fato(s)"));
    }

    #[test]
    fn manual_edits_survive_the_rewrite() {
        let dir = ws();
        append_facts(dir.path(), "build", &["usa pnpm".into()]).unwrap();
        write_index(dir.path()).unwrap();

        // A pessoa escreve por cima, antes e depois do bloco gerado.
        let index = read_index(dir.path()).unwrap();
        let edited = format!("Nota minha no topo.\n\n{index}\nRodapé meu.\n");
        std::fs::write(index_path(dir.path()), &edited).unwrap();

        append_facts(dir.path(), "testes", &["roda vitest".into()]).unwrap();
        write_index(dir.path()).unwrap();

        let index = read_index(dir.path()).unwrap();
        assert!(index.starts_with("Nota minha no topo."), "{index}");
        assert!(index.trim_end().ends_with("Rodapé meu."), "{index}");
        assert!(index.contains("[Testes](testes.md)"));
        assert_eq!(index.matches(BLOCK_BEGIN).count(), 1);
    }

    #[test]
    fn an_index_written_by_hand_keeps_its_text() {
        let dir = ws();
        append_facts(dir.path(), "build", &["usa pnpm".into()]).unwrap();
        std::fs::write(index_path(dir.path()), "# Minhas notas\n\nnada a ver\n").unwrap();

        write_index(dir.path()).unwrap();
        let index = read_index(dir.path()).unwrap();
        assert!(index.starts_with("# Minhas notas"));
        assert!(index.contains("nada a ver"));
        assert!(index.contains("[Build e dependências](build.md)"));
    }

    #[test]
    fn a_topic_file_keeps_the_prose_around_the_facts() {
        let dir = ws();
        append_facts(dir.path(), "build", &["usa pnpm".into()]).unwrap();
        let path = topic_path(dir.path(), "build");
        std::fs::write(
            &path,
            "# Build e dependências\n\nCuidado: o CI usa npm.\n\n- usa pnpm\n",
        )
        .unwrap();

        let added = append_facts(dir.path(), "build", &["vite 6 no dev".into()]).unwrap();
        assert_eq!(added, vec!["vite 6 no dev".to_string()]);
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("Cuidado: o CI usa npm."));
        assert!(body.contains("- usa pnpm"));
        assert!(body.contains("- vite 6 no dev"));
    }

    #[test]
    fn a_known_fact_does_not_touch_the_file() {
        let dir = ws();
        append_facts(dir.path(), "build", &["Usa pnpm, não npm.".into()]).unwrap();
        let path = topic_path(dir.path(), "build");
        let before = std::fs::read_to_string(&path).unwrap();

        // Mesma frase com outra caixa/pontuação: nada entra.
        let added = append_facts(dir.path(), "build", &["USA PNPM, NÃO NPM!".into()]).unwrap();
        assert!(added.is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn topic_names_from_the_model_cannot_escape_the_folder() {
        let dir = ws();
        for hostile in ["../../etc/passwd", "C:\\Windows\\x", "  ", "..", "a/b"] {
            let path = topic_path(dir.path(), hostile);
            assert_eq!(
                path.parent().unwrap(),
                memory_dir(dir.path()),
                "assunto {hostile:?} escapou para {path:?}"
            );
        }
        assert_eq!(slug("Build & Deps!"), "build-deps");
        assert_eq!(slug(""), "geral");
        assert_eq!(slug("Preferências"), "preferencias");
    }

    #[test]
    fn forgetting_removes_only_the_fact_line() {
        let dir = ws();
        append_facts(
            dir.path(),
            "build",
            &["usa pnpm".into(), "vite 6 no dev".into()],
        )
        .unwrap();
        let path = topic_path(dir.path(), "build");
        let body = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{body}\nObservação minha.\n")).unwrap();

        assert!(remove_fact(dir.path(), "USA PNPM!").unwrap());
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(!body.contains("- usa pnpm"));
        assert!(body.contains("- vite 6 no dev"));
        assert!(body.contains("Observação minha."));
        assert!(body.contains("# Build e dependências"));

        assert!(!remove_fact(dir.path(), "fato que nunca existiu").unwrap());
    }

    #[test]
    fn all_facts_reads_back_what_was_written() {
        let dir = ws();
        append_facts(dir.path(), "build", &["usa pnpm".into()]).unwrap();
        append_facts(dir.path(), "testes", &["roda vitest".into()]).unwrap();
        let mut facts = all_facts(dir.path());
        facts.sort();
        assert_eq!(
            facts,
            vec!["roda vitest".to_string(), "usa pnpm".to_string()]
        );
    }
}
