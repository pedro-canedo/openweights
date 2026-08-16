//! Utilidades de texto para caber saída de ferramenta na janela do modelo.
//!
//! Compiladores e runners de teste escrevem para humanos: cor, molduras,
//! milhares de linhas. O modelo lê o mesmo texto com um orçamento de alguns
//! milhares de bytes — então antes de resumir é preciso *limpar*.
//!
//! Duas limpezas moram aqui:
//! - **Cor.** Muita ferramenta liga cor mesmo sem terminal (`FORCE_COLOR`,
//!   `--color=always` no script do projeto). Se as sequências ANSI ficarem,
//!   cada linha vira `\x1b[31merror\x1b[0m` e todo casamento de prefixo
//!   ("linha começa com `error`") falha. Limpar é pré-requisito de parsing,
//!   não enfeite.
//! - **Comprimento.** Uma única linha de saída pode ter 4 KB (um comando
//!   ecoado, um JSON inteiro). Cortar por linha protege o orçamento sem
//!   perder as outras linhas.

/// Escape que abre toda sequência ANSI.
const ESC: char = '\u{1b}';

/// Sinal de fim de uma sequência OSC (`BEL`).
const BEL: char = '\u{7}';

/// Remove sequências ANSI (cor, cursor, título de janela) do texto.
///
/// Cobre as três formas que aparecem na prática: CSI (`ESC [ … letra`), OSC
/// (`ESC ] … BEL` ou `ESC ] … ESC \`) e as de dois caracteres (`ESC c`).
/// Quando a sequência está truncada (a saída foi cortada no meio), o resto é
/// descartado — é lixo de qualquer jeito.
pub fn strip_ansi(input: &str) -> String {
    if !input.contains(ESC) {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != ESC {
            out.push(ch);
            continue;
        }
        match chars.next() {
            // CSI: parâmetros até uma letra final.
            Some('[') => {
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() || c == '@' || c == '~' {
                        break;
                    }
                }
            }
            // OSC: texto até BEL ou `ESC \`.
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == BEL {
                        break;
                    }
                    if c == ESC {
                        // `ESC \` fecha a sequência; consome a barra.
                        chars.next_if(|n| *n == '\\');
                        break;
                    }
                }
            }
            // `ESC c`, `ESC (B` e afins: descarta só o próximo caractere.
            Some(_) | None => {}
        }
    }
    out
}

/// Corta uma linha em `max_chars`, marcando o corte.
pub fn clip_line(line: &str, max_chars: usize) -> String {
    let line = line.trim_end();
    if line.chars().count() <= max_chars {
        return line.to_string();
    }
    let kept: String = line.chars().take(max_chars).collect();
    format!("{kept}…")
}

/// Últimas `n` linhas não vazias do fim do texto (na ordem original).
pub fn tail_lines(text: &str, n: usize) -> Vec<&str> {
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(n);
    all[start..]
        .iter()
        .copied()
        .skip_while(|l| l.trim().is_empty())
        .collect()
}

/// Quantas linhas o texto tem (para contar o que ficou de fora).
pub fn line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

/// Junta stdout e stderr numa vista só.
///
/// Cada runner escolhe um cano diferente — o Jest manda *todo* o relatório
/// para stderr, o pytest para stdout, o cargo divide entre os dois. Parsear
/// os dois separadamente duplicaria toda a lógica; parsear a junção não perde
/// nada, porque os padrões que procuramos são âncoras de linha.
pub fn combined(stdout: &str, stderr: &str) -> String {
    let out = strip_ansi(stdout);
    let err = strip_ansi(stderr);
    match (out.trim().is_empty(), err.trim().is_empty()) {
        (true, _) => err,
        (_, true) => out,
        _ => format!("{out}\n{err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_ansi_removes_color_and_keeps_the_words() {
        let painted = "\u{1b}[31merror\u{1b}[0m: faltou ponto e vírgula";
        assert_eq!(strip_ansi(painted), "error: faltou ponto e vírgula");
    }

    #[test]
    fn strip_ansi_handles_osc_and_broken_sequences() {
        // Título de janela (OSC) + sequência cortada no fim da saída.
        let text = "\u{1b}]0;titulo\u{7}ok\u{1b}[3";
        assert_eq!(strip_ansi(text), "ok");
        // Texto sem escape nenhum volta idêntico.
        assert_eq!(strip_ansi("simples"), "simples");
    }

    #[test]
    fn clip_line_marks_the_cut_and_respects_utf8() {
        assert_eq!(clip_line("abc", 10), "abc");
        assert_eq!(clip_line("ãéíõü-demais", 5), "ãéíõü…");
    }

    #[test]
    fn tail_lines_takes_the_end_of_the_output() {
        let text = "um\ndois\ntres\nquatro";
        assert_eq!(tail_lines(text, 2), vec!["tres", "quatro"]);
        assert_eq!(tail_lines(text, 99).len(), 4);
    }

    #[test]
    fn combined_joins_both_pipes_without_empty_padding() {
        assert_eq!(combined("saida", ""), "saida");
        assert_eq!(combined("", "erro"), "erro");
        assert_eq!(combined("a", "b"), "a\nb");
    }
}
