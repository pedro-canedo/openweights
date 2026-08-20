//! Quanto tempo cada entrega vai levar.
//!
//! A conta é simples de propósito: os tokens que a entrega deve gastar
//! (`est_tokens`, escolhido na decomposição pelo tamanho da janela) divididos
//! pela velocidade REAL desta máquina com este modelo (`perf_runs.gen_tps`,
//! medido pelo próprio app), mais uma folga para o que não é geração —
//! ferramentas rodando, arquivos indo ao disco, o modelo relendo o plano.
//!
//! Sem medição não há estimativa: mostrar um número inventado seria pior que
//! não mostrar nenhum, porque a pessoa planeja a vida em cima dele.

use lr_store::Store;
use lr_types::scout::TaskPlan;

/// Velocidade mínima aceita. Abaixo disso a medição é ruído (a placa estava
/// ocupada, o primeiro token demorou) e a estimativa viraria hora cheia.
const PISO_TPS: f64 = 1.0;

/// Margem sobre o tempo de geração pura: uma etapa também lê, escreve e roda
/// comandos. Medido grosso — a estimativa é aproximada e a interface a mostra
/// com "≈".
const FOLGA: f64 = 1.3;

/// Teto de uma estimativa por etapa (2 h). Acima disso o número deixa de
/// informar e só assusta.
const TETO_S: u32 = 7_200;

/// Preenche `eta_seconds` de cada entrega ainda não concluída.
///
/// Devolve `true` quando pelo menos uma estimativa foi escrita — sem tok/s
/// medido para este modelo, o plano volta sem estimativa nenhuma.
pub(crate) fn estimar(store: &Store, model: &str, plan: &mut TaskPlan) -> bool {
    let tps = match store.latest_gen_tps(model) {
        Ok(Some(tps)) if tps >= PISO_TPS => tps,
        Ok(_) => return false,
        Err(e) => {
            log::warn!("não deu para ler a velocidade medida de {model}: {e}");
            return false;
        }
    };
    let mut mudou = false;
    for task in &mut plan.tasks {
        if task.status.is_finished() {
            continue;
        }
        if task.est_tokens == 0 {
            continue;
        }
        let segundos = (f64::from(task.est_tokens) / tps * FOLGA).ceil();
        let segundos = (segundos as u32).clamp(1, TETO_S);
        if task.eta_seconds != Some(segundos) {
            task.eta_seconds = Some(segundos);
            mudou = true;
        }
    }
    mudou
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_types::scout::{Task, TaskStatus};

    fn plano() -> TaskPlan {
        TaskPlan {
            goal: "x".into(),
            tasks: vec![
                Task {
                    est_tokens: 1_000,
                    ..Task::new("t1", "a")
                },
                Task {
                    est_tokens: 1_000,
                    status: TaskStatus::Done,
                    ..Task::new("t2", "b")
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn sem_medicao_nao_ha_estimativa() {
        let store = Store::open_in_memory().unwrap();
        let mut p = plano();
        assert!(!estimar(&store, "modelo-sem-medida", &mut p));
        assert!(p.tasks.iter().all(|t| t.eta_seconds.is_none()));
    }

    /// A estimativa sai da velocidade MEDIDA e não toca no que já terminou.
    #[test]
    fn estima_pelo_tps_medido_e_ignora_o_que_acabou() {
        let store = Store::open_in_memory().unwrap();
        store
            .add_perf_run(&lr_store::perf::PerfRun {
                machine_key: "m".into(),
                model_id: "modelo".into(),
                profile_key: "p".into(),
                build_number: 1,
                gen_tps: 10.0,
                prompt_tps: 100.0,
                gen_stddev: 0.1,
                gpu_bytes: None,
                source: "bench".into(),
                suspect: false,
                measured_at: 1,
            })
            .unwrap();
        let mut p = plano();
        assert!(estimar(&store, "modelo", &mut p));
        // 1000 tokens ÷ 10 tok/s × 1,3 de folga = 130 s.
        assert_eq!(p.tasks[0].eta_seconds, Some(130));
        assert_eq!(
            p.tasks[1].eta_seconds, None,
            "etapa concluída não recebe previsão"
        );
    }
}
