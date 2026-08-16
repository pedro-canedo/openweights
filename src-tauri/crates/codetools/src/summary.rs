//! Resumo de saída de teste.
//!
//! Uma suíte média cospe centenas de linhas; uma grande, dezenas de milhares.
//! Devolver isso ao modelo tem dois custos: enche a janela (e empurra para
//! fora justamente o código que ele estava editando) e afoga o sinal — o
//! `assert` que falhou fica a 400 linhas do início.
//!
//! O que o agente precisa é sempre a mesma coisa: **quantos passaram, quantos
//! falharam, e o texto do erro de cada falha**. É isso que este módulo extrai,
//! sempre dizendo quantas linhas ficaram de fora — omissão silenciosa faria o
//! modelo concluir que a saída acabou ali.
//!
//! ## Como o parsing se mantém honesto
//!
//! A contagem vem de um varredor genérico de pares `número palavra`, porque
//! todos os runners escrevem a mesma frase com pontuação diferente:
//!
//! | runner  | linha de contagem                                |
//! |---------|--------------------------------------------------|
//! | cargo   | `test result: FAILED. 1 passed; 2 failed; 1 ignored` |
//! | jest    | `Tests:       1 failed, 2 passed, 3 total`       |
//! | vitest  | `Tests  1 failed | 3 passed (4)`                 |
//! | pytest  | `=== 2 failed, 5 passed, 1 skipped in 0.42s ===` |
//! | mocha   | `3 passing (10ms)` / `1 failing`                 |
//!
//! Um varredor só resolve as cinco. O que muda de verdade entre runners é
//! **onde ficam os blocos de falha** — e isso sim tem um extrator por família.
//!
//! ## Quando os testes nem rodaram
//!
//! Compilação quebrada não é teste vermelho, e tratar como se fosse manda o
//! agente procurar bug em código que o compilador nunca aceitou. Se não houver
//! linha de contagem e houver erro de compilação, o resumo diz isso na
//! primeira linha e lista os erros.

use crate::diagnostics::{self, Diagnostic};
use crate::text::{clip_line, line_count, tail_lines};
use std::fmt::Write as _;

/// Falhas detalhadas no resumo (as demais viram contagem).
const MAX_FAILURES: usize = 8;

/// Linhas de trecho por falha.
const MAX_EXCERPT_LINES: usize = 12;

/// Largura máxima de uma linha de trecho.
const MAX_EXCERPT_CHARS: usize = 200;

/// Erros de compilação listados quando a suíte nem chegou a rodar.
const MAX_COMPILE_ERRORS: usize = 8;

/// Linhas do fim da saída quando não reconhecemos nenhum formato.
const FALLBACK_TAIL_LINES: usize = 40;

/// Família de runner — decide de onde vêm as falhas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framework {
    Cargo,
    /// Jest, Vitest e Mocha: o mesmo relatório, com enfeites diferentes.
    JsRunner,
    Pytest,
    Go,
    Dotnet,
    Unknown,
}

impl Framework {
    /// Adivinha a família pelo comando que vai rodar.
    pub fn from_command(argv: &[String]) -> Self {
        let line = argv.join(" ").to_lowercase();
        if line.starts_with("cargo") {
            Framework::Cargo
        } else if line.contains("pytest") || line.contains("unittest") {
            Framework::Pytest
        } else if line.starts_with("go ") || line.contains("gotestsum") {
            Framework::Go
        } else if line.starts_with("dotnet") {
            Framework::Dotnet
        } else if line.contains("jest")
            || line.contains("vitest")
            || line.contains("mocha")
            || line.starts_with("npm")
            || line.starts_with("pnpm")
            || line.starts_with("yarn")
            || line.starts_with("bun")
            || line.starts_with("npx")
        {
            Framework::JsRunner
        } else {
            Framework::Unknown
        }
    }
}

/// Quantos testes em cada estado.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    /// Erros de coleta/execução que o runner conta à parte (pytest).
    pub errors: u32,
    pub found: bool,
}

impl Counts {
    fn total(&self) -> u32 {
        self.passed + self.failed + self.skipped + self.errors
    }

    fn render(&self) -> String {
        let mut parts = vec![format!("{} passaram", self.passed)];
        parts.push(format!("{} falharam", self.failed));
        if self.skipped > 0 {
            parts.push(format!("{} ignorados", self.skipped));
        }
        if self.errors > 0 {
            parts.push(format!("{} com erro", self.errors));
        }
        format!("Testes: {} (total {})", parts.join(", "), self.total())
    }
}

