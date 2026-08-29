//! Totais "desde sempre" das estatísticas de serviço, por modelo.
//!
//! Os counters do `/metrics` do llama-server vivem no processo e morrem com
//! ele; o que sobrevive é esta tabela, alimentada por DELTAS já calculados
//! pelo coletor (com detecção de reset — ver `lr_engine::metrics`). Por
//! construção os totais aqui NUNCA regridem: só entram somas.
//!
//! Unidades: tokens são contagens inteiras; segundos são REAL (vêm dos
//! counters `*_seconds_total`); `updated_at` está em MILISSEGUNDOS
//! (`Store::now_ms`) — atenção ao ler junto de tabelas que gravam segundos.

use crate::{Store, StoreError};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS serve_totals (
    model_id TEXT PRIMARY KEY,
    prompt_tokens INTEGER NOT NULL DEFAULT 0,
    cached_tokens INTEGER NOT NULL DEFAULT 0,
    prompt_seconds REAL NOT NULL DEFAULT 0,
    predicted_tokens INTEGER NOT NULL DEFAULT 0,
    predicted_seconds REAL NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL
);
"#;

pub(crate) fn init(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(SCHEMA)?;
    Ok(())
}

/// Os totais acumulados de um modelo desde a primeira coleta (ou o último
/// "Limpar").
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServeTotalRow {
    pub model_id: String,
    /// Tokens de prompt processados (EXCLUI os reaproveitados do cache).
    pub prompt_tokens: i64,
    /// Tokens reaproveitados do cache de prompt.
    pub cached_tokens: i64,
    pub prompt_seconds: f64,
    /// Tokens gerados.
    pub predicted_tokens: i64,
    pub predicted_seconds: f64,
    /// Milissegundos desde a época.
    pub updated_at: i64,
}

impl Store {
    /// Soma um DELTA aos totais do modelo (upsert): a linha nasce com o delta
    /// e daí em diante só cresce. Deltas zerados nem deveriam chegar aqui —
    /// quem coleta pula a escrita — mas são inócuos.
    pub fn serve_totals_add(
        &self,
        model_id: &str,
        prompt_tokens: i64,
        cached_tokens: i64,
        prompt_seconds: f64,
        predicted_tokens: i64,
        predicted_seconds: f64,
    ) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO serve_totals (model_id, prompt_tokens, cached_tokens, prompt_seconds,
                                       predicted_tokens, predicted_seconds, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(model_id) DO UPDATE SET
                 prompt_tokens = prompt_tokens + excluded.prompt_tokens,
                 cached_tokens = cached_tokens + excluded.cached_tokens,
                 prompt_seconds = prompt_seconds + excluded.prompt_seconds,
                 predicted_tokens = predicted_tokens + excluded.predicted_tokens,
                 predicted_seconds = predicted_seconds + excluded.predicted_seconds,
                 updated_at = excluded.updated_at",
            params![
                model_id,
                prompt_tokens,
                cached_tokens,
                prompt_seconds,
                predicted_tokens,
                predicted_seconds,
                Self::now_ms(),
            ],
        )?;
        Ok(())
    }

    /// Todos os totais, por modelo, em ordem alfabética.
    pub fn serve_totals(&self) -> Result<Vec<ServeTotalRow>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT model_id, prompt_tokens, cached_tokens, prompt_seconds,
                    predicted_tokens, predicted_seconds, updated_at
             FROM serve_totals ORDER BY model_id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ServeTotalRow {
                    model_id: r.get(0)?,
                    prompt_tokens: r.get(1)?,
                    cached_tokens: r.get(2)?,
                    prompt_seconds: r.get(3)?,
                    predicted_tokens: r.get(4)?,
                    predicted_seconds: r.get(5)?,
                    updated_at: r.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// O "Limpar" da tela: zera o "desde sempre" apagando as linhas.
    ///
    /// Quem chama também zera o acumulador de sessão em memória — mas NUNCA a
    /// última leitura do coletor, senão o próximo scrape recontaria o counter
    /// inteiro como delta.
    pub fn serve_totals_clear(&self) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute("DELETE FROM serve_totals", [])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dois deltas do mesmo modelo se somam; modelos diferentes têm linhas
    /// independentes e a leitura volta em ordem alfabética.
    #[test]
    fn deltas_accumulate_per_model() {
        let s = Store::open_in_memory().unwrap();
        s.serve_totals_add("qwen.gguf", 100, 10, 1.5, 50, 2.0)
            .unwrap();
        s.serve_totals_add("qwen.gguf", 40, 6, 0.5, 25, 1.0)
            .unwrap();
        s.serve_totals_add("bge.gguf", 7, 0, 0.1, 0, 0.0).unwrap();

        let rows = s.serve_totals().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].model_id, "bge.gguf");
        let qwen = &rows[1];
        assert_eq!(qwen.model_id, "qwen.gguf");
        assert_eq!(qwen.prompt_tokens, 140);
        assert_eq!(qwen.cached_tokens, 16);
        assert!((qwen.prompt_seconds - 2.0).abs() < 1e-9);
        assert_eq!(qwen.predicted_tokens, 75);
        assert!((qwen.predicted_seconds - 3.0).abs() < 1e-9);
        assert!(qwen.updated_at > 0);
    }

    /// O "Limpar" apaga tudo — e acumular depois dele recomeça do zero.
    #[test]
    fn clear_empties_the_table_and_life_goes_on() {
        let s = Store::open_in_memory().unwrap();
        s.serve_totals_add("m.gguf", 10, 1, 0.1, 5, 0.2).unwrap();
        s.serve_totals_clear().unwrap();
        assert!(s.serve_totals().unwrap().is_empty());

        s.serve_totals_add("m.gguf", 3, 0, 0.05, 2, 0.1).unwrap();
        let rows = s.serve_totals().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].prompt_tokens, 3);
    }

    /// Banco criado ANTES desta tabela existir: o init a cria; reaplicar o
    /// init num banco que já a tem (e com dados) é no-op seguro.
    #[test]
    fn migration_is_idempotent_and_keeps_data() {
        // Banco "antigo": só o esquema base, sem serve_totals.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::SCHEMA).unwrap();
        let s = Store::init(conn).unwrap();

        s.serve_totals_add("m.gguf", 11, 2, 0.3, 7, 0.4).unwrap();

        // Reaplicar a migração não apaga nem duplica nada.
        {
            let conn = s.conn();
            init(&conn).unwrap();
        }
        let rows = s.serve_totals().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].prompt_tokens, 11);
        assert_eq!(rows[0].predicted_tokens, 7);
    }
}
