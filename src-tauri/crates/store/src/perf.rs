//! O que cada configuração rendeu nesta máquina.
//!
//! Aqui mora a **evidência**, e ela tem outro tempo de vida que a escolha
//! (`model_profiles`): a escolha é da pessoa e sobrevive à troca de placa; a
//! medição é da máquina, do build do llama.cpp e do arquivo, e caduca quando
//! qualquer um deles muda. Guardar as duas juntas apagaria a preferência
//! junto com o número velho.
//!
//! A chave carrega tudo que invalida uma medição, e por isso ela é comprida:
//! máquina + build + arquivo + configuração. Trocar o driver ou atualizar o
//! runtime gera uma chave nova, e a antiga simplesmente deixa de ser
//! encontrada — sem apagar nada, porque uma medição velha pode voltar a valer
//! se a pessoa voltar atrás.

use crate::{Store, StoreError};
use rusqlite::{Connection, Row, params};
use serde::{Deserialize, Serialize};

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS perf_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    machine_key TEXT NOT NULL,
    model_id TEXT NOT NULL,
    profile_key TEXT NOT NULL,
    build_number INTEGER NOT NULL,
    gen_tps REAL NOT NULL,
    prompt_tps REAL NOT NULL,
    gen_stddev REAL NOT NULL,
    gpu_bytes INTEGER,
    source TEXT NOT NULL,
    suspect INTEGER NOT NULL DEFAULT 0,
    measured_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_perf_lookup
    ON perf_runs(machine_key, model_id, profile_key);
"#;

pub(crate) fn init(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(SCHEMA)?;
    Ok(())
}

/// Uma medição gravada.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfRun {
    pub machine_key: String,
    pub model_id: String,
    pub profile_key: String,
    pub build_number: u64,
    pub gen_tps: f64,
    pub prompt_tps: f64,
    pub gen_stddev: f64,
    pub gpu_bytes: Option<u64>,
    /// `bench` (medido a pedido) ou `usage` (colhido do uso normal).
    pub source: String,
    /// A placa esquentou durante a corrida: o número serve, mas com ressalva.
    pub suspect: bool,
    pub measured_at: i64,
}

fn run_from(r: &Row<'_>) -> rusqlite::Result<PerfRun> {
    Ok(PerfRun {
        machine_key: r.get(0)?,
        model_id: r.get(1)?,
        profile_key: r.get(2)?,
        build_number: r.get::<_, i64>(3)?.max(0) as u64,
        gen_tps: r.get(4)?,
        prompt_tps: r.get(5)?,
        gen_stddev: r.get(6)?,
        gpu_bytes: r.get::<_, Option<i64>>(7)?.map(|v| v.max(0) as u64),
        source: r.get(8)?,
        suspect: r.get::<_, i64>(9)? != 0,
        measured_at: r.get(10)?,
    })
}

const COLUNAS: &str = "machine_key, model_id, profile_key, build_number, gen_tps, prompt_tps, \
                       gen_stddev, gpu_bytes, source, suspect, measured_at";

impl Store {
    #[allow(clippy::too_many_arguments)]
    pub fn add_perf_run(&self, run: &PerfRun) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO perf_runs (machine_key, model_id, profile_key, build_number,
                                    gen_tps, prompt_tps, gen_stddev, gpu_bytes, source,
                                    suspect, measured_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                run.machine_key,
                run.model_id,
                run.profile_key,
                run.build_number as i64,
                run.gen_tps,
                run.prompt_tps,
                run.gen_stddev,
                run.gpu_bytes.map(|v| v as i64),
                run.source,
                run.suspect as i64,
                run.measured_at,
            ],
        )?;
        Ok(())
    }

    /// Medições válidas para esta máquina e este modelo, da mais recente para
    /// a mais antiga.
    ///
    /// Filtra pelo build do llama.cpp: um número medido noutro runtime não
    /// descreve o que a pessoa vai sentir agora.
    pub fn perf_runs(
        &self,
        machine_key: &str,
        model_id: &str,
        build_number: u64,
    ) -> Result<Vec<PerfRun>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUNAS} FROM perf_runs
             WHERE machine_key = ?1 AND model_id = ?2 AND build_number = ?3
             ORDER BY measured_at DESC"
        ))?;
        let rows = stmt
            .query_map(
                params![machine_key, model_id, build_number as i64],
                run_from,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Melhor medição conhecida de uma configuração específica.
    pub fn best_perf(
        &self,
        machine_key: &str,
        model_id: &str,
        profile_key: &str,
        build_number: u64,
    ) -> Result<Option<PerfRun>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUNAS} FROM perf_runs
             WHERE machine_key = ?1 AND model_id = ?2 AND profile_key = ?3
               AND build_number = ?4 AND suspect = 0
             ORDER BY measured_at DESC LIMIT 1"
        ))?;
        let mut rows = stmt.query(params![
            machine_key,
            model_id,
            profile_key,
            build_number as i64
        ])?;
        Ok(match rows.next()? {
            Some(r) => Some(run_from(r)?),
            None => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(profile: &str, tps: f64, build: u64) -> PerfRun {
        PerfRun {
            machine_key: "maquina-a".into(),
            model_id: "m.gguf".into(),
            profile_key: profile.into(),
            build_number: build,
            gen_tps: tps,
            prompt_tps: tps * 10.0,
            gen_stddev: 0.3,
            gpu_bytes: Some(8 << 30),
            source: "bench".into(),
            suspect: false,
            measured_at: 1_000,
        }
    }

    #[test]
    fn a_measurement_comes_back_for_the_same_machine_and_build() {
        let s = Store::open_in_memory().unwrap();
        s.add_perf_run(&run("perfil-a", 41.0, 10441)).unwrap();

        let achados = s.perf_runs("maquina-a", "m.gguf", 10441).unwrap();
        assert_eq!(achados.len(), 1);
        assert!((achados[0].gen_tps - 41.0).abs() < 0.01);

        assert!(
            s.perf_runs("outra-maquina", "m.gguf", 10441)
                .unwrap()
                .is_empty()
        );
    }

    /// Atualizar o runtime muda o que a máquina rende: o número velho não
    /// pode continuar sendo apresentado como se descrevesse o de agora.
    #[test]
    fn a_new_llama_build_retires_the_old_numbers() {
        let s = Store::open_in_memory().unwrap();
        s.add_perf_run(&run("perfil-a", 41.0, 10441)).unwrap();
        assert!(
            s.perf_runs("maquina-a", "m.gguf", 10500)
                .unwrap()
                .is_empty()
        );
        // Sem apagar: voltar ao runtime antigo devolve a medição.
        assert_eq!(s.perf_runs("maquina-a", "m.gguf", 10441).unwrap().len(), 1);
    }

    #[test]
    fn the_best_known_number_ignores_the_suspect_ones() {
        let s = Store::open_in_memory().unwrap();
        let mut quente = run("perfil-a", 22.0, 10441);
        quente.suspect = true;
        quente.measured_at = 2_000;
        s.add_perf_run(&run("perfil-a", 41.0, 10441)).unwrap();
        s.add_perf_run(&quente).unwrap();

        let melhor = s
            .best_perf("maquina-a", "m.gguf", "perfil-a", 10441)
            .unwrap()
            .unwrap();
        assert!(
            (melhor.gen_tps - 41.0).abs() < 0.01,
            "a medição com a placa quente não pode virar a verdade"
        );
        assert!(
            s.best_perf("maquina-a", "m.gguf", "outro", 10441)
                .unwrap()
                .is_none()
        );
    }
}
