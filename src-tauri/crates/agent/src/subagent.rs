//! Delegação: um ajudante com contexto novo investiga e devolve um resumo.
//!
//! O problema é de janela. Um modelo local trabalha com 8k a 32k de contexto;
//! quando o agente precisa descobrir alguma coisa ("onde o roteamento de
//! modelos é decidido"), ele lê seis arquivos, enche a janela com o conteúdo
//! bruto e chega no passo seguinte sem espaço para pensar.
//!
//! A saída é delegar. [`AgentDelegate`] entrega a missão a um ajudante que
//! começa do zero — prompt de sistema próprio, cardápio próprio, histórico
//! vazio — e devolve ao agente principal só o resumo do que descobriu. A
//! janela do pai paga dez linhas, não vinte mil tokens.
//!
//! O ajudante não é um run novo: ele reusa o [`StepEngine`] e o [`ToolRunner`]
//! do laço normal, com o **mesmo** [`RunHandle`]. É isso que faz cancelar e
//! confirmar continuarem funcionando lá dentro, e é isso que mantém a trilha
//! de eventos em um lugar só.
//!
//! **Um ajudante por vez, de propósito.** A delegação roda dentro da chamada
//! de ferramenta do pai, então tudo que sai no `EventSink` entre
//! `subagent.started` e `subagent.finished` é dele — é assim que a interface
//! aninha a trilha sem cada evento carregar o id de quem o gerou. Rodar dois
//! em paralelo exigiria etiquetar cada evento com o `call_id` do ajudante (ou
//! dar um `EventSink` a cada um) antes de qualquer outra coisa.

use crate::events::EventSink;
use crate::menu::{self, MenuState, ToolsFind};
use crate::prompt::{PromptContext, build_system_prompt};
use crate::reliability::{ContextBudget, ErrorStreak, ReadLedger, RepeatDetector, StepBudget};
use crate::run::{StepEngine, StepOutcome, ToolRunner, head_chars};
use crate::verify::CommandRecord;
use crate::{AgentConfig, RunHandle};
use async_trait::async_trait;
use lr_engine::{ChatMessage, LlamaClient};
use lr_policy::{PermissionOverride, PolicyEngine};
use lr_store::Store;
use lr_tools::{
    SharedTool, Tool, ToolContext, ToolError, ToolOutput, ToolRegistry, ToolResult, arg_str,
};
use lr_types::agent::{
    RunEventKind, RunMode, RunOptions, RunStatus, SubagentRole, ToolCategory, ToolGroup,
    ToolPreview, ToolSpec,
};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// Nome da ferramenta. Vive numa constante porque o cardápio do ajudante o
/// procura para excluí-lo.
pub const AGENT_DELEGATE: &str = "agent_delegate";

/// Teto de passos de um ajudante. Ele é uma tacada só: o que não coube em
/// oito passos era missão grande demais, e quem decide o que fazer com isso é
/// o agente principal.
pub const MAX_SUBAGENT_STEPS: u32 = 8;

/// Teto de passos somando TODOS os ajudantes de um run.
///
/// O `max_steps` do run conta os passos do agente principal; o que roda
/// dentro de um ajudante passa por fora. Sem este segundo teto, um modelo que
/// gosta de delegar transformaria um pedido em dezenas de chamadas ao modelo
/// sem nada para segurá-lo. Três ajudantes cheios é um limite generoso para o
/// que a delegação serve.
pub const MAX_TOTAL_SUBAGENT_STEPS: u32 = 3 * MAX_SUBAGENT_STEPS;

/// Teto do resumo que volta ao agente principal. Delegar existe para poupar
/// janela — um resumo maior do que isto derrotaria o propósito.
pub const MAX_DIGEST_CHARS: usize = 2_000;

/// Teto do objetivo: é uma instrução, não um documento.
const MAX_OBJECTIVE_CHARS: usize = 600;

