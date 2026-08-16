//! Ferramentas de dados do agente: SQLite e CSV.
//!
//! Depois de código, tabela é o que as pessoas mais pedem para o agente olhar
//! — um `.csv` exportado do sistema, um `.db` de desenvolvimento. Sem estas
//! ferramentas o agente tenta ler o arquivo inteiro com `fs_read`, enche a
//! janela de contexto com dados brutos e ainda erra a conta.
//!
//! | Ferramenta     | Categoria | Para quê |
//! |----------------|-----------|----------|
//! | `sql_schema`   | Read      | tabelas, colunas e chaves de um `.db` |
//! | `sql_query`    | Read\*    | consultar (e, com permissão, alterar) |
//! | `csv_preview`  | Read      | primeiras linhas + tipo de cada coluna |
//! | `csv_query`    | Read      | SQL sobre um CSV, em memória |
//! | `data_summary` | Read      | estatísticas do arquivo inteiro |
//!
//! ## A regra que atravessa tudo: resumir, nunca despejar
//!
//! Nenhuma ferramenta daqui devolve um arquivo inteiro. Toda saída é amostra
//! ou agregado, cortada por linhas **e** por largura, e todo corte é anunciado
//! com o número do que ficou de fora ([`table`]). Um CSV de cinquenta mil
//! linhas não é "grande demais para ler": ele estoura a janela e derruba o
//! passo. `csv_query` existe para transformar essa pergunta impossível numa
//! tabela de dez linhas.
//!
//! ## \* Sobre a categoria do `sql_query`
//!
//! `sql_query` é leitura quando consulta e escrita quando `allow_write: true`.
//! [`Tool::category`] é fixa por ferramenta e diz o uso comum (leitura); quem
//! conta a verdade da chamada é [`Tool::category_for`], que enxerga os
//! argumentos. Na escrita ela devolve `Edit`, e daí sai tudo: confirmação com
//! o motivo certo, foto do banco antes de rodar e um "sempre permitir" dado
//! para consultas que não vale para alteração. A prévia diz em texto que a
//! chamada altera dados, que é o que a pessoa lê antes de decidir. Os
//! detalhes das outras camadas de proteção estão em [`sqlite`].
//!
//! [`Tool::category`]: lr_tools::Tool::category
//! [`Tool::category_for`]: lr_tools::Tool::category_for

pub mod csv;
pub mod csvtools;
pub mod guard;
pub mod sqlite;
pub mod table;

pub use csvtools::{CsvPreview, CsvQuery, DataSummary, TABLE_NAME};
pub use sqlite::{SqlQuery, SqlSchema};

use lr_tools::SharedTool;
use std::sync::Arc;

/// Todas as ferramentas de dados, prontas para o registro do agente.
pub fn data_tools() -> Vec<SharedTool> {
    vec![
        Arc::new(SqlSchema),
        Arc::new(SqlQuery),
        Arc::new(CsvPreview),
        Arc::new(CsvQuery),
        Arc::new(DataSummary),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_tools::ToolContext;
    use lr_types::agent::ToolCategory;
    use serde_json::json;

    #[test]
    fn the_catalog_has_the_five_tools() {
        let mut nomes: Vec<String> = data_tools().iter().map(|t| t.name().to_string()).collect();
        nomes.sort();
        assert_eq!(
            nomes,
            vec![
                "csv_preview",
                "csv_query",
                "data_summary",
                "sql_query",
                "sql_schema"
            ]
        );
    }

    /// Tudo aqui é leitura por padrão — inclusive o `sql_query`, cuja escrita
    /// é barrada pelo caminho descrito no topo do módulo.
    #[test]
    fn every_tool_reads_by_default() {
        for tool in data_tools() {
            assert_eq!(tool.category(), ToolCategory::Read, "{}", tool.name());
        }
    }

    /// `read_only` é uma promessa mais forte que a categoria: quem responde
    /// `true` entra no cardápio do modo planejamento e do ajudante
    /// explorador, que juram não alterar nada. `sql_query` pode escrever, e
    /// por isso fica de fora — mesmo aparecendo como leitura no catálogo.
    #[test]
    fn only_the_ones_that_can_never_write_promise_read_only() {
        for tool in data_tools() {
            let promete = tool.spec().read_only;
            if tool.name() == "sql_query" {
                assert!(!promete, "sql_query pode alterar o banco com allow_write");
            } else {
                assert!(promete, "{}", tool.name());
            }
        }
    }

    #[test]
    fn nothing_here_touches_the_network_or_runs_processes() {
        for tool in data_tools() {
            assert!(
                !matches!(
                    tool.category(),
                    ToolCategory::Network | ToolCategory::Execute
                ),
                "{}",
                tool.name()
            );
        }
    }

    /// Sem pasta de projeto não há arquivo para analisar — e a recusa tem de
    /// ser clara, não um panic.
    #[tokio::test]
    async fn without_a_project_folder_everything_refuses_politely() {
        let ctx = ToolContext::new(None, "c1");
        let args = json!({
            "path": "dados.csv",
            "db_path": "dados.db",
            "query": "SELECT 1 FROM dados"
        });
        for tool in data_tools() {
            let err = tool
                .execute(args.clone(), &ctx)
                .await
                .expect_err(tool.name());
            assert!(
                !err.to_model_message().is_empty(),
                "{} sem mensagem",
                tool.name()
            );
        }
    }

    /// O modelo escolhe a ferramenta lendo isto.
    #[test]
    fn every_parameter_is_described_in_portuguese() {
        for tool in data_tools() {
            let params = tool.parameters();
            let props = params["properties"].as_object().expect("properties");
            assert!(!props.is_empty(), "{} sem parâmetros", tool.name());
            for (nome, campo) in props {
                let desc = campo["description"].as_str().unwrap_or("");
                assert!(
                    desc.len() > 20,
                    "`{}` de `{}` sem descrição útil",
                    nome,
                    tool.name()
                );
            }
            assert!(
                tool.description().len() > 60,
                "descrição curta demais em {}",
                tool.name()
            );
        }
    }
}
