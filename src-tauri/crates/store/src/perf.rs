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
use rusqlite::{Connection, OptionalExtension, Row, params};
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
    measured_at INTEGER NOT NULL,
    gpu_name TEXT,
    profile_json TEXT,
    n_prompt INTEGER,
    n_depth INTEGER,
    power_limit_w INTEGER
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
    /// Nome da placa principal na hora da medição (`None` = CPU, ou linha
    /// gravada antes desta coluna existir).
    pub gpu_name: Option<String>,
    /// Limite de energia da placa em vigor na medição, em watts.
    ///
    /// Sem isto, duas corridas com limites diferentes ficariam com a MESMA
    /// identidade de configuração e o Δ% entre elas atribuiria à configuração
    /// uma diferença que era de watts. Com a coluna, comparar 370 W e 250 W
    /// deixa de ser uma armadilha e vira o experimento que o card de energia
    /// promete.
    pub power_limit_w: Option<u32>,
    /// Os pares INI legíveis do perfil medido, em JSON — é o que permite à
    /// tela mostrar a configuração sem decifrar o hash de `profile_key`.
    pub profile_json: Option<String>,
    /// Com que tamanho de prompt `prompt_tps` foi medido. `None` em linhas
    /// gravadas antes desta coluna — e é por isso que elas não ganham Δ de
    /// prompt: não dá para saber se são comparáveis.
    pub n_prompt: Option<u32>,
    /// Quantos tokens já havia no contexto durante a geração.
    pub n_depth: Option<u32>,
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
        gpu_name: r.get(11)?,
        profile_json: r.get(12)?,
        n_prompt: r.get::<_, Option<i64>>(13)?.map(|v| v.max(0) as u32),
        n_depth: r.get::<_, Option<i64>>(14)?.map(|v| v.max(0) as u32),
        power_limit_w: r.get::<_, Option<i64>>(15)?.map(|v| v.max(0) as u32),
    })
}

const COLUNAS: &str = "machine_key, model_id, profile_key, build_number, gen_tps, prompt_tps, \
                       gen_stddev, gpu_bytes, source, suspect, measured_at, gpu_name, \
                       profile_json, n_prompt, n_depth, power_limit_w";

