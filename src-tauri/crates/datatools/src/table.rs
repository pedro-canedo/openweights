//! Tabela de texto alinhada, com corte por linhas **e** por largura.
//!
//! Todo resultado de dados passa por aqui, e o motivo é simples: uma tabela de
//! 50 mil linhas ou de 80 colunas não "fica grande demais para ler", ela
//! **estoura a janela de contexto** e derruba o passo do agente. Cortar é
//! obrigatório; o que este módulo garante é que o corte seja sempre
//! **anunciado** — quem lê precisa saber que está vendo uma parte, senão
//! conclui coisa errada sobre o conjunto todo ("só há 20 pedidos").
//!
//! São três cortes independentes:
//! 1. **por célula**: valor comprido vira `início…`;
//! 2. **por largura total**: colunas que não cabem saem, e as que saíram são
//!    nomeadas (o modelo pode pedir de novo com `SELECT` mais específico);
//! 3. **por linhas**: o resto vira "… e mais N linha(s)".

/// Quantas linhas existem de verdade por trás do que está sendo mostrado.
///
/// A diferença importa: num CSV a gente leu o arquivo todo e sabe o total; num
/// banco, contar todas as linhas de uma tabela grande custa caro, então
/// paramos assim que sabemos que **há mais** — e o aviso precisa dizer isso
/// em vez de inventar um número.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowCount {
    Exact(usize),
    AtLeast(usize),
}

/// Limites de renderização.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Máximo de linhas de dados mostradas.
    pub max_rows: usize,
    /// Máximo de caracteres por célula.
    pub max_cell: usize,
    /// Largura máxima da tabela inteira, em caracteres.
    pub max_width: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_rows: 20,
            // 60 caracteres mostram um nome, um e-mail ou o começo de um
            // texto — o suficiente para reconhecer o valor.
            max_cell: 60,
            // Cabe numa tela comum sem quebrar linha e sem virar sopa.
            max_width: 160,
        }
    }
}

impl Limits {
    pub fn with_rows(mut self, rows: usize) -> Self {
        self.max_rows = rows;
        self
    }
}

/// Corta um texto em `max` caracteres, sinalizando o corte.
pub fn clip(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let keep = max.saturating_sub(1).max(1);
    let mut out: String = text.chars().take(keep).collect();
    out.push('…');
    out
}

/// Deixa o valor apresentável numa célula: sem quebra de linha e sem tab, que
/// destruiriam o alinhamento da tabela inteira.
pub fn one_line(value: &str) -> String {
    value
        .replace("\r\n", "⏎")
        .replace(['\n', '\r'], "⏎")
        .replace('\t', "    ")
}

/// Monta a tabela. `count` diz quantas linhas existiam de verdade, para o
/// aviso de corte não mentir.
pub fn render(headers: &[String], rows: &[Vec<String>], count: RowCount, limits: Limits) -> String {
    if headers.is_empty() {
        return "(a consulta não devolveu nenhuma coluna)\n".to_string();
    }

    let shown: Vec<&Vec<String>> = rows.iter().take(limits.max_rows).collect();

    // Largura de cada coluna: o maior entre o título e os valores mostrados.
    let mut widths: Vec<usize> = headers
        .iter()
        .map(|h| clip(h, limits.max_cell).chars().count())
        .collect();
    for row in &shown {
        for (i, cell) in row.iter().enumerate().take(widths.len()) {
            let len = clip(&one_line(cell), limits.max_cell).chars().count();
            widths[i] = widths[i].max(len);
        }
    }

    // Corte por largura: mantém as primeiras colunas que couberem. Sempre
    // sobra pelo menos uma, senão a tabela não diria nada.
    let mut keep = 0usize;
    let mut used = 0usize;
    for (i, w) in widths.iter().enumerate() {
        let extra = if i == 0 { *w } else { w + 3 }; // " | "
        if i > 0 && used + extra > limits.max_width {
            break;
        }
        used += extra;
        keep = i + 1;
    }
    let dropped: Vec<&String> = headers.iter().skip(keep).collect();

    let mut out = String::new();
    let cabecalho: Vec<String> = headers
        .iter()
        .take(keep)
        .enumerate()
        .map(|(i, h)| pad(&clip(h, limits.max_cell), widths[i]))
        .collect();
    out.push_str(&cabecalho.join(" | "));
    out.push('\n');
    out.push_str(
        &widths
            .iter()
            .take(keep)
            .map(|w| "-".repeat(*w))
            .collect::<Vec<_>>()
            .join("-+-"),
    );
    out.push('\n');

    for row in &shown {
        let linha: Vec<String> = (0..keep)
            .map(|i| {
                let cell = row.get(i).map(String::as_str).unwrap_or("");
                pad(&clip(&one_line(cell), limits.max_cell), widths[i])
            })
            .collect();
        out.push_str(linha.join(" | ").trim_end());
        out.push('\n');
    }

    match count {
        RowCount::Exact(total) if total > shown.len() => out.push_str(&format!(
            "... e mais {} linha(s) não mostradas (de {total} no total).\n",
            total - shown.len()
        )),
        RowCount::AtLeast(total) if total > shown.len() => out.push_str(&format!(
            "... há mais linhas além destas {} (o resultado foi cortado). Refine com WHERE, \
             agregue com GROUP BY ou aumente `max_rows`.\n",
            shown.len()
        )),
        _ => {}
    }
    if !dropped.is_empty() {
        let nomes: Vec<&str> = dropped.iter().take(12).map(|s| s.as_str()).collect();
        out.push_str(&format!(
            "[{} coluna(s) omitida(s) por largura: {}{}. Peça-as explicitamente no SELECT.]\n",
            dropped.len(),
            nomes.join(", "),
            if dropped.len() > nomes.len() {
                ", ..."
            } else {
                ""
            }
        ));
    }
    out
}

