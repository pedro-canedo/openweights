//! Extração de erros e avisos da saída de compiladores e linters.
//!
//! O que o agente precisa de uma compilação quebrada cabe em três coisas:
//! **arquivo, linha e o que está errado**. O resto — a moldura de setas, o
//! `help:` com a sugestão desenhada, o "For more information try rustc
//! --explain" — é para o humano que vai abrir o editor. Despejar tudo custa
//! milhares de tokens e, pior, enterra os dois erros que importam no meio de
//! duzentas linhas de contexto.
//!
//! Por que sem `regex`: os formatos são poucos e todos ancorados em prefixo ou
//! em `arquivo:linha:coluna`. Casar à mão é mais previsível (e mais rápido)
//! do que montar meia dúzia de expressões, e evita mais uma dependência.
//!
//! Um detalhe que só aparece no Windows: `C:\src\a.rs:10:5` tem três
//! dois-pontos. Por isso a posição é lida **da direita para a esquerda** — os
//! dois últimos campos numéricos são linha e coluna, e o que sobrar é o
//! caminho, letra de unidade incluída.

use crate::text::{clip_line, strip_ansi};
use std::fmt::Write as _;

/// Tamanho máximo de uma mensagem de diagnóstico no resumo.
const MAX_MESSAGE_CHARS: usize = 160;

/// Quantas linhas olhamos à frente procurando o `--> arquivo:linha` do rustc.
const RUSTC_LOOKAHEAD: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    fn label(self) -> &'static str {
        match self {
            Severity::Error => "erro",
            Severity::Warning => "aviso",
        }
    }
}

/// Um problema apontado por um compilador ou linter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub col: Option<u32>,
    /// Código da ferramenta (`E0308`, `TS2345`, `no-unused-vars`, `F401`).
    pub code: Option<String>,
    pub message: String,
}

impl Diagnostic {
    /// Uma linha só: `src/lib.rs:10:5 [E0308] mismatched types`.
    pub fn render(&self) -> String {
        let mut out = String::new();
        match (&self.file, self.line, self.col) {
            (Some(f), Some(l), Some(c)) => {
                let _ = write!(out, "{f}:{l}:{c}");
            }
            (Some(f), Some(l), None) => {
                let _ = write!(out, "{f}:{l}");
            }
            (Some(f), None, _) => out.push_str(f),
            _ => out.push_str("(sem arquivo)"),
        }
        if let Some(code) = &self.code {
            let _ = write!(out, " [{code}]");
        }
        let _ = write!(out, " {}", clip_line(&self.message, MAX_MESSAGE_CHARS));
        out
    }
}

/// Lê a saída inteira e devolve os diagnósticos que reconhecer, em ordem.
///
/// Formatos cobertos: rustc/cargo, tsc (as duas formas), eslint "stylish",
/// `arquivo:linha:coluna: mensagem` (go, ruff, flake8, gcc) e Maven/javac.
pub fn extract(text: &str) -> Vec<Diagnostic> {
    let clean = strip_ansi(text);
    let lines: Vec<&str> = clean.lines().collect();
    let mut out = Vec::new();
    // Cabeçalho de arquivo do eslint, válido até o próximo cabeçalho.
    let mut eslint_file: Option<String> = None;

    for (index, raw) in lines.iter().enumerate() {
        let line = raw.trim_end();
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(diag) = rustc_diagnostic(trimmed, &lines, index) {
            out.push(diag);
            continue;
        }
        if let Some(diag) = tsc_diagnostic(trimmed) {
            out.push(diag);
            continue;
        }
        if let Some(diag) = maven_diagnostic(trimmed) {
            out.push(diag);
            continue;
        }
        if let Some(diag) = eslint_row(line, eslint_file.as_deref()) {
            out.push(diag);
            continue;
        }
        if let Some(diag) = positional_diagnostic(trimmed) {
            out.push(diag);
            continue;
        }
        // Cabeçalho do eslint: caminho sozinho numa linha sem indentação.
        if !line.starts_with(char::is_whitespace) && looks_like_path(trimmed) {
            eslint_file = Some(trimmed.to_string());
        }
    }
    out
}

