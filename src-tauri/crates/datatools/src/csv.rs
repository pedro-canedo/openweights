//! Leitura de CSV e inferência de tipo por coluna.
//!
//! Escrevemos o analisador à mão em vez de trazer uma dependência nova porque
//! o formato é pequeno (RFC 4180 cabe numa máquina de estados de cinquenta
//! linhas) e porque o que realmente decide a qualidade aqui não é o
//! analisador, é o que se faz com o resultado: inferir tipo, resumir e cortar.
//!
//! O que ele trata, porque é o que aparece em CSV de verdade:
//! - **vírgula dentro de aspas** (`"São Paulo, SP"`) e **aspas escapadas**
//!   (`""`), que é como o Excel grava;
//! - **quebra de linha dentro do campo**, também entre aspas;
//! - **CRLF** do Windows e **BOM** do UTF-8 (o BOM sem tratamento vira parte
//!   do nome da primeira coluna e estraga toda consulta que a mencione);
//! - **separador `;`**, padrão do Excel em português — detectado, não
//!   configurado, porque quem pede uma análise não sabe (nem deveria saber)
//!   qual separador o arquivo usa;
//! - **linhas com menos ou mais campos** que o cabeçalho: completamos ou
//!   guardamos o excedente em vez de recusar o arquivo inteiro.

/// Separadores que tentamos reconhecer, em ordem de preferência.
const CANDIDATES: [char; 4] = [',', ';', '\t', '|'];

/// Tipo inferido de uma coluna.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnType {
    /// Todos os valores vazios.
    Empty,
    Integer,
    Real,
    Boolean,
    /// `AAAA-MM-DD`, `DD/MM/AAAA` e as mesmas com hora.
    Date,
    Text,
}

impl ColumnType {
    pub fn label(self) -> &'static str {
        match self {
            ColumnType::Empty => "vazia",
            ColumnType::Integer => "inteiro",
            ColumnType::Real => "decimal",
            ColumnType::Boolean => "booleano",
            ColumnType::Date => "data",
            ColumnType::Text => "texto",
        }
    }

    /// Tipo declarado na tabela SQLite temporária.
    ///
    /// Data e booleano ficam como texto de propósito: converter mudaria o que
    /// o usuário vê no resultado, e data em `AAAA-MM-DD` já ordena e compara
    /// corretamente como texto.
    pub fn sql_type(self) -> &'static str {
        match self {
            ColumnType::Integer => "INTEGER",
            ColumnType::Real => "REAL",
            _ => "TEXT",
        }
    }
}

/// Um CSV lido.
#[derive(Debug, Clone)]
pub struct Csv {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub delimiter: char,
    /// Paramos de ler porque bateu o teto de linhas.
    pub row_limit_hit: bool,
}

impl Csv {
    /// Tipo de cada coluna, na ordem do cabeçalho.
    pub fn column_types(&self) -> Vec<ColumnType> {
        (0..self.headers.len())
            .map(|i| {
                let valores: Vec<&str> = self
                    .rows
                    .iter()
                    .filter_map(|r| r.get(i).map(String::as_str))
                    .collect();
                infer_type(&valores)
            })
            .collect()
    }

    /// Valor de uma célula (vazio quando a linha é curta).
    pub fn cell(&self, row: usize, col: usize) -> &str {
        self.rows
            .get(row)
            .and_then(|r| r.get(col))
            .map(String::as_str)
            .unwrap_or("")
    }
}

/// Descobre o separador olhando a primeira linha (fora de aspas).
///
/// Vence quem aparece mais vezes; empate fica com a vírgula. Uma linha só
/// basta: é o cabeçalho, e cabeçalho com separador diferente do resto do
/// arquivo não existe na prática.
pub fn detect_delimiter(text: &str) -> char {
    let first: String = text
        .chars()
        .scan(false, |quoted, c| {
            if c == '"' {
                *quoted = !*quoted;
            }
            if c == '\n' && !*quoted {
                return None;
            }
            Some((c, *quoted))
        })
        .filter(|(_, quoted)| !*quoted)
        .map(|(c, _)| c)
        .collect();

    let mut best = (',', 0usize);
    for cand in CANDIDATES {
        let n = first.matches(cand).count();
        if n > best.1 {
            best = (cand, n);
        }
    }
    best.0
}

/// Lê o CSV inteiro (até `max_rows` linhas de dados).
pub fn parse(text: &str, max_rows: usize) -> Csv {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let delimiter = detect_delimiter(text);
    let (mut records, row_limit_hit) = split_records(text, delimiter, max_rows);

    let headers = if records.is_empty() {
        Vec::new()
    } else {
        let raw = records.remove(0);
        name_columns(&raw)
    };

    Csv {
        headers,
        rows: records,
        delimiter,
        row_limit_hit,
    }
}

