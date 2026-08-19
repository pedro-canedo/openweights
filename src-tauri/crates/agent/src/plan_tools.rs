//! Ferramentas meta que deixam o modelo conduzir o próprio plano.
//!
//! Elas não tocam em arquivo nem em processo: mexem só no plano do run, que
//! vive num `Arc<Mutex<TaskPlan>>` criado pelo laço e **injetado aqui na
//! construção**. É esse compartilhamento que permite o laço saber o que o
//! modelo decidiu sem ter que interpretar texto livre — o modelo chama
//! `task_complete`, o laço lê o plano.
//!
//! Duas delas (`task_complete` e `ask_user`) levantam o sinal `halt`: o
//! trecho de laço em andamento termina assim que a chamada volta. É o que faz
//! uma etapa acabar no instante em que o modelo diz que acabou, em vez de
//! gastar mais um passo só para escrever "pronto".
//!
//! Todas são [`ToolCategory::Meta`]: a política libera sem confirmação,
//! porque nada aqui sai do app.

use crate::scout;
use async_trait::async_trait;
use lr_tools::{SharedTool, Tool, ToolContext, ToolError, ToolOutput, ToolResult, arg_str};
use lr_types::agent::{ToolCategory, ToolSpec};
use lr_types::scout::{TaskPlan, TaskStatus, WorkMode};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// O plano do run, compartilhado entre o laço e as ferramentas meta.
pub type SharedPlan = Arc<Mutex<TaskPlan>>;

/// Teto de um handoff: é um resumo para a próxima etapa, não um relatório.
const MAX_HANDOFF_CHARS: usize = 400;
/// Teto de uma observação avulsa do plano.
const MAX_NOTE_CHARS: usize = 300;

/// Cria o plano compartilhado de um run.
pub fn shared_plan(plan: TaskPlan) -> SharedPlan {
    Arc::new(Mutex::new(plan))
}

/// Pega o plano mesmo se alguém tiver entrado em pânico segurando o cadeado:
/// perder o plano no meio do run seria pior do que seguir com ele.
pub(crate) fn lock_plan(plan: &SharedPlan) -> MutexGuard<'_, TaskPlan> {
    plan.lock().unwrap_or_else(|e| e.into_inner())
}

/// Cópia do plano para gravar/emitir sem segurar o cadeado.
pub(crate) fn snapshot(plan: &SharedPlan) -> TaskPlan {
    lock_plan(plan).clone()
}

/// Junta o texto em uma linha e corta no teto (sem partir UTF-8).
fn one_line(text: &str, max: usize) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() <= max {
        return joined;
    }
    joined.chars().take(max).collect::<String>() + "…"
}

/// Lê o índice de uma etapa tolerando as duas contagens.
///
/// O contrato é base 0 (igual a `depends_on`), mas a lista que o modelo vê é
/// numerada a partir de 1 — quando o número não existe no plano e o anterior
/// existe, vale o anterior. Errar para o que existe é melhor que gastar um
/// passo do run com uma mensagem de erro.
fn read_index(args: &Value, total: usize, fallback: usize) -> Option<usize> {
    let Some(raw) = args.get("index").and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
    }) else {
        return (fallback < total).then_some(fallback);
    };
    let idx = raw as usize;
    if idx < total {
        return Some(idx);
    }
    (idx > 0 && idx - 1 < total).then_some(idx - 1)
}

/// Estados que os modelos escrevem, em inglês e em português.
fn status_from_str(raw: &str) -> Option<TaskStatus> {
    match raw.trim().to_lowercase().as_str() {
        "pending" | "todo" | "pendente" => Some(TaskStatus::Pending),
        "running" | "doing" | "andamento" | "em andamento" => Some(TaskStatus::Running),
        "done" | "completed" | "ok" | "concluida" | "concluída" => Some(TaskStatus::Done),
        "blocked" | "bloqueada" | "bloqueado" => Some(TaskStatus::Blocked),
        "failed" | "error" | "falhou" | "erro" => Some(TaskStatus::Failed),
        "skipped" | "pulada" | "ignorada" => Some(TaskStatus::Skipped),
        _ => None,
    }
}

