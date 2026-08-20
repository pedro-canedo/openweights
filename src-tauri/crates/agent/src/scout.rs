//! Scout Rule: dividir o objetivo em entregas que cabem na janela.
//!
//! O público deste app roda modelo local, muitas vezes com 32k de contexto.
//! Uma tarefa grande de uma vez enche a janela, o modelo perde o fio e passa a
//! inventar. A regra é sempre a mesma: **quebrar o objetivo em entregas
//! pequenas e executar uma de cada vez, com contexto novo**.
//!
//! O que atravessa de uma etapa para a outra é um resumo de até três linhas
//! (`handoff`), nunca o histórico. Por isso a janela não cresce com o tamanho
//! do pedido — só com o tamanho de UMA etapa.
//!
//! Duas metades vivem aqui:
//! - [`decompose`]: pede o plano ao modelo com JSON Schema forçado. O
//!   llama-server converte o schema em gramática, e é isso que faz um modelo
//!   de 8B devolver algo parseável. Mesmo assim o resultado é validado com
//!   desconfiança e, no pior caso, vira um plano de uma entrega só — a
//!   decomposição nunca derruba o run.
//! - [`run_plan`]: executa o plano etapa a etapa, verificando cada entrega,
//!   gravando o plano a cada mudança e seguindo em frente quando uma trava.

use crate::judge;
use crate::plan_tools::{SharedPlan, lock_plan, snapshot};
use crate::reliability::StepBudget;
use crate::run::{StepEngine, StepOutcome, SystemPrompt, ToolRunner, head_chars};
use crate::verify::{self, VerifyReport};
use lr_engine::{ChatMessage, ChatRequest, LlamaClient, ToolCallReq};
use lr_types::agent::{RunEventKind, RunStatus, ToolSpec};
use lr_types::scout::{
    PendingQuestion, QuestionItem, Task, TaskPlan, TaskStatus, WindowBudget, WorkMode,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;

/// Teto de entregas de um plano. Acima disso a pessoa perde a noção do todo e
/// o modelo passa a planejar em vez de trabalhar.
pub const MAX_TASKS: usize = 12;
/// Teto de passos de UMA etapa. O teto global do run continua valendo por
/// cima deste.
pub const MAX_STEPS_PER_TASK: u32 = 8;
/// Tentativas por etapa antes de considerá-la travada.
const MAX_TASK_ATTEMPTS: u32 = 2;
/// Quantos handoffs entram no contexto da próxima etapa.
const DIGEST_TASKS: usize = 3;
/// Teto do resumo de uma etapa (ele será relido em todas as próximas).
const MAX_HANDOFF_CHARS: usize = 400;
/// Tokens para escrever o handoff — são três linhas.
const HANDOFF_MAX_TOKENS: u32 = 200;
/// Teto de caracteres do título e da instrução de uma etapa.
const MAX_TITLE_CHARS: usize = 120;
const MAX_INSTRUCTION_CHARS: usize = 1_200;

/// Instrução extra do modo planejamento (entra como segunda mensagem de
/// sistema, para sobreviver à reconstrução do prompt a cada passo).
pub const PLAN_MODE_BRIEF: &str = "\
## Modo planejamento
Você está planejando, não executando: investigue com as ferramentas de leitura e \
proponha o plano. Não altere nada.
Divida o pedido em entregas pequenas e independentes, cada uma com um critério objetivo \
de conclusão, e registre-as com `plan_create`. Depois responda em texto explicando o plano \
em poucas linhas.";

// ------------------------------------------------------------- cardápio ---

/// Cardápio de ferramentas de cada modo de trabalho.
///
/// - **conversa**: nenhuma — o modelo responde com o que sabe;
/// - **planejamento**: só o que é leitura, porque investigar não altera nada;
/// - **agente** e **laço**: tudo que o registro oferece.
pub fn menu_for(mode: WorkMode, specs: Vec<ToolSpec>) -> Vec<ToolSpec> {
    match mode {
        WorkMode::Chat => Vec::new(),
        WorkMode::Plan => specs.into_iter().filter(|s| s.read_only).collect(),
        WorkMode::Agent | WorkMode::Loop => specs,
    }
}

// -------------------------------------------------------- decomposição ---

/// JSON Schema entregue ao servidor. Fica propositalmente raso: o conversor
/// de schema em gramática do llama.cpp não aceita `$ref`/`$defs`, e modelo
/// pequeno erra mais a cada nível de aninhamento.
fn plan_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "tasks": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_TASKS,
                "items": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string" },
                        "instruction": { "type": "string" },
                        "done_when": { "type": "string" },
                        // Os caminhos que a entrega vai criar ou alterar. É o
                        // que a conferência final procura em disco: sem isto,
                        // "pronto" continuaria sendo palavra do modelo.
                        "files": { "type": "array", "items": { "type": "string" } },
                        // Comando que sai 0 quando a entrega está pronta — o
                        // Definition of Done executável. Vai em `required`
                        // (com "" = não há): modelo pequeno é mais confiável
                        // preenchendo sempre do que decidindo omitir.
                        "check_cmd": { "type": "string" },
                        "depends_on": { "type": "array", "items": { "type": "integer" } }
                    },
                    "required": ["title", "instruction", "done_when", "files", "check_cmd"]
                }
            },
            // Decisões que só a pessoa pode tomar, levantadas ANTES de
            // trabalhar. Vazio = o pedido está claro. Em `required` (com [])
            // pelo mesmo motivo do check_cmd: modelo pequeno preenche sempre
            // melhor do que decide omitir.
            "questions": {
                "type": "array",
                "maxItems": 4,
                "items": { "type": "string" }
            }
        },
        "required": ["tasks", "questions"]
    })
}