/// Máquina de estados do RFC 4180. Devolve os registros e se o teto bateu.
fn split_records(text: &str, delimiter: char, max_rows: usize) -> (Vec<Vec<String>>, bool) {
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    // Guarda se o registro corrente tem qualquer conteúdo — uma linha em
    // branco no fim do arquivo não é um registro vazio.
    let mut touched = false;

    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }

        match c {
            '"' if field.is_empty() => {
                in_quotes = true;
                touched = true;
            }
            // Aspas no meio de um campo já iniciado não abrem citação: é
            // texto solto (`5" de tela`), e recusar seria pior que aceitar.
            '"' => field.push('"'),
            c if c == delimiter => {
                record.push(std::mem::take(&mut field));
                touched = true;
            }
            '\r' => {
                // Só existe como parte do CRLF; sozinho, ignoramos.
                if chars.peek() == Some(&'\n') {
                    continue;
                }
            }
            '\n' => {
                record.push(std::mem::take(&mut field));
                if touched || record.len() > 1 || !record[0].is_empty() {
                    records.push(std::mem::take(&mut record));
                    // Cabeçalho + `max_rows` linhas de dados. O aviso de corte
                    // só vale se ainda sobrou arquivo para ler.
                    if records.len() > max_rows {
                        return (records, chars.peek().is_some());
                    }
                } else {
                    record.clear();
                }
                touched = false;
            }
            c => {
                field.push(c);
                touched = true;
            }
        }
    }

    if touched || !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    (records, false)
}

/// Nomes de coluna utilizáveis em SQL: sem vazio e sem repetição.
///
/// Coluna sem nome (`,,`) vira `coluna_3`; nome repetido ganha sufixo. Sem
/// isso o `CREATE TABLE` falharia e o agente ficaria sem saber por quê.
pub fn name_columns(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    for (i, name) in raw.iter().enumerate() {
        let base = name.trim();
        let mut candidate = if base.is_empty() {
            format!("coluna_{}", i + 1)
        } else {
            base.to_string()
        };
        let mut n = 2;
        while out.iter().any(|existing| existing == &candidate) {
            candidate = format!("{}_{n}", base.trim());
            n += 1;
        }
        out.push(candidate);
    }
    out
}

/// Um valor "vazio" para efeito de análise.
pub fn is_blank(value: &str) -> bool {
    value.trim().is_empty()
}

/// Inteiro, aceitando sinal e separador de milhar ausente.
pub fn as_integer(value: &str) -> Option<i64> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    v.parse::<i64>().ok()
}

/// Decimal, aceitando vírgula como separador decimal — é assim que o Excel em
/// português grava, e tratar `3,5` como texto tornaria a coluna inútil.
pub fn as_real(value: &str) -> Option<f64> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    if let Ok(n) = v.parse::<f64>() {
        return n.is_finite().then_some(n);
    }
    // Só troca a vírgula quando ela é claramente decimal (uma só, sem ponto).
    if v.matches(',').count() == 1 && !v.contains('.') {
        return v
            .replace(',', ".")
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite());
    }
    None
}

fn is_boolean(value: &str) -> bool {
    matches!(
        value.trim().to_lowercase().as_str(),
        "true" | "false" | "sim" | "nao" | "não" | "verdadeiro" | "falso"
    )
}

