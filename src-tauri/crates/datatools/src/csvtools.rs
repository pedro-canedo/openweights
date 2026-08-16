//! `csv_preview`, `csv_query` e `data_summary`.
//!
//! Planilha exportada em CSV é o segundo formato de trabalho mais comum depois
//! de código, e é onde o agente mais erra sem ajuda: ele "lê o arquivo" com
//! `fs_read`, recebe cinquenta mil linhas, enche o contexto e ainda responde
//! errado porque somou uma coluna que tinha um `n/d` no meio.
//!
//! As três ferramentas atacam isso na ordem em que a dúvida aparece:
//! *como é este arquivo* (`csv_preview`), *o que ele diz no todo*
//! (`data_summary`) e *responda esta pergunta específica* (`csv_query`).
//!
//! **Nunca devolvemos o arquivo inteiro.** Toda saída é amostra ou resumo, e
//! toda amostra diz quantas linhas ficaram de fora. Essa é a regra que impede
//! o agente de se estrangular sozinho — e a razão de `csv_query` existir:
//! responder "qual o total por categoria?" com uma tabela de dez linhas em vez
//! de cinquenta mil.
//!
//! O `csv_query` carrega o arquivo numa tabela SQLite **em memória**. Não há
//! banco em disco, não há arquivo criado, e a tabela chama-se sempre `dados`
//! (com `csv` como apelido, porque é o nome que os modelos tentam primeiro).

use crate::csv::{self, ColumnType, Csv};
use crate::guard;
use crate::sqlite::{harden, run_statement};
use crate::table::{self, Limits, RowCount};
use async_trait::async_trait;
use lr_tools::{Tool, ToolContext, ToolError, ToolOutput, ToolResult, arg_str, arg_u64};
use lr_types::agent::ToolCategory;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::HashMap;

/// Nome da tabela criada a partir do CSV.
pub const TABLE_NAME: &str = "dados";

/// Teto de bytes lidos de um CSV. Acima disso lemos só o começo e avisamos —
/// melhor uma resposta sobre parte do arquivo, dizendo que é parte, do que uma
/// recusa que deixa o agente sem saída.
const MAX_BYTES: usize = 16 * 1024 * 1024;

/// Teto de linhas guardadas na memória de uma vez.
const MAX_ROWS_PARSED: usize = 200_000;

/// Linhas mostradas por `csv_preview` quando ninguém pede um número.
const DEFAULT_PREVIEW_ROWS: u64 = 10;
const MAX_PREVIEW_ROWS: u64 = 100;

/// Valores mais comuns listados por coluna de texto no resumo.
const TOP_VALUES: usize = 3;

/// Acima disto a coluna é considerada "quase toda distinta" (um id) e a
/// contagem exata deixa de ser informativa.
const MAX_DISTINCT: usize = 1_000;

/// Colunas descritas no resumo antes de cortar.
const MAX_SUMMARY_COLUMNS: usize = 60;

/// Um CSV lido do projeto, com o que foi cortado na leitura.
pub struct LoadedCsv {
    pub rel: String,
    pub csv: Csv,
    pub bytes_cut: bool,
}

impl LoadedCsv {
    /// Aviso de corte, se houve.
    fn cut_notice(&self) -> String {
        if self.bytes_cut {
            format!(
                "\n[Arquivo maior que {} MB: li apenas o começo. Os números abaixo valem só para \
                 a parte lida.]\n",
                MAX_BYTES / (1024 * 1024)
            )
        } else if self.csv.row_limit_hit {
            format!(
                "\n[Arquivo com mais de {MAX_ROWS_PARSED} linhas: li apenas as primeiras. Os \
                 números abaixo valem só para a parte lida.]\n"
            )
        } else {
            String::new()
        }
    }
}