fn no_plan() -> ToolError {
    ToolError::InvalidArgs(
        "ainda não existe um plano nesta execução; crie um com `plan_create` antes".into(),
    )
}

// ------------------------------------------------------------ plan_create ---

/// Escreve (ou substitui) o plano inteiro.
pub struct PlanCreate {
    plan: SharedPlan,
    halt: Arc<AtomicBool>,
}

#[async_trait]
impl Tool for PlanCreate {
    fn name(&self) -> &str {
        "plan_create"
    }

    fn description(&self) -> &str {
        "Divide o objetivo em entregas pequenas e registra o plano. Cada etapa precisa caber \
         sozinha na janela do modelo e ter um critério objetivo de conclusão."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "goal": {
                    "type": "string",
                    "description": "O que a pessoa pediu, em uma frase."
                },
                "tasks": {
                    "type": "array",
                    "description": "As entregas, na ordem de execução.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "title": { "type": "string", "description": "Título curto da etapa." },
                            "instruction": { "type": "string", "description": "O que fazer nesta etapa, de forma concreta." },
                            "done_when": { "type": "string", "description": "Como conferir que a etapa terminou." },
                            "files": {
                                "type": "array",
                                "description": "Caminhos que esta etapa cria ou altera, relativos à pasta do projeto. É o que a conferência de entrega procura em disco.",
                                "items": { "type": "string" }
                            },
                            "depends_on": {
                                "type": "array",
                                "description": "Índices (a partir de 0) das etapas anteriores que precisam terminar antes desta.",
                                "items": { "type": "integer" }
                            }
                        },
                        "required": ["title", "instruction", "done_when"]
                    }
                }
            },
            "required": ["goal", "tasks"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Meta
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let goal = args
            .get("goal")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        let tasks = args.get("tasks").cloned().unwrap_or(Value::Null);
        let mut built =
            scout::plan_from_value(&goal, &json!({ "tasks": tasks }), 0).ok_or_else(|| {
                ToolError::InvalidArgs(
                    "mande `tasks` como uma lista de etapas com `title`, `instruction`, \
                     `done_when` e `files`"
                        .into(),
                )
            })?;

        let md = {
            let mut current = lock_plan(&self.plan);
            // O objetivo e as observações da investigação sobrevivem quando o
            // modelo não os repete na chamada.
            if built.goal.is_empty() {
                built.goal = current.goal.clone();
            }
            if built.notes.is_empty() {
                built.notes = current.notes.clone();
            }
            *current = built;
            current.to_markdown()
        };
        // O plano é a entrega deste passo: o laço para aqui e decide.
        self.halt.store(true, Ordering::SeqCst);
        Ok(ToolOutput::text(format!("Plano registrado.\n{md}")))
    }
}

// ------------------------------------------------------------ plan_update ---

/// Ajusta uma etapa já existente.
pub struct PlanUpdate {
    plan: SharedPlan,
}

#[async_trait]
impl Tool for PlanUpdate {
    fn name(&self) -> &str {
        "plan_update"
    }

