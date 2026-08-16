//! `sql_query` e `sql_schema`: consultar um banco SQLite do projeto.
//!
//! ## Como a escrita é barrada
//!
//! Não olhamos o texto do SQL para decidir se ele escreve — quem responde isso
//! é o próprio SQLite, por `sqlite3_stmt_readonly` (exposto como
//! [`rusqlite::Statement::readonly`]). Uma lista de palavras proibidas erraria
//! nos dois sentidos: deixaria passar `WITH x AS (...) INSERT ...` e recusaria
//! um `SELECT` de uma tabela chamada `updates`.
//!
//! São três camadas, e cada uma existe porque a de cima pode falhar:
//!
//! 1. [`crate::guard::check`] recusa `ATTACH`/`DETACH` e vários comandos numa
//!    chamada só — coisas que o SQLite consideraria legítimas;
//! 2. `stmt.readonly()` decide se a consulta escreve, e sem `allow_write` a
//!    recusa vem com a explicação de como repetir com permissão;
//! 3. a conexão é aberta **somente leitura** quando não há `allow_write`, o
//!    que faz qualquer escrita falhar no nível do arquivo mesmo que as duas
//!    primeiras camadas tenham deixado passar.
//!
//! ## Detalhes que parecem detalhe e não são
//!
//! - **Sem `SQLITE_OPEN_URI`.** O padrão do rusqlite inclui essa bandeira, e
//!   com ela um "caminho" como `file:/etc/x.db?mode=rwc` deixaria de ser um
//!   caminho e viraria uma URI — passando longe do `ctx.resolve`. Montamos as
//!   bandeiras à mão para deixá-la de fora.
//! - **Sem `SQLITE_OPEN_CREATE`.** Consultar um banco que não existe é erro de
//!   digitação; criar um arquivo vazio esconderia o engano.
//! - **Limite de anexos zerado.** Segunda camada contra `ATTACH`, agora no
//!   nível da conexão.
//! - **Tudo em [`tokio::task::spawn_blocking`]**: o rusqlite é síncrono e uma
//!   varredura de tabela grande travaria o executor do app inteiro.

use crate::guard;
use crate::table::{self, Limits, RowCount};
use async_trait::async_trait;
use lr_tools::{Tool, ToolContext, ToolError, ToolOutput, ToolResult, arg_bool, arg_str, arg_u64};
use lr_types::agent::{ToolCategory, ToolPreview};
use rusqlite::limits::Limit;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Linhas devolvidas quando ninguém pede um número.
pub const DEFAULT_MAX_ROWS: u64 = 20;

/// Teto de linhas por chamada.
pub const MAX_MAX_ROWS: u64 = 500;

/// Abre a conexão com as bandeiras certas (ver a nota no topo do módulo).
pub fn open(path: &Path, write: bool) -> ToolResult<Connection> {
    let flags = if write {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
    };
    let conn = Connection::open_with_flags(path, flags).map_err(|e| {
        ToolError::Other(format!(
            "Não consegui abrir `{}` como banco SQLite: {e}. Confira se é mesmo um arquivo .db / \
             .sqlite; para arquivos CSV use `csv_query`.",
            path.display()
        ))
    })?;
    harden(&conn);
    Ok(conn)
}

/// Fecha as portas que não usamos nesta conexão.
pub fn harden(conn: &Connection) {
    // Zero bancos anexáveis: `ATTACH` passa a falhar mesmo se escapar do
    // guard. `set_limit` só falha se o valor for inválido; ignorar o retorno
    // aqui não esconde nada acionável.
    let _ = conn.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0);
}

/// Traduz um erro do SQLite em algo que o modelo consegue usar.
pub fn sql_error(e: &rusqlite::Error, dica: &str) -> ToolError {
    let texto = e.to_string();
    if texto.contains("not a database") {
        return ToolError::Other(
            "Este arquivo não é um banco SQLite (o cabeçalho não confere). Se for um CSV, use \
             `csv_preview` ou `csv_query`."
                .into(),
        );
    }
    ToolError::Other(format!("O SQLite recusou a consulta: {texto}. {dica}"))
}

/// Recusa amigável de uma consulta que escreve.
fn write_refused(sql: &str) -> ToolError {
    let comando = guard::leading_keyword(sql);
    let comando = if comando.is_empty() {
        "Este comando".to_string()
    } else {
        format!("`{comando}`")
    };
    ToolError::Other(format!(
        "{comando} altera os dados do banco, e esta chamada veio sem permissão de escrita. Se a \
         intenção é mesmo alterar, repita com `allow_write: true` — o aplicativo vai pedir a \
         confirmação do usuário antes de executar. Para apenas consultar, use SELECT."
    ))
}