/// `AAAA-MM-DD` ou `DD/MM/AAAA`, com hora opcional depois.
pub fn is_date(value: &str) -> bool {
    let v = value.trim();
    let date_part = v.split([' ', 'T']).next().unwrap_or("");
    let iso = date_part.split('-').collect::<Vec<_>>();
    if iso.len() == 3
        && iso[0].len() == 4
        && iso
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    {
        return true;
    }
    let br = date_part.split('/').collect::<Vec<_>>();
    br.len() == 3
        && br[2].len() == 4
        && br
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// Tipo de uma coluna a partir dos valores dela.
///
/// Um único valor fora do padrão derruba a coluna para texto: é o
/// comportamento certo, porque tratar `"n/d"` como número faria a média
/// mentir. Vazios não contam — coluna com buraco continua sendo do tipo dos
/// valores que tem.
pub fn infer_type(values: &[&str]) -> ColumnType {
    let preenchidos: Vec<&str> = values.iter().copied().filter(|v| !is_blank(v)).collect();
    if preenchidos.is_empty() {
        return ColumnType::Empty;
    }
    if preenchidos.iter().all(|v| as_integer(v).is_some()) {
        return ColumnType::Integer;
    }
    if preenchidos.iter().all(|v| as_real(v).is_some()) {
        return ColumnType::Real;
    }
    if preenchidos.iter().all(|v| is_boolean(v)) {
        return ColumnType::Boolean;
    }
    if preenchidos.iter().all(|v| is_date(v)) {
        return ColumnType::Date;
    }
    ColumnType::Text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_simple_file() {
        let csv = parse("a,b\n1,2\n3,4\n", 100);
        assert_eq!(csv.headers, ["a", "b"]);
        assert_eq!(csv.rows.len(), 2);
        assert_eq!(csv.rows[1], ["3", "4"]);
        assert_eq!(csv.delimiter, ',');
    }

    #[test]
    fn a_comma_inside_quotes_stays_in_the_field() {
        let csv = parse("cidade,uf\n\"São Paulo, SP\",SP\n", 100);
        assert_eq!(csv.rows[0][0], "São Paulo, SP");
        assert_eq!(csv.rows[0][1], "SP");
    }

    #[test]
    fn escaped_quotes_become_one_quote() {
        let csv = parse("frase\n\"ela disse \"\"oi\"\" e saiu\"\n", 100);
        assert_eq!(csv.rows[0][0], "ela disse \"oi\" e saiu");
    }

    #[test]
    fn a_newline_inside_quotes_does_not_end_the_row() {
        let csv = parse("id,obs\n1,\"linha um\nlinha dois\"\n2,ok\n", 100);
        assert_eq!(csv.rows.len(), 2);
        assert_eq!(csv.rows[0][1], "linha um\nlinha dois");
        assert_eq!(csv.rows[1][0], "2");
    }

    #[test]
    fn crlf_and_bom_are_handled() {
        let csv = parse("\u{feff}a,b\r\n1,2\r\n", 100);
        assert_eq!(
            csv.headers,
            ["a", "b"],
            "o BOM não pode virar parte do nome"
        );
        assert_eq!(csv.rows[0], ["1", "2"]);
    }

    #[test]
    fn empty_fields_and_empty_column_names_survive() {
        let csv = parse("a,,c\n1,,3\n", 100);
        assert_eq!(csv.headers, ["a", "coluna_2", "c"]);
        assert_eq!(csv.rows[0], ["1", "", "3"]);
    }

    #[test]
    fn repeated_column_names_are_made_unique() {
        let csv = parse("nome,nome,nome\n1,2,3\n", 100);
        assert_eq!(csv.headers, ["nome", "nome_2", "nome_3"]);
    }

    #[test]
    fn the_semicolon_delimiter_is_detected() {
        let csv = parse("nome;valor\nAna;3,5\n", 100);
        assert_eq!(csv.delimiter, ';');
        assert_eq!(csv.headers, ["nome", "valor"]);
        assert_eq!(csv.rows[0], ["Ana", "3,5"]);
    }

    #[test]
    fn short_and_long_rows_do_not_break_anything() {
        let csv = parse("a,b,c\n1\n1,2,3,4\n", 100);
        assert_eq!(csv.rows[0], ["1"]);
        assert_eq!(csv.rows[1].len(), 4);
        assert_eq!(csv.cell(0, 2), "", "linha curta devolve vazio");
    }

    #[test]
    fn the_row_limit_is_reported() {
        let text = "a\n".to_string() + &"1\n".repeat(50);
        let csv = parse(&text, 10);
        assert!(csv.row_limit_hit);
        assert_eq!(csv.rows.len(), 10);
    }

    #[test]
    fn trailing_blank_lines_are_not_rows() {
        let csv = parse("a,b\n1,2\n\n", 100);
        assert_eq!(csv.rows.len(), 1);
    }

    #[test]
    fn types_are_inferred_per_column() {
        assert_eq!(infer_type(&["1", "2", "-3"]), ColumnType::Integer);
        assert_eq!(infer_type(&["1.5", "2", ""]), ColumnType::Real);
        assert_eq!(infer_type(&["3,5", "1,25"]), ColumnType::Real);
        assert_eq!(infer_type(&["sim", "NÃO"]), ColumnType::Boolean);
        assert_eq!(infer_type(&["2024-01-02", "2024-12-30"]), ColumnType::Date);
        assert_eq!(infer_type(&["02/01/2024"]), ColumnType::Date);
        assert_eq!(
            infer_type(&["2024-01-02 10:30", "2024-03-01T08:00"]),
            ColumnType::Date
        );
        assert_eq!(infer_type(&["", "  "]), ColumnType::Empty);
    }

    #[test]
    fn one_stray_value_turns_the_column_into_text() {
        // Se `n/d` virasse zero, a média mentiria.
        assert_eq!(infer_type(&["1", "2", "n/d"]), ColumnType::Text);
        assert_eq!(infer_type(&["2024-01-02", "ontem"]), ColumnType::Text);
    }

    #[test]
    fn column_types_follow_the_header_order() {
        let csv = parse("id,nome,valor\n1,Ana,3.5\n2,Bruno,4\n", 100);
        assert_eq!(
            csv.column_types(),
            [ColumnType::Integer, ColumnType::Text, ColumnType::Real]
        );
    }
}
