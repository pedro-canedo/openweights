//! Scout Rule: quebrar o objetivo em entregas pequenas.
//!
//! Modelo local costuma trabalhar com janela curta (32k é comum). Empurrar
//! uma tarefa grande de uma vez enche o contexto, o modelo perde o fio e
//! começa a inventar. A saída é sempre a mesma: dividir.
//!
//! O plano é a memória do trabalho. Cada tarefa roda com contexto **novo** —
//! o que ela precisa saber do que veio antes chega por um resumo curto
//! (`handoff`), não pelo histórico inteiro. Assim a janela nunca cresce com o
//! tamanho da tarefa, só com o tamanho de UMA etapa.

use serde::{Deserialize, Serialize};

/// Como o agente trabalha nesta conversa. Independe do nível de autorização
/// (aprovar/automático), que continua em `RunMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum WorkMode {
    /// Conversa: responde sem usar ferramentas.
    Chat,
    /// Planejamento: investiga (só leitura) e propõe o plano. Não executa
    /// nada até a pessoa aprovar. Vale para conversa e para agente.
    Plan,
    /// Agente: executa a tarefa pedida, um passo por vez.
    #[default]
    Agent,
    /// Laço: executa o plano inteiro, tarefa a tarefa, verificando cada
    /// entrega antes de seguir. Só existe no modo agente.
    Loop,
}

impl WorkMode {
    /// Modos em que o agente pode alterar alguma coisa.
    pub fn can_execute(self) -> bool {
        matches!(self, WorkMode::Agent | WorkMode::Loop)
    }

    /// Modos que produzem um plano antes de agir.
    pub fn plans_first(self) -> bool {
        matches!(self, WorkMode::Plan | WorkMode::Loop)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TaskStatus {
    #[default]
    Pending,
    Running,
    Done,
    /// Precisa de algo que não temos (decisão da pessoa, credencial, etc.).
    Blocked,
    Failed,
    Skipped,
}

impl TaskStatus {
    pub fn is_finished(self) -> bool {
        matches!(self, TaskStatus::Done | TaskStatus::Skipped)
    }
}

/// Uma entrega pequena: o que fazer, como saber que ficou pronto e o que
/// sobrou para a próxima.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    /// Título curto (aparece na lista).
    pub title: String,
    /// Instrução concreta para esta etapa.
    pub instruction: String,
    /// Como conferir que terminou — vira a verificação da etapa.
    pub done_when: String,
    pub status: TaskStatus,
    /// Resumo do que esta tarefa produziu, escrito para quem vem depois.
    /// É isto (e não o histórico) que atravessa para a próxima etapa.
    #[serde(default)]
    pub handoff: Option<String>,
    /// Arquivos tocados (a interface mostra e a verificação confere).
    #[serde(default)]
    pub files: Vec<String>,
    /// Índice das tarefas que precisam terminar antes desta.
    #[serde(default)]
    pub depends_on: Vec<usize>,
    /// Custo estimado em tokens (decomposição usa para não estourar a janela).
    #[serde(default)]
    pub est_tokens: u32,
    #[serde(default)]
    pub error: Option<String>,
}

impl Task {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            instruction: String::new(),
            done_when: String::new(),
            status: TaskStatus::Pending,
            handoff: None,
            files: Vec::new(),
            depends_on: Vec::new(),
            est_tokens: 0,
            error: None,
        }
    }
}

/// O plano de trabalho de uma execução.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TaskPlan {
    /// O que a pessoa pediu, em uma frase.
    pub goal: String,
    pub tasks: Vec<Task>,
    /// Índice da tarefa em andamento.
    #[serde(default)]
    pub current: usize,
    /// Observações da investigação feita antes de planejar.
    #[serde(default)]
    pub notes: String,
    /// `true` depois que a pessoa aprovou o plano (modo planejamento).
    #[serde(default)]
    pub approved: bool,
}

impl TaskPlan {
    pub fn progress(&self) -> (usize, usize) {
        (
            self.tasks.iter().filter(|t| t.status.is_finished()).count(),
            self.tasks.len(),
        )
    }