fn pad(text: &str, width: usize) -> String {
    let count = text.chars().count();
    format!("{text}{}", " ".repeat(width.saturating_sub(count)))
}

/// Formata um número tirando zeros à direita — `10.0` vira `10`, `0.5` fica
/// `0.5`. Ponto como separador decimal: é o que o SQL usa e o que evita
/// confundir separador de milhar com separador decimal.
pub fn fmt_num(value: f64) -> String {
    if value.is_nan() {
        return "?".into();
    }
    if value == value.trunc() && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    let mut s = format!("{value:.4}");
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linhas(rows: &[[&str; 2]]) -> Vec<Vec<String>> {
        rows.iter()
            .map(|r| r.iter().map(|s| s.to_string()).collect())
            .collect()
    }

    fn cabecalhos(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn columns_are_aligned() {
        let rows = linhas(&[["1", "Ana"], ["200", "Bernardo"]]);
        let text = render(
            &cabecalhos(&["id", "nome"]),
            &rows,
            RowCount::Exact(2),
            Limits::default(),
        );
        let l: Vec<&str> = text.lines().collect();
        // O `|` da segunda coluna cai na mesma coluna em todas as linhas.
        let bar = |s: &str| s.find('|').unwrap();
        assert_eq!(bar(l[0]), bar(l[2]));
        assert_eq!(bar(l[0]), bar(l[3]));
        assert!(l[1].contains("-+-"), "{}", l[1]);
    }

    #[test]
    fn long_cells_are_clipped() {
        let rows = linhas(&[["1", &"x".repeat(200)]]);
        let limits = Limits {
            max_cell: 10,
            ..Default::default()
        };
        let text = render(
            &cabecalhos(&["id", "texto"]),
            &rows,
            RowCount::Exact(1),
            limits,
        );
        assert!(text.contains('…'), "{text}");
        for line in text.lines() {
            assert!(line.chars().count() < 40, "linha larga demais: {line}");
        }
    }

    #[test]
    fn extra_rows_are_announced_with_the_real_total() {
        let rows: Vec<Vec<String>> = (0..50)
            .map(|i| vec![i.to_string(), format!("nome{i}")])
            .collect();
        let text = render(
            &cabecalhos(&["id", "nome"]),
            &rows,
            RowCount::Exact(1234),
            Limits::default().with_rows(5),
        );
        assert_eq!(text.lines().filter(|l| l.starts_with("nome")).count(), 0);
        assert!(text.contains("e mais 1229 linha(s)"), "{text}");
        assert!(text.contains("de 1234 no total"), "{text}");
    }

    /// Quando não sabemos o total, o aviso não pode inventar um número.
    #[test]
    fn an_unknown_total_is_announced_without_a_number() {
        let rows: Vec<Vec<String>> = (0..6)
            .map(|i| vec![i.to_string(), format!("nome{i}")])
            .collect();
        let text = render(
            &cabecalhos(&["id", "nome"]),
            &rows,
            RowCount::AtLeast(6),
            Limits::default().with_rows(5),
        );
        assert!(text.contains("há mais linhas além destas 5"), "{text}");
        assert!(!text.contains("no total"), "{text}");
    }

    #[test]
    fn columns_that_do_not_fit_are_dropped_by_name() {
        let headers: Vec<String> = (0..10).map(|i| format!("coluna_longa_{i}")).collect();
        let row: Vec<String> = (0..10).map(|i| format!("valor_{i}")).collect();
        let limits = Limits {
            max_width: 50,
            ..Default::default()
        };
        let text = render(&headers, &[row], RowCount::Exact(1), limits);
        assert!(text.contains("coluna(s) omitida(s) por largura"), "{text}");
        assert!(text.contains("coluna_longa_9"), "{text}");
        for line in text.lines().take(3) {
            assert!(line.chars().count() <= 60, "{line}");
        }
    }

    #[test]
    fn newlines_inside_a_value_do_not_break_the_layout() {
        let rows = linhas(&[["1", "primeira\nsegunda"]]);
        let text = render(
            &cabecalhos(&["id", "texto"]),
            &rows,
            RowCount::Exact(1),
            Limits::default(),
        );
        // Cabeçalho, régua, uma linha de dados: nada a mais.
        assert_eq!(text.lines().count(), 3, "{text}");
        assert!(text.contains("primeira⏎segunda"), "{text}");
    }

    #[test]
    fn a_query_without_columns_says_so() {
        assert!(render(&[], &[], RowCount::Exact(0), Limits::default()).contains("nenhuma coluna"));
    }

    #[test]
    fn numbers_are_formatted_without_noise() {
        assert_eq!(fmt_num(10.0), "10");
        assert_eq!(fmt_num(0.5), "0.5");
        assert_eq!(fmt_num(617.53), "617.53");
        assert_eq!(fmt_num(1.0 / 3.0), "0.3333");
    }
}