/// Tudo que um ajudante precisa para existir, montado uma vez pelo laço.
///
/// Os três acumuladores no fim são compartilhados com o pai: o que o ajudante
/// escreveu e rodou precisa chegar na verificação final e no `UsageStats` do
/// run, senão o trabalho dele não é conferido nem contado.
pub struct SubagentDeps {
    /// Onde o modelo atende. Cada ajudante abre o próprio cliente.
    pub base_url: String,
    pub api_key: Option<String>,
    /// Cabeçalhos e dialeto do provedor: o ajudante fala com o MESMO destino
    /// do run que o criou, senão herdaria o local sem querer.
    pub headers: Vec<(String, String)>,
    pub dialect: lr_engine::Dialect,
    pub registry: Arc<ToolRegistry>,
    pub store: Arc<Store>,
    pub config: Arc<AgentConfig>,
    /// O MESMO handle do run: é ele que faz o cancelamento e as confirmações
    /// atravessarem para dentro do ajudante.
    pub handle: Arc<RunHandle>,
    pub sink: Arc<EventSink>,
    /// Modelo e amostragem — o ajudante roda no modelo que já está carregado.
    pub opts: RunOptions,
    /// Pasta do projeto.
    pub workspace: Option<PathBuf>,
    /// Janela do modelo: manda no tamanho do cardápio e na compactação.
    pub n_ctx: Option<u32>,
    /// Famílias de ferramentas que a pessoa deixou ligadas.
    pub groups: Vec<ToolGroup>,
    pub overrides: Vec<PermissionOverride>,
    pub mode: RunMode,
    /// Arquivos que os ajudantes alteraram — o pai junta aos dele antes da
    /// verificação final, senão o que o ajudante escreveu não é conferido.
    pub written: Arc<Mutex<Vec<String>>>,
    /// Comandos que eles rodaram (mesma razão).
    pub commands: Arc<Mutex<Vec<CommandRecord>>>,
    /// Passos gastos por todos os ajudantes (entra no `UsageStats` do run).
    ///
    /// Também é o freio: o teto do run (`max_steps`) não enxerga o que roda
    /// dentro de um ajudante, então sem este contador dez delegações seriam
    /// oitenta chamadas ao modelo fora de qualquer conta.
    pub steps: Arc<AtomicU32>,
    /// Chamadas de ferramenta dos ajudantes (o `UsageStats` do run subcontaria
    /// sem isto).
    pub tool_calls: Arc<AtomicU32>,
    /// Fatos da memória e instruções da pessoa: valem para o ajudante também
    /// — "usa pnpm, não npm" não deixa de ser verdade porque quem trabalha é
    /// o ajudante.
    pub memory: Vec<String>,
    pub user_system: Option<String>,
}

/// O que os ajudantes de um run deixam para o pai.
///
/// Um objeto só em vez de quatro `Arc` soltos: quem monta o run não deveria
/// precisar lembrar de ligar cada fio, e esquecer um deles significa arquivo
/// alterado que não é conferido no fim.
#[derive(Debug, Default)]
pub struct Ledger {
    pub written: Arc<Mutex<Vec<String>>>,
    pub commands: Arc<Mutex<Vec<CommandRecord>>>,
    pub steps: Arc<AtomicU32>,
    pub tool_calls: Arc<AtomicU32>,
}

impl Ledger {
    /// Junta ao que o agente principal fez. Sem duplicar: pai e ajudante
    /// costumam tocar os mesmos arquivos.
    pub fn merge_into(&self, written: &mut Vec<String>, commands: &mut Vec<CommandRecord>) {
        for file in lock(&self.written).iter() {
            if !written.contains(file) {
                written.push(file.clone());
            }
        }
        commands.extend(lock(&self.commands).iter().cloned());
    }

    pub fn steps(&self) -> u32 {
        self.steps.load(Ordering::Relaxed)
    }

    pub fn tool_calls(&self) -> u32 {
        self.tool_calls.load(Ordering::Relaxed)
    }
}

/// A ferramenta que entrega uma missão a um ajudante.
pub struct AgentDelegate {
    deps: SubagentDeps,
}

impl AgentDelegate {
    pub fn new(deps: SubagentDeps) -> Self {
        Self { deps }
    }

    /// Monta o cardápio DO AJUDANTE: curado contra o objetivo dele, com as
    /// mesmas famílias e a mesma janela do run.
    async fn helper_menu(&self, objective: &str, role: SubagentRole) -> Arc<MenuState> {
        let specs: Vec<ToolSpec> = self
            .deps
            .registry
            .specs()
            .await
            .into_iter()
            // Sem recursão. A exclusão é por nome porque contar com o bom
            // senso do modelo não é um mecanismo.
            .filter(|s| s.name != AGENT_DELEGATE)
            // Explorador investiga e relata: nada que altere o computador da
            // pessoa chega a ser oferecido a ele.
            .filter(|s| !role.read_only() || s.read_only)
            .collect();

        let available = specs.len();
        let mut curated = menu::curate(specs, objective, &self.deps.groups, self.deps.n_ctx);
        // Família desligada é escolha da pessoa e vale para o ajudante
        // também: não fica guardada para o `tools_find` ressuscitar.
        curated
            .rest
            .retain(|s| self.deps.groups.contains(&menu::group_of(s)));
        Arc::new(MenuState::new(curated, available))
    }