impl Store {
    #[allow(clippy::too_many_arguments)]
    pub fn add_perf_run(&self, run: &PerfRun) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO perf_runs (machine_key, model_id, profile_key, build_number,
                                    gen_tps, prompt_tps, gen_stddev, gpu_bytes, source,
                                    suspect, measured_at, gpu_name, profile_json,
                                    n_prompt, n_depth, power_limit_w)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
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
                run.gpu_name,
                run.profile_json,
                run.n_prompt.map(|v| v as i64),
                run.n_depth.map(|v| v as i64),
                run.power_limit_w.map(|v| v as i64),
            ],
        )?;
        Ok(())
    }

    /// Medições válidas para esta máquina e este modelo, da mais recente para
    /// a mais antiga.
    ///
    /// Filtra pelo build do llama.cpp: um número medido noutro runtime não
    /// descreve o que a pessoa vai sentir agora.
    /// Tokens por segundo mais recente deste modelo NESTA máquina, seja qual
    /// for o perfil ou o build. É a régua para dizer quanto uma entrega vai
    /// demorar: medição de verdade, não chute.
    ///
    /// Medição suspeita (placa quente) entra: para uma estimativa ela ainda
    /// vale muito mais do que não ter número nenhum.
    pub fn latest_gen_tps(&self, model_id: &str) -> Result<Option<f64>, StoreError> {
        let conn = self.conn();
        let tps: Option<f64> = conn
            .query_row(
                "SELECT gen_tps FROM perf_runs
                 WHERE model_id = ?1 AND gen_tps > 0
                 ORDER BY measured_at DESC LIMIT 1",
                params![model_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(tps)
    }

    pub fn perf_runs(
        &self,
        machine_key: &str,
        model_id: &str,
        build_number: u64,
    ) -> Result<Vec<PerfRun>, StoreError> {
        let conn = self.conn();
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
        let conn = self.conn();
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

    /// O histórico de medições desta máquina para um modelo, da mais recente
    /// para a mais antiga, SEM filtro de build: a tela mostra a série inteira
    /// e marca onde o motor mudou. Medições de outra placa/driver ficam de
    /// fora por construção — a `machine_key` delas é outra, e a série antiga
    /// simplesmente se "aposenta" sem ser apagada.
    pub fn perf_history_rows(
        &self,
        machine_key: &str,
        model_id: &str,
        limit: usize,
    ) -> Result<Vec<PerfRun>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            // `id DESC` desempata medições do mesmo instante (um bench de
            // vários perfis grava todas com o mesmo carimbo).
            "SELECT {COLUNAS} FROM perf_runs
             WHERE machine_key = ?1 AND model_id = ?2
             ORDER BY measured_at DESC, id DESC LIMIT ?3"
        ))?;
        let rows = stmt
            .query_map(params![machine_key, model_id, limit as i64], run_from)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Uso real por configuração: média de tokens/s das respostas do chat,
    /// colhida passivamente a cada mensagem.
    ///
    /// Nota: `messages` NÃO tem `machine_key` — este agregado não é recortado
    /// por GPU/driver, ao contrário do histórico de bench.
    pub fn perf_usage_rows(&self, model_id: &str) -> Result<Vec<UsageRow>, StoreError> {
        // O chat grava o id como o Router o expõe — às vezes sem ".gguf".
        // Casar pelo nome E pelo stem evita que o agregado suma por causa do
        // sufixo (a tela sempre pergunta pelo nome de arquivo).
        let stem = model_id.trim_end_matches(".gguf");
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT profile_key, AVG(tokens_per_sec), COUNT(*) FROM messages
             WHERE model_id IN (?1, ?2) AND tokens_per_sec IS NOT NULL
               AND profile_key IS NOT NULL
             GROUP BY profile_key
             ORDER BY COUNT(*) DESC",
        )?;
        let rows = stmt
            .query_map(params![model_id, stem], |r| {
                Ok(UsageRow {
                    profile_key: r.get(0)?,
                    avg_tps: r.get(1)?,
                    samples: r.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

/// Uso real agregado de uma configuração (respostas do chat).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRow {
    pub profile_key: String,
    pub avg_tps: f64,
    pub samples: i64,
}

/// O que a comparação com a linha anterior rende.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Delta {
    /// Variação da GERAÇÃO, em pontos percentuais.
    pub gen_pct: Option<f64>,
    pub gen_reason: &'static str,
    /// Variação do PROCESSAMENTO DO PROMPT. Separada porque tem uma condição
    /// a mais: só compara medições feitas com o MESMO tamanho de prompt.
    pub prompt_pct: Option<f64>,
    pub prompt_reason: &'static str,
}

/// Delta de cada linha do histórico sobre a antecessora imediata.
///
/// `rows` vem da mais recente para a mais antiga (como devolve
/// [`Store::perf_history_rows`]); a antecessora de `rows[i]` é `rows[i+1]`.
///
/// Razões possíveis: `"ok"`, `"first"` (sem antecessora ou sem base
/// positiva), `"buildChange"` (o motor mudou entre as duas), `"suspect"` (uma
/// delas foi medida com a placa quente) e — só do lado do prompt —
/// `"promptChanged"`, quando as duas medições usaram prompts de tamanhos
/// diferentes. Esse último caso não é raro: perfil com especialistas na CPU
/// é medido com prompt longo justamente porque é lá que o micro-lote aparece,
/// e comparar esse número com o de um prompt curto seria inventar uma piora.
///
/// A geração NÃO depende do tamanho do prompt — depende da profundidade do
/// contexto —, então ela é comparada sempre que a profundidade bate.
pub fn annotate_deltas(rows: &[PerfRun]) -> Vec<Delta> {
    rows.iter()
        .enumerate()
        .map(|(i, atual)| {
            let nada = |razao: &'static str| Delta {
                gen_pct: None,
                gen_reason: razao,
                prompt_pct: None,
                prompt_reason: razao,
            };
            let Some(anterior) = rows.get(i + 1) else {
                return nada("first");
            };
            if anterior.build_number != atual.build_number {
                return nada("buildChange");
            }
            if atual.suspect || anterior.suspect {
                return nada("suspect");
            }
            if atual.n_depth.unwrap_or(0) != anterior.n_depth.unwrap_or(0) {
                return nada("promptChanged");
            }
            // Limite de energia diferente NÃO anula o percentual: medir o
            // efeito dos watts é justamente o que o card de energia oferece.
            // O que muda é a razão — a tela precisa dizer que a diferença é
            // de watts, e não creditá-la à configuração.
            let energia_mudou = match (atual.power_limit_w, anterior.power_limit_w) {
                (Some(a), Some(b)) => a != b,
                _ => false,
            };

            let razao_ok = if energia_mudou { "powerChanged" } else { "ok" };
            let (gen_pct, gen_reason) = if anterior.gen_tps > 0.0 {
                (
                    Some((atual.gen_tps - anterior.gen_tps) / anterior.gen_tps * 100.0),
                    razao_ok,
                )
            } else {
                // Sem base positiva não existe percentual honesto — e um
                // infinito aqui viraria `null` no JSON com razão "ok".
                (None, "first")
            };

            let mesmo_prompt = atual.n_prompt.is_some() && atual.n_prompt == anterior.n_prompt;
            let (prompt_pct, prompt_reason) = if !mesmo_prompt {
                (None, "promptChanged")
            } else if anterior.prompt_tps > 0.0 {
                (
                    Some((atual.prompt_tps - anterior.prompt_tps) / anterior.prompt_tps * 100.0),
                    razao_ok,
                )
            } else {
                (None, "first")
            };

            Delta {
                gen_pct,
                gen_reason,
                prompt_pct,
                prompt_reason,
            }
        })
        .collect()
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
            gpu_name: Some("RTX de Teste".into()),
            profile_json: None,
            n_prompt: Some(512),
            n_depth: Some(0),
            power_limit_w: None,
        }
    }

    /// Watts diferentes NÃO invalidam a comparação — medir o efeito da
    /// energia é o objetivo. O que muda é a razão, para a tela poder dizer
    /// que a diferença veio dos watts e não da configuração.
    #[test]
    fn a_different_power_limit_is_flagged_but_still_compared() {
        let mut novo = run("perfil-a", 40.0, 100);
        let mut antigo = run("perfil-a", 50.0, 100);
        novo.power_limit_w = Some(250);
        antigo.power_limit_w = Some(370);

        let annos = annotate_deltas(&[novo, antigo]);
        assert_eq!(annos[0].gen_reason, "powerChanged");
        let pct = annos[0].gen_pct.expect("o percentual continua valendo");
        assert!(
            (pct - (-20.0)).abs() < 0.01,
            "40 contra 50 é -20%, deu {pct}"
        );
    }

    /// Mesmo limite, ou limite desconhecido em uma das linhas: comparação
    /// normal. Marcar tudo seria ruído.
    #[test]
    fn the_same_power_limit_compares_as_usual() {
        let mut a = run("perfil-a", 44.0, 100);
        let mut b = run("perfil-a", 40.0, 100);
        a.power_limit_w = Some(370);
        b.power_limit_w = Some(370);
        assert_eq!(annotate_deltas(&[a, b])[0].gen_reason, "ok");

        // Linha antiga, gravada antes da coluna existir.
        let c = run("perfil-a", 44.0, 100);
        let mut d = run("perfil-a", 40.0, 100);
        d.power_limit_w = Some(370);
        assert_eq!(annotate_deltas(&[c, d])[0].gen_reason, "ok");
    }

    /// Como `run`, mas com o instante escolhido — o histórico ordena por ele.
    fn run_at(profile: &str, tps: f64, build: u64, at: i64) -> PerfRun {
        PerfRun {
            measured_at: at,
            ..run(profile, tps, build)
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

    /// Banco criado antes de `gpu_name`/`profile_json`: a migração aditiva
    /// tem de acrescentá-las sem tocar as linhas antigas — e reaplicar a
    /// migração num banco já migrado é no-op seguro.
    #[test]
    fn migration_adds_gpu_and_profile_columns_to_an_old_db() {
        let conn = Connection::open_in_memory().unwrap();
        // Esquema ANTIGO de perf_runs, à mão; as demais tabelas o init cria.
        conn.execute_batch(
            r#"
            CREATE TABLE perf_runs (
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
            "#,
        )
        .unwrap();
        conn.execute(
            "INSERT INTO perf_runs (machine_key, model_id, profile_key, build_number,
                                    gen_tps, prompt_tps, gen_stddev, gpu_bytes, source,
                                    suspect, measured_at)
             VALUES ('maquina-a', 'm.gguf', 'perfil-a', 10441,
                     41.0, 410.0, 0.3, NULL, 'bench', 0, 500)",
            [],
        )
        .unwrap();

        let s = crate::Store::init(conn).unwrap();

        // A linha antiga continua legível, com as colunas novas em NULL.
        let rows = s.perf_history_rows("maquina-a", "m.gguf", 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].gpu_name, None);
        assert_eq!(rows[0].profile_json, None);

        // Gravação nova preenche as colunas novas.
        let mut novo = run_at("perfil-a", 43.0, 10441, 900);
        novo.profile_json = Some(r#"[["ctx-size","16384"]]"#.into());
        s.add_perf_run(&novo).unwrap();
        let rows = s.perf_history_rows("maquina-a", "m.gguf", 10).unwrap();
        assert_eq!(rows[0].gpu_name.as_deref(), Some("RTX de Teste"));
        assert_eq!(
            rows[0].profile_json.as_deref(),
            Some(r#"[["ctx-size","16384"]]"#)
        );

        // Idempotência: reaplicar a migração não muda nada.
        {
            let conn = s.conn();
            crate::ensure_column(&conn, "perf_runs", "gpu_name", "TEXT").unwrap();
            crate::ensure_column(&conn, "perf_runs", "profile_json", "TEXT").unwrap();
        }
        assert_eq!(
            s.perf_history_rows("maquina-a", "m.gguf", 10)
                .unwrap()
                .len(),
            2
        );
    }

    /// O histórico não filtra por build (a série inteira aparece), ordena da
    /// mais recente para a mais antiga e respeita o LIMIT — é o LIMIT+1 do
    /// chamador que dá a antecessora da borda.
    #[test]
    fn history_crosses_builds_orders_desc_and_limits() {
        let s = Store::open_in_memory().unwrap();
        s.add_perf_run(&run_at("a", 40.0, 10441, 100)).unwrap();
        s.add_perf_run(&run_at("a", 42.0, 10441, 200)).unwrap();
        s.add_perf_run(&run_at("a", 50.0, 10500, 300)).unwrap();
        // De outra máquina: fora da série.
        let mut outra = run_at("a", 99.0, 10500, 400);
        outra.machine_key = "outra-maquina".into();
        s.add_perf_run(&outra).unwrap();

        let rows = s.perf_history_rows("maquina-a", "m.gguf", 10).unwrap();
        assert_eq!(rows.len(), 3, "builds diferentes convivem na mesma série");
        assert_eq!(rows[0].measured_at, 300);
        assert_eq!(rows[2].measured_at, 100);

        let rows = s.perf_history_rows("maquina-a", "m.gguf", 2).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].measured_at, 300);
    }

    /// Mesmo build, ninguém suspeito: o delta existe e é o percentual sobre a
    /// antecessora; a linha mais antiga da janela fica sem base ("first").
    #[test]
    fn delta_exists_within_the_same_build() {
        let rows = vec![run_at("a", 44.0, 10441, 200), run_at("a", 40.0, 10441, 100)];
        let annos = annotate_deltas(&rows);
        assert_eq!(annos[0].gen_reason, "ok");
        assert!((annos[0].gen_pct.unwrap() - 10.0).abs() < 1e-9);
        assert_eq!(annos[1].gen_reason, "first");
        assert_eq!(annos[1].gen_pct, None);
    }

    /// Motor atualizado entre as duas medições: números de builds diferentes
    /// não se comparam.
    #[test]
    fn delta_is_absent_across_a_build_change() {
        let rows = vec![run_at("a", 50.0, 10500, 200), run_at("a", 40.0, 10441, 100)];
        let annos = annotate_deltas(&rows);
        assert_eq!(annos[0].gen_pct, None);
        assert_eq!(annos[0].gen_reason, "buildChange");
    }

    /// Uma medição suspeita no meio contamina os dois deltas que a tocam: o
    /// dela (a própria linha) e o da linha seguinte (a antecessora).
    #[test]
    fn a_suspect_row_blocks_both_neighboring_deltas() {
        let mut quente = run_at("a", 22.0, 10441, 200);
        quente.suspect = true;
        let rows = vec![
            run_at("a", 41.0, 10441, 300),
            quente,
            run_at("a", 40.0, 10441, 100),
        ];
        let annos = annotate_deltas(&rows);
        assert_eq!(annos[0].gen_reason, "suspect", "antecessora suspeita");
        assert_eq!(annos[1].gen_reason, "suspect", "a própria linha é suspeita");
        assert_eq!(annos[2].gen_reason, "first");
        assert!(annos.iter().all(|d| d.gen_pct.is_none()));
    }

    /// Série de uma linha só: sem antecessora, sem delta.
    #[test]
    fn a_single_row_has_no_delta() {
        let rows = vec![run_at("a", 40.0, 10441, 100)];
        let annos = annotate_deltas(&rows);
        assert_eq!(annos.len(), 1);
        assert_eq!(annos[0].gen_reason, "first");
        assert_eq!(annos[0].gen_pct, None);
    }

    /// O ponto do vídeo, virado teste: um perfil que tira especialistas da
    /// placa é medido com prompt LONGO, e comparar esse número com o de um
    /// prompt curto inventaria uma piora que não existe. A geração, que não
    /// depende do tamanho do prompt, continua comparável.
    #[test]
    fn a_longer_prompt_is_not_a_worse_prompt() {
        let mut longo = run_at("moe", 18.0, 10441, 200);
        longo.n_prompt = Some(4096);
        longo.prompt_tps = 340.0;
        let mut curto = run_at("denso", 16.0, 10441, 100);
        curto.n_prompt = Some(512);
        curto.prompt_tps = 900.0;

        let annos = annotate_deltas(&[longo, curto]);
        assert_eq!(annos[0].prompt_pct, None, "prompts de tamanhos diferentes");
        assert_eq!(annos[0].prompt_reason, "promptChanged");
        assert_eq!(annos[0].gen_reason, "ok", "gerar não depende do prompt");
        assert!((annos[0].gen_pct.unwrap() - 12.5).abs() < 1e-9);
    }

    /// Mesmo tamanho de prompt: aí sim o ganho de leitura é um ganho.
    #[test]
    fn the_same_prompt_size_makes_the_prompt_delta_honest() {
        let mut depois = run_at("ub2048", 17.0, 10441, 200);
        depois.n_prompt = Some(4096);
        depois.prompt_tps = 345.0;
        let mut antes = run_at("ub512", 17.0, 10441, 100);
        antes.n_prompt = Some(4096);
        antes.prompt_tps = 23.0;

        let annos = annotate_deltas(&[depois, antes]);
        assert_eq!(annos[0].prompt_reason, "ok");
        assert!(annos[0].prompt_pct.unwrap() > 1000.0, "22 → 345 tok/s");
    }

    /// Linha antiga (sem a coluna) não ganha Δ de prompt: não dá para saber
    /// se ela é comparável, e chutar que sim é o defeito que se quer evitar.
    #[test]
    fn a_row_from_before_the_column_gets_no_prompt_delta() {
        let mut nova = run_at("a", 40.0, 10441, 200);
        nova.n_prompt = Some(512);
        let mut antiga = run_at("a", 40.0, 10441, 100);
        antiga.n_prompt = None;

        let annos = annotate_deltas(&[nova, antiga]);
        assert_eq!(annos[0].prompt_reason, "promptChanged");
        assert_eq!(annos[0].gen_reason, "ok", "a geração ainda se compara");
    }

    /// Perfil vazio não tem chave (`key()` = None), mas o bench grava `""`:
    /// a normalização dos dois lados tem de cair na mesma string vazia.
    #[test]
    fn the_empty_profile_key_normalizes_to_the_recorded_empty_string() {
        use lr_types::tuning::ModelProfile;
        let chave_vigente = ModelProfile::default().key().unwrap_or_default();
        assert_eq!(chave_vigente, "");

        let s = Store::open_in_memory().unwrap();
        s.add_perf_run(&run_at("", 40.0, 10441, 100)).unwrap();
        let rows = s.perf_history_rows("maquina-a", "m.gguf", 10).unwrap();
        assert_eq!(rows[0].profile_key, chave_vigente);
    }

    /// A média de uso vem das respostas do chat, agrupada por configuração,
    /// ignorando mensagens sem tokens/s ou sem chave de perfil.
    #[test]
    fn usage_aggregates_chat_messages_by_profile() {
        let s = Store::open_in_memory().unwrap();
        let chat = s.create_chat("Conversa", None).unwrap();
        let msg = |tps: Option<f64>, profile: Option<&str>| {
            s.add_message(
                chat,
                "assistant",
                "ok",
                tps,
                Some(10),
                Some(1000),
                Some("m.gguf"),
                profile,
                None,
            )
            .unwrap();
        };
        msg(Some(40.0), Some("perfil-a"));
        msg(Some(44.0), Some("perfil-a"));
        msg(Some(30.0), Some("perfil-b"));
        msg(Some(99.0), None); // sem perfil: fora do agregado
        msg(None, Some("perfil-a")); // sem medição: fora do agregado
        // O Router às vezes expõe o id sem o sufixo — a mensagem gravada
        // assim precisa entrar no MESMO agregado do arquivo.
        s.add_message(
            chat,
            "assistant",
            "ok",
            Some(44.0),
            Some(10),
            Some(1000),
            Some("m"),
            Some("perfil-b"),
            None,
        )
        .unwrap();

        let uso = s.perf_usage_rows("m.gguf").unwrap();
        assert_eq!(uso.len(), 2);
        let a = uso.iter().find(|u| u.profile_key == "perfil-a").unwrap();
        assert_eq!(a.samples, 2);
        assert!((a.avg_tps - 42.0).abs() < 1e-9);
        let b = uso.iter().find(|u| u.profile_key == "perfil-b").unwrap();
        assert_eq!(b.samples, 2, "o id sem .gguf cai no mesmo agregado");
        assert!((b.avg_tps - 37.0).abs() < 1e-9);

        assert!(s.perf_usage_rows("outro.gguf").unwrap().is_empty());
    }
}
