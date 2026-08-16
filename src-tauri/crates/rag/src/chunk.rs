//! Divisão de um arquivo em trechos indexáveis.
//!
//! Duas exigências moldam o algoritmo:
//!
//! 1. **Citação precisa.** Todo trecho carrega `caminho`, linha inicial e
//!    linha final. Sem isso o modelo cita "está em algum lugar de auth.rs",
//!    que não ajuda ninguém. Por isso a divisão é por LINHA, nunca no meio
//!    de uma (exceto linha gigante — ver abaixo).
//! 2. **Contexto que não corta a ideia no meio.** Trechos vizinhos se
//!    sobrepõem em ~12%: se a assinatura da função cai no fim de um trecho e o
//!    corpo no começo do outro, a sobreposição mantém os dois pesquisáveis.
//!
//! O tamanho alvo (400–512 tokens) é o ponto onde embeddings de texto ainda
//! representam bem um trecho: menor vira ruído sem contexto, maior dilui o
//! assunto em vários e o vetor deixa de apontar para nada em específico.
//!
//! Tokens são ESTIMADOS por caracteres. Contar de verdade exigiria o
//! tokenizador do modelo (uma ida ao servidor por trecho) — caro demais para
//! um número que só precisa acertar a ordem de grandeza.

/// Média empírica de caracteres por token em código e prosa mistos. Código
/// tem mais pontuação (mais tokens por caractere) que texto corrido; 4 fica
/// no meio e erra para o lado seguro em ambos.
const CHARS_PER_TOKEN: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Primeira linha do trecho, base 1 (como o editor mostra).
    pub start_line: u32,
    /// Última linha do trecho, base 1 e **inclusiva**.
    pub end_line: u32,
    pub content: String,
}

#[derive(Debug, Clone, Copy)]
pub struct ChunkOptions {
    /// Teto absoluto por trecho.
    pub max_tokens: usize,
    /// Tamanho desejado: ao alcançá-lo, o trecho fecha na próxima linha.
    pub target_tokens: usize,
    /// Fração do alvo repetida no começo do trecho seguinte (0.10–0.15).
    pub overlap_ratio: f32,
}

impl Default for ChunkOptions {
    fn default() -> Self {
        Self {
            max_tokens: 512,
            target_tokens: 400,
            overlap_ratio: 0.12,
        }
    }
}

/// Estimativa de tokens de um texto.
pub fn estimate_tokens(s: &str) -> usize {
    let chars = s.chars().count();
    if chars == 0 {
        0
    } else {
        chars.div_ceil(CHARS_PER_TOKEN)
    }
}

/// Divide o texto em trechos com sobreposição, preservando as linhas.
pub fn chunk_text(text: &str, opts: &ChunkOptions) -> Vec<Chunk> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let max_tokens = opts.max_tokens.max(16);
    let target = opts.target_tokens.clamp(16, max_tokens);
    let overlap_tokens = ((target as f32) * opts.overlap_ratio.clamp(0.0, 0.5)) as usize;

    let mut out = Vec::new();
    let mut i = 0usize;

    while i < lines.len() {
        let mut tokens = 0usize;
        let mut j = i;
        while j < lines.len() {
            // +1 pela quebra de linha, que o modelo também consome.
            let line_tokens = estimate_tokens(lines[j]) + 1;
            if tokens > 0 && tokens + line_tokens > max_tokens {
                break;
            }
            tokens += line_tokens;
            j += 1;
            if tokens >= target {
                break;
            }
        }
        // Linha isolada maior que o teto: só ela não avançaria nunca.
        if j == i {
            j = i + 1;
        }

        push_slice(&mut out, &lines[i..j], i as u32 + 1, max_tokens);

        if j >= lines.len() {
            break;
        }

        // Volta algumas linhas para o próximo trecho começar sobreposto.
        let mut back = j;
        let mut acc = 0usize;
        while back > i + 1 && acc < overlap_tokens {
            back -= 1;
            acc += estimate_tokens(lines[back]) + 1;
        }
        // `back > i` garante progresso: nunca reinicia no mesmo lugar.
        i = back.max(i + 1);
    }

    out
}