    pub fn is_complete(&self) -> bool {
        !self.tasks.is_empty() && self.tasks.iter().all(|t| t.status.is_finished())
    }

    /// Próxima tarefa executável: pendente e com as dependências prontas.
    pub fn next_index(&self) -> Option<usize> {
        self.tasks.iter().position(|t| {
            t.status == TaskStatus::Pending
                && t.depends_on
                    .iter()
                    .all(|i| self.tasks.get(*i).is_some_and(|d| d.status.is_finished()))
        })
    }

    /// Resumo do que já foi feito, para abrir o contexto da próxima tarefa.
    /// É o coração da Scout Rule: cabe em poucas linhas, não no histórico.
    pub fn handoff_digest(&self, limit: usize) -> String {
        let done: Vec<&Task> = self
            .tasks
            .iter()
            .filter(|t| t.status.is_finished() && t.handoff.is_some())
            .collect();
        if done.is_empty() {
            return String::new();
        }
        done.iter()
            .rev()
            .take(limit)
            .rev()
            .map(|t| {
                let h = t.handoff.as_deref().unwrap_or("");
                format!("- {}: {}", t.title, h)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Lista em markdown para a pessoa acompanhar (e para o modelo reler).
    pub fn to_markdown(&self) -> String {
        let (done, total) = self.progress();
        let mut out = format!("Objetivo: {}\nProgresso: {done}/{total}\n", self.goal);
        for (i, t) in self.tasks.iter().enumerate() {
            let mark = match t.status {
                TaskStatus::Done => "x",
                TaskStatus::Running => ">",
                TaskStatus::Blocked => "!",
                TaskStatus::Failed => "e",
                TaskStatus::Skipped => "-",
                TaskStatus::Pending => " ",
            };
            out.push_str(&format!("{}. [{mark}] {}\n", i + 1, t.title));
        }
        out
    }
}

/// Eventos do plano — a interface desenha a divisão do trabalho a partir
/// deles (o usuário quer ver a tarefa sendo quebrada e avançando).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum PlanEventKind {
    #[serde(rename = "plan.created")]
    PlanCreated { plan: TaskPlan },
    #[serde(rename = "plan.updated")]
    PlanUpdated { plan: TaskPlan },
    #[serde(rename = "plan.approved")]
    PlanApproved,
    #[serde(rename = "task.started")]
    TaskStarted { index: usize, task: Task },
    #[serde(rename = "task.finished")]
    TaskFinished { index: usize, task: Task },
}

/// Orçamento de janela para a decomposição.
#[derive(Debug, Clone, Copy)]
pub struct WindowBudget {
    /// Janela do modelo (de `/props`).
    pub n_ctx: u32,
    /// Fração reservada para o trabalho de UMA tarefa.
    pub task_ratio: f32,
}

impl Default for WindowBudget {
    fn default() -> Self {
        // 32k é o comum em modelo local; a fração deixa espaço para o
        // sistema, o handoff e a resposta.
        Self {
            n_ctx: 32_768,
            task_ratio: 0.45,
        }
    }
}

impl WindowBudget {
    pub fn new(n_ctx: u32) -> Self {
        Self {
            n_ctx: n_ctx.max(2048),
            ..Default::default()
        }
    }

    /// Tokens que uma tarefa pode ocupar.
    pub fn per_task(&self) -> u32 {
        (self.n_ctx as f32 * self.task_ratio) as u32
    }

    /// Quantas tarefas pedir na decomposição, dado o tamanho do pedido.
    /// Janela pequena → mais tarefas, cada uma menor.
    pub fn suggested_tasks(&self, prompt_tokens: u32) -> u32 {
        let base = match self.n_ctx {
            0..=8_192 => 8,
            8_193..=16_384 => 6,
            16_385..=49_152 => 5,
            _ => 4,
        };
        // Pedido grande já ocupa janela: divide mais.
        let extra = (prompt_tokens / self.per_task().max(1)).min(4);
        (base + extra).clamp(2, 12)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_with(states: &[TaskStatus]) -> TaskPlan {
        TaskPlan {
            goal: "arrumar o projeto".into(),
            tasks: states
                .iter()
                .enumerate()
                .map(|(i, s)| Task {
                    status: *s,
                    handoff: s.is_finished().then(|| format!("fiz a {i}")),
                    ..Task::new(format!("t{i}"), format!("tarefa {i}"))
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn work_mode_capabilities() {
        assert!(!WorkMode::Chat.can_execute());
        assert!(!WorkMode::Plan.can_execute(), "planejar não altera nada");
        assert!(WorkMode::Agent.can_execute());
        assert!(WorkMode::Loop.can_execute());
        assert!(WorkMode::Plan.plans_first());
        assert!(WorkMode::Loop.plans_first());
        assert!(!WorkMode::Agent.plans_first());
    }

    #[test]
    fn progress_and_completion() {
        let p = plan_with(&[TaskStatus::Done, TaskStatus::Pending, TaskStatus::Done]);
        assert_eq!(p.progress(), (2, 3));
        assert!(!p.is_complete());

        let done = plan_with(&[TaskStatus::Done, TaskStatus::Skipped]);
        assert!(done.is_complete(), "pular conta como resolvido");
        assert!(
            !TaskPlan::default().is_complete(),
            "plano vazio não está pronto"
        );
    }

    #[test]
    fn next_index_respects_dependencies() {
        let mut p = plan_with(&[TaskStatus::Pending, TaskStatus::Pending]);
        p.tasks[1].depends_on = vec![0];
        assert_eq!(p.next_index(), Some(0));

        p.tasks[0].status = TaskStatus::Done;
        assert_eq!(p.next_index(), Some(1));

        p.tasks[1].status = TaskStatus::Running;
        assert_eq!(p.next_index(), None, "nada pendente e liberado");
    }

    #[test]
    fn blocked_task_does_not_release_dependents() {
        let mut p = plan_with(&[TaskStatus::Blocked, TaskStatus::Pending]);
        p.tasks[1].depends_on = vec![0];
        assert_eq!(p.next_index(), None);
    }

    #[test]
    fn handoff_digest_keeps_only_the_recent_ones() {
        let p = plan_with(&[TaskStatus::Done; 5]);
        let digest = p.handoff_digest(2);
        assert_eq!(digest.lines().count(), 2);
        assert!(digest.contains("fiz a 4"), "mantém as últimas");
        assert!(!digest.contains("fiz a 0"));

        assert!(
            plan_with(&[TaskStatus::Pending])
                .handoff_digest(3)
                .is_empty()
        );
    }

    #[test]
    fn markdown_shows_progress_marks() {
        let p = plan_with(&[TaskStatus::Done, TaskStatus::Running, TaskStatus::Pending]);
        let md = p.to_markdown();
        assert!(md.contains("Progresso: 1/3"));
        assert!(md.contains("1. [x]"));
        assert!(md.contains("2. [>]"));
        assert!(md.contains("3. [ ]"));
    }

    #[test]
    fn small_window_asks_for_more_tasks() {
        let small = WindowBudget::new(8_192);
        let large = WindowBudget::new(131_072);
        assert!(
            small.suggested_tasks(500) > large.suggested_tasks(500),
            "janela menor precisa de entregas menores"
        );
        assert!(small.per_task() < small.n_ctx);
    }

    #[test]
    fn a_long_request_is_split_further() {
        let b = WindowBudget::new(32_768);
        assert!(b.suggested_tasks(30_000) > b.suggested_tasks(200));
        assert!(b.suggested_tasks(300_000) <= 12, "sempre dentro do teto");
    }

    #[test]
    fn plan_events_serialize_for_the_ui() {
        let ev = PlanEventKind::TaskStarted {
            index: 1,
            task: Task::new("t1", "ler o README"),
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["kind"], "task.started");
        assert_eq!(v["index"], 1);
        assert_eq!(v["task"]["title"], "ler o README");
        assert_eq!(v["task"]["status"], "pending");
    }
}
