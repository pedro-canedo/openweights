//! Diff unificado para a prévia de edições.
//!
//! É o que a pessoa lê na tela de confirmação antes de deixar o agente
//! gravar um arquivo. Precisa ser: honesto (mostra exatamente o que muda),
//! curto (a tela não é um editor) e barato (roda antes de cada edição).

use similar::TextDiff;

/// Linhas de contexto ao redor de cada trecho alterado.
const CONTEXT_LINES: usize = 3;

/// Teto do texto do diff. Acima disso a leitura já não ajuda ninguém e
/// ainda por cima entope o prompt do modelo.
pub const MAX_DIFF_CHARS: usize = 8_000;

/// Acima deste tamanho somado nem tentamos comparar linha a linha: em
/// arquivos gigantes o algoritmo fica caro e o resultado, ilegível.
const MAX_INPUT_BYTES: usize = 2 * 1024 * 1024;

/// Gera um diff unificado curto entre dois conteúdos.
///
/// Devolve string vazia quando não há mudança nenhuma — quem chama usa isso
/// para dizer "nada a fazer" em vez de mostrar uma prévia em branco.
pub fn unified(path: &str, before: &str, after: &str) -> String {
    if before == after {
        return String::new();
    }

    if before.len() + after.len() > MAX_INPUT_BYTES {
        return format!(
            "--- a/{path}\n+++ b/{path}\n[arquivo grande demais para comparar linha a linha: {} bytes antes, {} bytes depois]\n",
            before.len(),
            after.len()
        );
    }

    let diff = TextDiff::from_lines(before, after);
    let text = diff
        .unified_diff()
        .context_radius(CONTEXT_LINES)
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string();

    truncate_diff(&text)
}

/// Corta o diff no teto avisando quantas linhas ficaram de fora.
fn truncate_diff(text: &str) -> String {
    if text.len() <= MAX_DIFF_CHARS {
        return text.to_string();
    }

    // Corta em fronteira de linha (e de caractere) para não partir UTF-8
    // nem deixar meia linha de diff na tela.
    let mut cut = MAX_DIFF_CHARS.min(text.len());
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let head = &text[..cut];
    let head = match head.rfind('\n') {
        Some(pos) => &head[..=pos],
        None => head,
    };

    let shown = head.lines().count();
    let total = text.lines().count();
    format!("{head}[...diff cortado: mostrando {shown} de {total} linhas...]\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_change_returns_empty() {
        assert_eq!(unified("a.txt", "igual\n", "igual\n"), "");
        assert_eq!(unified("a.txt", "", ""), "");
    }

    #[test]
    fn creation_shows_every_line_as_added() {
        let d = unified("src/novo.rs", "", "fn main() {}\n");
        assert!(d.contains("--- a/src/novo.rs"), "cabeçalho a/: {d}");
        assert!(d.contains("+++ b/src/novo.rs"), "cabeçalho b/: {d}");
        assert!(d.contains("+fn main() {}"), "linha adicionada: {d}");
        assert!(!d.contains("-fn main"), "não deveria remover nada: {d}");
    }

    #[test]
    fn edit_in_the_middle_keeps_context() {
        let before = "um\ndois\ntrês\nquatro\ncinco\n";
        let after = "um\ndois\nTRÊS\nquatro\ncinco\n";
        let d = unified("n.txt", before, after);
        assert!(d.contains("-três"), "{d}");
        assert!(d.contains("+TRÊS"), "{d}");
        // Contexto: as vizinhas aparecem sem sinal de + ou -.
        assert!(d.contains(" dois"), "{d}");
        assert!(d.contains(" quatro"), "{d}");
        assert!(d.contains("@@"), "deveria ter cabeçalho de trecho: {d}");
    }

    #[test]
    fn far_apart_lines_are_not_all_included() {
        // 200 linhas, muda só a primeira e a última: o meio fica de fora.
        let before: String = (0..200).map(|i| format!("linha {i}\n")).collect();
        let mut after: Vec<String> = (0..200).map(|i| format!("linha {i}\n")).collect();
        after[0] = "PRIMEIRA\n".into();
        after[199] = "ÚLTIMA\n".into();
        let after: String = after.concat();

        let d = unified("g.txt", &before, &after);
        assert!(d.contains("+PRIMEIRA"), "{d}");
        assert!(d.contains("+ÚLTIMA"), "{d}");
        assert!(!d.contains("linha 100"), "o miolo não deveria entrar: {d}");
    }

    #[test]
    fn long_diff_is_truncated_with_a_notice() {
        let before = String::new();
        let after: String = (0..5_000)
            .map(|i| format!("linha de teste {i}\n"))
            .collect();
        let d = unified("grande.txt", &before, &after);
        assert!(d.len() <= MAX_DIFF_CHARS + 200, "tamanho: {}", d.len());
        assert!(d.contains("diff cortado"), "deveria avisar do corte");
        assert!(d.ends_with('\n'));
    }

    #[test]
    fn truncation_keeps_utf8_valid() {
        let after: String = (0..5_000)
            .map(|i| format!("çãõé acentuado {i}\n"))
            .collect();
        let d = unified("acentos.txt", "", &after);
        assert!(std::str::from_utf8(d.as_bytes()).is_ok());
    }

    #[test]
    fn huge_inputs_get_a_summary_instead_of_a_diff() {
        let before = "x".repeat(MAX_INPUT_BYTES);
        let after = format!("{before}y");
        let d = unified("enorme.bin", &before, &after);
        assert!(d.contains("grande demais"), "{d}");
    }
}