/// Monta o pedido de decomposição.
fn decompose_request(
    model: &str,
    goal: &str,
    context: &str,
    wanted: u32,
    per_task: u32,
) -> ChatRequest {
    let mut user = format!(
        "Objetivo da pessoa:\n{goal}\n\n\
         Divida este objetivo em cerca de {wanted} entregas pequenas, na ordem de execução.\n\
         Regras:\n\
         - cada entrega tem que caber sozinha em {per_task} tokens de contexto;\n\
         - `instruction` é concreta: diz o que fazer, em quais arquivos, com que resultado;\n\
         - `done_when` é conferível (um arquivo existe, um comando passa, um texto aparece);\n\
         - `files` lista os caminhos que ESTA entrega cria ou altera, relativos à pasta do \
         projeto; deixe vazio só quando a entrega não mexe em arquivo nenhum;\n\
         - `check_cmd` é um comando de terminal que sai com código 0 quando a entrega está \
         pronta (um teste, um build, um grep no arquivo criado); para entrega de código, \
         escreva o teste PRIMEIRO e use-o aqui; use \"\" quando não houver comando que \
         confira;\n\
         - `depends_on` só aponta para entregas ANTERIORES, pelo índice começando em 0;\n\
         - não invente etapas de \"revisar\" ou \"planejar\": só trabalho de verdade;\n\
         - se faltar uma decisão que MUDA o plano (stack, escopo, onde salvar), liste até 4 \
         perguntas em `questions` e NÃO chute — mas nunca pergunte o que dá para descobrir \
         lendo o projeto; deixe [] quando o pedido está claro.\n"
    );
    if !context.trim().is_empty() {
        user.push_str(&format!(
            "\nO que já se sabe do projeto (use, não repita):\n{}\n",
            head_chars(context.trim(), 2_000)
        ));
    }
    user.push_str("\nResponda somente com o JSON pedido.");

    let mut req = ChatRequest::new(
        model,
        vec![
            ChatMessage::system(
                "Você divide trabalho em entregas pequenas para um agente que roda em um \
                 computador doméstico, com janela de contexto curta. Responda em JSON, no \
                 idioma do pedido.",
            ),
            ChatMessage::user(user),
        ],
    );
    req.temperature = Some(0.2);
    req.max_tokens = Some(1_400);
    // O llama-server transforma o schema em gramática: é o que garante um
    // plano parseável mesmo de um modelo pequeno.
    req.with_extra(
        "response_format",
        json!({
            "type": "json_schema",
            "json_schema": { "name": "plano", "strict": true, "schema": plan_schema() }
        }),
    )
}

/// Pede ao modelo a divisão do objetivo em entregas.
///
/// `context` são as observações da investigação (modo planejamento) ou vazio.
/// Devolve `Err` só quando o servidor não respondeu — resposta ilegível vira
/// um plano de uma entrega só, porque parar o run seria pior.
pub async fn decompose(
    client: &LlamaClient,
    model: &str,
    goal: &str,
    budget: WindowBudget,
    context: &str,
    max_tasks: u32,
) -> Result<TaskPlan, String> {
    let per_task = budget.per_task();
    let prompt_tokens = prompt_size(client, model, goal, context).await;
    // A janela diz quantas etapas cabem em CONTEXTO; `max_tasks` diz quantas
    // cabem no orçamento de PASSOS que a pessoa deu. Planejar oito entregas
    // com passos para duas é prometer o que não vai ser cumprido.
    let wanted = budget.suggested_tasks(prompt_tokens).min(max_tasks.max(1));

    let request = decompose_request(model, goal, context, wanted, per_task);
    let outcome = client
        .complete_once(&request)
        .await
        .map_err(|e| e.to_string())?;

    let mut plan = json_from_text(&outcome.content)
        .and_then(|v| plan_from_value(goal, &v, per_task))
        .unwrap_or_else(|| {
            log::warn!(
                "o modelo não devolveu um plano utilizável ({} bytes); sigo com uma entrega só",
                outcome.content.len()
            );
            single_task_plan(goal, per_task)
        });
    // O teto de passos é o que a pessoa (ou a automação) escolheu, e ponto.
    // `wanted` é só sugestão no prompt — um modelo que devolve 12 entregas
    // para um orçamento de 1 estouraria o combinado; corta aqui, não lá.
    let teto = (max_tasks.max(1) as usize).min(MAX_TASKS);
    if plan.tasks.len() > teto {
        log::warn!(
            "o modelo devolveu {} entregas para um orçamento de {teto}; corto o excesso",
            plan.tasks.len()
        );
        plan.tasks.truncate(teto);
    }
    Ok(with_notes(plan, context))
}

/// Anota o que a investigação descobriu (a interface mostra abaixo do plano).
fn with_notes(mut plan: TaskPlan, context: &str) -> TaskPlan {
    if plan.notes.is_empty() && !context.trim().is_empty() {
        plan.notes = head_chars(context.trim(), 400);
    }
    plan
}