/// Lê e interpreta um CSV do projeto.
pub fn load(args: &Value, ctx: &ToolContext) -> ToolResult<LoadedCsv> {
    let rel = arg_str(args, "path")?;
    let path = ctx.resolve(&rel)?;
    if !path.exists() {
        return Err(ToolError::NotFound(rel));
    }
    if path.is_dir() {
        return Err(ToolError::InvalidArgs(format!(
            "`{rel}` é uma pasta. Aponte o arquivo .csv (procure com `fs_glob`)."
        )));
    }

    let bytes = std::fs::read(&path)?;
    let bytes_cut = bytes.len() > MAX_BYTES;
    let fatia = if bytes_cut {
        &bytes[..MAX_BYTES]
    } else {
        &bytes[..]
    };
    // `from_utf8_lossy` em vez de recusar: CSV exportado em Latin-1 é comum, e
    // trocar um acento por `<?>` é melhor do que travar a análise inteira.
    let texto = String::from_utf8_lossy(fatia);

    let mut csv = csv::parse(&texto, MAX_ROWS_PARSED);
    if bytes_cut && !csv.rows.is_empty() {
        // A última linha do corte quase certamente ficou pela metade.
        csv.rows.pop();
    }

    if csv.headers.is_empty() {
        return Err(ToolError::Other(format!(
            "`{rel}` está vazio — não há nem cabeçalho para ler."
        )));
    }
    Ok(LoadedCsv {
        rel,
        csv,
        bytes_cut,
    })
}

fn path_inside(args: &Value, ctx: &ToolContext) -> bool {
    match arg_str(args, "path") {
        Ok(rel) => ctx.resolve(&rel).is_ok(),
        Err(_) => true,
    }
}

/// Cabeçalho comum: arquivo, tamanho e separador.
fn header_line(loaded: &LoadedCsv) -> String {
    let sep = match loaded.csv.delimiter {
        '\t' => "tabulação".to_string(),
        c => format!("\"{c}\""),
    };
    format!(
        "Arquivo: {} — {} linha(s) de dados, {} coluna(s), separador {sep}.\n",
        loaded.rel,
        loaded.csv.rows.len(),
        loaded.csv.headers.len()
    )
}

// ----------------------------------------------------------- csv_preview ---

/// Ferramenta `csv_preview`.
pub struct CsvPreview;

#[async_trait]
impl Tool for CsvPreview {
    fn name(&self) -> &str {
        "csv_preview"
    }

    fn description(&self) -> &str {
        "Mostra as primeiras linhas de um arquivo CSV com o tipo de cada coluna (inteiro, \
         decimal, data, texto). Use antes de qualquer análise: é o que revela os nomes reais das \
         colunas e o separador do arquivo. Nunca devolve o arquivo inteiro."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Caminho do arquivo .csv, relativo à raiz do projeto (ex.: dados/vendas.csv)."
                },
                "rows": {
                    "type": "integer",
                    "description": "Quantas linhas mostrar (1 a 100; padrão 10).",
                    "minimum": 1,
                    "maximum": 100
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Read
    }

    fn within_workspace(&self, args: &Value, ctx: &ToolContext) -> bool {
        path_inside(args, ctx)
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let rows = arg_u64(&args, "rows", DEFAULT_PREVIEW_ROWS).clamp(1, MAX_PREVIEW_ROWS) as usize;
        let loaded = load(&args, ctx)?;
        let tipos = loaded.csv.column_types();

        let mut body = header_line(&loaded);
        body.push_str(&loaded.cut_notice());

        body.push_str("\nColunas:\n");
        for (nome, tipo) in loaded.csv.headers.iter().zip(&tipos) {
            body.push_str(&format!("  {nome} — {}\n", tipo.label()));
        }

        body.push_str("\nPrimeiras linhas:\n");
        body.push_str(&table::render(
            &loaded.csv.headers,
            &loaded.csv.rows,
            RowCount::Exact(loaded.csv.rows.len()),
            Limits::default().with_rows(rows),
        ));
        Ok(ToolOutput::text(body).truncated_to(ctx.max_output_bytes))
    }
}

// ------------------------------------------------------------- csv_query ---

/// Ferramenta `csv_query`.
pub struct CsvQuery;