/// Uma falha com o trecho que explica por quê.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub name: String,
    pub excerpt: Vec<String>,
}

/// O resumo completo, antes de virar texto.
#[derive(Debug, Clone)]
pub struct TestSummary {
    pub framework: Framework,
    pub counts: Counts,
    pub failures: Vec<Failure>,
    /// Falhas contadas mas não detalhadas.
    pub extra_failures: usize,
    /// Erros de compilação, quando a suíte nem rodou.
    pub compile_errors: Vec<Diagnostic>,
    /// Linhas totais da saída original.
    pub total_lines: usize,
    /// Nota adicional (módulo ausente, nenhum teste encontrado…).
    pub note: Option<String>,
    /// Últimas linhas, usadas quando não reconhecemos nada.
    fallback: Vec<String>,
}

impl TestSummary {
    /// A suíte passou?
    ///
    /// Exige **pelo menos um teste**: uma suíte vazia sai com código 0 e sem
    /// falha nenhuma, e chamar isso de verde ensinaria o agente a concluir que
    /// está tudo certo quando na verdade nada foi verificado.
    pub fn all_green(&self) -> bool {
        self.counts.found
            && self.counts.total() > 0
            && self.counts.failed == 0
            && self.counts.errors == 0
    }
}

/// Lê a saída e monta o resumo.
pub fn summarize(framework: Framework, output: &str) -> TestSummary {
    let counts = count_tests(framework, output);
    let mut failures = failures_of(framework, output);
    let extra = failures.len().saturating_sub(MAX_FAILURES);
    failures.truncate(MAX_FAILURES);

    // Sem contagem e com erro de compilador: a suíte não rodou.
    let compile_errors = if counts.found {
        Vec::new()
    } else {
        diagnostics::extract(output)
    };

    let note = if !counts.found && compile_errors.is_empty() {
        no_tests_note(output)
    } else {
        None
    };

    TestSummary {
        framework,
        counts,
        failures,
        extra_failures: extra,
        compile_errors,
        total_lines: line_count(output),
        note,
        fallback: tail_lines(output, FALLBACK_TAIL_LINES)
            .into_iter()
            .map(|l| clip_line(l, MAX_EXCERPT_CHARS))
            .collect(),
    }
}

/// Transforma o resumo em texto para o modelo, dentro do orçamento de bytes.
pub fn render(summary: &TestSummary, budget: usize) -> String {
    let mut out = String::new();

    if !summary.compile_errors.is_empty() {
        out.push_str(
            "Os testes NÃO chegaram a rodar: o projeto não compila. Corrija os erros abaixo e \
             rode de novo.\n",
        );
        out.push_str(&diagnostics::render_list(
            &summary.compile_errors,
            MAX_COMPILE_ERRORS,
        ));
        finish(&mut out, summary, budget);
        return out;
    }

    if summary.counts.found {
        out.push_str(&summary.counts.render());
        if summary.counts.total() == 0 {
            out.push_str(
                "\nAtenção: o runner não encontrou teste nenhum. Isso NÃO é o mesmo que passar — \
                 confira o filtro e onde os testes moram.",
            );
        }
    } else if let Some(note) = &summary.note {
        out.push_str(note);
    } else {
        out.push_str(
            "Não consegui identificar a contagem de testes nesta saída; segue o fim dela:",
        );
        for line in &summary.fallback {
            out.push('\n');
            out.push_str(line);
        }
    }

    for (i, failure) in summary.failures.iter().enumerate() {
        let _ = write!(
            out,
            "\n\nFalha {}/{}: {}",
            i + 1,
            summary.failures.len() + summary.extra_failures,
            failure.name
        );
        for line in &failure.excerpt {
            out.push_str("\n  ");
            out.push_str(line);
        }
    }
    if summary.extra_failures > 0 {
        let _ = write!(
            out,
            "\n\n… e mais {} falha(s) não detalhada(s). Conserte estas primeiro e rode de novo.",
            summary.extra_failures
        );
    }

    finish(&mut out, summary, budget);
    out
}

/// Fecha o resumo: quantas linhas ficaram de fora e corte de segurança.
fn finish(out: &mut String, summary: &TestSummary, budget: usize) {
    let shown = line_count(out);
    if summary.total_lines > shown {
        let _ = write!(
            out,
            "\n\n(saída completa: {} linhas; o resumo mostra {}. Use `terminal_run` se precisar \
             do log inteiro.)",
            summary.total_lines, shown
        );
    }
    if out.len() > budget {
        let mut cut = budget.min(out.len());
        while cut > 0 && !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
        out.push_str("\n[...resumo cortado no limite de tamanho...]");
    }
}