    /// Um [`ToolRunner`] só para o ajudante.
    ///
    /// Mesma política e mesmo `RunHandle` do pai (cancelar e confirmar
    /// continuam valendo), mas memória de leitura, de repetição e de erro
    /// zeradas: elas descrevem UM contexto, e o dele é novo. O checkpoint
    /// também recomeça — a primeira alteração feita aqui merece a própria
    /// foto.
    fn helper_runner(&self, local_tools: Vec<SharedTool>) -> ToolRunner {
        ToolRunner {
            run_id: self.deps.handle.id.clone(),
            mode: self.deps.mode,
            workspace: self.deps.workspace.clone(),
            sink: self.deps.sink.clone(),
            registry: self.deps.registry.clone(),
            store: self.deps.store.clone(),
            handle: self.deps.handle.clone(),
            config: self.deps.config.clone(),
            policy: PolicyEngine::new(self.deps.overrides.clone()),
            overrides: self.deps.overrides.clone(),
            snapshotted: Default::default(),
            full_snapshot: false,
            reads: ReadLedger::default(),
            repeats: RepeatDetector::default(),
            errors: ErrorStreak::default(),
            written: Vec::new(),
            task_written0: 0,
            exige_escrita: false,
            reescritas: Default::default(),
            commands: Vec::new(),
            focus_md: None,
            tool_calls: 0,
            local_tools,
            // O ajudante não tem ferramenta meta que encerre o trecho: o
            // sinal existe só porque o `ToolRunner` o exige.
            halt: Arc::new(AtomicBool::new(false)),
            counters: Default::default(),
            dentro_de_programa: false,
            escalonar_apos_programa: None,
            saida_ao_vivo: None,
            // O ajudante trabalha no modo nativo: ele existe para poupar a
            // janela do agente principal com uma investigação curta, e um
            // programa dentro de um subagente esconderia duas camadas de
            // chamadas atrás de um único resultado.
            code_menu: None,
        }
    }

    /// Devolve ao pai o que o ajudante deixou para trás.
    fn merge(&self, runner: &ToolRunner, steps: u32) {
        self.deps.steps.fetch_add(steps, Ordering::Relaxed);
        self.deps
            .tool_calls
            .fetch_add(runner.tool_calls, Ordering::Relaxed);
        {
            let mut written = lock(&self.deps.written);
            for file in &runner.written {
                if !written.contains(file) {
                    written.push(file.clone());
                }
            }
        }
        lock(&self.deps.commands).extend(runner.commands.iter().cloned());
    }
}

#[async_trait]
impl Tool for AgentDelegate {
    fn name(&self) -> &str {
        AGENT_DELEGATE
    }

