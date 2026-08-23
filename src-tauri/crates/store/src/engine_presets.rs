//! Presets nomeados de configuração do motor.
//!
//! Um preset é um [`ModelProfile`] parcial com nome — "MTP turbo", "Economia
//! de VRAM" — que a pessoa aplica sobre o perfil de um modelo com um clique.
//! Vive em tabela própria, e não na `presets` (que é de amostragem e já
//! aparece inteira no seletor do chat): namespacing lá exigiria filtro em
//! todo consumidor e um nome de preset de sampling colidiria com um de motor.
//!
//! O `scope` existe para o futuro "preset global" sem migração: hoje tudo é
//! `model`.

use crate::{Store, StoreError};
use rusqlite::{Connection, params};

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS engine_presets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    scope TEXT NOT NULL DEFAULT 'model',
    json TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(name, scope)
);
"#;

/// Um preset como sai do banco. O `json` é um `ModelProfile` parcial — quem
/// interpreta é a camada de comandos, que também conhece os embutidos.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnginePresetRow {
    pub id: i64,
    pub name: String,
    pub scope: String,
    pub json: String,
}

pub(crate) fn init(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(SCHEMA)?;
    Ok(())
}

impl Store {
    pub fn list_engine_presets(&self, scope: &str) -> Result<Vec<EnginePresetRow>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, name, scope, json FROM engine_presets
             WHERE scope = ?1 ORDER BY name COLLATE NOCASE",
        )?;
        let rows = stmt
            .query_map([scope], |r| {
                Ok(EnginePresetRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    scope: r.get(2)?,
                    json: r.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Grava por nome (upsert): salvar de novo com o mesmo nome atualiza.
    pub fn save_engine_preset(
        &self,
        name: &str,
        scope: &str,
        json: &str,
    ) -> Result<i64, StoreError> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO engine_presets (name, scope, json, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(name, scope) DO UPDATE SET
                json = excluded.json,
                updated_at = excluded.updated_at",
            params![name, scope, json, Store::now()],
        )?;
        let id = conn.query_row(
            "SELECT id FROM engine_presets WHERE name = ?1 AND scope = ?2",
            params![name, scope],
            |r| r.get(0),
        )?;
        Ok(id)
    }

    pub fn delete_engine_preset(&self, id: i64) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute("DELETE FROM engine_presets WHERE id = ?1", [id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    #[test]
    fn a_preset_round_trips_and_upserts_by_name() {
        let s = store();
        assert!(s.list_engine_presets("model").unwrap().is_empty());

        let id = s
            .save_engine_preset("MTP turbo", "model", r#"{"spec":"mtp"}"#)
            .unwrap();
        let de_novo = s
            .save_engine_preset("MTP turbo", "model", r#"{"spec":"mtp","specDraftNMax":4}"#)
            .unwrap();
        assert_eq!(id, de_novo, "mesmo nome = mesma linha, atualizada");

        let lista = s.list_engine_presets("model").unwrap();
        assert_eq!(lista.len(), 1);
        assert!(lista[0].json.contains("specDraftNMax"));
    }

    #[test]
    fn scopes_do_not_mix() {
        let s = store();
        s.save_engine_preset("Padrão", "model", "{}").unwrap();
        s.save_engine_preset("Padrão", "global", "{}").unwrap();
        assert_eq!(s.list_engine_presets("model").unwrap().len(), 1);
        assert_eq!(s.list_engine_presets("global").unwrap().len(), 1);
    }

    #[test]
    fn deleting_removes_only_that_preset() {
        let s = store();
        let a = s.save_engine_preset("A", "model", "{}").unwrap();
        s.save_engine_preset("B", "model", "{}").unwrap();
        s.delete_engine_preset(a).unwrap();
        let lista = s.list_engine_presets("model").unwrap();
        assert_eq!(lista.len(), 1);
        assert_eq!(lista[0].name, "B");
    }
}