/// Cria a tabela em memória e enche com o CSV.
///
/// Colunas numéricas entram como número (e não como texto) porque é isso que
/// faz `WHERE valor > 100`, `ORDER BY` e `AVG()` funcionarem. Campo vazio vira
/// `NULL` — assim `COUNT(coluna)` conta preenchidos, que é o que se espera.
pub fn load_into_memory(csv: &Csv) -> ToolResult<Connection> {
    let conn = Connection::open_in_memory()
        .map_err(|e| ToolError::Other(format!("não consegui preparar a análise: {e}")))?;
    harden(&conn);

    let tipos = csv.column_types();
    let colunas: Vec<String> = csv
        .headers
        .iter()
        .zip(&tipos)
        .map(|(nome, tipo)| format!("{} {}", quote_ident(nome), tipo.sql_type()))
        .collect();

    let criar = format!(
        "CREATE TABLE {TABLE_NAME} ({});\nCREATE VIEW csv AS SELECT * FROM {TABLE_NAME};",
        colunas.join(", ")
    );
    conn.execute_batch(&criar).map_err(|e| {
        ToolError::Other(format!(
            "não consegui montar a tabela a partir do CSV: {e}. Confira o cabeçalho do arquivo \
             com `csv_preview`."
        ))
    })?;

    let marcadores = vec!["?"; csv.headers.len()].join(", ");
    let inserir = format!("INSERT INTO {TABLE_NAME} VALUES ({marcadores})");

    let tx = conn
        .unchecked_transaction()
        .map_err(|e| ToolError::Other(format!("não consegui carregar os dados: {e}")))?;
    {
        let mut stmt = tx
            .prepare(&inserir)
            .map_err(|e| ToolError::Other(format!("não consegui carregar os dados: {e}")))?;
        for linha in &csv.rows {
            let valores: Vec<rusqlite::types::Value> = (0..csv.headers.len())
                .map(|i| {
                    let bruto = linha.get(i).map(String::as_str).unwrap_or("");
                    to_sql_value(bruto, tipos[i])
                })
                .collect();
            stmt.execute(rusqlite::params_from_iter(valores))
                .map_err(|e| ToolError::Other(format!("não consegui carregar os dados: {e}")))?;
        }
    }
    tx.commit()
        .map_err(|e| ToolError::Other(format!("não consegui carregar os dados: {e}")))?;

    Ok(conn)
}

/// Identificador citado, com as aspas internas escapadas.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn to_sql_value(raw: &str, tipo: ColumnType) -> rusqlite::types::Value {
    use rusqlite::types::Value as V;
    if csv::is_blank(raw) {
        return V::Null;
    }
    match tipo {
        ColumnType::Integer => csv::as_integer(raw).map(V::Integer),
        ColumnType::Real => csv::as_real(raw).map(V::Real),
        _ => None,
    }
    .unwrap_or_else(|| V::Text(raw.to_string()))
}

#[async_trait]
impl Tool for CsvQuery {
    fn name(&self) -> &str {
        "csv_query"
    }