    fn description(&self) -> &str {
        "Entrega uma missão fechada a um ajudante que trabalha sozinho e devolve só um resumo. \
         O que ele lê não ocupa lugar na sua janela — por isso delegue quando a resposta exigir \
         vasculhar o projeto ou ler muitos arquivos, quando a tarefa correr ao lado do que você \
         está pensando, ou quando o resultado couber em poucas linhas. NÃO delegue o que você \
         resolve em um passo (ler UM arquivo, rodar UM comando): delegar custa mais caro do que \
         fazer. Escreva a missão inteira em `objective`, com o que espera de volta: o ajudante \
         não vê nada da sua conversa."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "objective": {
                    "type": "string",
                    "description": "A missão completa, em uma ou duas frases: o que descobrir ou fazer e o que você quer de volta. Ele não vê a sua conversa, então nada pode ficar subentendido."
                },
                "role": {
                    "type": "string",
                    "enum": ["explorer", "coder"],
                    "description": "`explorer` (padrão) só lê: investiga e relata. `coder` também altera arquivos e roda comandos."
                }
            },
            "required": ["objective"]
        })
    }

    fn category(&self) -> ToolCategory {
        // Delegar em si não faz nada no computador: quem pede confirmação são
        // as ferramentas que o ajudante usar.
        ToolCategory::Meta
    }

    async fn preview(&self, args: &Value, _ctx: &ToolContext) -> Option<ToolPreview> {
        let objective = args.get("objective").and_then(Value::as_str)?.trim();
        Some(ToolPreview::Text {
            body: format!(
                "Delegar a um ajudante {}.\nMissão: {}",
                role_label(role_from(args)),
                head_chars(objective, 400)
            ),
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let objective = arg_str(&args, "objective")?;
        let objective = objective.trim();
        if objective.is_empty() {
            return Err(ToolError::InvalidArgs(
                "diga em `objective` qual é a missão do ajudante".into(),
            ));
        }
        // O corte vale para o evento e para o prompt: o que a interface mostra
        // é exatamente a missão que rodou.
        let objective = head_chars(objective, MAX_OBJECTIVE_CHARS);
        let role = role_from(&args);
        let call_id = ctx.call_id.clone();

        // Orçamento de ajudantes esgotado: recusa em texto, para o modelo
        // seguir sozinho em vez de receber um erro e insistir.
        let gastos = self.deps.steps.load(Ordering::Relaxed);
        if gastos >= MAX_TOTAL_SUBAGENT_STEPS {
            return Ok(ToolOutput::text(format!(
                "O orçamento de ajudantes deste pedido acabou ({gastos} passos usados). \
                 Faça esta parte você mesmo, com as ferramentas que você já tem."
            )));
        }

        self.deps.sink.emit(RunEventKind::SubagentStarted {
            call_id: call_id.clone(),
            objective: objective.clone(),
            role,
        });

        let menu = self.helper_menu(&objective, role).await;
        // A porta de saída só faz sentido quando ficou coisa de fora.
        let finder: Option<SharedTool> = menu.is_partial().then(|| {
            let tool: SharedTool = Arc::new(ToolsFind::new(menu.clone(), self.deps.sink.clone()));
            menu.pin_active(tool.spec());
            tool
        });
        let tools_partial = menu.is_partial();
        let tool_names = menu.active_names();

        let mut runner = self.helper_runner(finder.into_iter().collect());
        let client = LlamaClient::new(&self.deps.base_url)
            .with_optional_api_key(self.deps.api_key.clone())
            .with_dialect(self.deps.dialect)
            .with_headers(self.deps.headers.clone())
            .with_stream_deadlines(
                Some(self.deps.config.first_token_timeout),
                Some(self.deps.config.idle_timeout),
            );
        let engine = StepEngine {
            client: &client,
            sink: self.deps.sink.clone(),
            handle: self.deps.handle.clone(),
            store: self.deps.store.clone(),
            run_id: self.deps.handle.id.clone(),
            opts: &self.deps.opts,
            menu: menu.clone(),
            groups: self.deps.groups.clone(),
            tools_on: std::sync::atomic::AtomicBool::new(true),
            recusas_de_tools: std::sync::atomic::AtomicU32::new(0),
            context: ContextBudget::new(self.deps.n_ctx, self.deps.config.context_ratio),
        };

        let workspace = self
            .deps
            .workspace
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());
        let mode = self.deps.mode;
        let memory = self.deps.memory.clone();
        let user_system = self.deps.user_system.clone();
        let mission = objective.clone();
        // Sem plano e sem Focus Chain: o ajudante é uma tacada só, então o
        // prompt não muda de um passo para o outro.
        let build_system = move |_focus: Option<&String>| -> String {
            let mut prompt = build_system_prompt(&PromptContext {
                code_signatures: None,
                workspace: workspace.as_deref(),
                focus_md: None,
                memory: &memory,
                tools: &tool_names,
                user_system: user_system.as_deref(),
                mode,
                tools_partial,
                skill_phase: if role.read_only() {
                    crate::skills::SkillPhase::None
                } else {
                    crate::skills::SkillPhase::Build
                },
            });
            prompt.push_str(&helper_brief(&mission, role));
            prompt
        };

        // O objetivo aparece nas duas mensagens de propósito: no sistema ele
        // fixa o contrato do resumo, no turno da pessoa ele é o pedido que o
        // modelo pequeno de fato obedece.
        let mut messages = vec![
            ChatMessage::system(build_system(None)),
            ChatMessage::user(objective.clone()),
        ];
        let mut budget = StepBudget::new(MAX_SUBAGENT_STEPS);
        let outcome = engine
            .drive(
                &mut runner,
                &mut messages,
                &mut budget,
                MAX_SUBAGENT_STEPS,
                &build_system,
            )
            .await;

        self.merge(&runner, outcome.steps);
        let files = runner.written.clone();
        let report = digest(role, &outcome, &objective, &files);

        self.deps.sink.emit(RunEventKind::SubagentFinished {
            call_id,
            status: outcome.status,
            steps: outcome.steps,
            summary: report.clone(),
        });

        // Um fim ruim volta como TEXTO: o pai precisa poder decidir o que
        // fazer, e um `Err` viraria só mais um erro genérico na conta da
        // escalada. A exceção é o cancelamento do run — aí o laço do pai tem
        // que parar junto.
        if outcome.status == RunStatus::Cancelled {
            return Err(ToolError::Cancelled);
        }
        Ok(ToolOutput::text(report).with_changed(files))
    }
}