// -------------------------------------------------------------- contagem ---

/// Soma as contagens da saída conforme a família do runner.
fn count_tests(framework: Framework, output: &str) -> Counts {
    match framework {
        Framework::Cargo => sum_lines(output, |line| line.trim_start().starts_with("test result:")),
        Framework::Go => count_go(output),
        Framework::Pytest => last_line(output, is_pytest_summary),
        Framework::JsRunner => count_js(output),
        Framework::Dotnet => last_line(output, |l| {
            let l = l.trim_start();
            l.starts_with("Passed!") || l.starts_with("Failed!") || l.starts_with("Test Run")
        }),
        // Sem família conhecida: tenta todas, na ordem do mais específico.
        Framework::Unknown => {
            let cargo = sum_lines(output, |l| l.trim_start().starts_with("test result:"));
            if cargo.found {
                return cargo;
            }
            let js = count_js(output);
            if js.found {
                return js;
            }
            let py = last_line(output, is_pytest_summary);
            if py.found {
                return py;
            }
            count_go(output)
        }
    }
}

/// Soma todas as linhas que casam (cargo roda um binário por alvo).
fn sum_lines(output: &str, matches: impl Fn(&str) -> bool) -> Counts {
    let mut total = Counts::default();
    for line in output.lines().filter(|l| matches(l)) {
        let counts = scan_counts(line);
        if counts.found {
            total.passed += counts.passed;
            total.failed += counts.failed;
            total.skipped += counts.skipped;
            total.errors += counts.errors;
            total.found = true;
        }
    }
    total
}

/// Usa a ÚLTIMA linha que casa (o rodapé é o número final).
fn last_line(output: &str, matches: impl Fn(&str) -> bool) -> Counts {
    output
        .lines()
        .filter(|l| matches(l))
        .map(scan_counts)
        .rfind(|c| c.found)
        .unwrap_or_default()
}

fn is_pytest_summary(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("=") && line.contains("=") && {
        let lower = line.to_lowercase();
        lower.contains("passed")
            || lower.contains("failed")
            || lower.contains("error")
            || lower.contains("no tests ran")
    }
}

/// Contagem do mundo JS.
///
/// Jest e Vitest imprimem um rodapé `Tests: …` — vale o último, que é o total.
/// Mocha não imprime rodapé nenhum: escreve `3 passing` e `1 failing` em
/// linhas separadas, e aí é preciso **somar** as duas. Usar a última linha nos
/// dois casos daria "0 passaram, 1 falhou" num mocha com 3 verdes.
fn count_js(output: &str) -> Counts {
    let footer = last_line(output, is_js_summary);
    if footer.found {
        return footer;
    }
    sum_lines(output, is_mocha_line)
}

/// Rodapé do jest (`Tests:  1 failed, 2 passed, 3 total`) e do vitest
/// (`Tests  1 failed | 3 passed (4)`).
fn is_js_summary(line: &str) -> bool {
    let line = line.trim_start();
    (line.starts_with("Tests:") || line.starts_with("Tests "))
        && (line.contains("passed") || line.contains("failed") || line.contains("skipped"))
}

/// Linha solta do mocha: `  3 passing (12ms)` / `  1 failing`.
fn is_mocha_line(line: &str) -> bool {
    let line = line.trim_start();
    (line.contains("passing") || line.contains("failing") || line.contains("pending"))
        && line
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
}

/// Go não imprime totais: contamos os marcadores por teste.
fn count_go(output: &str) -> Counts {
    let mut counts = Counts::default();
    for line in output.lines() {
        let line = line.trim_start();
        if line.starts_with("--- PASS:") {
            counts.passed += 1;
            counts.found = true;
        } else if line.starts_with("--- FAIL:") {
            counts.failed += 1;
            counts.found = true;
        } else if line.starts_with("--- SKIP:") {
            counts.skipped += 1;
            counts.found = true;
        }
    }
    counts
}