/// Tamanho do pedido em tokens — quanto maior, mais fina a divisão.
async fn prompt_size(client: &LlamaClient, model: &str, goal: &str, context: &str) -> u32 {
    let probe = ChatRequest::new(model, vec![ChatMessage::user(format!("{goal}\n{context}"))]);
    client.input_tokens(&probe).await.unwrap_or_else(|e| {
        log::debug!("não consegui medir o pedido ({e}); uso a estimativa por caracteres");
        ((goal.chars().count() + context.chars().count()) / 4) as u32
    })
}

/// Plano de uma entrega só: a saída quando a decomposição não deu certo.
pub fn single_task_plan(goal: &str, per_task: u32) -> TaskPlan {
    let goal = goal.trim();
    TaskPlan {
        goal: goal.to_string(),
        tasks: vec![Task {
            instruction: goal.to_string(),
            done_when: "O pedido foi atendido e você explicou o que fez.".into(),
            est_tokens: per_task,
            ..Task::new("t1", head_chars(goal, MAX_TITLE_CHARS))
        }],
        ..Default::default()
    }
}

/// Constrói um plano a partir do JSON do modelo (ou de `plan_create`).
///
/// Devolve `None` quando não sobrou nenhuma entrega utilizável — quem chama
/// decide o que fazer com isso.
pub fn plan_from_value(goal: &str, value: &Value, per_task: u32) -> Option<TaskPlan> {
    let items = task_list(value)?;
    let mut tasks: Vec<Task> = Vec::new();

    for item in items {
        if tasks.len() == MAX_TASKS {
            break; // teto: o resto seria plano demais e trabalho de menos
        }
        let index = tasks.len();
        let title = clip(
            field(item, &["title", "name", "titulo", "título"]),
            MAX_TITLE_CHARS,
        );
        let instruction = clip(
            field(
                item,
                &[
                    "instruction",
                    "instrucao",
                    "instrução",
                    "task",
                    "description",
                ],
            ),
            MAX_INSTRUCTION_CHARS,
        );
        // Sem título nem instrução não há entrega nenhuma.
        let (title, instruction) = match (title.is_empty(), instruction.is_empty()) {
            (true, true) => continue,
            (true, false) => (clip(&instruction, MAX_TITLE_CHARS), instruction),
            (false, true) => (title.clone(), title),
            (false, false) => (title, instruction),
        };
        let done_when = match clip(
            field(
                item,
                &[
                    "done_when",
                    "doneWhen",
                    "criterio",
                    "critério",
                    "acceptance",
                ],
            ),
            MAX_TITLE_CHARS * 2,
        ) {
            s if s.is_empty() => format!("{title} — concluída e explicada."),
            s => s,
        };

        let check_cmd = clip(
            field(
                item,
                &["check_cmd", "checkCmd", "check", "test_cmd", "comando"],
            ),
            MAX_INSTRUCTION_CHARS,
        );
        tasks.push(Task {
            instruction,
            done_when,
            files: arquivos(item),
            depends_on: dependencies(item, index),
            est_tokens: per_task,
            check_cmd: (!check_cmd.trim().is_empty()).then(|| check_cmd.trim().to_string()),
            ..Task::new(format!("t{}", index + 1), title)
        });
    }

    // Perguntas pré-trabalho: decisões que mudam o plano. Só as que vieram
    // como texto simples; teto de 4 (mais que isso é o modelo empurrando a
    // tarefa de volta).
    let questions: Vec<QuestionItem> = value
        .get("questions")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .take(4)
                .map(|s| QuestionItem {
                    text: clip(s, MAX_INSTRUCTION_CHARS),
                    options: Vec::new(),
                })
                .collect()
        })
        .unwrap_or_default();

    (!tasks.is_empty()).then(|| TaskPlan {
        goal: goal.trim().to_string(),
        tasks,
        pending_question: (!questions.is_empty()).then_some(PendingQuestion {
            items: questions,
            task_index: None,
            asked_at_ms: 0,
        }),
        ..Default::default()
    })
}

/// Acha a lista de entregas em qualquer das formas que o modelo usa.
fn task_list(value: &Value) -> Option<Vec<&Value>> {
    let array = match value {
        Value::Array(items) => items,
        Value::Object(map) => ["tasks", "steps", "plan", "etapas", "tarefas"]
            .iter()
            .find_map(|k| map.get(*k).and_then(Value::as_array))
            // Último recurso: a primeira lista do objeto, com qualquer nome.
            .or_else(|| map.values().find_map(Value::as_array))?,
        _ => return None,
    };
    Some(array.iter().collect())
}

