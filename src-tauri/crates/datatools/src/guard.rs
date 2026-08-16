//! Análise defensiva do SQL antes de entregá-lo ao SQLite.
//!
//! O SQLite responde a três perguntas melhor do que qualquer expressão
//! regular: se a consulta escreve (`sqlite3_stmt_readonly`), se a sintaxe está
//! certa e se a coluna existe. Então **não** tentamos interpretar SQL aqui —
//! a verificação de escrita é do próprio SQLite (ver `sqlite.rs`).
//!
//! Restam duas coisas que precisam ser barradas *antes*, porque o SQLite as
//! considera legítimas:
//!
//! 1. **`ATTACH`/`DETACH`.** `sqlite3_stmt_readonly` devolve "somente leitura"
//!    para um `ATTACH` — e um `ATTACH '/caminho/de/fora.db'` leria um arquivo
//!    fora da pasta do projeto, furando exatamente a garantia que o
//!    `ctx.resolve` dá. (Zeramos também o limite de bancos anexados na
//!    conexão; isto aqui é a primeira das duas camadas, e a que consegue dar
//!    uma mensagem de erro compreensível.)
//! 2. **Mais de um comando na mesma chamada.** `conn.prepare` compila só o
//!    primeiro e **descarta o resto em silêncio**: `SELECT 1; DROP TABLE x`
//!    passaria pela checagem de "é somente leitura" mostrando apenas o
//!    `SELECT`. Um comando por chamada elimina a classe inteira de problema.
//!
//! Para isso basta saber onde o texto é código e onde é literal — daí o
//! [`skeleton`], que apaga o conteúdo de strings, identificadores citados e
//! comentários mantendo as posições. É o mínimo de análise que responde à
//! pergunta com segurança, e ele erra para o lado seguro: o que ele não
//! entende continua visível para a checagem.

use lr_tools::{ToolError, ToolResult};

/// Substitui por espaço tudo que é conteúdo de literal ou comentário,
/// preservando o tamanho e as posições do texto original.
///
/// Assim `WHERE nome = 'ATTACH; --'` vira `WHERE nome = '          '`: o
/// `ATTACH` e o `;` de dentro da string não são confundidos com código.
pub fn skeleton(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            // Literal de texto: '...' com '' como aspa escapada.
            '\'' => {
                out.push('\'');
                for c in chars.by_ref() {
                    if c == '\'' {
                        out.push('\'');
                        break;
                    }
                    out.push(if c == '\n' { '\n' } else { ' ' });
                }
            }
            // Identificadores citados: "col", [col], `col`.
            '"' | '[' | '`' => {
                let fim = match c {
                    '[' => ']',
                    outro => outro,
                };
                out.push(c);
                for c in chars.by_ref() {
                    if c == fim {
                        out.push(c);
                        break;
                    }
                    out.push(if c == '\n' { '\n' } else { ' ' });
                }
            }
            '-' if chars.peek() == Some(&'-') => {
                out.push_str("  ");
                chars.next();
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                    out.push(' ');
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                out.push_str("  ");
                chars.next();
                let mut anterior = ' ';
                for c in chars.by_ref() {
                    let fecha = anterior == '*' && c == '/';
                    out.push(if c == '\n' { '\n' } else { ' ' });
                    if fecha {
                        break;
                    }
                    anterior = c;
                }
            }
            outro => out.push(outro),
        }
    }
    out
}

/// Palavras-chave do texto (fora de literais), em maiúsculas.
pub fn keywords(sql: &str) -> Vec<String> {
    skeleton(sql)
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| !t.is_empty())
        .map(|t| t.to_uppercase())
        .collect()
}

/// Primeira palavra do comando (`SELECT`, `UPDATE`, `WITH`...).
pub fn leading_keyword(sql: &str) -> String {
    keywords(sql).into_iter().next().unwrap_or_default()
}

/// Recusa o que o SQLite aceitaria mas nós não podemos permitir.
pub fn check(sql: &str) -> ToolResult<()> {
    let texto = sql.trim();
    if texto.is_empty() {
        return Err(ToolError::InvalidArgs(
            "a consulta está vazia — escreva o SQL a executar (ex.: SELECT * FROM clientes LIMIT 10)"
                .into(),
        ));
    }

    let esqueleto = skeleton(texto);

    // Um comando por chamada.
    if let Some(pos) = esqueleto.find(';')
        && !esqueleto[pos + 1..].trim().is_empty()
    {
        return Err(ToolError::InvalidArgs(
            "mande um comando SQL por chamada. Vieram vários separados por `;` — só o primeiro \
             seria executado, e isso esconderia o que os outros fazem. Divida em chamadas."
                .into(),
        ));
    }

    let palavras = keywords(texto);
    if palavras.iter().any(|k| k == "ATTACH" || k == "DETACH") {
        return Err(ToolError::InvalidArgs(
            "`ATTACH`/`DETACH` não são permitidos: eles abririam outro arquivo de banco, \
             possivelmente fora da pasta do projeto. Consulte um banco por chamada."
                .into(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skeleton_blanks_string_contents_but_keeps_length() {
        let sql = "SELECT * FROM t WHERE nome = 'ATTACH; oi'";
        let esq = skeleton(sql);
        assert_eq!(esq.chars().count(), sql.chars().count());
        assert!(esq.starts_with("SELECT * FROM t WHERE nome = '"));
        assert!(!esq.contains("ATTACH"));
    }

    #[test]
    fn comments_are_blanked_too() {
        assert!(!skeleton("SELECT 1 -- ; DROP TABLE t\n").contains("DROP"));
        assert!(!skeleton("SELECT /* ; DELETE */ 1").contains("DELETE"));
    }

    #[test]
    fn quoted_identifiers_do_not_hide_real_code() {
        let esq = skeleton("SELECT \"coluna; estranha\" FROM t; DROP TABLE t");
        assert!(esq.contains("DROP"), "{esq}");
    }

    #[test]
    fn a_semicolon_inside_a_string_is_not_a_second_command() {
        assert!(check("SELECT * FROM t WHERE s = 'a;b'").is_ok());
        assert!(check("SELECT 1;").is_ok(), "ponto e vírgula final é normal");
        assert!(check("SELECT 1;  \n ").is_ok());
    }

    #[test]
    fn two_commands_are_refused_with_the_reason() {
        let err = check("SELECT 1; DROP TABLE clientes").unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("um comando SQL por chamada"), "{msg}");
    }

    #[test]
    fn a_hidden_second_command_after_a_comment_is_refused() {
        assert!(check("SELECT 1 -- inofensivo\n; DELETE FROM clientes").is_err());
    }

    #[test]
    fn attach_and_detach_are_refused() {
        for sql in [
            "ATTACH DATABASE '/etc/segredo.db' AS x",
            "attach '/tmp/outro.db' as o",
            "DETACH DATABASE x",
        ] {
            let err = check(sql).unwrap_err();
            assert!(err.to_model_message().contains("ATTACH"), "{sql}");
        }
        // A palavra dentro de um texto não pode disparar o alarme.
        assert!(check("SELECT * FROM t WHERE nota = 'ATTACH'").is_ok());
    }

    #[test]
    fn an_empty_query_says_what_to_write() {
        let msg = check("   ").unwrap_err().to_model_message();
        assert!(msg.contains("SELECT"), "{msg}");
    }

    #[test]
    fn leading_keyword_ignores_comments_and_case() {
        assert_eq!(leading_keyword("  -- nota\n select 1"), "SELECT");
        assert_eq!(leading_keyword("/* x */ UPDATE t SET a=1"), "UPDATE");
        assert_eq!(leading_keyword(""), "");
    }
}