    fn description(&self) -> &str {
        "Atualiza uma etapa do plano: estado, resumo do que ela produziu ou uma observação. \
         Use quando algo mudar no meio do caminho."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "index": {
                    "type": "integer",
                    "description": "Índice da etapa, começando em 0."
                },
                "status": {
                    "type": "string",
                    "description": "Novo estado da etapa.",
                    "enum": ["pending", "running", "done", "blocked", "failed", "skipped"]
                },
                "handoff": {
                    "type": "string",
                    "description": "Resumo do que a etapa produziu, escrito para quem vem depois."
                },
                "note": {
                    "type": "string",
                    "description": "Observação sobre o plano como um todo."
                }
            },
            "required": ["index"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Meta
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let mut plan = lock_plan(&self.plan);
        let total = plan.tasks.len();
        if total == 0 {
            return Err(no_plan());
        }
        let current = plan.current;
        let index = read_index(&args, total, current).ok_or_else(|| {
            ToolError::InvalidArgs(format!(
                "não existe essa etapa; o plano tem {total} etapa(s), numeradas de 0 a {}",
                total - 1
            ))
        })?;

        let mut changes: Vec<String> = Vec::new();
        {
            let task = &mut plan.tasks[index];
            if let Some(raw) = args.get("status").and_then(Value::as_str) {
                match status_from_str(raw) {
                    Some(status) => {
                        task.status = status;
                        changes.push(format!("estado {raw}"));
                    }
                    None => {
                        return Err(ToolError::InvalidArgs(format!(
                            "`{raw}` não é um estado; use pending, running, done, blocked, \
                             failed ou skipped"
                        )));
                    }
                }
            }
            if let Some(handoff) = args.get("handoff").and_then(Value::as_str)
                && !handoff.trim().is_empty()
            {
                task.handoff = Some(one_line(handoff, MAX_HANDOFF_CHARS));
                changes.push("resumo".into());
            }
        }
        if let Some(note) = args.get("note").and_then(Value::as_str)
            && !note.trim().is_empty()
        {
            let note = one_line(note, MAX_NOTE_CHARS);
            if plan.notes.is_empty() {
                plan.notes = note;
            } else {
                plan.notes = format!("{}\n{note}", plan.notes);
            }
            changes.push("observação".into());
        }

        if changes.is_empty() {
            return Err(ToolError::InvalidArgs(
                "nada para atualizar: mande `status`, `handoff` ou `note`".into(),
            ));
        }
        Ok(ToolOutput::text(format!(
            "Etapa {} atualizada ({}).\n{}",
            index + 1,
            changes.join(", "),
            plan.to_markdown()
        )))
    }
}

// ---------------------------------------------------------- task_complete ---

/// Fecha a etapa em andamento com o resumo que atravessa para a próxima.
pub struct TaskComplete {
    plan: SharedPlan,
    halt: Arc<AtomicBool>,
}

#[async_trait]
impl Tool for TaskComplete {
    fn name(&self) -> &str {
        "task_complete"
    }

    fn description(&self) -> &str {
        "Marca a etapa atual como concluída. O `handoff` é a ÚNICA coisa que a próxima etapa vai \
         saber sobre esta: escreva em até 3 linhas o que você fez e o que ela precisa saber."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "index": {
                    "type": "integer",
                    "description": "Índice da etapa concluída, começando em 0. Omita para a etapa atual."
                },
                "handoff": {
                    "type": "string",
                    "description": "Até 3 linhas: o que foi feito e o que a próxima etapa precisa saber (arquivos, decisões, pendências)."
                }
            },
            "required": ["handoff"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Meta
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let handoff = arg_str(&args, "handoff")?;
        if handoff.trim().is_empty() {
            return Err(ToolError::InvalidArgs(
                "o `handoff` não pode ser vazio: é o que a próxima etapa vai receber".into(),
            ));
        }

        let mut plan = lock_plan(&self.plan);
        let total = plan.tasks.len();
        if total == 0 {
            return Err(no_plan());
        }
        let current = plan.current;
        let index = read_index(&args, total, current).ok_or_else(|| {
            ToolError::InvalidArgs(format!(
                "não existe essa etapa; o plano tem {total} etapa(s), numeradas de 0 a {}",
                total - 1
            ))
        })?;

        let task = &mut plan.tasks[index];
        task.status = TaskStatus::Done;
        task.handoff = Some(one_line(&handoff, MAX_HANDOFF_CHARS));
        task.error = None;

        let (done, total) = plan.progress();
        // Etapa fechada: o laço retoma o comando e abre a próxima com
        // contexto novo.
        self.halt.store(true, Ordering::SeqCst);
        Ok(ToolOutput::text(format!(
            "Etapa {} concluída ({done}/{total}).\n{}",
            index + 1,
            plan.to_markdown()
        )))
    }
}