/// Varre pares `número palavra` e distribui nos baldes.
///
/// É o coração da contagem: funciona com `;`, `,`, `|` ou espaço como
/// separador, então serve para cargo, jest, vitest, pytest e mocha sem uma
/// regra por runner.
fn scan_counts(line: &str) -> Counts {
    let mut counts = Counts::default();
    let tokens: Vec<&str> = line
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';' || c == '|' || c == '(')
        .filter(|t| !t.is_empty())
        .collect();

    let mut i = 0;
    while i + 1 < tokens.len() {
        let Ok(value) = tokens[i].trim_matches(['=', '.', ')']).parse::<u32>() else {
            i += 1;
            continue;
        };
        let word = tokens[i + 1]
            .trim_matches(['=', '.', ',', ')', ':'])
            .to_lowercase();
        let bucket = match word.as_str() {
            "passed" | "passing" | "ok" | "succeeded" => Some(&mut counts.passed),
            "failed" | "failing" | "failures" => Some(&mut counts.failed),
            "ignored" | "skipped" | "todo" | "pending" | "deselected" | "xfailed" => {
                Some(&mut counts.skipped)
            }
            "error" | "errors" => Some(&mut counts.errors),
            _ => None,
        };
        if let Some(slot) = bucket {
            *slot += value;
            counts.found = true;
            i += 2;
        } else {
            i += 1;
        }
    }
    counts
}

/// "nenhum teste rodou" tem cara de sucesso e não é — avisa em voz alta.
fn no_tests_note(output: &str) -> Option<String> {
    let lower = output.to_lowercase();
    let empty = [
        "no tests ran",
        "no tests found",
        "no test files found",
        "0 tests",
        "testing: warning: no tests to run",
        "no test specified",
    ];
    if empty.iter().any(|m| lower.contains(m)) {
        return Some(
            "Nenhum teste foi executado — o runner não encontrou testes. Confira o filtro e o \
             caminho antes de concluir que está tudo certo."
                .to_string(),
        );
    }
    None
}

// ---------------------------------------------------------------- falhas ---

fn failures_of(framework: Framework, output: &str) -> Vec<Failure> {
    match framework {
        Framework::Cargo => cargo_failures(output),
        Framework::Pytest => pytest_failures(output),
        Framework::JsRunner => js_failures(output),
        Framework::Go => go_failures(output),
        Framework::Dotnet => dotnet_failures(output),
        Framework::Unknown => {
            let mut all = cargo_failures(output);
            if all.is_empty() {
                all = pytest_failures(output);
            }
            if all.is_empty() {
                all = js_failures(output);
            }
            if all.is_empty() {
                all = go_failures(output);
            }
            all
        }
    }
}

/// Blocos `---- nome stdout ----` do cargo.
fn cargo_failures(output: &str) -> Vec<Failure> {
    let mut out = Vec::new();
    let lines: Vec<&str> = output.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();
        let Some(name) = line
            .strip_prefix("---- ")
            .and_then(|rest| rest.strip_suffix(" ----"))
            .map(|n| n.trim_end_matches(" stdout").trim().to_string())
        else {
            i += 1;
            continue;
        };

        let mut excerpt = Vec::new();
        i += 1;
        while i < lines.len() {
            let body = lines[i];
            let trimmed = body.trim();
            if trimmed.starts_with("---- ") || trimmed == "failures:" {
                break;
            }
            // Ruído fixo do runner: não ajuda o modelo a consertar nada.
            let noise = trimmed.starts_with("note: run with `RUST_BACKTRACE")
                || trimmed.starts_with("note: Some tests");
            if !trimmed.is_empty() && !noise && excerpt.len() < MAX_EXCERPT_LINES {
                excerpt.push(clip_line(trimmed, MAX_EXCERPT_CHARS));
            }
            i += 1;
        }
        out.push(Failure { name, excerpt });
    }
    out
}

/// Resumo curto do pytest (`FAILED caminho::teste - motivo`) ou, na falta
/// dele, os blocos `____ nome ____` com as linhas `E   …`.
fn pytest_failures(output: &str) -> Vec<Failure> {
    let mut out = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        for prefix in ["FAILED ", "ERROR "] {
            if let Some(rest) = line.strip_prefix(prefix) {
                let (name, reason) = match rest.split_once(" - ") {
                    Some((n, r)) => (n.trim().to_string(), vec![clip_line(r, MAX_EXCERPT_CHARS)]),
                    None => (rest.trim().to_string(), Vec::new()),
                };
                out.push(Failure {
                    name,
                    excerpt: reason,
                });
            }
        }
    }
    if !out.is_empty() {
        return out;
    }

    // Sem "short test summary info": lê os blocos completos.
    let lines: Vec<&str> = output.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        let Some(name) = underscored_title(line) else {
            i += 1;
            continue;
        };
        let mut excerpt = Vec::new();
        i += 1;
        while i < lines.len() {
            let body = lines[i];
            let trimmed = body.trim();
            if underscored_title(trimmed).is_some() || trimmed.starts_with("====") {
                break;
            }
            // As linhas `E   …` são a asserção que quebrou.
            if (trimmed.starts_with("E ") || trimmed.starts_with('>'))
                && excerpt.len() < MAX_EXCERPT_LINES
            {
                excerpt.push(clip_line(trimmed, MAX_EXCERPT_CHARS));
            }
            i += 1;
        }
        out.push(Failure { name, excerpt });
    }
    out
}