/// Conta erros e avisos de uma lista de diagnósticos.
pub fn counts(diags: &[Diagnostic]) -> (usize, usize) {
    let errors = diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    (errors, diags.len() - errors)
}

/// Monta o bloco de texto com os primeiros `limit` diagnósticos.
pub fn render_list(diags: &[Diagnostic], limit: usize) -> String {
    let (errors, warnings) = counts(diags);
    let mut out = format!("{errors} erro(s) e {warnings} aviso(s).");
    if diags.is_empty() {
        return out;
    }
    // Erro antes de aviso: é o que trava o trabalho.
    let mut ordered: Vec<&Diagnostic> = diags.iter().collect();
    ordered.sort_by_key(|d| d.severity != Severity::Error);

    let shown = ordered.len().min(limit);
    let _ = write!(out, " Primeiros {shown}:");
    for (i, diag) in ordered.iter().take(shown).enumerate() {
        let _ = write!(
            out,
            "\n  {}. [{}] {}",
            i + 1,
            diag.severity.label(),
            diag.render()
        );
    }
    if ordered.len() > shown {
        let _ = write!(
            out,
            "\n  … e mais {} não listado(s).",
            ordered.len() - shown
        );
    }
    out
}

// ---------------------------------------------------------------- formatos ---

/// `error[E0308]: mismatched types` + `  --> src/lib.rs:1:38` logo abaixo.
fn rustc_diagnostic(line: &str, lines: &[&str], index: usize) -> Option<Diagnostic> {
    let (severity, rest) = match line.strip_prefix("error") {
        Some(rest) => (Severity::Error, rest),
        None => (Severity::Warning, line.strip_prefix("warning")?),
    };

    // `error[E0308]: …` ou `error: …` — qualquer outra coisa (`errors`,
    // `warning-free`) não é cabeçalho de diagnóstico.
    let (code, after) = match rest.strip_prefix('[') {
        Some(tail) => {
            let (code, tail) = tail.split_once(']')?;
            (Some(code.to_string()), tail)
        }
        None => (None, rest),
    };
    let message = after.strip_prefix(':')?.trim().to_string();
    if message.is_empty() {
        return None;
    }
    // Linhas de fechamento do cargo: não são problemas, são contagem.
    let noise = [
        "could not compile",
        "aborting due to",
        "build failed",
        "test failed",
        "unused manifest key",
    ];
    if noise.iter().any(|n| message.starts_with(n)) {
        return None;
    }

    // A posição vem em `--> arquivo:linha:coluna` poucas linhas abaixo.
    let mut file = None;
    let mut line_no = None;
    let mut col = None;
    for probe in lines.iter().skip(index + 1).take(RUSTC_LOOKAHEAD) {
        let probe = probe.trim();
        if let Some(pos) = probe.strip_prefix("--> ") {
            if let Some((f, l, c)) = split_position(pos.trim()) {
                file = Some(f);
                line_no = Some(l);
                col = c;
            }
            break;
        }
        // Outro cabeçalho começou: este diagnóstico não tem posição.
        if probe.starts_with("error") || probe.starts_with("warning") {
            break;
        }
    }

    Some(Diagnostic {
        severity,
        file,
        line: line_no,
        col,
        code,
        message,
    })
}

/// `src/a.ts(12,3): error TS2345: msg` e `src/a.ts:12:3 - error TS2345: msg`.
fn tsc_diagnostic(line: &str) -> Option<Diagnostic> {
    let (head, tail) = if let Some((head, tail)) = line.split_once("): ") {
        // Forma com parênteses: `arquivo(linha,coluna)`.
        let (file, pos) = head.rsplit_once('(')?;
        let (l, c) = pos.split_once(',')?;
        let file = file.to_string();
        let line_no = l.trim().parse().ok()?;
        let col = c.trim().parse().ok();
        (Some((file, line_no, col)), tail)
    } else if let Some((head, tail)) = line.split_once(" - ") {
        // Forma "pretty": `arquivo:linha:coluna - error TS…`.
        (split_position(head.trim()), tail)
    } else {
        return None;
    };
    let (file, line_no, col) = head?;

    let (severity, rest) = match tail.strip_prefix("error") {
        Some(rest) => (Severity::Error, rest),
        None => (Severity::Warning, tail.strip_prefix("warning")?),
    };
    let rest = rest.trim();
    let (code, message) = match rest.split_once(':') {
        Some((code, msg)) if code.starts_with("TS") => (Some(code.to_string()), msg.trim()),
        _ => (None, rest.trim_start_matches(':').trim()),
    };

    Some(Diagnostic {
        severity,
        file: Some(file),
        line: Some(line_no),
        col,
        code,
        message: message.to_string(),
    })
}