/// Papel pedido pelo modelo. Qualquer coisa fora do enum vira `Explorer`, que
/// é o papel que não mexe em nada.
fn role_from(args: &Value) -> SubagentRole {
    match args.get("role").and_then(Value::as_str) {
        Some(raw) if raw.trim().eq_ignore_ascii_case("coder") => SubagentRole::Coder,
        _ => SubagentRole::Explorer,
    }
}

fn role_label(role: SubagentRole) -> &'static str {
    match role {
        SubagentRole::Explorer => "explorador, só leitura",
        SubagentRole::Coder => "programador",
    }
}

/// A seção que transforma o prompt do agente no prompt de um ajudante.
fn helper_brief(objective: &str, role: SubagentRole) -> String {
    let limite = match role {
        SubagentRole::Explorer => {
            "Você só pode LER: investigue e relate, sem alterar arquivo nem rodar comando."
        }
        SubagentRole::Coder => "Você pode ler e alterar o projeto, mas só o que a missão pedir.",
    };
    format!(
        "\n## Você é um ajudante\n\
         O agente principal delegou uma missão a você e não vê nada do que você fizer.\n\
         Missão: {objective}\n\
         {limite}\n\
         Não faça plano nem trabalho fora da missão: você tem poucos passos.\n\
         Ao terminar, responda SEM chamar ferramenta, em até 10 linhas: o que você descobriu \
         ou fez, os caminhos dos arquivos que importam e o que ficou pendente. Esse texto é a \
         ÚNICA coisa que o agente principal vai receber.\n"
    )
}

/// O que volta para o agente principal: uma linha de cabeçalho e o resumo.
fn digest(role: SubagentRole, outcome: &StepOutcome, objective: &str, files: &[String]) -> String {
    let mut out = format!(
        "Ajudante ({}) — {} passo(s).",
        role_label(role),
        outcome.steps
    );
    if !files.is_empty() {
        out.push_str(&format!(" Arquivos alterados: {}.", files.join(", ")));
    }
    out.push('\n');
    out.push_str(&body(outcome, objective));
    out
}

/// O corpo do resumo, por desfecho. Nenhum deles é um beco sem saída: o pai
/// sempre sai daqui sabendo o que pode tentar em seguida.
fn body(outcome: &StepOutcome, objective: &str) -> String {
    let said = outcome.text.trim();
    match outcome.status {
        RunStatus::Done if !said.is_empty() => head_chars(said, MAX_DIGEST_CHARS),
        RunStatus::Done => format!(
            "Ele terminou sem escrever o resumo, então não trouxe nada de útil. Faça você mesmo \
             ou delegue de novo com uma missão mais específica: «{objective}»."
        ),
        RunStatus::MaxSteps => format!(
            "Ele parou no limite de {MAX_SUBAGENT_STEPS} passos sem terminar. O que chegou a \
             relatar:\n{}\nDivida a missão em partes menores e delegue de novo, ou faça você \
             mesmo.",
            told(said)
        ),
        RunStatus::Escalated => format!(
            "Ele foi interrompido: {}\nO que chegou a relatar:\n{}\nSiga por outro caminho — \
             repetir a mesma missão deve dar no mesmo.",
            outcome
                .escalation
                .as_deref()
                .unwrap_or("não conseguiu avançar."),
            told(said)
        ),
        RunStatus::Cancelled => "A execução foi cancelada.".to_string(),
        _ => format!(
            "Ele não conseguiu trabalhar: o modelo não respondeu. Tente você mesmo ou delegue de \
             novo daqui a pouco. Missão: «{objective}»."
        ),
    }
}

fn told(text: &str) -> String {
    match text {
        "" => "(ele não chegou a relatar nada)".to_string(),
        t => head_chars(t, MAX_DIGEST_CHARS),
    }
}

/// Pega um acumulador mesmo depois de um pânico: perder o rastro do que o
/// ajudante fez é pior do que seguir com a lista.
fn lock<T>(shared: &Arc<Mutex<T>>) -> MutexGuard<'_, T> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests;