    fn description(&self) -> &str {
        "Responde uma pergunta sobre um arquivo CSV usando SQL: o arquivo é carregado numa tabela \
         temporária em memória chamada `dados` (apelido `csv`) e a consulta roda em cima dela. \
         Use para somar, agrupar, filtrar e ordenar sem despejar o arquivo inteiro — ex.: \
         SELECT categoria, SUM(valor) AS total FROM dados GROUP BY categoria ORDER BY total DESC. \
         O arquivo NÃO é modificado: só comandos de leitura são aceitos."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Caminho do arquivo .csv, relativo à raiz do projeto (ex.: dados/vendas.csv)."
                },
                "query": {
                    "type": "string",
                    "description": "Um único SELECT sobre a tabela `dados`. Use os nomes de coluna exatamente como aparecem no cabeçalho do CSV (veja com `csv_preview`)."
                },
                "max_rows": {
                    "type": "integer",
                    "description": "Máximo de linhas no resultado (1 a 500; padrão 20).",
                    "minimum": 1,
                    "maximum": 500
                }
            },
            "required": ["path", "query"],
            "additionalProperties": false
        })
    }

    /// Leitura: o CSV nunca é reescrito e o banco é temporário, em memória.
    fn category(&self) -> ToolCategory {
        ToolCategory::Read
    }

    fn within_workspace(&self, args: &Value, ctx: &ToolContext) -> bool {
        path_inside(args, ctx)
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let query = arg_str(&args, "query")?;
        guard::check(&query)?;
        let max_rows = arg_u64(&args, "max_rows", crate::sqlite::DEFAULT_MAX_ROWS)
            .clamp(1, crate::sqlite::MAX_MAX_ROWS) as usize;
        let loaded = load(&args, ctx)?;

        let cabecalho = header_line(&loaded);
        let aviso = loaded.cut_notice();
        let colunas = loaded.csv.headers.join(", ");
        let sql = query.trim().to_string();

        let corpo = tokio::task::spawn_blocking(move || {
            let conn = load_into_memory(&loaded.csv)?;
            run_statement(
                &conn,
                &sql,
                max_rows,
                // Escrever aqui não faria mal (é memória), mas aceitar
                // `UPDATE` daria a impressão falsa de que o CSV mudou.
                false,
                &format!("A tabela se chama `{TABLE_NAME}` e tem as colunas: {colunas}."),
            )
        })
        .await
        .map_err(|e| ToolError::Other(format!("a análise não pôde ser executada: {e}")))??;

        Ok(ToolOutput::text(format!("{cabecalho}{aviso}\n{corpo}"))
            .truncated_to(ctx.max_output_bytes))
    }
}

// ---------------------------------------------------------- data_summary ---

/// Ferramenta `data_summary`.
pub struct DataSummary;

/// Estatística de uma coluna.
fn describe_column(loaded: &LoadedCsv, index: usize, tipo: ColumnType) -> String {
    let total = loaded.csv.rows.len();
    let valores: Vec<&str> = (0..total).map(|r| loaded.csv.cell(r, index)).collect();
    let vazios = valores.iter().filter(|v| csv::is_blank(v)).count();
    let preenchidos: Vec<&str> = valores
        .iter()
        .copied()
        .filter(|v| !csv::is_blank(v))
        .collect();

    let mut linhas = format!("  vazios: {vazios} de {total}\n");

    match tipo {
        ColumnType::Integer | ColumnType::Real => {
            let numeros: Vec<f64> = preenchidos.iter().filter_map(|v| csv::as_real(v)).collect();
            if numeros.is_empty() {
                return linhas;
            }
            let soma: f64 = numeros.iter().sum();
            let min = numeros.iter().cloned().fold(f64::INFINITY, f64::min);
            let max = numeros.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            linhas.push_str(&format!(
                "  mínimo {} · máximo {} · média {} · soma {}\n",
                table::fmt_num(min),
                table::fmt_num(max),
                table::fmt_num(soma / numeros.len() as f64),
                table::fmt_num(soma)
            ));
        }
        ColumnType::Date => {
            let mut ordenados: Vec<&str> = preenchidos.clone();
            ordenados.sort_unstable();
            if let (Some(primeiro), Some(ultimo)) = (ordenados.first(), ordenados.last()) {
                linhas.push_str(&format!("  de {primeiro} até {ultimo}\n"));
            }
        }
        _ => {}
    }

    // Contagem de distintos e mais comuns vale para qualquer tipo: é o que
    // revela a coluna que só tem dois valores e a que é chave.
    //
    // Em mapa, não em lista: uma busca linear por valor deixaria isto
    // quadrático, e um CSV de duzentas mil linhas travaria a análise.
    if !preenchidos.is_empty() {
        let mut mapa: HashMap<&str, usize> = HashMap::new();
        for valor in &preenchidos {
            *mapa.entry(*valor).or_insert(0) += 1;
            // Coluna praticamente toda distinta (um id) não rende contagem
            // útil; paramos e dizemos isso em vez de listar mil valores.
            if mapa.len() > MAX_DISTINCT {
                linhas.push_str(&format!(
                    "  valores distintos: mais de {MAX_DISTINCT} (quase todos diferentes)\n"
                ));
                return linhas;
            }
        }
        let mut contagem: Vec<(&str, usize)> = mapa.into_iter().collect();
        // Desempate pelo próprio valor: sem isso a ordem viria do mapa e o
        // mesmo arquivo daria saídas diferentes a cada execução.
        contagem.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let mais: Vec<String> = contagem
            .iter()
            .take(TOP_VALUES)
            .map(|(v, n)| format!("\"{}\" ({n}x)", table::clip(&table::one_line(v), 40)))
            .collect();
        linhas.push_str(&format!(
            "  {} valor(es) distinto(s); mais comuns: {}\n",
            contagem.len(),
            mais.join(", ")
        ));
    }
    linhas
}