/// `______ test_nome ______` → `test_nome`.
fn underscored_title(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if !trimmed.starts_with("___") || !trimmed.ends_with("___") {
        return None;
    }
    let name = trimmed.trim_matches('_').trim();
    (!name.is_empty() && !name.contains("___")).then(|| name.to_string())
}

/// Jest (`● suite › caso`) e Vitest (`FAIL arquivo > caso`).
///
/// O jest imprime as DUAS formas — um `FAIL arquivo` por suíte e um `●` por
/// caso — e contar as duas duplicaria cada falha. Por isso: se há bala (`●`),
/// ela manda; o cabeçalho `FAIL` só é usado quando é tudo o que existe, que é
/// o caso do vitest.
fn js_failures(output: &str) -> Vec<Failure> {
    let bullets = js_blocks(output, true);
    if !bullets.is_empty() {
        return bullets;
    }
    js_blocks(output, false)
}

fn js_blocks(output: &str, bullets_only: bool) -> Vec<Failure> {
    let mut out = Vec::new();
    let lines: Vec<&str> = output.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim();
        let name = if bullets_only {
            jest_bullet(trimmed)
        } else {
            jest_bullet(trimmed).or_else(|| vitest_header(trimmed))
        };
        let Some(name) = name else {
            i += 1;
            continue;
        };

        let mut excerpt = Vec::new();
        i += 1;
        while i < lines.len() {
            let body = lines[i];
            let trimmed = body.trim();
            let boundary = jest_bullet(trimmed).is_some()
                || vitest_header(trimmed).is_some()
                || trimmed.starts_with("Tests:")
                || trimmed.starts_with("Test Suites:")
                || trimmed.starts_with("Test Files")
                || trimmed.starts_with("Snapshots:");
            if boundary {
                break;
            }
            if !trimmed.is_empty() && excerpt.len() < MAX_EXCERPT_LINES {
                excerpt.push(clip_line(trimmed, MAX_EXCERPT_CHARS));
            }
            i += 1;
        }
        out.push(Failure { name, excerpt });
    }
    out
}

/// `● soma › soma dois números` (e não `● Console`, que é seção de log).
fn jest_bullet(line: &str) -> Option<String> {
    let rest = line.strip_prefix('●')?.trim();
    if rest.is_empty() || rest.eq_ignore_ascii_case("Console") {
        return None;
    }
    // `● Cannot find module …` é erro de suíte inteira: também interessa.
    Some(rest.to_string())
}

/// `FAIL  src/a.test.ts > soma > soma dois` (cabeçalho do vitest).
fn vitest_header(line: &str) -> Option<String> {
    let rest = line
        .strip_prefix("FAIL ")
        .or_else(|| line.strip_prefix("✕ "))
        .or_else(|| line.strip_prefix("× "))?
        .trim();
    (!rest.is_empty()).then(|| rest.to_string())
}

/// `--- FAIL: TestSoma (0.00s)` e as linhas indentadas abaixo.
fn go_failures(output: &str) -> Vec<Failure> {
    let mut out = Vec::new();
    let lines: Vec<&str> = output.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let Some(rest) = lines[i].trim_start().strip_prefix("--- FAIL: ") else {
            i += 1;
            continue;
        };
        let name = rest.split_whitespace().next().unwrap_or(rest).to_string();
        let mut excerpt = Vec::new();
        i += 1;
        while i < lines.len() {
            let trimmed = lines[i].trim();
            if trimmed.starts_with("--- ") || trimmed.starts_with("=== ") || trimmed == "FAIL" {
                break;
            }
            if !trimmed.is_empty() && excerpt.len() < MAX_EXCERPT_LINES {
                excerpt.push(clip_line(trimmed, MAX_EXCERPT_CHARS));
            }
            i += 1;
        }
        out.push(Failure { name, excerpt });
    }
    out
}