// --------------------------------------------------------------- ask_user ---

/// Devolve a decisão para a pessoa e pausa o plano.
pub struct AskUser {
    plan: SharedPlan,
    halt: Arc<AtomicBool>,
}

#[async_trait]
impl Tool for AskUser {
    fn name(&self) -> &str {
        "ask_user"
    }

    fn description(&self) -> &str {
        "Faz uma pergunta à pessoa quando falta uma decisão que você não pode tomar. A execução \
         para na etapa atual até haver resposta — use só quando não houver caminho seguro."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "A pergunta, direta e em uma frase."
                },
                "options": {
                    "type": "array",
                    "description": "Respostas possíveis, quando forem poucas e conhecidas.",
                    "items": { "type": "string" }
                }
            },
            "required": ["question"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Meta
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let question = arg_str(&args, "question")?;
        if question.trim().is_empty() {
            return Err(ToolError::InvalidArgs(
                "mande a pergunta em `question`".into(),
            ));
        }
        let question = one_line(&question, MAX_NOTE_CHARS);
        let options: Vec<String> = args
            .get("options")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|s| one_line(s, 80))
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let full = if options.is_empty() {
            question.clone()
        } else {
            format!("{question} (opções: {})", options.join(" / "))
        };

        {
            let mut plan = lock_plan(&self.plan);
            let current = plan.current;
            // Sem plano (modo agente) a pergunta ainda vale: só não há etapa
            // para marcar.
            if let Some(task) = plan.tasks.get_mut(current) {
                task.status = TaskStatus::Blocked;
                task.error = Some(full.clone());
            }
        }
        self.halt.store(true, Ordering::SeqCst);
        Ok(ToolOutput::text(format!(
            "Pergunta registrada; a execução para aqui até a pessoa responder: {full}"
        )))
    }
}

// ------------------------------------------------------------- montagem ---

/// As ferramentas meta de um run, já ligadas ao plano dele.
pub struct PlanTools {
    pub tools: Vec<SharedTool>,
    /// Levantado por `plan_create`, `task_complete` e `ask_user`: o trecho de
    /// laço em andamento termina assim que a chamada volta.
    pub halt: Arc<AtomicBool>,
    pub plan: SharedPlan,
}

impl PlanTools {
    /// Cardápio de cada modo de trabalho:
    /// - **planejamento**: escrever o plano e perguntar — nada executa;
    /// - **laço**: conduzir o plano em andamento (não recriar: substituir o
    ///   plano no meio da execução apagaria o progresso das etapas prontas);
    /// - **conversa** e **agente**: nenhuma, o comportamento não muda.
    pub fn for_mode(mode: WorkMode, plan: SharedPlan) -> Self {
        let halt = Arc::new(AtomicBool::new(false));
        let tools: Vec<SharedTool> = match mode {
            WorkMode::Plan => vec![
                Arc::new(PlanCreate {
                    plan: plan.clone(),
                    halt: halt.clone(),
                }),
                Arc::new(AskUser {
                    plan: plan.clone(),
                    halt: halt.clone(),
                }),
            ],
            WorkMode::Loop => vec![
                Arc::new(PlanUpdate { plan: plan.clone() }),
                Arc::new(TaskComplete {
                    plan: plan.clone(),
                    halt: halt.clone(),
                }),
                Arc::new(AskUser {
                    plan: plan.clone(),
                    halt: halt.clone(),
                }),
            ],
            WorkMode::Chat | WorkMode::Agent => Vec::new(),
        };
        Self { tools, halt, plan }
    }