/// Empurra as linhas como um trecho. Uma linha absurdamente longa (arquivo
/// minificado, JSON numa linha só) é quebrada por caracteres — todos os
/// pedaços apontam para a mesma linha, que continua sendo a verdade.
fn push_slice(out: &mut Vec<Chunk>, lines: &[&str], start_line: u32, max_tokens: usize) {
    let content = lines.join("\n");
    if content.trim().is_empty() {
        return;
    }
    if estimate_tokens(&content) <= max_tokens {
        out.push(Chunk {
            start_line,
            end_line: start_line + lines.len() as u32 - 1,
            content,
        });
        return;
    }

    let max_chars = max_tokens * CHARS_PER_TOKEN;
    let chars: Vec<char> = content.chars().collect();
    for piece in chars.chunks(max_chars) {
        let s: String = piece.iter().collect();
        if s.trim().is_empty() {
            continue;
        }
        out.push(Chunk {
            start_line,
            end_line: start_line + lines.len() as u32 - 1,
            content: s,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numbered(n: usize) -> String {
        (1..=n)
            .map(|i| format!("linha {i} com um pouco de conteudo para ocupar espaco"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn short_file_becomes_a_single_chunk() {
        let text = "fn main() {\n    println!(\"oi\");\n}";
        let chunks = chunk_text(text, &ChunkOptions::default());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 3);
        assert_eq!(chunks[0].content, text);
    }

    #[test]
    fn empty_and_blank_files_produce_nothing() {
        assert!(chunk_text("", &ChunkOptions::default()).is_empty());
        assert!(chunk_text("\n\n   \n", &ChunkOptions::default()).is_empty());
    }

    #[test]
    fn chunks_overlap_and_line_numbers_match_the_source() {
        let text = numbered(200);
        let lines: Vec<&str> = text.lines().collect();
        let chunks = chunk_text(&text, &ChunkOptions::default());
        assert!(
            chunks.len() > 2,
            "esperava vários trechos, veio {}",
            chunks.len()
        );

        for c in &chunks {
            assert!(c.start_line >= 1);
            assert!(c.end_line >= c.start_line);
            assert!(c.end_line as usize <= lines.len());
            // O conteúdo tem que ser exatamente as linhas anunciadas.
            let expected = lines[(c.start_line - 1) as usize..c.end_line as usize].join("\n");
            assert_eq!(c.content, expected, "trecho não bate com as linhas");
        }

        // Sobreposição: cada trecho recomeça ANTES do fim do anterior.
        for w in chunks.windows(2) {
            assert!(
                w[1].start_line <= w[0].end_line,
                "sem sobreposição entre {}-{} e {}-{}",
                w[0].start_line,
                w[0].end_line,
                w[1].start_line,
                w[1].end_line
            );
            assert!(w[1].start_line > w[0].start_line, "trecho não avançou");
        }

        // Cobertura: nenhuma linha do arquivo fica de fora.
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks.last().unwrap().end_line as usize, lines.len());
    }

    #[test]
    fn chunks_respect_the_token_ceiling() {
        let opts = ChunkOptions::default();
        for c in chunk_text(&numbered(300), &opts) {
            assert!(
                estimate_tokens(&c.content) <= opts.max_tokens,
                "trecho com {} tokens estourou o teto",
                estimate_tokens(&c.content)
            );
        }
    }

    #[test]
    fn very_long_single_line_is_split_by_characters() {
        let opts = ChunkOptions::default();
        let text = "x".repeat(opts.max_tokens * 4 * 3 + 10);
        let chunks = chunk_text(&text, &opts);
        assert!(
            chunks.len() >= 3,
            "linha gigante deveria virar vários trechos"
        );
        for c in &chunks {
            assert_eq!(c.start_line, 1);
            assert_eq!(c.end_line, 1);
            assert!(estimate_tokens(&c.content) <= opts.max_tokens);
        }
        // Nada de perder conteúdo no caminho.
        let total: usize = chunks.iter().map(|c| c.content.chars().count()).sum();
        assert_eq!(total, text.chars().count());
    }

    #[test]
    fn always_makes_progress_even_with_tiny_settings() {
        let opts = ChunkOptions {
            max_tokens: 16,
            target_tokens: 16,
            overlap_ratio: 0.5,
        };
        let chunks = chunk_text(&numbered(40), &opts);
        assert!(!chunks.is_empty());
        for w in chunks.windows(2) {
            assert!(w[1].start_line > w[0].start_line, "laço não avançou");
        }
    }
}