/// Primeiro campo não vazio entre os nomes aceitos.
fn field(item: &Value, keys: &[&str]) -> String {
    if let Value::String(s) = item {
        // Modelo preguiçoso manda `["fazer isto", "fazer aquilo"]`.
        return s.trim().to_string();
    }
    keys.iter()
        .find_map(|k| item.get(*k).and_then(Value::as_str))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// Dependências válidas de uma etapa.
///
/// Só valem índices de etapas ANTERIORES: uma dependência para frente (ou
/// para si mesma) trava o plano inteiro, porque nunca fica pronta. Índice
/// inválido é descartado — perder uma dependência é melhor que recusar o
/// plano e cair no plano de uma entrega só.
/// Caminhos que a entrega declara que vai criar ou alterar.
///
/// São a evidência da conferência final: sem eles, "a tarefa acabou" volta a
/// ser a palavra do modelo. Caminho absoluto é descartado — a conferência
/// resolve contra a pasta do projeto, e um `/etc/...` aqui só produziria
/// pendência eterna.
fn arquivos(item: &Value) -> Vec<String> {
    item.get("files")
        .or_else(|| item.get("arquivos"))
        .or_else(|| item.get("paths"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| {
                    !s.is_empty()
                        && !s.starts_with('/')
                        && !s.starts_with('\\')
                        && !s.contains("..")
                        && !s.contains(':')
                })
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn dependencies(item: &Value, index: usize) -> Vec<usize> {
    let mut out: Vec<usize> = item
        .get("depends_on")
        .or_else(|| item.get("dependsOn"))
        .or_else(|| item.get("depende_de"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| {
                    v.as_u64()
                        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                })
                .map(|n| n as usize)
                .filter(|n| *n < index)
                .collect()
        })
        .unwrap_or_default();
    out.sort_unstable();
    out.dedup();
    out
}

fn clip(text: impl AsRef<str>, max: usize) -> String {
    let text = text.as_ref().trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect::<String>() + "…"
}

/// Acha o JSON no meio do que o modelo escreveu.
///
/// Com o schema forçado a resposta costuma vir limpa, mas nem todo build do
/// servidor honra `response_format` — então ainda aceitamos JSON dentro de
/// cerca de código ou cercado de conversa.
pub(crate) fn json_from_text(raw: &str) -> Option<Value> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        return Some(v);
    }
    let fenced = strip_fences(text);
    if let Ok(v) = serde_json::from_str::<Value>(&fenced) {
        return Some(v);
    }
    let slice = balanced_slice(&fenced)?;
    serde_json::from_str::<Value>(slice).ok()
}

/// Tira a cerca de código ```json ... ``` em volta do conteúdo.
fn strip_fences(text: &str) -> String {
    let Some(start) = text.find("```") else {
        return text.to_string();
    };
    let after = &text[start + 3..];
    let body = match after.find('\n') {
        // A primeira linha da cerca é a linguagem (```json).
        Some(nl) if after[..nl].trim().chars().all(char::is_alphanumeric) => &after[nl + 1..],
        _ => after,
    };
    match body.find("```") {
        Some(end) => body[..end].trim().to_string(),
        None => body.trim().to_string(),
    }
}

