//! Configuração de carga escolhida para cada modelo.
//!
//! É a **escolha** — do usuário ou do otimizador — e por isso a chave é o
//! modelo, não a máquina. Trocar de placa não apaga o que a pessoa quis; o
//! que muda é o veredito sobre aquilo, e é justamente isso que permite a
//! frase "você pediu 32k; nesta placa nova não cabe mais". A **evidência**
//! medida (tok/s) mora noutra tabela, com outro tempo de vida.
//!
//! Antes disto o único ajuste por modelo era a janela, guardada como um JSON
//! dentro de um único `setting` — que já era o limite daquele formato.

use crate::{Store, StoreError};
use lr_types::tuning::ModelProfile;
use rusqlite::{Connection, params};

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS model_profiles (
    model_id TEXT PRIMARY KEY,
    profile_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
"#;

/// Setting antigo, migrado uma única vez.
const LEGACY_CTX_SETTING: &str = "model_ctx_sizes";

pub(crate) fn init(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(SCHEMA)?;
    migrate_ctx_sizes(conn)?;
    Ok(())
}

/// Traz as janelas fixadas no formato antigo para a tabela.
///
/// Roda uma vez: depois de migrado, o setting é apagado. Um perfil que já
/// exista para o modelo manda — a migração nunca sobrescreve escolha nova.
fn migrate_ctx_sizes(conn: &Connection) -> Result<(), StoreError> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [LEGACY_CTX_SETTING],
            |r| r.get(0),
        )
        .ok();
    let Some(raw) = raw else {
        return Ok(());
    };
    let mapa: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&raw).unwrap_or_default();

    for (modelo, valor) in mapa {
        let Some(ctx) = valor.as_u64().map(|n| n as u32) else {
            continue;
        };
        let perfil = ModelProfile {
            ctx: Some(ctx),
            ..Default::default()
        };
        let json = serde_json::to_string(&perfil).unwrap_or_else(|_| "{}".into());
        conn.execute(
            "INSERT OR IGNORE INTO model_profiles (model_id, profile_json, updated_at)
             VALUES (?1, ?2, ?3)",
            params![modelo, json, Store::now()],
        )?;
    }
    conn.execute("DELETE FROM settings WHERE key = ?1", [LEGACY_CTX_SETTING])?;
    Ok(())
}

impl Store {
    /// Perfil de um modelo. Ausente = nada escolhido.
    pub fn model_profile(&self, model_id: &str) -> Result<Option<ModelProfile>, StoreError> {
        let conn = self.conn();
        let raw: Option<String> = conn
            .query_row(
                "SELECT profile_json FROM model_profiles WHERE model_id = ?1",
                [model_id],
                |r| r.get(0),
            )
            .ok();
        Ok(raw.and_then(|j| serde_json::from_str(&j).ok()))
    }

    /// Todos os perfis, para montar o INI de uma vez.
    pub fn model_profiles(&self) -> Result<Vec<(String, ModelProfile)>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT model_id, profile_json FROM model_profiles")?;
        let rows = stmt
            .query_map([], |r| {
                let id: String = r.get(0)?;
                let json: String = r.get(1)?;
                Ok((id, json))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(id, json)| serde_json::from_str(&json).ok().map(|p| (id, p)))
            .collect())
    }

    /// Grava (ou apaga, quando o perfil não tem escolha nenhuma).
    pub fn set_model_profile(
        &self,
        model_id: &str,
        profile: &ModelProfile,
    ) -> Result<(), StoreError> {
        let conn = self.conn();
        if profile.is_empty() {
            conn.execute("DELETE FROM model_profiles WHERE model_id = ?1", [model_id])?;
            return Ok(());
        }
        let json = serde_json::to_string(profile).unwrap_or_else(|_| "{}".into());
        conn.execute(
            "INSERT INTO model_profiles (model_id, profile_json, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(model_id) DO UPDATE SET
                profile_json = excluded.profile_json,
                updated_at = excluded.updated_at",
            params![model_id, json, Store::now()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_types::tuning::{KvType, ProfileSource};

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    #[test]
    fn a_profile_comes_back_the_way_it_went_in() {
        let s = store();
        assert!(s.model_profile("m.gguf").unwrap().is_none());

        let p = ModelProfile {
            ctx: Some(16384),
            ngl: Some(36),
            kv_k: Some(KvType::Q8_0),
            source: ProfileSource::Recommended,
            ..Default::default()
        };
        s.set_model_profile("m.gguf", &p).unwrap();
        assert_eq!(s.model_profile("m.gguf").unwrap().unwrap(), p);
    }

    /// Voltar tudo para automático não pode deixar uma linha vazia para trás
    /// dizendo que há escolha.
    #[test]
    fn an_empty_profile_erases_the_choice() {
        let s = store();
        s.set_model_profile(
            "m.gguf",
            &ModelProfile {
                ctx: Some(8192),
                ..Default::default()
            },
        )
        .unwrap();
        s.set_model_profile("m.gguf", &ModelProfile::default())
            .unwrap();
        assert!(s.model_profile("m.gguf").unwrap().is_none());
        assert!(s.model_profiles().unwrap().is_empty());
    }

    #[test]
    fn the_old_context_setting_becomes_a_profile() {
        let s = store();
        s.set_setting(
            LEGACY_CTX_SETTING,
            r#"{"antigo.gguf": 32768, "quebrado.gguf": "abc"}"#,
        )
        .unwrap();
        // A migração roda na abertura do banco; aqui chamamos à mão porque o
        // setting foi escrito depois.
        {
            let conn = s.conn();
            migrate_ctx_sizes(&conn).unwrap();
        }

        let p = s.model_profile("antigo.gguf").unwrap().unwrap();
        assert_eq!(p.ctx, Some(32768));
        assert!(
            s.model_profile("quebrado.gguf").unwrap().is_none(),
            "valor inválido não vira perfil"
        );
        assert!(
            s.get_setting(LEGACY_CTX_SETTING).unwrap().is_none(),
            "o setting antigo sai de cena depois de migrado"
        );
    }

    #[test]
    fn migration_never_overwrites_a_newer_choice() {
        let s = store();
        let escolhido = ModelProfile {
            ctx: Some(8192),
            ngl: Some(10),
            ..Default::default()
        };
        s.set_model_profile("m.gguf", &escolhido).unwrap();
        s.set_setting(LEGACY_CTX_SETTING, r#"{"m.gguf": 65536}"#)
            .unwrap();
        {
            let conn = s.conn();
            migrate_ctx_sizes(&conn).unwrap();
        }
        assert_eq!(s.model_profile("m.gguf").unwrap().unwrap(), escolhido);
    }
}