/// Valor de uma célula como texto para a tabela.
fn cell(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Null => "NULL".to_string(),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => table::fmt_num(f),
        ValueRef::Text(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        // Despejar binário no resultado só gastaria contexto.
        ValueRef::Blob(bytes) => format!("<binário de {} bytes>", bytes.len()),
    }
}

/// Executa um comando já validado e devolve o texto do resultado.
///
/// Compartilhado com o `csv_query`, que roda o mesmo caminho contra um banco
/// em memória.
pub fn run_statement(
    conn: &Connection,
    sql: &str,
    max_rows: usize,
    allow_write: bool,
    dica: &str,
) -> ToolResult<String> {
    let mut stmt = conn.prepare(sql).map_err(|e| sql_error(&e, dica))?;

    if !stmt.readonly() && !allow_write {
        return Err(write_refused(sql));
    }

    // Comando sem colunas de saída (INSERT/UPDATE/DELETE/CREATE): o que
    // interessa é quantas linhas mudaram.
    if stmt.column_count() == 0 {
        let afetadas = stmt.raw_execute().map_err(|e| sql_error(&e, dica))?;
        return Ok(format!(
            "Comando executado. {afetadas} linha(s) afetada(s)."
        ));
    }

    let headers: Vec<String> = stmt
        .column_names()
        .into_iter()
        .map(str::to_string)
        .collect();
    let colunas = headers.len();

    let mut rows = stmt.query([]).map_err(|e| sql_error(&e, dica))?;
    let mut coletadas: Vec<Vec<String>> = Vec::new();
    let mut ha_mais = false;
    while let Some(row) = rows.next().map_err(|e| sql_error(&e, dica))? {
        if coletadas.len() == max_rows {
            // Uma linha a mais só para saber que existe mais — não varremos a
            // tabela inteira só para contar.
            ha_mais = true;
            break;
        }
        let mut linha = Vec::with_capacity(colunas);
        for i in 0..colunas {
            linha.push(cell(row.get_ref(i).map_err(|e| sql_error(&e, dica))?));
        }
        coletadas.push(linha);
    }

    if coletadas.is_empty() {
        return Ok(format!(
            "A consulta rodou e não devolveu nenhuma linha. Colunas pedidas: {}.",
            headers.join(", ")
        ));
    }

    let total = if ha_mais {
        RowCount::AtLeast(coletadas.len() + 1)
    } else {
        RowCount::Exact(coletadas.len())
    };
    Ok(table::render(
        &headers,
        &coletadas,
        total,
        Limits::default().with_rows(max_rows),
    ))
}

/// Resolve o caminho do banco dentro do projeto e confirma que ele existe.
fn resolve_db(args: &Value, ctx: &ToolContext) -> ToolResult<(String, PathBuf)> {
    let rel = arg_str(args, "db_path")?;
    let path = ctx.resolve(&rel)?;
    if !path.exists() {
        return Err(ToolError::NotFound(rel));
    }
    if path.is_dir() {
        return Err(ToolError::InvalidArgs(format!(
            "`{rel}` é uma pasta, não um arquivo de banco. Procure o arquivo .db com `fs_glob`."
        )));
    }
    Ok((rel, path))
}

fn db_inside(args: &Value, ctx: &ToolContext) -> bool {
    match arg_str(args, "db_path") {
        Ok(rel) => ctx.resolve(&rel).is_ok(),
        Err(_) => true,
    }
}

// ------------------------------------------------------------- sql_query ---

/// Ferramenta `sql_query`.
pub struct SqlQuery;

#[async_trait]
impl Tool for SqlQuery {
    fn name(&self) -> &str {
        "sql_query"
    }