/// `[ERROR] /caminho/Foo.java:[12,5] cannot find symbol`
fn maven_diagnostic(line: &str) -> Option<Diagnostic> {
    let (severity, rest) = match line.strip_prefix("[ERROR] ") {
        Some(rest) => (Severity::Error, rest),
        None => (Severity::Warning, line.strip_prefix("[WARNING] ")?),
    };
    let (file, tail) = rest.split_once(":[")?;
    let (pos, message) = tail.split_once(']')?;
    let (l, c) = pos.split_once(',')?;
    Some(Diagnostic {
        severity,
        file: Some(file.to_string()),
        line: l.trim().parse().ok(),
        col: c.trim().parse().ok(),
        code: None,
        message: message.trim().to_string(),
    })
}

/// Linha de resultado do eslint: `  12:3  error  mensagem  regra`.
///
/// O formato "stylish" alinha as colunas com **dois ou mais espaços**, e é
/// esse o separador usado aqui. Dividir por espaço simples misturaria a
/// mensagem com o nome da regra — não dá para adivinhar pelo texto qual das
/// palavras finais é a regra (`semi`, `eqeqeq` e `camelcase` parecem palavras
/// comuns).
fn eslint_row(line: &str, file: Option<&str>) -> Option<Diagnostic> {
    if !line.starts_with(char::is_whitespace) {
        return None;
    }
    let columns: Vec<&str> = line
        .trim()
        .split("  ")
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .collect();

    let (l, c) = columns.first()?.split_once(':')?;
    let line_no: u32 = l.parse().ok()?;
    let col: u32 = c.parse().ok()?;
    let severity = match *columns.get(1)? {
        "error" => Severity::Error,
        "warning" => Severity::Warning,
        _ => return None,
    };
    let rest = columns.get(2..).unwrap_or_default();
    if rest.is_empty() {
        return None;
    }
    // A última coluna é a regra — quando existe uma coluna a mais.
    let (message, code) = match rest.split_last() {
        Some((last, head)) if !head.is_empty() && !last.contains(' ') => {
            (head.join(" "), Some(last.to_string()))
        }
        _ => (rest.join(" "), None),
    };
    Some(Diagnostic {
        severity,
        file: file.map(str::to_string),
        line: Some(line_no),
        col: Some(col),
        code,
        message,
    })
}

/// `arquivo:linha:coluna: mensagem` — go, ruff, flake8, gcc.
fn positional_diagnostic(line: &str) -> Option<Diagnostic> {
    let (head, message) = line.split_once(": ")?;
    let (file, line_no, col) = split_position(head)?;
    if !looks_like_path(&file) {
        return None;
    }
    let message = message.trim();
    if message.is_empty() {
        return None;
    }
    // Ferramentas como o ruff prefixam o código: `F401 [*] import não usado`.
    let (code, message) = match message.split_once(' ') {
        Some((first, rest)) if is_rule_code(first) => (Some(first.to_string()), rest.trim()),
        _ => (None, message),
    };
    let severity = if message.to_lowercase().starts_with("warning") {
        Severity::Warning
    } else {
        Severity::Error
    };
    Some(Diagnostic {
        severity,
        file: Some(file),
        line: Some(line_no),
        col,
        code,
        message: message.to_string(),
    })
}

// ----------------------------------------------------------------- apoio ---