    /// Metadados para o cardápio que vai ao modelo.
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.iter().map(|t| t.spec()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_types::scout::Task;

    fn ctx() -> ToolContext {
        // Nada aqui toca em disco: funciona sem pasta escolhida.
        ToolContext::new(None, "call-1")
    }

    fn plan_with_two() -> SharedPlan {
        shared_plan(TaskPlan {
            goal: "arrumar o projeto".into(),
            tasks: vec![
                Task {
                    instruction: "ler tudo".into(),
                    ..Task::new("t1", "ler o README")
                },
                Task {
                    instruction: "escrever".into(),
                    ..Task::new("t2", "escrever o teste")
                },
            ],
            ..Default::default()
        })
    }

    fn tools_of(mode: WorkMode, plan: SharedPlan) -> PlanTools {
        PlanTools::for_mode(mode, plan)
    }

    fn tool<'a>(set: &'a PlanTools, name: &str) -> &'a SharedTool {
        set.tools
            .iter()
            .find(|t| t.name() == name)
            .unwrap_or_else(|| panic!("faltou a ferramenta {name}"))
    }

    #[tokio::test]
    async fn plan_create_replaces_the_plan_and_stops_the_step() {
        let plan = shared_plan(TaskPlan::default());
        let set = tools_of(WorkMode::Plan, plan.clone());
        let out = tool(&set, "plan_create")
            .execute(
                json!({
                    "goal": "publicar o site",
                    "tasks": [
                        {"title": "escrever o texto", "instruction": "redigir index.md", "done_when": "index.md existe"},
                        {"title": "publicar", "instruction": "rodar o deploy", "done_when": "deploy sem erro", "depends_on": [0]}
                    ]
                }),
                &ctx(),
            )
            .await
            .unwrap();

        let saved = snapshot(&plan);
        assert_eq!(saved.goal, "publicar o site");
        assert_eq!(saved.tasks.len(), 2);
        assert_eq!(saved.tasks[1].depends_on, vec![0]);
        assert!(out.content.contains("escrever o texto"));
        assert!(set.halt.load(Ordering::SeqCst), "o plano encerra o passo");
    }

    #[tokio::test]
    async fn plan_create_without_usable_tasks_says_the_shape() {
        let set = tools_of(WorkMode::Plan, shared_plan(TaskPlan::default()));
        let err = tool(&set, "plan_create")
            .execute(json!({"goal": "x", "tasks": []}), &ctx())
            .await
            .unwrap_err();
        assert!(err.to_model_message().contains("title"));
    }

    #[tokio::test]
    async fn task_complete_marks_done_with_the_handoff() {
        let plan = plan_with_two();
        let set = tools_of(WorkMode::Loop, plan.clone());
        tool(&set, "task_complete")
            .execute(
                json!({"index": 0, "handoff": "li o README:\n  o projeto usa pnpm"}),
                &ctx(),
            )
            .await
            .unwrap();

        let saved = snapshot(&plan);
        assert_eq!(saved.tasks[0].status, TaskStatus::Done);
        assert_eq!(
            saved.tasks[0].handoff.as_deref(),
            Some("li o README: o projeto usa pnpm"),
            "o handoff vira uma linha só (ele entra no digest da próxima)"
        );
        assert!(set.halt.load(Ordering::SeqCst));
        assert_eq!(saved.progress(), (1, 2));
    }

    #[tokio::test]
    async fn task_complete_without_index_closes_the_current_step() {
        let plan = plan_with_two();
        lock_plan(&plan).current = 1;
        let set = tools_of(WorkMode::Loop, plan.clone());
        tool(&set, "task_complete")
            .execute(json!({"handoff": "escrevi o teste"}), &ctx())
            .await
            .unwrap();

        let saved = snapshot(&plan);
        assert_eq!(saved.tasks[1].status, TaskStatus::Done);
        assert_eq!(saved.tasks[0].status, TaskStatus::Pending);
    }

    /// A lista que o modelo lê é numerada a partir de 1; aceitar o número
    /// dele custa menos que gastar um passo do run com um erro.
    #[tokio::test]
    async fn task_complete_tolerates_one_based_counting() {
        let plan = plan_with_two();
        let set = tools_of(WorkMode::Loop, plan.clone());
        tool(&set, "task_complete")
            .execute(json!({"index": 2, "handoff": "fiz a segunda"}), &ctx())
            .await
            .unwrap();
        assert_eq!(snapshot(&plan).tasks[1].status, TaskStatus::Done);
    }

    #[tokio::test]
    async fn plan_update_changes_status_handoff_and_note() {
        let plan = plan_with_two();
        let set = tools_of(WorkMode::Loop, plan.clone());
        tool(&set, "plan_update")
            .execute(
                json!({"index": 1, "status": "skipped", "handoff": "não era preciso", "note": "o teste já existia"}),
                &ctx(),
            )
            .await
            .unwrap();

        let saved = snapshot(&plan);
        assert_eq!(saved.tasks[1].status, TaskStatus::Skipped);
        assert_eq!(saved.tasks[1].handoff.as_deref(), Some("não era preciso"));
        assert_eq!(saved.notes, "o teste já existia");
        assert!(
            !set.halt.load(Ordering::SeqCst),
            "atualizar não encerra o passo"
        );
    }

    #[tokio::test]
    async fn plan_update_refuses_an_unknown_status_and_an_empty_change() {
        let plan = plan_with_two();
        let set = tools_of(WorkMode::Loop, plan.clone());
        let err = tool(&set, "plan_update")
            .execute(json!({"index": 0, "status": "quase"}), &ctx())
            .await
            .unwrap_err();
        assert!(err.to_model_message().contains("blocked"));

        let err = tool(&set, "plan_update")
            .execute(json!({"index": 0}), &ctx())
            .await
            .unwrap_err();
        assert!(err.to_model_message().contains("nada para atualizar"));
        assert_eq!(snapshot(&plan).tasks[0].status, TaskStatus::Pending);
    }

    #[tokio::test]
    async fn updating_a_step_that_does_not_exist_says_the_range() {
        let plan = plan_with_two();
        let set = tools_of(WorkMode::Loop, plan.clone());
        let err = tool(&set, "plan_update")
            .execute(json!({"index": 9, "status": "done"}), &ctx())
            .await
            .unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("2 etapa"), "{msg}");
        assert!(msg.contains("0 a 1"), "{msg}");
    }

    #[tokio::test]
    async fn ask_user_blocks_the_current_step_and_stops_the_loop() {
        let plan = plan_with_two();
        lock_plan(&plan).current = 1;
        let set = tools_of(WorkMode::Loop, plan.clone());
        let out = tool(&set, "ask_user")
            .execute(
                json!({"question": "apago o arquivo antigo?", "options": ["sim", "não"]}),
                &ctx(),
            )
            .await
            .unwrap();

        let saved = snapshot(&plan);
        assert_eq!(saved.tasks[1].status, TaskStatus::Blocked);
        let reason = saved.tasks[1].error.clone().unwrap();
        assert!(reason.contains("apago o arquivo antigo?"));
        assert!(reason.contains("sim / não"), "{reason}");
        assert!(out.content.contains("para aqui"));
        assert!(set.halt.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn plan_tools_need_no_approval_and_no_workspace() {
        let set = tools_of(WorkMode::Loop, plan_with_two());
        for spec in set.specs() {
            assert_eq!(spec.category, ToolCategory::Meta, "{}", spec.name);
            assert!(
                spec.read_only,
                "{} deveria passar no filtro de leitura",
                spec.name
            );
        }
        assert!(tool(&set, "ask_user").within_workspace(&json!({}), &ctx()));
    }

    #[test]
    fn each_work_mode_gets_its_own_tools() {
        let names = |mode| {
            PlanTools::for_mode(mode, shared_plan(TaskPlan::default()))
                .tools
                .iter()
                .map(|t| t.name().to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(WorkMode::Plan), vec!["plan_create", "ask_user"]);
        assert_eq!(
            names(WorkMode::Loop),
            vec!["plan_update", "task_complete", "ask_user"],
            "no laço o plano é conduzido, não recriado"
        );
        assert!(names(WorkMode::Agent).is_empty());
        assert!(names(WorkMode::Chat).is_empty());
    }
}