    fn description(&self) -> &str {
        "Roda um comando SQL num arquivo de banco SQLite que esteja dentro da pasta do projeto e \
         devolve o resultado como tabela de texto. Um comando por chamada. Consultas de leitura \
         (SELECT) rodam direto; qualquer comando que altere dados exige `allow_write: true` e \
         confirmação do usuário. Chame `sql_schema` antes para saber os nomes das tabelas."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "db_path": {
                    "type": "string",
                    "description": "Caminho do arquivo .db/.sqlite, relativo à raiz do projeto (ex.: dados/loja.db)."
                },
                "query": {
                    "type": "string",
                    "description": "Um único comando SQL, sem ponto e vírgula extra (ex.: SELECT nome, total FROM pedidos ORDER BY total DESC)."
                },
                "max_rows": {
                    "type": "integer",
                    "description": "Máximo de linhas no resultado (1 a 500; padrão 20). O resto é omitido com aviso.",
                    "minimum": 1,
                    "maximum": 500
                },
                "allow_write": {
                    "type": "boolean",
                    "description": "true para permitir comandos que ALTERAM o banco (INSERT/UPDATE/DELETE/CREATE/DROP). Padrão false. Só use quando o objetivo for mesmo modificar os dados."
                }
            },
            "required": ["db_path", "query"],
            "additionalProperties": false
        })
    }

    /// O catálogo mostra a ferramenta pelo uso comum: consultar.
    fn category(&self) -> ToolCategory {
        ToolCategory::Read
    }

    /// Com `allow_write`, a chamada é escrita — e é assim que a política
    /// precisa vê-la: confirmação com o motivo certo, foto do banco antes e
    /// nada de valer o "sempre permitir" que a pessoa deu vendo consultas.
    fn category_for(&self, args: &Value) -> ToolCategory {
        if arg_bool(args, "allow_write", false) {
            ToolCategory::Edit
        } else {
            ToolCategory::Read
        }
    }

    fn within_workspace(&self, args: &Value, ctx: &ToolContext) -> bool {
        db_inside(args, ctx)
    }

    /// Em escrita, o arquivo do banco entra no checkpoint.
    fn files_at_risk(&self, args: &Value, ctx: &ToolContext) -> Vec<String> {
        if !arg_bool(args, "allow_write", false) {
            return Vec::new();
        }
        match arg_str(args, "db_path").ok().map(|rel| ctx.resolve(&rel)) {
            Some(Ok(path)) => vec![ctx.relativize(&path)],
            _ => Vec::new(),
        }
    }

    async fn preview(&self, args: &Value, _ctx: &ToolContext) -> Option<ToolPreview> {
        let query = arg_str(args, "query").ok()?;
        let db = arg_str(args, "db_path").ok()?;
        let aviso = if arg_bool(args, "allow_write", false) {
            "ATENÇÃO: esta chamada pode ALTERAR os dados do banco (veio com allow_write).\n\n"
        } else {
            "Somente leitura: nenhuma alteração será gravada.\n\n"
        };
        Some(ToolPreview::Text {
            body: format!("{aviso}Banco: {db}\n\n{}", query.trim()),
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let (rel, path) = resolve_db(&args, ctx)?;
        let query = arg_str(&args, "query")?;
        guard::check(&query)?;
        let max_rows = arg_u64(&args, "max_rows", DEFAULT_MAX_ROWS).clamp(1, MAX_MAX_ROWS) as usize;
        let allow_write = arg_bool(&args, "allow_write", false);

        let sql = query.trim().to_string();
        let escrita = allow_write;
        let corpo = tokio::task::spawn_blocking(move || {
            let conn = open(&path, escrita)?;
            run_statement(
                &conn,
                &sql,
                max_rows,
                escrita,
                "Use `sql_schema` para conferir os nomes de tabelas e colunas.",
            )
        })
        .await
        .map_err(|e| ToolError::Other(format!("a consulta não pôde ser executada: {e}")))??;

        Ok(ToolOutput::text(format!("Banco: {rel}\n\n{corpo}"))
            .with_changed(if allow_write { vec![rel] } else { Vec::new() })
            .truncated_to(ctx.max_output_bytes))
    }
}

// ------------------------------------------------------------ sql_schema ---

/// Ferramenta `sql_schema`.
pub struct SqlSchema;

/// Descrição de uma tabela/visão do banco.
fn describe(conn: &Connection) -> ToolResult<String> {
    let dica = "O arquivo abriu, mas não consegui ler o catálogo.";
    let mut stmt = conn
        .prepare(
            "SELECT name, type FROM sqlite_master \
             WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY type, name",
        )
        .map_err(|e| sql_error(&e, dica))?;
    let objetos: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| sql_error(&e, dica))?
        .filter_map(Result::ok)
        .collect();

    if objetos.is_empty() {
        return Ok("O banco abriu, mas não tem nenhuma tabela ainda.".to_string());
    }

    let mut out = format!("{} tabela(s)/visão(ões):\n", objetos.len());
    for (nome, tipo) in &objetos {
        let rotulo = if tipo == "view" { "VISÃO" } else { "TABELA" };
        let linhas = contar(conn, nome);
        out.push_str(&format!("\n{rotulo} {nome}{linhas}\n"));

        let mut cols = conn
            .prepare("SELECT name, type, \"notnull\", pk FROM pragma_table_info(?1)")
            .map_err(|e| sql_error(&e, dica))?;
        let colunas: Vec<(String, String, i64, i64)> = cols
            .query_map([nome], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .map_err(|e| sql_error(&e, dica))?
            .filter_map(Result::ok)
            .collect();

        let largura = colunas
            .iter()
            .map(|c| c.0.chars().count())
            .max()
            .unwrap_or(4)
            .min(40);
        for (nome_col, tipo_col, notnull, pk) in &colunas {
            let tipo_col = if tipo_col.is_empty() { "?" } else { tipo_col };
            let mut marcas = Vec::new();
            if *pk > 0 {
                marcas.push("chave primária");
            }
            if *notnull > 0 {
                marcas.push("obrigatório");
            }
            let extra = if marcas.is_empty() {
                String::new()
            } else {
                format!("  ({})", marcas.join(", "))
            };
            out.push_str(&format!(
                "  {:<largura$}  {tipo_col}{extra}\n",
                nome_col,
                largura = largura
            ));
        }

        // Chaves estrangeiras: é o que o modelo precisa para escrever o JOIN
        // certo sem adivinhar.
        if let Ok(mut fks) =
            conn.prepare("SELECT \"table\", \"from\", \"to\" FROM pragma_foreign_key_list(?1)")
            && let Ok(lista) = fks.query_map([nome], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
        {
            for (destino, de, para) in lista.filter_map(Result::ok) {
                let para = para.unwrap_or_else(|| "(chave primária)".into());
                out.push_str(&format!("  -> {nome}.{de} referencia {destino}.{para}\n"));
            }
        }
    }
    Ok(out)
}

/// Conta as linhas da tabela; silencioso quando não dá (visão quebrada, por
/// exemplo) — o esquema é útil mesmo sem o número.
fn contar(conn: &Connection, nome: &str) -> String {
    let sql = format!("SELECT COUNT(*) FROM \"{}\"", nome.replace('"', "\"\""));
    match conn.query_row(&sql, [], |row| row.get::<_, i64>(0)) {
        Ok(n) => format!(" ({n} linha(s))"),
        Err(_) => String::new(),
    }
}

#[async_trait]
impl Tool for SqlSchema {
    fn name(&self) -> &str {
        "sql_schema"
    }

    fn description(&self) -> &str {
        "Lista as tabelas de um banco SQLite do projeto com suas colunas, tipos, chaves e \
         quantidade de linhas. Use SEMPRE antes de escrever uma consulta com `sql_query`: é o \
         que evita inventar nome de tabela ou de coluna."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "db_path": {
                    "type": "string",
                    "description": "Caminho do arquivo .db/.sqlite, relativo à raiz do projeto (ex.: dados/loja.db)."
                }
            },
            "required": ["db_path"],
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Read
    }

    fn within_workspace(&self, args: &Value, ctx: &ToolContext) -> bool {
        db_inside(args, ctx)
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let (rel, path) = resolve_db(&args, ctx)?;
        let corpo = tokio::task::spawn_blocking(move || {
            let conn = open(&path, false)?;
            describe(&conn)
        })
        .await
        .map_err(|e| ToolError::Other(format!("não consegui ler o esquema: {e}")))??;

        Ok(ToolOutput::text(format!("Banco: {rel}\n{corpo}")).truncated_to(ctx.max_output_bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Projeto com um banco de exemplo em `dados/loja.db`.
    fn projeto() -> (TempDir, ToolContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("dados")).unwrap();
        let db = dir.path().join("dados/loja.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE clientes (
                 id INTEGER PRIMARY KEY,
                 nome TEXT NOT NULL,
                 cidade TEXT
             );
             CREATE TABLE pedidos (
                 id INTEGER PRIMARY KEY,
                 cliente_id INTEGER REFERENCES clientes(id),
                 total REAL
             );
             INSERT INTO clientes (id, nome, cidade) VALUES
                 (1, 'Ana', 'São Paulo'), (2, 'Bruno', 'Recife'), (3, 'Carla', NULL);
             INSERT INTO pedidos (id, cliente_id, total) VALUES
                 (1, 1, 99.9), (2, 1, 10.0), (3, 2, 55.5);",
        )
        .unwrap();
        drop(conn);
        let ctx = ToolContext::new(Some(dir.path().to_path_buf()), "call-1");
        (dir, ctx)
    }

    async fn consulta(args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        SqlQuery.execute(args, ctx).await
    }

    #[tokio::test]
    async fn schema_lists_tables_columns_and_keys() {
        let (_d, ctx) = projeto();
        let out = SqlSchema
            .execute(json!({"db_path": "dados/loja.db"}), &ctx)
            .await
            .unwrap();
        let t = &out.content;
        assert!(t.contains("TABELA clientes (3 linha(s))"), "{t}");
        assert!(t.contains("TABELA pedidos"), "{t}");
        assert!(t.contains("nome"), "{t}");
        assert!(t.contains("chave primária"), "{t}");
        assert!(t.contains("obrigatório"), "{t}");
        assert!(t.contains("referencia clientes.id"), "{t}");
    }

    #[tokio::test]
    async fn a_select_comes_back_as_an_aligned_table() {
        let (_d, ctx) = projeto();
        let out = consulta(
            json!({"db_path": "dados/loja.db", "query": "SELECT id, nome FROM clientes ORDER BY id"}),
            &ctx,
        )
        .await
        .unwrap();
        let t = &out.content;
        assert!(t.contains("id | nome"), "{t}");
        assert!(t.contains("Ana"), "{t}");
        assert!(t.contains("Carla"), "{t}");
        assert!(out.changed_files.is_empty(), "leitura não altera nada");
    }

    #[tokio::test]
    async fn null_and_numbers_are_readable() {
        let (_d, ctx) = projeto();
        let out = consulta(
            json!({"db_path": "dados/loja.db", "query": "SELECT cidade, 10.0 AS n FROM clientes WHERE id = 3"}),
            &ctx,
        )
        .await
        .unwrap();
        assert!(out.content.contains("NULL"), "{}", out.content);
        assert!(out.content.contains("10"), "{}", out.content);
    }

    #[tokio::test]
    async fn writing_commands_are_refused_without_permission() {
        let (_d, ctx) = projeto();
        for sql in [
            "UPDATE clientes SET nome = 'X'",
            "DELETE FROM clientes",
            "INSERT INTO clientes (id, nome) VALUES (9, 'Z')",
            "DROP TABLE clientes",
            "CREATE TABLE novo (a INTEGER)",
        ] {
            let err = consulta(json!({"db_path": "dados/loja.db", "query": sql}), &ctx)
                .await
                .unwrap_err();
            let msg = err.to_model_message();
            assert!(msg.contains("allow_write"), "{sql}: {msg}");
        }
        // E nada foi alterado.
        let out = consulta(
            json!({"db_path": "dados/loja.db", "query": "SELECT COUNT(*) AS n FROM clientes"}),
            &ctx,
        )
        .await
        .unwrap();
        assert!(out.content.contains('3'), "{}", out.content);
    }

    #[tokio::test]
    async fn with_permission_a_write_runs_and_reports_the_rows() {
        let (_d, ctx) = projeto();
        let out = consulta(
            json!({
                "db_path": "dados/loja.db",
                "query": "UPDATE clientes SET cidade = 'Belém' WHERE id = 3",
                "allow_write": true
            }),
            &ctx,
        )
        .await
        .unwrap();
        assert!(
            out.content.contains("1 linha(s) afetada(s)"),
            "{}",
            out.content
        );
        assert_eq!(out.changed_files, vec!["dados/loja.db"]);

        let depois = consulta(
            json!({"db_path": "dados/loja.db", "query": "SELECT cidade FROM clientes WHERE id = 3"}),
            &ctx,
        )
        .await
        .unwrap();
        assert!(depois.content.contains("Belém"), "{}", depois.content);
    }

    /// A escrita se anuncia como escrita: é daí que saem a confirmação, a
    /// foto do banco e o aviso na prévia.
    #[tokio::test]
    async fn a_write_always_needs_confirmation() {
        let (_d, ctx) = projeto();
        let leitura = json!({"db_path": "dados/loja.db", "query": "SELECT 1"});
        let escrita = json!({
            "db_path": "dados/loja.db",
            "query": "DELETE FROM clientes",
            "allow_write": true
        });
        assert_eq!(SqlQuery.category_for(&leitura), ToolCategory::Read);
        assert_eq!(SqlQuery.category_for(&escrita), ToolCategory::Edit);
        assert!(SqlQuery.within_workspace(&leitura, &ctx));
        assert!(SqlQuery.within_workspace(&escrita, &ctx));
        assert_eq!(SqlQuery.files_at_risk(&escrita, &ctx), ["dados/loja.db"]);
        assert!(SqlQuery.files_at_risk(&leitura, &ctx).is_empty());

        match SqlQuery.preview(&escrita, &ctx).await.unwrap() {
            ToolPreview::Text { body } => assert!(body.contains("ALTERAR"), "{body}"),
            other => panic!("esperava texto, veio {other:?}"),
        }
    }

    #[tokio::test]
    async fn attach_is_refused_even_with_write_permission() {
        let (_d, ctx) = projeto();
        let err = consulta(
            json!({
                "db_path": "dados/loja.db",
                "query": "ATTACH DATABASE '/etc/passwd' AS fora",
                "allow_write": true
            }),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(err.to_model_message().contains("ATTACH"));
    }

    #[tokio::test]
    async fn several_commands_in_one_call_are_refused() {
        let (_d, ctx) = projeto();
        let err = consulta(
            json!({"db_path": "dados/loja.db", "query": "SELECT 1; DROP TABLE clientes"}),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(
            err.to_model_message()
                .contains("um comando SQL por chamada")
        );
    }

    #[tokio::test]
    async fn a_database_outside_the_project_is_refused() {
        let (_d, ctx) = projeto();
        for caminho in ["../fora.db", "/etc/passwd"] {
            let args = json!({"db_path": caminho, "query": "SELECT 1"});
            assert!(!SqlQuery.within_workspace(&args, &ctx), "{caminho}");
            let err = consulta(args, &ctx).await.unwrap_err();
            assert!(
                matches!(err, ToolError::OutsideWorkspace(_)),
                "{caminho}: {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_missing_file_says_so() {
        let (_d, ctx) = projeto();
        let err = consulta(
            json!({"db_path": "dados/naoexiste.db", "query": "SELECT 1"}),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)), "{err:?}");
        // E não criou o arquivo por engano.
        assert!(!_d.path().join("dados/naoexiste.db").exists());
    }

    #[tokio::test]
    async fn a_file_that_is_not_a_database_is_explained() {
        let (dir, ctx) = projeto();
        std::fs::write(dir.path().join("dados/nao.db"), "isto e texto puro\n").unwrap();
        let err = consulta(
            json!({"db_path": "dados/nao.db", "query": "SELECT 1 FROM t"}),
            &ctx,
        )
        .await
        .unwrap_err();
        // Seja a recusa na abertura ou na primeira consulta, a mensagem tem
        // de nomear o problema e apontar a ferramenta certa.
        let msg = err.to_model_message();
        assert!(msg.contains("SQLite"), "{msg}");
        assert!(msg.contains("csv_"), "{msg}");
    }

    #[tokio::test]
    async fn a_syntax_error_points_to_the_schema_tool() {
        let (_d, ctx) = projeto();
        let err = consulta(
            json!({"db_path": "dados/loja.db", "query": "SELECT * FROM inexistente"}),
            &ctx,
        )
        .await
        .unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("sql_schema"), "{msg}");
        assert!(msg.to_lowercase().contains("inexistente"), "{msg}");
    }

    #[tokio::test]
    async fn max_rows_limits_the_result_and_warns() {
        let (_d, ctx) = projeto();
        let out = consulta(
            json!({
                "db_path": "dados/loja.db",
                "query": "SELECT id, nome FROM clientes ORDER BY id",
                "max_rows": 1
            }),
            &ctx,
        )
        .await
        .unwrap();
        assert!(out.content.contains("Ana"), "{}", out.content);
        assert!(!out.content.contains("Bruno"), "{}", out.content);
        assert!(out.content.contains("há mais linhas"), "{}", out.content);
    }

    #[tokio::test]
    async fn an_empty_result_is_not_an_error() {
        let (_d, ctx) = projeto();
        let out = consulta(
            json!({"db_path": "dados/loja.db", "query": "SELECT nome FROM clientes WHERE id = 99"}),
            &ctx,
        )
        .await
        .unwrap();
        assert!(out.content.contains("nenhuma linha"), "{}", out.content);
    }

    #[test]
    fn attached_databases_are_blocked_at_the_connection_level() {
        // Segunda camada: mesmo que o guard falhasse, o SQLite recusa.
        let conn = Connection::open_in_memory().unwrap();
        harden(&conn);
        let erro = conn.execute_batch("ATTACH ':memory:' AS x");
        assert!(erro.is_err(), "o ATTACH deveria ser recusado pela conexão");
    }
}