#[async_trait]
impl Tool for DataSummary {
    fn name(&self) -> &str {
        "data_summary"
    }

    fn description(&self) -> &str {
        "Resume um arquivo CSV inteiro: número de linhas e colunas, tipo de cada coluna, quantos \
         valores vazios, mínimo/máximo/média/soma das numéricas, intervalo das datas e os valores \
         mais comuns das de texto. Use para entender os dados antes de decidir a análise."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Caminho do arquivo .csv, relativo à raiz do projeto (ex.: dados/vendas.csv)."
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Read
    }

    fn within_workspace(&self, args: &Value, ctx: &ToolContext) -> bool {
        path_inside(args, ctx)
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let loaded = load(&args, ctx)?;
        let tipos = loaded.csv.column_types();

        let mut body = header_line(&loaded);
        body.push_str(&loaded.cut_notice());

        if loaded.csv.rows.is_empty() {
            body.push_str("\nO arquivo tem cabeçalho mas nenhuma linha de dados.\nColunas: ");
            body.push_str(&loaded.csv.headers.join(", "));
            body.push('\n');
            return Ok(ToolOutput::text(body));
        }

        for (i, (nome, tipo)) in loaded
            .csv
            .headers
            .iter()
            .zip(&tipos)
            .take(MAX_SUMMARY_COLUMNS)
            .enumerate()
        {
            body.push_str(&format!("\nColuna `{nome}` — {}\n", tipo.label()));
            body.push_str(&describe_column(&loaded, i, *tipo));
        }
        if loaded.csv.headers.len() > MAX_SUMMARY_COLUMNS {
            body.push_str(&format!(
                "\n[{} coluna(s) não descritas: o arquivo tem colunas demais.]\n",
                loaded.csv.headers.len() - MAX_SUMMARY_COLUMNS
            ));
        }

        Ok(ToolOutput::text(body).truncated_to(ctx.max_output_bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// CSV com tudo que costuma quebrar analisador: vírgula dentro de aspas,
    /// coluna vazia, número, data e um valor faltando.
    const VENDAS: &str = "\
id,cliente,cidade,valor,data,obs
1,Ana,\"São Paulo, SP\",100.50,2024-01-02,
2,Bruno,Recife,29.90,2024-01-03,
3,Ana,\"Belo Horizonte, MG\",250.00,2024-02-10,
4,Carla,Recife,,2024-02-11,
5,Ana,Recife,10.10,2024-03-01,
";

    fn projeto() -> (TempDir, ToolContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("dados")).unwrap();
        std::fs::write(dir.path().join("dados/vendas.csv"), VENDAS).unwrap();
        let ctx = ToolContext::new(Some(dir.path().to_path_buf()), "call-1");
        (dir, ctx)
    }

    #[tokio::test]
    async fn preview_shows_the_columns_their_types_and_a_few_rows() {
        let (_d, ctx) = projeto();
        let out = CsvPreview
            .execute(json!({"path": "dados/vendas.csv", "rows": 2}), &ctx)
            .await
            .unwrap();
        let t = &out.content;
        assert!(t.contains("5 linha(s) de dados, 6 coluna(s)"), "{t}");
        assert!(t.contains("id — inteiro"), "{t}");
        assert!(t.contains("valor — decimal"), "{t}");
        assert!(t.contains("data — data"), "{t}");
        assert!(t.contains("obs — vazia"), "{t}");
        assert!(t.contains("São Paulo, SP"), "a vírgula entre aspas: {t}");
        assert!(t.contains("e mais 3 linha(s)"), "{t}");
        assert!(!t.contains("Carla"), "só as duas primeiras: {t}");
    }

    #[tokio::test]
    async fn preview_refuses_a_path_outside_the_project() {
        let (_d, ctx) = projeto();
        let args = json!({"path": "../fora.csv"});
        assert!(!CsvPreview.within_workspace(&args, &ctx));
        let err = CsvPreview.execute(args, &ctx).await.unwrap_err();
        assert!(matches!(err, ToolError::OutsideWorkspace(_)), "{err:?}");
    }

    #[tokio::test]
    async fn a_missing_csv_says_so() {
        let (_d, ctx) = projeto();
        let err = CsvPreview
            .execute(json!({"path": "dados/naoexiste.csv"}), &ctx)
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)), "{err:?}");
        assert!(err.to_model_message().contains("Liste a pasta"));
    }

    #[tokio::test]
    async fn query_groups_and_sums_with_real_numbers() {
        let (_d, ctx) = projeto();
        let out = CsvQuery
            .execute(
                json!({
                    "path": "dados/vendas.csv",
                    "query": "SELECT cliente, SUM(valor) AS total FROM dados GROUP BY cliente ORDER BY total DESC"
                }),
                &ctx,
            )
            .await
            .unwrap();
        let t = &out.content;
        // 100.50 + 250.00 + 10.10 = 360.6 — só soma certo se a coluna virou número.
        assert!(t.contains("360.6"), "{t}");
        assert!(t.contains("Ana"), "{t}");
    }

    #[tokio::test]
    async fn an_empty_cell_becomes_null_and_does_not_count() {
        let (_d, ctx) = projeto();
        let out = CsvQuery
            .execute(
                json!({
                    "path": "dados/vendas.csv",
                    "query": "SELECT COUNT(*) AS linhas, COUNT(valor) AS com_valor FROM dados"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.content.contains('5'), "{}", out.content);
        assert!(out.content.contains('4'), "{}", out.content);
    }

    #[tokio::test]
    async fn the_csv_alias_also_works() {
        let (_d, ctx) = projeto();
        let out = CsvQuery
            .execute(
                json!({"path": "dados/vendas.csv", "query": "SELECT COUNT(*) AS n FROM csv"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.content.contains('5'), "{}", out.content);
    }

    #[tokio::test]
    async fn an_unknown_column_lists_the_real_ones() {
        let (_d, ctx) = projeto();
        let err = CsvQuery
            .execute(
                json!({"path": "dados/vendas.csv", "query": "SELECT preco FROM dados"}),
                &ctx,
            )
            .await
            .unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("cliente"), "{msg}");
        assert!(msg.contains("dados"), "{msg}");
    }

    #[tokio::test]
    async fn writing_to_the_csv_table_is_refused() {
        let (_d, ctx) = projeto();
        let err = CsvQuery
            .execute(
                json!({"path": "dados/vendas.csv", "query": "UPDATE dados SET valor = 0"}),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(err.to_model_message().contains("altera os dados"));
        // O arquivo continua intacto.
        assert_eq!(
            std::fs::read_to_string(_d.path().join("dados/vendas.csv")).unwrap(),
            VENDAS
        );
    }

    #[tokio::test]
    async fn query_results_are_limited_by_rows() {
        let (_d, ctx) = projeto();
        let out = CsvQuery
            .execute(
                json!({
                    "path": "dados/vendas.csv",
                    "query": "SELECT id FROM dados ORDER BY id",
                    "max_rows": 2
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.content.contains("há mais linhas"), "{}", out.content);
    }

    #[tokio::test]
    async fn summary_reports_nulls_min_max_mean_and_top_values() {
        let (_d, ctx) = projeto();
        let out = DataSummary
            .execute(json!({"path": "dados/vendas.csv"}), &ctx)
            .await
            .unwrap();
        let t = &out.content;
        assert!(t.contains("Coluna `valor` — decimal"), "{t}");
        assert!(t.contains("vazios: 1 de 5"), "{t}");
        assert!(t.contains("mínimo 10.1"), "{t}");
        assert!(t.contains("máximo 250"), "{t}");
        assert!(t.contains("média 97.625"), "{t}");
        assert!(t.contains("Coluna `cliente` — texto"), "{t}");
        assert!(t.contains("\"Ana\" (3x)"), "{t}");
        assert!(t.contains("de 2024-01-02 até 2024-03-01"), "{t}");
    }

    #[tokio::test]
    async fn a_header_only_file_is_summarised_without_crashing() {
        let (dir, ctx) = projeto();
        std::fs::write(dir.path().join("dados/vazio.csv"), "a,b,c\n").unwrap();
        let out = DataSummary
            .execute(json!({"path": "dados/vazio.csv"}), &ctx)
            .await
            .unwrap();
        assert!(
            out.content.contains("nenhuma linha de dados"),
            "{}",
            out.content
        );
        assert!(out.content.contains("a, b, c"), "{}", out.content);
    }

    #[tokio::test]
    async fn a_completely_empty_file_explains_itself() {
        let (dir, ctx) = projeto();
        std::fs::write(dir.path().join("dados/nada.csv"), "").unwrap();
        let err = DataSummary
            .execute(json!({"path": "dados/nada.csv"}), &ctx)
            .await
            .unwrap_err();
        assert!(err.to_model_message().contains("vazio"));
    }

    #[tokio::test]
    async fn a_semicolon_file_is_understood_without_configuration() {
        let (dir, ctx) = projeto();
        std::fs::write(
            dir.path().join("dados/br.csv"),
            "produto;preco\ncafé;12,50\nchá;8,00\n",
        )
        .unwrap();
        let out = CsvQuery
            .execute(
                json!({"path": "dados/br.csv", "query": "SELECT SUM(preco) AS total FROM dados"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.content.contains("20.5"), "{}", out.content);
    }

    #[tokio::test]
    async fn column_names_with_spaces_can_be_queried() {
        let (dir, ctx) = projeto();
        std::fs::write(
            dir.path().join("dados/espaco.csv"),
            "nome do cliente,valor total\nAna,10\n",
        )
        .unwrap();
        let out = CsvQuery
            .execute(
                json!({
                    "path": "dados/espaco.csv",
                    "query": "SELECT \"nome do cliente\" FROM dados"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.content.contains("Ana"), "{}", out.content);
    }

    /// Muitas linhas com poucas categorias e ids únicos: as categorias são
    /// contadas, os ids não. (Também é o teste que segura a contagem em tempo
    /// linear — em lista, este arquivo levaria segundos.)
    #[tokio::test]
    async fn a_high_cardinality_column_does_not_get_counted_one_by_one() {
        let (dir, ctx) = projeto();
        let mut texto = String::from("id,categoria\n");
        for i in 0..20_000 {
            texto.push_str(&format!("id-{i},cat{}\n", i % 50));
        }
        std::fs::write(dir.path().join("dados/muitas.csv"), &texto).unwrap();

        let out = DataSummary
            .execute(json!({"path": "dados/muitas.csv"}), &ctx)
            .await
            .unwrap();
        let t = &out.content;
        assert!(t.contains(&format!("mais de {MAX_DISTINCT}")), "{t}");
        assert!(t.contains("50 valor(es) distinto(s)"), "{t}");
    }

    #[tokio::test]
    async fn a_big_file_is_read_only_in_part_and_says_so() {
        let (dir, ctx) = projeto();
        let mut texto = String::from("id,nome\n");
        for i in 0..MAX_ROWS_PARSED + 10 {
            texto.push_str(&format!("{i},nome{i}\n"));
        }
        std::fs::write(dir.path().join("dados/grande.csv"), &texto).unwrap();
        let out = CsvPreview
            .execute(json!({"path": "dados/grande.csv", "rows": 2}), &ctx)
            .await
            .unwrap();
        assert!(
            out.content.contains("li apenas as primeiras"),
            "{}",
            out.content
        );
    }
}