/// Maior bloco `{...}` ou `[...]` equilibrado do texto.
fn balanced_slice(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let start = bytes.iter().position(|b| *b == b'{' || *b == b'[')?;
    let open = bytes[start];
    let close = if open == b'{' { b'}' } else { b']' };

    let (mut depth, mut in_string, mut escaped) = (0i32, false, false);
    for (i, b) in bytes.iter().enumerate().skip(start) {
        if in_string {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match *b {
            b'"' => in_string = true,
            b if b == open => depth += 1,
            b if b == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

// --------------------------------------------------------- laço do plano ---

/// Tudo que o laço de plano precisa. Montado por `execute_run`.
pub(crate) struct PlanRun<'a> {
    pub engine: &'a StepEngine<'a>,
    /// Monta o prompt de sistema com o plano atual dentro.
    pub build_system: &'a SystemPrompt<'a>,
    pub plan: SharedPlan,
    pub workspace: Option<PathBuf>,
    /// O pedido da pessoa, em uma frase.
    pub goal: String,
    /// Observações da investigação (vazio quando não houve).
    pub context: String,
    pub window: WindowBudget,
    /// Quantas entregas cabem no teto de passos deste run.
    pub max_tasks: u32,
    /// O comando de aceitação (`check_cmd`) pode rodar? Falso quando o modelo
    /// não suporta ferramentas ou quando a pessoa DESLIGOU a família de
    /// terminal — o harness não pode executar shell por um caminho que o
    /// próprio menu proíbe ao modelo.
    pub checar_com_terminal: bool,
}

/// O que o plano inteiro produziu.
#[derive(Debug)]
pub(crate) struct PlanOutcome {
    pub status: RunStatus,
    /// Texto final para a conversa.
    pub summary: String,
    /// Pergunta pendente, quando o modelo chamou `ask_user`.
    pub question: Option<String>,
    /// A forma ESTRUTURADA da pausa (o que a interface e o banco recebem).
    pub pending: Option<PendingQuestion>,
    pub steps: u32,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

impl Default for PlanOutcome {
    fn default() -> Self {
        Self {
            // `RunStatus` vem do crate de tipos e não tem `Default`; aqui o
            // estado inicial é "rodando" até o laço decidir outra coisa.
            status: RunStatus::Running,
            summary: String::new(),
            question: None,
            pending: None,
            steps: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
        }
    }
}

/// Executa o plano inteiro: uma entrega por vez, cada uma com contexto novo.
///
/// O laço nunca reaproveita o histórico entre etapas — o que passa adiante é
/// o `handoff_digest`. Uma etapa que falha duas vezes vira `Blocked` e o laço
/// segue para a próxima que não depende dela.
pub(crate) async fn run_plan(
    ctx: &PlanRun<'_>,
    runner: &mut ToolRunner,
    steps: &mut StepBudget,
) -> PlanOutcome {
    let mut out = PlanOutcome::default();

    // 1. O plano. Se ninguém escreveu um ainda, o modelo divide agora.
    if snapshot(&ctx.plan).tasks.is_empty() {
        let plan = match decompose(
            ctx.engine.client,
            &ctx.engine.opts.model,
            &ctx.goal,
            ctx.window,
            &ctx.context,
            ctx.max_tasks,
        )
        .await
        {
            Ok(plan) => plan,
            Err(e) => {
                log::warn!("não consegui dividir o objetivo ({e}); sigo com uma entrega só");
                ctx.engine.sink.emit(RunEventKind::RunError {
                    message: format!(
                        "Não consegui dividir a tarefa ({e}); vou tentar em uma entrega só."
                    ),
                    retryable: true,
                });
                single_task_plan(&ctx.goal, ctx.window.per_task())
            }
        };
        *lock_plan(&ctx.plan) = plan;
    }
    publish(ctx, "plano criado");

    // Perguntas ANTES de trabalhar: a decomposição levantou decisões que só
    // a pessoa pode tomar. NADA executa até a resposta — a pausa é durável
    // (perguntas persistidas com o plano; `run_answer` limpa e retoma).
    if let Some(mut q) = snapshot(&ctx.plan).pending_question {
        q.asked_at_ms = crate::events::now_ms() as i64;
        lock_plan(&ctx.plan).pending_question = Some(q.clone());
        publish(ctx, "perguntas pendentes");
        out.question = Some(q.to_text());
        out.pending = Some(q);
        out.status = RunStatus::Escalated;
        out.summary = summarize(
            &snapshot(&ctx.plan),
            out.status,
            out.question.as_deref(),
            steps.max(),
        );
        return out;
    }

    // 2. Uma entrega por vez.
    let mut attempts: HashMap<usize, u32> = HashMap::new();
    let status = loop {
        if ctx.engine.handle.is_cancelled() {
            break RunStatus::Cancelled;
        }
        // O estado do PLANO decide primeiro: sem etapa executável o desfecho
        // é dele (concluído, ou travado → Escalated com as travadas nomeadas)
        // — não "parei no limite", mesmo que o orçamento tenha acabado junto.
        let Some(index) = next_task(&ctx.plan) else {
            break RunStatus::Done;
        };
        if steps.used() >= steps.max() {
            break RunStatus::MaxSteps;
        }

        let (task, digest, total, md) = start_task(&ctx.plan, index);
        publish(ctx, "etapa iniciada");

        // Cada etapa pede ferramentas diferentes: "rodar a suíte" quer
        // `test_run`, "commitar" quer `git_commit`. Recurar aqui é o que faz
        // o cardápio caber na janela sem esconder o que a etapa precisa.
        ctx.engine.refocus_menu(&task.instruction);

        // Contexto NOVO: sistema + objetivo + resumo do que já foi feito +
        // ESTA etapa. Nada do histórico anterior entra aqui — é a Scout Rule
        // em quatro linhas.
        runner.reset_task_memory();
        runner.focus_md = Some(md.clone());
        let mut messages = vec![
            ChatMessage::system((ctx.build_system)(Some(&md))),
            ChatMessage::user(task_brief(&ctx.goal, &digest, index, total, &task)),
        ];

        let files_before = runner.written.len();
        let commands_before = runner.commands.len();

        // Camada TDD do Definition of Done: o comando de aceitação roda ANTES
        // do trabalho (vermelho esperado — o teste que ainda falha) e DEPOIS
        // dele. Os dois registros caem na fatia desta etapa; como têm o MESMO
        // display, a conferência considera só o último — o vermelho inicial
        // não reprova, o vermelho final sim. Só roda quando o terminal está
        // ao alcance do run (família ligada + modelo com ferramentas).
        let check_cmd = ctx
            .checar_com_terminal
            .then(|| task.check_cmd.clone())
            .flatten()
            .filter(|c| !c.trim().is_empty())
            .map(|c| c.trim().to_string());
        let check_before = match &check_cmd {
            Some(cmd) => match run_check(runner, index, "antes", cmd).await {
                Checagem::Codigo(code) => code,
                Checagem::Parar(status) => {
                    requeue(&ctx.plan, index);
                    publish(ctx, "etapa bloqueada");
                    break status;
                }
            },
            None => None,
        };

        let step = ctx
            .engine
            .drive(
                runner,
                &mut messages,
                steps,
                MAX_STEPS_PER_TASK,
                ctx.build_system,
            )
            .await;
        out.steps += step.steps;
        out.prompt_tokens += step.prompt_tokens;
        out.completion_tokens += step.completion_tokens;

        let check_after = match &check_cmd {
            Some(cmd) if !ctx.engine.handle.is_cancelled() => {
                match run_check(runner, index, "depois", cmd).await {
                    Checagem::Codigo(code) => code,
                    Checagem::Parar(status) => {
                        requeue(&ctx.plan, index);
                        publish(ctx, "etapa bloqueada");
                        break status;
                    }
                }
            }
            _ => None,
        };
        anota_checks(&ctx.plan, index, check_before, check_after);

        let mut files: Vec<String> = runner.written[files_before..].to_vec();
        // A mesma etapa pode salvar um arquivo duas vezes; a lista que vai ao
        // plano e à conferência não precisa repetir.
        let mut vistos = std::collections::HashSet::new();
        files.retain(|f| vistos.insert(f.clone()));
        let commands = runner.commands[commands_before..].to_vec();

        // O modelo chamou `ask_user`: o plano para onde está.
        if let Some(question) = pending_question(&ctx.plan, index) {
            out.question = Some(question);
            out.pending = snapshot(&ctx.plan).pending_question;
            publish(ctx, "etapa bloqueada");
            break RunStatus::Escalated;
        }

        if step.status == RunStatus::Cancelled {
            requeue(&ctx.plan, index);
            publish(ctx, "cancelado");
            break RunStatus::Cancelled;
        }
        // Teto global estourado no meio da etapa: ela volta para a fila.
        if step.status == RunStatus::MaxSteps && steps.used() >= steps.max() {
            requeue(&ctx.plan, index);
            publish(ctx, "sem passos");
            break RunStatus::MaxSteps;
        }

        // Conferência da entrega: o que ela diz ter escrito existe mesmo?
        let report = verify::verify(ctx.workspace.as_deref(), &files, &commands);
        if let Some(r) = &report {
            ctx.engine.sink.emit(RunEventKind::Verification {
                passed: r.passed,
                notes: format!("{}: {}", task.title, r.notes),
            });
        }

        // Terceira camada: o juiz do `done_when`. Só quando NADA mecânico
        // decidiu — etapa sem arquivo e sem comando de aceitação QUE TENHA
        // RODADO (um check negado pela pessoa não é evidência), ou um
        // comando que já passava antes (então ele não prova o trabalho DESTA
        // etapa). O juiz nunca reverte reprovação mecânica.
        let teste_ja_passava = check_before == Some(0) && check_after == Some(0);
        let sem_evidencia = files.is_empty() && check_after.is_none();
        let veredito = if step.status == RunStatus::Done
            && report.as_ref().is_none_or(|r| r.passed)
            && (sem_evidencia || teste_ja_passava)
        {
            judge::julgar(
                ctx.engine.client,
                &ctx.engine.opts.model,
                &task,
                &step.text,
                &files,
                &commands,
                teste_ja_passava
                    .then_some("o comando de aceitação JÁ passava antes desta etapa começar"),
            )
            .await
        } else {
            None
        };
        if let Some(v) = &veredito {
            ctx.engine.sink.emit(RunEventKind::Verification {
                passed: v.passou,
                notes: format!("{} (juiz do critério): {}", task.title, v.porque),
            });
        }

        if let Some(reason) = failure_of(&step, report.as_ref(), veredito.as_ref()) {
            let tries = attempts.entry(index).and_modify(|n| *n += 1).or_insert(1);
            fail_task(
                &ctx.plan,
                index,
                &reason,
                files,
                *tries >= MAX_TASK_ATTEMPTS,
            );
            publish(ctx, "etapa falhou");
            continue;
        }

        // Deu certo: o resumo que a próxima etapa vai receber.
        let handoff = match existing_handoff(&ctx.plan, index) {
            // Veio de `task_complete`: o modelo já escreveu.
            Some(h) => h,
            None => ask_handoff(ctx.engine.client, &ctx.engine.opts.model, &task, &step.text).await,
        };
        finish_task(&ctx.plan, index, handoff, files);
        publish(ctx, "etapa concluída");
    };

    let plan = snapshot(&ctx.plan);
    out.status = match status {
        // Não sobrou nada executável: ou terminou, ou o resto travou.
        RunStatus::Done if !plan.is_complete() => RunStatus::Escalated,
        other => other,
    };
    out.summary = summarize(&plan, out.status, out.question.as_deref(), steps.max());
    out
}

/// Próxima etapa liberada (respeita `depends_on`).
fn next_task(plan: &SharedPlan) -> Option<usize> {
    lock_plan(plan).next_index()
}

/// Marca a etapa como em andamento e devolve o que o contexto novo precisa.
fn start_task(plan: &SharedPlan, index: usize) -> (Task, String, usize, String) {
    let mut p = lock_plan(plan);
    p.current = index;
    p.tasks[index].status = TaskStatus::Running;
    // Carimbo do começo: é o que dá duração real à etapa na linha do tempo.
    // Uma retentativa recomeça o relógio — o que a pessoa quer saber é
    // quanto durou a tentativa que valeu.
    p.tasks[index].started_at_ms = Some(crate::events::now_ms() as i64);
    p.tasks[index].finished_at_ms = None;
    let digest = p.handoff_digest(DIGEST_TASKS);
    let total = p.tasks.len();
    let md = p.to_markdown();
    (p.tasks[index].clone(), digest, total, md)
}

/// A etapa virou `Blocked` durante a execução? Só `ask_user` faz isso.
fn pending_question(plan: &SharedPlan, index: usize) -> Option<String> {
    let p = lock_plan(plan);
    let task = p.tasks.get(index)?;
    (task.status == TaskStatus::Blocked)
        .then(|| task.error.clone())
        .flatten()
}

fn requeue(plan: &SharedPlan, index: usize) {
    lock_plan(plan).tasks[index].status = TaskStatus::Pending;
}

fn existing_handoff(plan: &SharedPlan, index: usize) -> Option<String> {
    lock_plan(plan).tasks[index]
        .handoff
        .clone()
        .filter(|h| !h.trim().is_empty())
}

/// Etapa que não deu certo: volta para a fila ou trava, conforme a tentativa.
fn fail_task(plan: &SharedPlan, index: usize, reason: &str, files: Vec<String>, give_up: bool) {
    let mut p = lock_plan(plan);
    let task = &mut p.tasks[index];
    task.status = if give_up {
        TaskStatus::Blocked
    } else {
        TaskStatus::Pending
    };
    task.error = Some(clip(reason, MAX_HANDOFF_CHARS));
    task.files = files;
    // Travada de vez também fecha o relógio; reenfileirada não — a próxima
    // tentativa recomeça a contagem em `start_task`.
    if give_up {
        task.finished_at_ms = Some(crate::events::now_ms() as i64);
    }
}

fn finish_task(plan: &SharedPlan, index: usize, handoff: String, files: Vec<String>) {
    let mut p = lock_plan(plan);
    let task = &mut p.tasks[index];
    task.status = TaskStatus::Done;
    task.handoff = Some(handoff);
    task.error = None;
    task.files = files;
    task.finished_at_ms = Some(crate::events::now_ms() as i64);
}

/// O que a execução do comando de aceitação produziu.
enum Checagem {
    /// Código de saída — `None` = o comando não chegou a rodar (negado) ou
    /// não informou código. NUNCA herda registro velho de outra fase.
    Codigo(Option<i32>),
    /// A chamada devolveu um sinal terminal (confirmações abandonadas, teto
    /// de erros): o laço do plano precisa PARAR, não seguir sem ninguém.
    Parar(RunStatus),
}

/// Roda o comando de aceitação da etapa pelo MESMO caminho de uma chamada do
/// modelo — aprovação, trilha, timeout e registro em `commands` inclusos.
///
/// Duas diferenças deliberadas em relação a uma chamada do modelo:
/// - o detector de repetição fica de fora (via `dentro_de_programa`, a mesma
///   isenção do Code Mode): o check roda o MESMO comando de propósito, antes
///   e depois — e o modelo continua com o direito de rodá-lo ele próprio;
/// - o código de saída vem SÓ dos registros novos DESTA chamada: um check
///   negado não pode herdar o exit de uma execução antiga como evidência.
async fn run_check(runner: &mut ToolRunner, index: usize, fase: &str, cmd: &str) -> Checagem {
    let tc = ToolCallReq {
        id: crate::new_id(&format!("check_{index}_{fase}")),
        name: "terminal_run".into(),
        arguments_json: json!({ "command": cmd }).to_string(),
    };
    let antes = runner.commands.len();
    let isento = runner.dentro_de_programa;
    runner.dentro_de_programa = true;
    let flow = runner.call("verificacao", 0, &tc).await;
    runner.dentro_de_programa = isento;
    match flow {
        crate::run::CallFlow::Cancelled => Checagem::Parar(RunStatus::Cancelled),
        crate::run::CallFlow::Escalate(_) => Checagem::Parar(RunStatus::Escalated),
        _ => Checagem::Codigo(
            runner.commands[antes..]
                .iter()
                .rev()
                .find(|c| c.display == cmd)
                .and_then(|c| c.exit_code),
        ),
    }
}

/// Guarda no plano a evidência TDD da etapa (código antes/depois do check).
fn anota_checks(plan: &SharedPlan, index: usize, before: Option<i32>, after: Option<i32>) {
    if before.is_none() && after.is_none() {
        return;
    }
    let mut p = lock_plan(plan);
    if let Some(task) = p.tasks.get_mut(index) {
        task.check_before = before;
        task.check_after = after;
    }
}

/// Por que a etapa não passou — `None` quando passou.
fn failure_of(
    step: &StepOutcome,
    report: Option<&VerifyReport>,
    veredito: Option<&judge::Veredito>,
) -> Option<String> {
    if let Some(r) = report.filter(|r| !r.passed) {
        return Some(format!("a conferência não passou: {}", r.notes));
    }
    if let Some(v) = veredito.filter(|v| !v.passou) {
        return Some(format!("o juiz do critério reprovou: {}", v.porque));
    }
    match step.status {
        RunStatus::Done => None,
        RunStatus::MaxSteps => Some(format!(
            "a etapa passou de {MAX_STEPS_PER_TASK} passos sem concluir"
        )),
        RunStatus::Escalated => Some(
            step.escalation
                .clone()
                .unwrap_or_else(|| "a etapa foi interrompida".into()),
        ),
        _ => Some("o modelo não respondeu nesta etapa".into()),
    }
}

/// Grava o plano e avisa a interface. Chamado a cada mudança: é assim que a
/// pessoa vê a divisão do trabalho acontecendo.
fn publish(ctx: &PlanRun<'_>, motivo: &str) {
    publish_plan(ctx.engine, &ctx.plan, motivo);
}

/// Grava e emite o plano — o quadro na tela é o que a pessoa usa para saber
/// em que etapa o run está.
pub(crate) fn publish_plan(engine: &StepEngine<'_>, plan: &SharedPlan, motivo: &str) {
    // Quanto falta, em tempo: a estimativa é recalculada a cada publicação
    // porque o que sobrou do plano muda a cada etapa.
    {
        let mut p = lock_plan(plan);
        crate::eta::estimar(&engine.store, &engine.opts.model, &mut p);
    }
    let plan = snapshot(plan);
    match serde_json::to_string(&plan) {
        Ok(json) => {
            if let Err(e) = engine.store.set_run_plan(&engine.run_id, &json) {
                log::warn!("não consegui gravar o plano ({motivo}): {e}");
            }
        }
        Err(e) => log::warn!("plano não serializou ({motivo}): {e}"),
    }
    // O plano ESTRUTURADO viaja na trilha: a interface deixa de precisar de
    // uma consulta extra (com atraso) a cada mudança, e o replay reconstrói
    // a linha do tempo das etapas sem ir ao banco.
    engine.sink.emit(RunEventKind::Plan {
        event: lr_types::scout::PlanEventKind::PlanUpdated { plan: plan.clone() },
    });
    let md = plan.to_markdown();
    let _ = engine.store.set_run_focus(&engine.run_id, &md);
    engine.sink.emit(RunEventKind::FocusUpdated { todo_md: md });
}

/// O texto que abre o contexto de uma etapa.
///
/// É deliberadamente curto: objetivo, o resumo das etapas anteriores e a
/// instrução desta. O modelo não recebe (nem pode receber) o histórico — se
/// algo importante não estiver no digest, não existe para ele.
fn task_brief(goal: &str, digest: &str, index: usize, total: usize, task: &Task) -> String {
    let mut out = format!("Objetivo geral: {goal}\n\n");
    if !digest.trim().is_empty() {
        out.push_str("Etapas já concluídas (resumo do que elas deixaram pronto):\n");
        out.push_str(digest);
        out.push_str("\n\n");
    }
    out.push_str(&format!(
        "Sua etapa agora — {} de {}: {}\n{}\nPronto quando: {}\n",
        index + 1,
        total,
        task.title,
        task.instruction,
        task.done_when
    ));
    if let Some(cmd) = task.check_cmd.as_deref().filter(|c| !c.trim().is_empty()) {
        out.push_str(&format!(
            "O comando de aceitação `{cmd}` PRECISA sair com código 0 ao fim — o harness o \
             executa e confere.\n"
        ));
    }
    out.push('\n');
    if let Some(erro) = task.error.as_deref().filter(|e| !e.trim().is_empty()) {
        out.push_str(&format!(
            "A tentativa anterior desta etapa não deu certo: {erro}\nTente outro caminho.\n\n"
        ));
    }
    out.push_str(
        "Faça SÓ esta etapa. Você não tem o histórico das anteriores: o resumo acima é tudo o \
         que sobrou delas. Ao terminar, chame `task_complete` com um resumo de até 3 linhas do \
         que a próxima etapa precisa saber. Se faltar uma decisão que só a pessoa pode tomar, \
         chame `ask_user`.",
    );
    out
}

/// Pede ao modelo o resumo que atravessa para a próxima etapa.
///
/// Vale um `complete_once` curto: é a única coisa que sobra desta etapa, e
/// deixar o modelo escrever custa menos contexto do que arrastar o histórico.
async fn ask_handoff(client: &LlamaClient, model: &str, task: &Task, text: &str) -> String {
    let fallback = || match text.trim() {
        "" => format!("{} — concluída, sem detalhes relatados.", task.title),
        t => clip(t, MAX_HANDOFF_CHARS),
    };

    let mut ask = ChatRequest::new(
        model,
        vec![ChatMessage::user(format!(
            "Você acabou de executar a etapa \"{}\" de um trabalho maior.\n\
             O que você relatou:\n{}\n\n\
             Escreva em até 3 linhas, em texto corrido, o que ficou pronto e o que a próxima \
             etapa precisa saber (arquivos tocados, decisões, pendências). Não repita o \
             objetivo, não invente nada, não use listas.",
            task.title,
            head_chars(text.trim(), 1_500)
        ))],
    );
    ask.temperature = Some(0.2);
    ask.max_tokens = Some(HANDOFF_MAX_TOKENS);

    match client.complete_once(&ask).await {
        Ok(o) if !o.content.trim().is_empty() => as_handoff(&o.content),
        Ok(_) => fallback(),
        Err(e) => {
            log::warn!("não consegui resumir a etapa `{}`: {e}", task.title);
            fallback()
        }
    }
}

/// Três linhas, em uma linha só: o digest da próxima etapa é uma lista.
fn as_handoff(text: &str) -> String {
    let body = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(3)
        .collect::<Vec<_>>()
        .join(" ");
    clip(body, MAX_HANDOFF_CHARS)
}

/// A frase que a pessoa lê no fim do run.
fn summarize(plan: &TaskPlan, status: RunStatus, question: Option<&str>, max_steps: u32) -> String {
    let (done, total) = plan.progress();
    if let Some(q) = question {
        return format!("Parei na etapa {} para perguntar: {q}", plan.current + 1);
    }
    match status {
        RunStatus::Done => format!("Plano concluído: {done} de {total} entregas."),
        RunStatus::MaxSteps => {
            // Acabar o orçamento não é desculpa para não dizer O QUE faltou.
            let pendentes: Vec<&str> = plan
                .tasks
                .iter()
                .filter(|t| !t.status.is_finished())
                .map(|t| t.title.as_str())
                .collect();
            if pendentes.is_empty() {
                format!("Parei no limite de {max_steps} passos com {done} de {total} entregas.")
            } else {
                format!(
                    "Parei no limite de {max_steps} passos com {done} de {total} entregas. \
                     Faltou: {}.",
                    pendentes.join("; ")
                )
            }
        }
        RunStatus::Cancelled => {
            format!("Execução cancelada com {done} de {total} entregas prontas.")
        }
        _ => {
            let travadas: Vec<String> = plan
                .tasks
                .iter()
                .filter(|t| matches!(t.status, TaskStatus::Blocked | TaskStatus::Failed))
                .map(|t| match t.error.as_deref() {
                    Some(e) => format!("{} ({e})", t.title),
                    None => t.title.clone(),
                })
                .collect();
            if travadas.is_empty() {
                format!("Parei com {done} de {total} entregas.")
            } else {
                format!(
                    "Parei com {done} de {total} entregas. Travou em: {}.",
                    travadas.join("; ")
                )
            }
        }
    }
}

#[cfg(test)]
mod tests;
