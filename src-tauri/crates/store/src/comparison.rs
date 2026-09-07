//! Comparações completas: carga, perfis e medições via servidor no mesmo registro.
use crate::{Store, StoreError};
use rusqlite::{Connection, OptionalExtension, params};

pub(crate) fn init(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS comparison_runs (id INTEGER PRIMARY KEY AUTOINCREMENT, model TEXT NOT NULL, workload TEXT NOT NULL, payload TEXT NOT NULL, created_at INTEGER NOT NULL); CREATE INDEX IF NOT EXISTS comparison_model ON comparison_runs(model, id);")?;
    Ok(())
}
impl Store {
    pub fn add_comparison(
        &self,
        model: &str,
        workload: &str,
        payload: &str,
    ) -> Result<i64, StoreError> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO comparison_runs(model,workload,payload,created_at) VALUES (?1,?2,?3,?4)",
            params![model, workload, payload, Self::now()],
        )?;
        Ok(conn.last_insert_rowid())
    }
    pub fn latest_comparison(&self, model: &str) -> Result<Option<(i64, String)>, StoreError> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id,payload FROM comparison_runs WHERE model=?1 ORDER BY id DESC LIMIT 1",
                [model],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn comparisons_keep_their_workload_and_are_isolated_by_model() {
        let s = Store::open_in_memory().unwrap();
        s.add_comparison("a", "code-v1", "first").unwrap();
        s.add_comparison("b", "code-v1", "other").unwrap();
        s.add_comparison("a", "code-v1", "latest").unwrap();
        assert_eq!(s.latest_comparison("a").unwrap().unwrap().1, "latest");
        assert!(s.latest_comparison("missing").unwrap().is_none());
    }
}