/// `  Failed NomeDoTeste [12 ms]` do `dotnet test`.
fn dotnet_failures(output: &str) -> Vec<Failure> {
    let mut out = Vec::new();
    let lines: Vec<&str> = output.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let Some(rest) = lines[i].trim_start().strip_prefix("Failed ") else {
            i += 1;
            continue;
        };
        let name = rest.split(" [").next().unwrap_or(rest).trim().to_string();
        if name.is_empty() || name.starts_with('!') {
            i += 1;
            continue;
        }
        let mut excerpt = Vec::new();
        i += 1;
        while i < lines.len() {
            let trimmed = lines[i].trim();
            if trimmed.starts_with("Failed ") || trimmed.starts_with("Passed ") {
                break;
            }
            if !trimmed.is_empty() && excerpt.len() < MAX_EXCERPT_LINES {
                excerpt.push(clip_line(trimmed, MAX_EXCERPT_CHARS));
            }
            i += 1;
        }
        out.push(Failure { name, excerpt });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Saída real de `cargo test` com duas falhas e um ignorado.
    const CARGO: &str = r#"   Compiling fixture v0.1.0 (/tmp/fixture)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.26s
     Running unittests src/lib.rs (/home/u/.cache/target/debug/deps/fixture-f06b989124248768)

running 4 tests
test tests::lento ... ignored
test tests::entra_em_panico ... FAILED
test tests::soma_negativos ... FAILED
test tests::soma_positivos ... ok

failures:

---- tests::entra_em_panico stdout ----

thread 'tests::entra_em_panico' (148481) panicked at src/lib.rs:11:28:
faltou o arquivo de config
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- tests::soma_negativos stdout ----

thread 'tests::soma_negativos' (148482) panicked at src/lib.rs:9:27:
assertion `left == right` failed
  left: -4
 right: -5


failures:
    tests::entra_em_panico
    tests::soma_negativos

test result: FAILED. 1 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s

error: test failed, to rerun pass `--lib`
"#;

    /// Saída de `jest` (que escreve tudo em stderr).
    const JEST: &str = r#"  console.log
    depurando

 FAIL  src/soma.test.js
  soma
    ✓ soma dois números (3 ms)
    ✕ soma negativos (5 ms)
    ○ skipped soma zero

  ● soma › soma negativos

    expect(received).toBe(expected) // Object.is equality

    Expected: -4
    Received: -5

      12 |   test('soma negativos', () => {
    > 13 |     expect(soma(-2, -2)).toBe(-5);
         |                          ^
      14 |   });

      at Object.toBe (src/soma.test.js:13:26)

Test Suites: 1 failed, 1 total
Tests:       1 failed, 1 skipped, 1 passed, 3 total
Snapshots:   0 total
Time:        1.204 s
"#;

    /// Saída de `pytest` com o resumo curto no fim.
    const PYTEST: &str = r#"============================= test session starts ==============================
platform linux -- Python 3.12.3, pytest-8.1.1, pluggy-1.4.0
rootdir: /projeto
collected 7 items

tests/test_soma.py .F..                                                  [ 57%]
tests/test_api.py .E.                                                    [100%]

=================================== FAILURES ===================================
_______________________________ test_soma_negativos ____________________________

    def test_soma_negativos():
>       assert soma(-2, -2) == -5
E       assert -4 == -5
E        +  where -4 = soma(-2, -2)

tests/test_soma.py:9: AssertionError
=========================== short test summary info ============================
FAILED tests/test_soma.py::test_soma_negativos - assert -4 == -5
ERROR tests/test_api.py::test_cria - ConnectionError: recusou a conexão
========================= 1 failed, 5 passed, 1 error in 0.42s =================
"#;

    fn summary_of(fw: Framework, text: &str) -> TestSummary {
        summarize(fw, text)
    }

    #[test]
    fn cargo_counts_come_from_the_result_line() {
        let s = summary_of(Framework::Cargo, CARGO);
        assert!(s.counts.found);
        assert_eq!(s.counts.passed, 1);
        assert_eq!(s.counts.failed, 2);
        assert_eq!(s.counts.skipped, 1);
        assert!(!s.all_green());
    }

    #[test]
    fn cargo_failures_carry_the_panic_message() {
        let s = summary_of(Framework::Cargo, CARGO);
        assert_eq!(s.failures.len(), 2, "{:#?}", s.failures);
        assert_eq!(s.failures[0].name, "tests::entra_em_panico");
        assert!(
            s.failures[0]
                .excerpt
                .iter()
                .any(|l| l.contains("faltou o arquivo de config")),
            "{:#?}",
            s.failures[0]
        );
        // O aviso de RUST_BACKTRACE é ruído fixo e não deve ocupar espaço.
        assert!(
            !s.failures[0]
                .excerpt
                .iter()
                .any(|l| l.contains("RUST_BACKTRACE")),
            "{:#?}",
            s.failures[0]
        );
        assert_eq!(s.failures[1].name, "tests::soma_negativos");
        let texto = s.failures[1].excerpt.join(" ");
        assert!(texto.contains("left: -4"), "{texto}");
        assert!(texto.contains("right: -5"), "{texto}");
    }

    #[test]
    fn cargo_multiple_binaries_have_their_counts_summed() {
        let text = "test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n\
                    test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured";
        let s = summary_of(Framework::Cargo, text);
        assert_eq!(s.counts.passed, 5, "duas suítes somam");
        assert_eq!(s.counts.failed, 1);
    }

    #[test]
    fn jest_counts_and_failure_are_extracted() {
        let s = summary_of(Framework::JsRunner, JEST);
        assert_eq!(s.counts.passed, 1, "{:?}", s.counts);
        assert_eq!(s.counts.failed, 1);
        assert_eq!(s.counts.skipped, 1);
        assert_eq!(s.failures.len(), 1, "{:#?}", s.failures);
        assert_eq!(s.failures[0].name, "soma › soma negativos");
        let texto = s.failures[0].excerpt.join(" ");
        assert!(texto.contains("Expected: -4"), "{texto}");
        assert!(texto.contains("Received: -5"), "{texto}");
    }

    #[test]
    fn vitest_pipe_separated_counts_work_too() {
        let text = " FAIL  src/a.test.ts > soma > negativos\nAssertionError: expected -4 to be -5\n\
                    \n Test Files  1 failed | 1 passed (2)\n      Tests  1 failed | 3 passed (4)\n";
        let s = summary_of(Framework::JsRunner, text);
        assert_eq!(s.counts.failed, 1, "{:?}", s.counts);
        assert_eq!(s.counts.passed, 3, "{:?}", s.counts);
        assert_eq!(s.failures.len(), 1);
        assert!(s.failures[0].name.contains("src/a.test.ts"));
    }

    #[test]
    fn pytest_prefers_the_short_summary_lines() {
        let s = summary_of(Framework::Pytest, PYTEST);
        assert_eq!(s.counts.passed, 5, "{:?}", s.counts);
        assert_eq!(s.counts.failed, 1);
        assert_eq!(s.counts.errors, 1);
        assert_eq!(s.failures.len(), 2, "{:#?}", s.failures);
        assert_eq!(
            s.failures[0].name,
            "tests/test_soma.py::test_soma_negativos"
        );
        assert_eq!(s.failures[0].excerpt, vec!["assert -4 == -5"]);
        assert!(s.failures[1].name.contains("test_cria"));
    }

    #[test]
    fn pytest_without_short_summary_falls_back_to_the_blocks() {
        let text = PYTEST
            .split("=========================== short test summary info")
            .next()
            .unwrap();
        let s = summary_of(Framework::Pytest, text);
        assert_eq!(s.failures.len(), 1, "{:#?}", s.failures);
        assert_eq!(s.failures[0].name, "test_soma_negativos");
        assert!(
            s.failures[0]
                .excerpt
                .iter()
                .any(|l| l.contains("assert -4 == -5")),
            "{:#?}",
            s.failures[0]
        );
    }

    #[test]
    fn go_counts_the_markers_and_keeps_the_message() {
        let text = "=== RUN   TestSoma\n--- PASS: TestSoma (0.00s)\n\
                    === RUN   TestNegativos\n    soma_test.go:12: esperava -5, veio -4\n\
                    --- FAIL: TestNegativos (0.00s)\n    soma_test.go:13: contexto extra\nFAIL\n\
                    exit status 1\nFAIL\texemplo/soma\t0.002s\n";
        let s = summary_of(Framework::Go, text);
        assert_eq!(s.counts.passed, 1, "{:?}", s.counts);
        assert_eq!(s.counts.failed, 1);
        assert_eq!(s.failures.len(), 1);
        assert_eq!(s.failures[0].name, "TestNegativos");
        assert!(
            s.failures[0]
                .excerpt
                .iter()
                .any(|l| l.contains("contexto extra")),
            "{:#?}",
            s.failures[0]
        );
    }

    #[test]
    fn a_compile_error_says_the_suite_never_ran() {
        let text = "error[E0308]: mismatched types\n --> src/lib.rs:1:38\n\
                    error: could not compile `fixture` (lib) due to 1 previous error\n";
        let s = summary_of(Framework::Cargo, text);
        assert!(!s.counts.found);
        assert_eq!(s.compile_errors.len(), 1, "{:#?}", s.compile_errors);
        let rendered = render(&s, 4_000);
        assert!(
            rendered.starts_with("Os testes NÃO chegaram a rodar"),
            "{rendered}"
        );
        assert!(rendered.contains("src/lib.rs:1:38"), "{rendered}");
    }

    #[test]
    fn zero_tests_is_flagged_instead_of_passing_silently() {
        let s = summary_of(Framework::Pytest, "===== no tests ran in 0.01s =====\n");
        let rendered = render(&s, 4_000);
        assert!(
            rendered.contains("Nenhum teste foi executado"),
            "{rendered}"
        );
    }

    #[test]
    fn a_suite_with_zero_tests_is_not_green() {
        // Sai com código 0 e sem falha nenhuma — e mesmo assim não verificou nada.
        let text = "running 0 tests\n\
                    test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n";
        let s = summary_of(Framework::Cargo, text);
        assert!(s.counts.found);
        assert!(!s.all_green(), "suíte vazia não pode contar como verde");
        let rendered = render(&s, 4_000);
        assert!(
            rendered.contains("não encontrou teste nenhum"),
            "{rendered}"
        );
    }

    #[test]
    fn a_green_run_is_short_and_says_so() {
        let text = "running 3 tests\ntest a ... ok\n\
                    test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n";
        let s = summary_of(Framework::Cargo, text);
        assert!(s.all_green());
        let rendered = render(&s, 4_000);
        assert!(
            rendered.starts_with("Testes: 3 passaram, 0 falharam"),
            "{rendered}"
        );
        assert!(
            rendered.len() < 300,
            "resumo verde tem de ser curto: {rendered}"
        );
    }

    #[test]
    fn render_reports_how_many_lines_were_left_out() {
        let s = summary_of(Framework::Cargo, CARGO);
        let rendered = render(&s, 8_000);
        assert!(rendered.contains("saída completa:"), "{rendered}");
        assert!(rendered.contains("linhas"), "{rendered}");
    }

    #[test]
    fn render_respects_the_byte_budget() {
        let mut huge = String::from("test result: FAILED. 0 passed; 40 failed; 0 ignored\n");
        for i in 0..40 {
            huge.push_str(&format!(
                "---- teste_numero_{i} stdout ----\nlinha de erro bem comprida {}\n",
                "x".repeat(300)
            ));
        }
        let s = summary_of(Framework::Cargo, &huge);
        let rendered = render(&s, 1_500);
        assert!(rendered.len() <= 1_600, "tamanho {}", rendered.len());
        assert!(rendered.contains("cortado"), "{rendered}");
    }

    #[test]
    fn only_the_first_failures_are_detailed_and_the_rest_are_counted() {
        let mut text = String::from("test result: FAILED. 0 passed; 20 failed; 0 ignored\n");
        for i in 0..20 {
            text.push_str(&format!("---- t{i} stdout ----\nquebrou em t{i}\n"));
        }
        let s = summary_of(Framework::Cargo, &text);
        assert_eq!(s.failures.len(), MAX_FAILURES);
        assert_eq!(s.extra_failures, 12);
        let rendered = render(&s, 20_000);
        assert!(rendered.contains("e mais 12 falha(s)"), "{rendered}");
    }

    #[test]
    fn framework_is_guessed_from_the_command() {
        let of = |line: &str| {
            Framework::from_command(&line.split(' ').map(str::to_string).collect::<Vec<_>>())
        };
        assert_eq!(of("cargo test"), Framework::Cargo);
        assert_eq!(of("python3 -m pytest"), Framework::Pytest);
        assert_eq!(of("go test ./..."), Framework::Go);
        assert_eq!(of("npm run test"), Framework::JsRunner);
        assert_eq!(of("npx vitest run"), Framework::JsRunner);
        assert_eq!(of("dotnet test"), Framework::Dotnet);
        assert_eq!(of("make check"), Framework::Unknown);
    }

    #[test]
    fn an_unknown_runner_still_finds_counts_and_shows_the_tail() {
        // Mocha: nenhuma família reconhecida pelo comando, formato próprio.
        let text = "  soma\n    ✓ soma dois\n    1) soma negativos\n\n  \
                    1 passing (12ms)\n  1 failing\n";
        let s = summary_of(Framework::Unknown, text);
        assert_eq!(s.counts.passed, 1, "{:?}", s.counts);
        assert_eq!(s.counts.failed, 1, "{:?}", s.counts);

        let sem_nada = summary_of(Framework::Unknown, "linha um\nlinha dois\n");
        let rendered = render(&sem_nada, 2_000);
        assert!(rendered.contains("Não consegui identificar"), "{rendered}");
        assert!(rendered.contains("linha dois"), "{rendered}");
    }
}