/// Separa `caminho:linha:coluna` (ou `caminho:linha`) lendo da direita.
fn split_position(text: &str) -> Option<(String, u32, Option<u32>)> {
    let (head, last) = text.rsplit_once(':')?;
    match last.trim().parse::<u32>() {
        // `…:linha:coluna`
        Ok(col) => match head.rsplit_once(':') {
            Some((file, mid)) => match mid.trim().parse::<u32>() {
                Ok(line) if !file.is_empty() => Some((file.to_string(), line, Some(col))),
                // Só um número: era `arquivo:linha`.
                _ => Some((head.to_string(), col, None)),
            },
            None if !head.is_empty() => Some((head.to_string(), col, None)),
            None => None,
        },
        Err(_) => None,
    }
}

/// Tem cara de caminho de arquivo (e não de frase solta)?
fn looks_like_path(text: &str) -> bool {
    if text.contains(' ') || text.is_empty() {
        return false;
    }
    let has_ext = text
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, ext)| !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or(false);
    has_ext || text.contains('/') || text.contains('\\')
}

/// `F401`, `E0308`, `no-unused-vars`: código de regra, não palavra comum.
fn is_rule_code(text: &str) -> bool {
    let letters = text.chars().filter(|c| c.is_ascii_uppercase()).count();
    let digits = text.chars().filter(|c| c.is_ascii_digit()).count();
    text.len() <= 8 && digits > 0 && letters > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Saída real de `cargo build` com dois erros de tipo.
    const CARGO_BUILD: &str = r#"   Compiling fixture v0.1.0 (/tmp/fixture)
error[E0308]: mismatched types
 --> src/lib.rs:1:38
  |
1 | pub fn soma(a: i32, b: i32) -> u32 { a + b }
  |                                ---   ^^^^^ expected `u32`, found `i32`
  |                                |
  |                                expected `u32` because of return type
  |
help: you can convert an `i32` to a `u32` and panic if the converted value doesn't fit
  |
1 | pub fn soma(a: i32, b: i32) -> u32 { (a + b).try_into().unwrap() }
  |                                      +     +++++++++++++++++++++

error[E0308]: mismatched types
 --> src/lib.rs:2:38
  |
2 | pub fn oi() -> String { let x: i32 = "texto"; format!("{x}") }
  |                                ---   ^^^^^^^ expected `i32`, found `&str`

warning: unused variable: `y`
 --> src/lib.rs:5:9

For more information about this error, try `rustc --explain E0308`.
error: could not compile `fixture` (lib) due to 2 previous errors
"#;

    #[test]
    fn cargo_errors_keep_file_line_and_code() {
        let diags = extract(CARGO_BUILD);
        let (errors, warnings) = counts(&diags);
        assert_eq!(errors, 2, "diagnósticos: {diags:#?}");
        assert_eq!(warnings, 1);

        let first = &diags[0];
        assert_eq!(first.file.as_deref(), Some("src/lib.rs"));
        assert_eq!(first.line, Some(1));
        assert_eq!(first.col, Some(38));
        assert_eq!(first.code.as_deref(), Some("E0308"));
        assert_eq!(first.message, "mismatched types");
        assert_eq!(first.render(), "src/lib.rs:1:38 [E0308] mismatched types");

        assert_eq!(diags[2].severity, Severity::Warning);
        assert_eq!(diags[2].line, Some(5));
    }

    #[test]
    fn the_cargo_closing_lines_are_not_counted_as_errors() {
        // "could not compile … due to 2 previous errors" é contagem, não erro.
        let diags = extract(CARGO_BUILD);
        assert!(
            !diags
                .iter()
                .any(|d| d.message.contains("could not compile")),
            "{diags:#?}"
        );
    }

    #[test]
    fn typescript_both_shapes_are_understood() {
        let text = "src/app.ts(12,3): error TS2345: Argument of type 'string' is not assignable.\n\
                    src/b.tsx:4:1 - error TS1005: ';' expected.";
        let diags = extract(text);
        assert_eq!(diags.len(), 2, "{diags:#?}");
        assert_eq!(diags[0].file.as_deref(), Some("src/app.ts"));
        assert_eq!(diags[0].line, Some(12));
        assert_eq!(diags[0].code.as_deref(), Some("TS2345"));
        assert_eq!(diags[1].file.as_deref(), Some("src/b.tsx"));
        assert_eq!(diags[1].line, Some(4));
        assert_eq!(diags[1].code.as_deref(), Some("TS1005"));
    }

    #[test]
    fn eslint_stylish_output_binds_rows_to_the_file_header() {
        let text = "/projeto/src/a.js\n  \
                    1:7  error  'x' is assigned a value but never used  no-unused-vars\n  \
                    9:1  warning  Unexpected console statement  no-console\n\n\
                    /projeto/src/b.js\n  \
                    3:5  error  Missing semicolon  semi\n\n\
                    ✖ 3 problems (2 errors, 1 warning)";
        let diags = extract(text);
        assert_eq!(diags.len(), 3, "{diags:#?}");
        assert_eq!(diags[0].file.as_deref(), Some("/projeto/src/a.js"));
        assert_eq!(diags[0].code.as_deref(), Some("no-unused-vars"));
        assert_eq!(diags[0].line, Some(1));
        assert_eq!(diags[1].severity, Severity::Warning);
        assert_eq!(diags[2].file.as_deref(), Some("/projeto/src/b.js"));
        assert_eq!(diags[2].message, "Missing semicolon");
    }

    #[test]
    fn go_ruff_and_maven_positions_are_extracted() {
        let go = extract("./main.go:10:2: undefined: fmt.Printn");
        assert_eq!(go.len(), 1);
        assert_eq!(go[0].file.as_deref(), Some("./main.go"));
        assert_eq!(go[0].line, Some(10));
        assert_eq!(go[0].message, "undefined: fmt.Printn");

        let ruff = extract("app/main.py:1:1: F401 [*] `os` imported but unused");
        assert_eq!(ruff[0].code.as_deref(), Some("F401"));
        assert_eq!(ruff[0].message, "[*] `os` imported but unused");

        let maven = extract("[ERROR] /p/src/Foo.java:[12,5] cannot find symbol");
        assert_eq!(maven[0].file.as_deref(), Some("/p/src/Foo.java"));
        assert_eq!(maven[0].line, Some(12));
        assert_eq!(maven[0].col, Some(5));
    }

    #[test]
    fn windows_paths_with_a_drive_letter_survive() {
        let diags = extract(r"C:\projeto\src\main.go:10:2: undefined: foo");
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert_eq!(diags[0].file.as_deref(), Some(r"C:\projeto\src\main.go"));
        assert_eq!(diags[0].line, Some(10));
        assert_eq!(diags[0].col, Some(2));
    }

    #[test]
    fn prose_is_not_mistaken_for_a_diagnostic() {
        let text = "Compiling fixture v0.1.0\n\
                    note: run with `RUST_BACKTRACE=1`\n\
                    Finished dev profile in 0.26s\n\
                    algo: outra coisa qualquer";
        assert!(extract(text).is_empty(), "{:#?}", extract(text));
    }

    #[test]
    fn colored_output_is_parsed_after_stripping_ansi() {
        let painted = "\u{1b}[1m\u{1b}[31merror[E0425]\u{1b}[0m: cannot find value `x`\n \
                       \u{1b}[34m-->\u{1b}[0m src/main.rs:3:5";
        let diags = extract(painted);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert_eq!(diags[0].code.as_deref(), Some("E0425"));
        assert_eq!(diags[0].file.as_deref(), Some("src/main.rs"));
    }

    #[test]
    fn render_list_puts_errors_first_and_says_what_was_left_out() {
        let diags = extract(CARGO_BUILD);
        let text = render_list(&diags, 1);
        assert!(text.starts_with("2 erro(s) e 1 aviso(s)."), "{text}");
        assert!(text.contains("Primeiros 1:"), "{text}");
        assert!(text.contains("[erro]"), "{text}");
        assert!(text.contains("e mais 2"), "{text}");
    }

    #[test]
    fn split_position_reads_from_the_right() {
        assert_eq!(
            split_position("src/a.rs:10:5"),
            Some(("src/a.rs".into(), 10, Some(5)))
        );
        assert_eq!(
            split_position("src/a.rs:10"),
            Some(("src/a.rs".into(), 10, None))
        );
        assert_eq!(split_position("sem numero"), None);
    }
}
