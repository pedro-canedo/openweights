//! O laço do agente: pensar, agir, ler o resultado, repetir — até responder.
//!
//! Duas metades vivem aqui:
//! - [`ToolRunner`]: uma chamada de ferramenta do começo ao fim (política,
//!   confirmação, checkpoint, execução, trilha). Não conhece o modelo, o que
//!   a torna testável sem rede.
//! - [`execute_run`]: os passos, o streaming e os guard-rails em volta.
//!
//! Invariante que sustenta tudo: **toda chamada de ferramenta produz uma
//! mensagem `role: "tool"`, na mesma ordem em que o modelo pediu**. Sem isso
//! o template perde o pareamento e o passo seguinte sai incoerente.

use crate::checkpoint;
use crate::codemode;
use crate::events::EventSink;
use crate::menu;
use crate::plan_tools::{PlanTools, SharedPlan, shared_plan, snapshot};
use crate::prompt::{PromptContext, build_system_prompt};
use crate::reliability::{
    ContextBudget, ErrorStreak, ReadLedger, Repeat, RepeatDetector, StepBudget, apply_compaction,
    compaction_request, plan_compaction,
};
use crate::scout::{self, PlanRun};
use crate::subagent;
use crate::verify::{self, CommandRecord};
use crate::{AgentConfig, RunHandle, StartRun, new_id};
use lr_engine::{ChatDelta, ChatMessage, ChatRequest, LlamaClient, ServerProps, ToolCallReq};
use lr_policy::{Decision, PermissionOverride, PolicyEngine, ToolRequest, classify};
use lr_store::Store;
use lr_tools::{SharedTool, ToolContext, ToolError, ToolOutput, ToolRegistry, ToolResult};
use lr_types::agent::{
    ApprovalDecision, ApprovalSource, PolicyScope, RunEventKind, RunMode, RunOptions, RunStatus,
    ToolCategory, ToolGroup, ToolOrigin, ToolPolicy, ToolSpec, ToolTier, ToolsOffReason,
    UsageStats,
};
use lr_types::scout::{TaskPlan, WindowBudget, WorkMode};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// Ferramenta interna que mantém o plano do run.
const FOCUS_TOOL: &str = "todo_update";

/// Quantas vezes o laço insiste com um modelo que anunciou e não fez.
///
/// Duas: a primeira cobre o deslize; a partir da terceira, insistir é o
/// mesmo laço em outro nome, e a pessoa merece ver a resposta que existe.
const MAX_CUTUCADAS: u32 = 2;

/// Teto de retornos "essa ferramenta não existe" para uma chamada escrita em
/// texto. A contagem é separada da cutucada porque aqui o modelo está
/// tentando agir — só errou o nome —, e errar o nome é consertável.
const MAX_NOMES_ERRADOS: u32 = 3;

/// Novas tentativas para um erro TRANSITÓRIO de stream (rede, 5xx genérico,
/// silêncio no prazo). O retry é do MESMO pedido, com as MESMAS ferramentas,
/// e acontece antes de qualquer ferramenta rodar — repetir a chamada ao
/// modelo é seguro; repetir um `git commit` não seria.
pub(crate) const MAX_TENTATIVAS_STREAM: u32 = 2;

/// Recusas de template toleradas antes de desligar as ferramentas de vez.
///
/// A recusa é quase determinística (o template não sabe renderizar `tools`),
/// mas UMA recusa pode ser outro 500 vestido de template — e o desligamento
/// permanente custa o recurso inteiro. Na primeira, o passo segue sem
/// ferramentas e o próximo volta a oferecê-las; na segunda, desliga e avisa.
const MAX_RECUSAS_DE_TOOLS: u32 = 2;

/// Quantas vezes o laço aceita que o modelo emita uma chamada com JSON
/// quebrado antes de desistir. Três: é erro comum e consertável, mas se ele
/// não conserta com a dica, insistir só queima a placa.
const MAX_JSON_QUEBRADO: u32 = 3;

/// O que dizer quando o servidor recusa o passo por causa dos argumentos.
///
/// Não basta dizer "deu erro": o modelo precisa da SAÍDA. Quem cai aqui está
/// tentando escrever um arquivo inteiro numa string só, e a saída é escrever
/// em pedaços. A mensagem ENDURECE a cada recaída: visto em campo (um 9B
/// escrevendo um jogo de 14 KB numa chamada), o conselho geral não muda o
/// comportamento — a ordem concreta com número muda.
fn aviso_json_quebrado(recaida: u32) -> String {
    if recaida <= 1 {
        "Sua última chamada de ferramenta foi recusada: o JSON dos argumentos veio \
         inválido (uma aspa não fechada num conteúdo longo). Não mande o arquivo \
         inteiro de uma vez. Crie primeiro uma base curta com `fs_write` e depois \
         acrescente o resto com `fs_append`, em pedaços de até ~40 linhas."
            .to_string()
    } else {
        // Segunda recaída: sair do JSON de vez. É o que o mercado faz quando
        // o modelo não aguenta o escape (patch em texto no Codex, XML no
        // Cline): o conteúdo viaja como TEXTO puro no stream — que nunca
        // quebra — e quem monta o JSON da escrita é o harness.
        "Quebrou de novo pelo MESMO motivo: conteúdo longo dentro do JSON da \
         chamada. NÃO use mais a ferramenta para este arquivo. Responda em TEXTO \
         PURO, neste formato exato:\n\nARQUIVO: caminho/do/arquivo\n```\n(conteúdo \
         completo do arquivo)\n```\n\nUm arquivo por resposta. Eu gravo para você."
            .to_string()
    }
}

/// Um arquivo entregue em texto puro: `ARQUIVO: caminho` + bloco cercado.
///
/// É o fallback para o que o JSON de tool call não aguenta. Contra um modelo
/// pequeno no llama.cpp, o 500 de "argumentos inválidos" acontece DENTRO do
/// servidor, antes de qualquer byte chegar aqui — não há fragmento para
/// recuperar. A saída do mercado (patch freeform do Codex, XML do Cline) é a
/// mesma: tirar o conteúdo de dentro do JSON. O modelo manda texto, que
/// nunca quebra; o JSON da escrita — com o escape certo — é montado por nós.
fn arquivo_em_texto(texto: &str) -> Option<(String, String)> {
    let inicio = texto.find("ARQUIVO:")?;
    let resto = &texto[inicio + "ARQUIVO:".len()..];
    let (linha_caminho, depois) = resto.split_once('\n')?;
    let caminho = linha_caminho.trim().trim_matches('`').trim();
    if caminho.is_empty() || caminho.contains(' ') {
        return None;
    }
    // O bloco cercado logo depois. A cerca de abertura pode trazer a
    // linguagem (```html); o conteúdo vai até a PRÓXIMA cerca — a última do
    // texto, para um ``` DENTRO do conteúdo não cortar o arquivo no meio.
    let abre = depois.find("```")?;
    let apos_cerca = &depois[abre + 3..];
    let inicio_conteudo = apos_cerca.find('\n')? + 1;
    let corpo = &apos_cerca[inicio_conteudo..];
    let fecha = corpo.rfind("\n```")?;
    let conteudo = &corpo[..fecha];
    if conteudo.trim().is_empty() {
        return None;
    }
    Some((caminho.to_string(), format!("{conteudo}\n")))
}

/// Os empurrões. Curtos de propósito: modelo pequeno afogado em instrução
/// esquece o pedido original.
const CUTUCADA_ANUNCIO: &str = "Nada aconteceu: você descreveu o que ia fazer e não \
                                chamou ferramenta nenhuma. Execute agora a primeira \
                                ação, chamando a ferramenta.";
const CUTUCADA_VAZIA: &str = "Você não respondeu nada. Continue a tarefa: chame a \
                              próxima ferramenta, ou responda em texto se ela já \
                              estiver pronta.";
const CUTUCADA_TEXTO: &str = "Você escreveu a chamada da ferramenta como TEXTO, então \
                              ela não rodou. Use o mecanismo de ferramentas do próprio \
                              modelo (tool call), não um bloco de código.";

/// Este passo precisa de um empurrão? Devolve o que dizer.
///
/// Três formas de um modelo pequeno encerrar a tarefa sem fazê-la — e as três
/// terminavam com o run marcado como CONCLUÍDO, que é o pior desfecho
/// possível: sucesso anunciado, trabalho nenhum.
fn cutucada_para(texto: &str) -> Option<&'static str> {
    if texto.trim().is_empty() {
        return Some(CUTUCADA_VAZIA);
    }
    if chamada_em_texto(texto) {
        return Some(CUTUCADA_TEXTO);
    }
    if anuncio_sem_acao(texto) {
        return Some(CUTUCADA_ANUNCIO);
    }
    None
}

/// A chamada que o modelo escreveu como texto, pronta para rodar.
///
/// Último recurso, e só depois de [`MAX_CUTUCADAS`] pedidos para usar o
/// mecanismo de verdade. Há modelo local que simplesmente não emite tool call
/// pelo endpoint (visto com o qwen2.5-coder-14b), e desistir dele seria
/// deixar de ser agent-first justamente com quem mais precisa de ajuda.
///
/// Não abre exceção de segurança nenhuma: o nome é conferido contra o
/// cardápio ativo e a chamada passa pela MESMA política, confirmação e
/// checkpoint de qualquer outra.
fn chamada_escrita(texto: &str, permitidas: &[String]) -> Option<ToolCallReq> {
    let bruto = bloco_de_chamada(texto)?;
    let v: Value = serde_json::from_str(&bruto).ok()?;
    let nome = v.get("name")?.as_str()?.to_string();
    if !permitidas.contains(&nome) {
        return None;
    }
    let args = match v.get("arguments") {
        Some(Value::String(s)) => s.clone(),
        Some(outro) => outro.to_string(),
        None => "{}".to_string(),
    };
    Some(ToolCallReq {
        id: new_id("call"),
        name: nome,
        arguments_json: args,
    })
}

/// O JSON de uma chamada de ferramenta escondido no texto, se houver.
///
/// Modelo local que não emite tool call escreve a chamada de três jeitos:
/// dentro de `<tool_call>`, num bloco de código, ou solto depois de uma frase
/// ("Vamos listar os arquivos: {...}"). Os três caem aqui.
///
/// A varredura acha o primeiro `{` e fecha a chave contando aninhamento,
/// ignorando o que está dentro de string — assim um `}` no meio de um texto
/// não corta o objeto no lugar errado.
fn bloco_de_chamada(texto: &str) -> Option<String> {
    // Fora da tag `<tool_call>`, exigimos os DOIS campos: um JSON solto com
    // só um `name` é comum demais em prosa para virar chamada.
    let candidato = |t: &str, exige_args: bool| -> Option<String> {
        let bytes: Vec<char> = t.chars().collect();
        let inicio = bytes.iter().position(|c| *c == '{')?;
        let mut nivel = 0usize;
        let mut em_string = false;
        let mut escapado = false;
        for (i, &c) in bytes.iter().enumerate().skip(inicio) {
            if em_string {
                match c {
                    _ if escapado => escapado = false,
                    '\\' => escapado = true,
                    '"' => em_string = false,
                    _ => {}
                }
                continue;
            }
            match c {
                '"' => em_string = true,
                '{' => nivel += 1,
                '}' => {
                    nivel -= 1;
                    if nivel == 0 {
                        let json: String = bytes[inicio..=i].iter().collect();
                        let ok = json.contains("\"name\"")
                            && (!exige_args || json.contains("\"arguments\""));
                        return ok.then_some(json);
                    }
                }
                _ => {}
            }
        }
        None
    };

    if let Some((_, resto)) = texto.split_once("<tool_call>")
        && let Some(achado) = candidato(resto.split("</tool_call>").next().unwrap_or(resto), false)
    {
        return Some(achado);
    }
    // Bloco de código primeiro: é onde o JSON está mais bem delimitado.
    for bloco in texto.split("```").skip(1).step_by(2) {
        if let Some(achado) = candidato(bloco, true) {
            return Some(achado);
        }
    }
    candidato(texto, true)
}

/// O modelo escreveu a chamada em vez de fazê-la?
fn chamada_em_texto(texto: &str) -> bool {
    bloco_de_chamada(texto).is_some()
}

/// O modelo anunciou uma ação em vez de executá-la?
///
/// Modelo pequeno faz isso o tempo todo: "Vou criar os três arquivos.
/// Começando pelo app.py:" — e para. Como resposta em texto significa
/// "terminei", o run encerrava como concluído com a pasta vazia.
///
/// O teste é conservador de propósito, porque um falso positivo faz o laço
/// insistir depois de uma resposta legítima: só pega frase que TERMINA
/// prometendo — dois-pontos no fim, ou verbo de intenção na última linha.
fn anuncio_sem_acao(texto: &str) -> bool {
    let corpo = texto.trim();
    if corpo.is_empty() {
        return false;
    }
    // Bloco de código no fim é entrega, não promessa (o modelo mostrou algo).
    if corpo.ends_with("```") {
        return false;
    }
    if corpo.ends_with(':') {
        return true;
    }
    let ultima = corpo
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .to_lowercase();
    const PROMESSAS: [&str; 12] = [
        "vou ",
        "irei ",
        "vamos ",
        "agora vou",
        "deixa eu",
        "começando por",
        "começando com",
        "primeiro,",
        "i'll ",
        "i will ",
        "let me ",
        "next, i",
    ];
    PROMESSAS.iter().any(|p| ultima.contains(p))
}

/// Esta escrita é um retrocesso? (`vezes` já conta a de agora.)
///
/// O corte é generoso de propósito: encolher um pouco costuma ser limpeza
/// legítima. Menos de 60% do maior tamanho que o arquivo já teve, na segunda
/// escrita ou depois, é outra coisa.
fn reescrita_encolhendo(vezes: u32, maior: u64, agora: u64) -> bool {
    vezes >= 2 && maior > 0 && agora * 10 < maior * 6
}

/// Quanto do resultado vai para a prévia do evento (a UI expande depois).
const PREVIEW_CHARS: usize = 400;
/// Mensagens preservadas intactas na compactação (≈ dois passos).
const KEEP_TAIL_MESSAGES: usize = 6;

/// Erros de ferramenta tolerados no run INTEIRO, contando os que a streak
/// perdoou. O padrão erro–sucesso–erro–sucesso nunca escalava: qualquer
/// sucesso zerava a contagem de seguidos — e a leitura "um sucesso é
/// informação nova" está certa, então a streak fica como está e este teto
/// TOTAL entra por cima. É o único mecanismo que escala direto, sem cutucar.
const MAX_ERROS_TOTAIS: u32 = 8;

/// Passos sem progresso até a cutucada, e até escalar.
///
/// "Sem progresso" é observável: nenhum arquivo mudou, nenhum comando novo
/// rodou com sucesso, e todo resultado foi erro ou repetição. Três passos
/// assim ganham uma cutucada; se nem ela mudar nada, o quinto para o run —
/// só o teto de erros escala sem avisar antes.
const STALL_NUDGE: u32 = 3;
const STALL_ESCALATE: u32 = 5;

/// Teto de passos da rodada de conserto pós-verificação. Curto: ou o
/// conserto é direto (criar o arquivo que falta, rodar de novo o comando),
/// ou não é conserto — e o orçamento global continua valendo por cima.
const MAX_PASSOS_DE_CONSERTO: u32 = 4;

/// Contadores que atravessam o run inteiro — inclusive as etapas do modo
/// laço, que zeram a memória de contexto (`reset_task_memory`) mas NÃO podem
/// zerar isto: um modelo que só anuncia consumiria o orçamento repetindo o
/// padrão etapa após etapa, com o contador local sempre voltando a zero.
#[derive(Debug, Default)]
pub(crate) struct RunCounters {
    pub cutucadas: u32,
    pub nomes_errados: u32,
    pub jsons_quebrados: u32,
    /// Erros de ferramenta no run inteiro (a streak é por etapa; isto não).
    pub erros_totais: u32,
    /// Passos seguidos sem mudança observável no projeto.
    pub sem_progresso: u32,
    /// Confirmações que expiraram sem resposta.
    pub aprovacoes_expiradas: u32,
    // Rascunho do passo corrente (zerado a cada passo).
    pub passo_teve_chamada: bool,
    pub passo_progrediu: bool,
    pub passo_teve_recusa: bool,
}

impl RunCounters {
    /// Abre o rascunho de um passo novo.
    fn begin_step(&mut self) {
        self.passo_teve_chamada = false;
        self.passo_progrediu = false;
        self.passo_teve_recusa = false;
    }

    /// Fecha o passo e diz o que fazer com a estagnação.
    ///
    /// Recusa da pessoa NÃO conta como estagnação: o modelo recebeu
    /// informação real e vai tentar outro caminho — punir isso seria punir o
    /// uso cuidadoso.
    fn end_step(&mut self) -> Option<Stall> {
        if !self.passo_teve_chamada || self.passo_teve_recusa {
            return None;
        }
        if self.passo_progrediu {
            self.sem_progresso = 0;
            return None;
        }
        self.sem_progresso += 1;
        if self.sem_progresso >= STALL_ESCALATE {
            Some(Stall::Escalate)
        } else if self.sem_progresso == STALL_NUDGE {
            Some(Stall::Nudge)
        } else {
            None
        }
    }
}

/// Completa as chamadas de um lote interrompido com um resultado sintético.
///
/// A invariante "toda tool call produz uma mensagem `role: \"tool\"`, na
/// ordem" não é estética: sem ela, um histórico retomado depois quebra o
/// pareamento do template e o passo seguinte sai incoerente.
fn fecha_lote(messages: &mut Vec<ChatMessage>, restantes: &[ToolCallReq], motivo: &str) {
    for tc in restantes {
        messages.push(ChatMessage::tool_result(&tc.id, &tc.name, motivo));
    }
}

/// O veredito de estagnação de um passo.
#[derive(Debug, PartialEq, Eq)]
enum Stall {
    Nudge,
    Escalate,
}

const AVISO_ESTAGNACAO: &str = "Os últimos passos não mudaram NADA no projeto: nenhum \
     arquivo alterado, nenhum comando novo bem-sucedido, só leituras repetidas ou erros. \
     Pare de explorar e execute a próxima ação concreta da tarefa; se algo estiver \
     impedindo, diga em texto o que falta.";

// ------------------------------------------------------------ uma chamada ---

/// O que fazer depois de uma chamada de ferramenta.
pub(crate) enum CallFlow {
    /// Resultado para devolver ao modelo.
    ///
    /// `ok` separa "a ferramenta rodou" de "falhou ou foi recusada". No modo
    /// nativo os dois viram texto na conversa e o modelo se vira; quem chama
    /// de dentro de um programa precisa da diferença, para transformar a
    /// falha numa exceção que o `catch` do script consegue tratar.
    Result { msg: ChatMessage, ok: bool },
    /// Guard-rail estourou: encerra o run e explica para a pessoa.
    Escalate(String),
    /// O run foi cancelado no meio da chamada.
    Cancelled,
}

/// Executa uma chamada de ferramenta respeitando a política do run.
pub(crate) struct ToolRunner {
    pub run_id: String,
    pub mode: RunMode,
    pub workspace: Option<PathBuf>,
    pub sink: Arc<EventSink>,
    pub registry: Arc<ToolRegistry>,
    pub store: Arc<Store>,
    pub handle: Arc<RunHandle>,
    pub config: Arc<AgentConfig>,
    /// Overrides do usuário; a política é remontada quando um "sempre
    /// permitir" chega no meio do run.
    pub overrides: Vec<PermissionOverride>,
    pub policy: PolicyEngine,
    /// Arquivos que já entraram numa foto nesta execução.
    ///
    /// Era um sim/não, e isso perdia dado: a primeira alteração tirava a
    /// foto, a segunda mexia num arquivo que nunca foi fotografado, e
    /// desfazer devolvia só metade. Vazio + `full_snapshot` falso = nada
    /// fotografado ainda.
    pub snapshotted: std::collections::HashSet<String>,
    /// Já houve uma foto do projeto inteiro (ferramenta que não sabe dizer o
    /// que vai mudar). Depois dela, tudo está coberto.
    pub full_snapshot: bool,
    pub reads: ReadLedger,
    pub repeats: RepeatDetector,
    pub errors: ErrorStreak,
    /// Arquivos alterados (alimenta a verificação final).
    pub written: Vec<String>,
    /// Por arquivo: quantas vezes foi escrito neste run e o MAIOR tamanho que
    /// já teve. É o que permite perceber a espiral de reescrita.
    pub reescritas: std::collections::HashMap<String, (u32, u64)>,
    pub commands: Vec<CommandRecord>,
    pub focus_md: Option<String>,
    pub tool_calls: u32,
    /// Ferramentas que existem só neste run (as meta do plano). Ficam fora do
    /// registro compartilhado porque carregam o plano DESTE run; são
    /// procuradas antes dele.
    pub local_tools: Vec<SharedTool>,
    /// Levantado por uma ferramenta meta quando o trecho de laço atual acabou
    /// (etapa concluída, plano escrito ou pergunta para a pessoa).
    pub halt: Arc<AtomicBool>,
    /// Contadores do run inteiro (cutucadas, erros totais, estagnação).
    pub counters: RunCounters,
    /// Guard-rail que estourou DENTRO de um programa (teto de erros, por
    /// exemplo). O resultado da ferramenta volta como erro, e o laço encerra
    /// o run no passo seguinte — que é o que `CallFlow::Escalate` faria se
    /// coubesse no tipo de retorno de uma ferramenta.
    pub escalonar_apos_programa: Option<String>,
    /// Cardápio vivo, quando o Code Mode está ligado (`None` = modo nativo).
    ///
    /// É o cardápio, e não uma cópia das specs, porque o modo laço recura as
    /// ferramentas a cada etapa: a biblioteca que o programa importa tem que
    /// acompanhar o que está ativo AGORA.
    pub code_menu: Option<Arc<menu::MenuState>>,
}

impl ToolRunner {
    fn workspace_str(&self) -> Option<String> {
        self.workspace
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
    }

    /// Ferramenta local com este nome, se houver.
    fn local(&self, name: &str) -> Option<SharedTool> {
        self.local_tools
            .iter()
            .find(|t| t.name() == name)
            .map(Arc::clone)
    }

    /// Metadados da ferramenta: o programa primeiro, depois as locais do run,
    /// depois o registro.
    ///
    /// `run_code` não está em registro nenhum de propósito: ela existe só
    /// enquanto o Code Mode está ligado, e a descrição dela carrega as
    /// assinaturas do cardápio DESTE run.
    async fn spec_for(&self, name: &str) -> Option<ToolSpec> {
        if name == codemode::RUN_CODE
            && let Some(menu) = &self.code_menu
            && let Some(spec) = menu.programa_spec()
        {
            return Some(spec);
        }
        match self.local(name) {
            Some(tool) => Some(tool.spec()),
            None => self.registry.spec_of(name).await,
        }
    }

    /// Executa pela ferramenta local ou pelo registro.
    async fn dispatch(&self, name: &str, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        match self.local(name) {
            Some(tool) => tool.execute(args, ctx).await,
            None => self.registry.execute(name, args, ctx).await,
        }
    }

    /// Uma ferramenta meta pediu para encerrar o trecho atual. Lê e desarma.
    pub(crate) fn take_halt(&self) -> bool {
        self.halt.swap(false, Ordering::SeqCst)
    }

    /// Esquece o que era memória de UM contexto.
    ///
    /// Cada etapa do plano recomeça com histórico vazio: acusar o modelo de
    /// reler um arquivo "que já está no histórico" seria mentira, porque o
    /// histórico não existe mais. O que atravessa (arquivos escritos,
    /// comandos, checkpoint) fica.
    pub(crate) fn reset_task_memory(&mut self) {
        self.reads = ReadLedger::default();
        self.repeats = RepeatDetector::default();
        self.errors = ErrorStreak::default();
    }

    fn tool_context(&self, call_id: &str) -> ToolContext {
        let mut ctx = ToolContext::new(self.workspace.clone(), call_id);
        ctx.max_output_bytes = self.config.max_output_bytes;
        ctx
    }

    /// Uma chamada, do pedido do modelo ao resultado de volta.
    pub(crate) async fn call(
        &mut self,
        step_id: &str,
        step_index: u32,
        tc: &ToolCallReq,
    ) -> CallFlow {
        self.tool_calls += 1;
        self.counters.passo_teve_chamada = true;
        let call_id = tc.id.clone();
        let name = tc.name.clone();

        // Ferramenta inexistente: erro acionável, sem passar pela política.
        let Some(spec) = self.spec_for(&name).await else {
            let message =
                format!("A ferramenta `{name}` não existe. Use apenas as ferramentas listadas.");
            self.trace_rejection(step_id, &call_id, &name, &tc.arguments_json, None, &message);
            return self.record_failure(&call_id, &name, message);
        };

        // JSON inválido é comum em modelos pequenos: vira erro de ferramenta,
        // nunca queda do run.
        let args = match tc.arguments() {
            Ok(Value::Object(map)) => Value::Object(map),
            Ok(Value::Null) => json!({}),
            Ok(_) => {
                let message = "Os argumentos precisam ser um objeto JSON com os campos do \
                               schema da ferramenta."
                    .to_string();
                self.trace_rejection(
                    step_id,
                    &call_id,
                    &name,
                    &tc.arguments_json,
                    Some(&spec),
                    &message,
                );
                return self.record_failure(&call_id, &name, message);
            }
            Err(e) => {
                let message = format!(
                    "Não consegui ler os argumentos: {e}. Reenvie a chamada com um objeto \
                     JSON válido (aspas duplas, sem vírgula sobrando)."
                );
                self.trace_rejection(
                    step_id,
                    &call_id,
                    &name,
                    &tc.arguments_json,
                    Some(&spec),
                    &message,
                );
                return self.record_failure(&call_id, &name, message);
            }
        };

        // Girando em falso: avisa e, se insistir, devolve para a pessoa.
        match self.repeats.observe(&name, &args) {
            Repeat::Fresh => {}
            Repeat::Warn { .. } => {
                let message = RepeatDetector::warning_for(&name);
                self.trace_rejection(
                    step_id,
                    &call_id,
                    &name,
                    &args.to_string(),
                    Some(&spec),
                    &message,
                );
                return CallFlow::Result {
                    msg: ChatMessage::tool_result(&call_id, &name, message),
                    ok: false,
                };
            }
            Repeat::Escalate { .. } => {
                return CallFlow::Escalate(RepeatDetector::escalation_message(&name));
            }
        }

        // Releitura: devolve o ponteiro para o que já está no histórico.
        let read_key = ReadLedger::key_for(&name, &args);
        if let Some(key) = &read_key
            && let Some(previous) = self.reads.seen(key)
        {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let message = ReadLedger::duplicate_message(&path, previous);
            self.trace_rejection(
                step_id,
                &call_id,
                &name,
                &args.to_string(),
                Some(&spec),
                &message,
            );
            return CallFlow::Result {
                msg: ChatMessage::tool_result(&call_id, &name, message),
                ok: true,
            };
        }

        let ctx = self.tool_context(&call_id);
        let builtin = self
            .local(&name)
            .or_else(|| self.registry.get(&name).cloned());
        let preview = match &builtin {
            Some(tool) => tool.preview(&args, &ctx).await,
            // O programa não é uma ferramenta do registro, mas é justamente o
            // caso em que a prévia mais importa: aprovar um programa é
            // aprovar tudo que ele vai fazer.
            None if name == codemode::RUN_CODE => {
                args.get("code").and_then(Value::as_str).map(|code| {
                    lr_types::agent::ToolPreview::Text {
                        body: format!("Rodar este programa:\n\n{}", head_chars(code, 2_000)),
                    }
                })
            }
            None => None,
        };
        let within = builtin
            .as_ref()
            .map(|t| t.within_workspace(&args, &ctx))
            .unwrap_or(true);

        // A mesma ferramenta pode ser leitura ou escrita conforme o pedido
        // (`sql_query` consulta por padrão e altera com permissão explícita).
        // Quem sabe disso são os argumentos, não o catálogo.
        let category = builtin
            .as_ref()
            .map(|t| t.category_for(&args))
            .unwrap_or(spec.category);

        // `terminal_run` e afins: a classificação do comando é o que faz o
        // modo automático continuar pedindo confirmação para o que não dá
        // para analisar.
        let command_text = (category == ToolCategory::Execute)
            .then(|| {
                args.get("command")
                    .or_else(|| args.get("cmd"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .flatten();
        let analysis = command_text.as_deref().map(classify);

        let mut request = ToolRequest::new(&name, category);
        if !within {
            request = request.outside_workspace();
        }
        // Chamada mais pesada que o feitio da ferramenta: o "sempre permitir"
        // não vale para ela.
        if category != spec.category && !matches!(category, ToolCategory::Read | ToolCategory::Meta)
        {
            request = request.escalated();
        }
        if let Some(a) = &analysis {
            request = request.with_command(a.class);
        }
        let decision = self.policy.decide(&request, self.mode);

        let args_json = args.to_string();
        let _ = self.store.create_tool_call(
            &call_id,
            &self.run_id,
            Some(step_id),
            &name,
            &origin_str(&spec.origin),
            &args_json,
        );
        self.sink.emit(RunEventKind::ToolRequested {
            call_id: call_id.clone(),
            tool: name.clone(),
            origin: spec.origin.clone(),
            category,
            tier: spec.tier,
            args_json: args_json.clone(),
            preview,
            requires_approval: matches!(decision, Decision::Ask { .. }),
        });

        // Política e, se preciso, a pessoa.
        let (source, args) = match decision {
            Decision::Allow { source } => {
                self.log_approval(&call_id, &name, "allowOnce", source, &args);
                self.sink.emit(RunEventKind::ToolApproved {
                    call_id: call_id.clone(),
                    source,
                });
                (source, args)
            }
            Decision::Deny { reason } => {
                return self.denied(&call_id, &name, &reason, ApprovalSource::Policy);
            }
            Decision::Ask { reason } => match self.ask_user(&call_id, &name, args, &reason).await {
                Ask::Approved { source, args } => {
                    self.sink.emit(RunEventKind::ToolApproved {
                        call_id: call_id.clone(),
                        source,
                    });
                    (source, args)
                }
                Ask::Denied(reason) => {
                    return self.denied(&call_id, &name, &reason, ApprovalSource::User);
                }
                Ask::Cancelled => return CallFlow::Cancelled,
                Ask::Abandoned => {
                    return CallFlow::Escalate(
                        "Duas confirmações expiraram sem resposta; parei a execução \
                         para você decidir."
                            .into(),
                    );
                }
            },
        };
        let _ = source;

        // Foto antes da primeira alteração — depois de aprovada, antes de rodar.
        //
        // A categoria é o critério principal, mas uma ferramenta que declara
        // arquivos em risco também merece a foto mesmo sendo de outra
        // categoria: `web_download` é de rede e sobrescreve arquivo do
        // projeto. Quem sabe o que vai mudar é a própria ferramenta.
        let files = builtin
            .as_ref()
            .map(|t| t.files_at_risk(&args, &ctx))
            .unwrap_or_default();
        if PolicyEngine::needs_checkpoint(&request) || !files.is_empty() {
            // Só o que ainda não foi fotografado. Quem não declara arquivo
            // (um comando de terminal, um `cargo test`) manda lista vazia, e
            // isso significa "fotografe o que puder" — uma vez por run.
            let novos: Vec<String> = files
                .iter()
                .filter(|f| !self.snapshotted.contains(*f))
                .cloned()
                .collect();
            let precisa = if files.is_empty() {
                !self.full_snapshot
            } else {
                !novos.is_empty() && !self.full_snapshot
            };
            if precisa {
                self.take_checkpoint(&name, novos).await;
            }
        }

        self.execute(
            &name,
            &call_id,
            args,
            read_key,
            step_id,
            step_index,
            command_text,
            spec.category,
        )
        .await
    }

    /// Executa a chamada: pelo registro, ou rodando o programa do Code Mode.
    ///
    /// `run_code` não pode ser uma ferramenta comum do registro porque ela
    /// **reentra no runner**: cada chamada que o programa faz volta por
    /// `ToolRunner::call`, com política, foto do projeto e trilha. Uma
    /// `Tool` recebe `&self`, e isso não caberia.
    async fn despachar(
        &mut self,
        name: &str,
        call_id: &str,
        args: Value,
        step_id: &str,
        step_index: u32,
        ctx: &ToolContext,
    ) -> ToolResult<ToolOutput> {
        if name == codemode::RUN_CODE {
            return self
                .rodar_programa(call_id, args, step_id, step_index)
                .await;
        }
        self.dispatch(name, args, ctx).await
    }

    /// Roda o programa do Code Mode.
    ///
    /// Sobe a ponte, executa o Node e atende cada chamada que o programa
    /// fizer — cada uma voltando por [`ToolRunner::call`], com política,
    /// confirmação, foto do projeto e trilha. Do lado do modelo isto é UM
    /// passo; do lado do harness podem ser dezenas de chamadas, e é essa a
    /// diferença que o Code Mode compra.
    async fn rodar_programa(
        &mut self,
        call_id: &str,
        args: Value,
        step_id: &str,
        step_index: u32,
    ) -> ToolResult<ToolOutput> {
        let Some(root) = self.workspace.clone() else {
            return Err(ToolError::Other(
                "o Code Mode precisa de uma pasta de projeto escolhida.".into(),
            ));
        };
        let code = args
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if code.is_empty() {
            return Err(ToolError::InvalidArgs(
                "`code` está vazio — mande o programa que resolve a tarefa".into(),
            ));
        }

        let specs = self
            .code_menu
            .as_ref()
            .map(|m| m.script_specs())
            .unwrap_or_default();
        // As peças que o projeto criou entram junto: elas são funções do
        // programa, não ferramentas do registro (ver `lr_codemode::plugins`).
        let plugins = lr_codemode::carregar_plugins(&root);

        let (ponte, fila) = lr_codemode::Bridge::start().map_err(|e| {
            ToolError::Other(format!("não consegui abrir a ponte do Code Mode: {e}"))
        })?;

        let mut pedido = lr_codemode::ScriptRequest::new(code, root, specs)
            .with_plugins(plugins)
            .with_bridge(ponte.url(), ponte.token());
        // O caminho do Node é perguntado agora, não no boot do app: quem
        // baixou o runtime no meio da sessão passa a poder rodar programa
        // sem reiniciar nada.
        let node = self.config.node_path.as_ref().and_then(|resolve| resolve());
        pedido.node = lr_codemode::node_program(node.as_deref());
        pedido.max_output_bytes = self.config.max_output_bytes;

        // A saída aparece na UI enquanto o programa roda, como a de qualquer
        // comando: um programa que demora não pode parecer travado.
        let sink = self.sink.clone();
        let cid = call_id.to_string();
        let script = lr_codemode::run_script(pedido, move |pedaco| {
            sink.emit(RunEventKind::ToolOutput {
                call_id: cid.clone(),
                chunk: pedaco.to_string(),
                truncated: false,
            });
        });

        let mut despacho = DespachoDoScript {
            runner: self,
            step_id: step_id.to_string(),
            step_index,
            chamadas: 0,
            escalonar: None,
        };
        let (saida, contagem, parada) = codemode::hospedar(fila, script, &mut despacho).await;
        let escalonar = despacho.escalonar.take();
        // A ponte fecha aqui: nenhuma chamada é atendida depois do programa.
        drop(ponte);

        if let Some(motivo) = escalonar {
            self.escalonar_apos_programa = Some(motivo.clone());
            return Err(ToolError::Other(motivo));
        }
        let resultado = match saida {
            Some(Ok(resultado)) => resultado,
            Some(Err(e)) => return Err(ToolError::Other(e.to_string())),
            None => {
                return Err(ToolError::Other(
                    parada.unwrap_or_else(|| "o programa foi interrompido".into()),
                ));
            }
        };

        let outcome = resultado.spawn;
        let mut corpo = String::new();
        let stdout = outcome.stdout.trim_end();
        let stderr = outcome.stderr.trim_end();
        if !stdout.is_empty() {
            corpo.push_str(stdout);
        }
        if !stderr.is_empty() {
            corpo.push_str("\n\n[erros do programa]\n");
            corpo.push_str(stderr);
        }
        if outcome.timed_out {
            corpo.push_str("\n\n[o programa passou do tempo e foi interrompido]");
        } else if stdout.is_empty() {
            corpo.push_str(
                "(o programa não imprimiu nada — use `say(...)` para o resultado voltar \
                 para você)",
            );
        }
        corpo.push_str(&contagem.rodape());
        if !resultado.isolado {
            // Honestidade na trilha: sem o modo de permissões do Node, o
            // programa PODE ter mexido em arquivo sem passar pela política.
            corpo.push_str(
                "\n[atenção: o Node desta máquina é anterior à versão 22 e o programa rodou \
                 sem isolamento de arquivos]",
            );
        }

        Ok(ToolOutput::text(corpo)
            .with_exit_code(outcome.exit_code)
            .truncated_to(self.config.max_output_bytes))
    }

    /// Roda a ferramenta e traduz o resultado para o modelo.
    #[allow(clippy::too_many_arguments)]
    async fn execute(
        &mut self,
        name: &str,
        call_id: &str,
        args: Value,
        read_key: Option<String>,
        step_id: &str,
        step_index: u32,
        comando: Option<String>,
        category: ToolCategory,
    ) -> CallFlow {
        let _ = self.store.set_tool_call_state(call_id, "running");
        self.sink.emit(RunEventKind::ToolStarted {
            call_id: call_id.to_string(),
        });

        let ctx = self.tool_context(call_id);
        let started = Instant::now();
        // O handle é clonado porque o outro braço do `select!` precisa do
        // `&mut self` (o programa do Code Mode reentra no runner a cada
        // chamada que faz).
        let handle = self.handle.clone();
        let outcome = tokio::select! {
            biased;
            _ = handle.cancelled() => {
                let _ = self.store.finish_tool_call(call_id, false, "", 0, Some("cancelado"));
                return CallFlow::Cancelled;
            }
            r = self.despachar(name, call_id, args.clone(), step_id, step_index, &ctx) => r,
        };
        let duration_ms = started.elapsed().as_millis() as u64;

        // Um teto estourado DENTRO do programa não cabe no retorno de uma
        // ferramenta, então ele viaja num campo e é lido aqui: o run encerra
        // igual ao que aconteceria se a chamada fosse solta.
        if let Some(motivo) = self.escalonar_apos_programa.take() {
            let _ = self
                .store
                .finish_tool_call(call_id, false, "", 0, Some(&motivo));
            self.sink.emit(RunEventKind::ToolResult {
                call_id: call_id.to_string(),
                ok: false,
                result_preview: head_chars(&motivo, PREVIEW_CHARS),
                bytes_total: motivo.len() as u64,
                duration_ms,
            });
            return CallFlow::Escalate(motivo);
        }

        match outcome {
            Ok(out) => {
                let truncated = out.bytes_total > out.content.len() as u64;
                self.sink.emit(RunEventKind::ToolOutput {
                    call_id: call_id.to_string(),
                    chunk: out.content.clone(),
                    truncated,
                });
                self.sink.emit(RunEventKind::ToolResult {
                    call_id: call_id.to_string(),
                    ok: true,
                    result_preview: head_chars(&out.content, PREVIEW_CHARS),
                    bytes_total: out.bytes_total,
                    duration_ms,
                });
                let result_json = json!({
                    "content": out.content,
                    "changedFiles": out.changed_files,
                })
                .to_string();
                let _ =
                    self.store
                        .finish_tool_call(call_id, true, &result_json, out.bytes_total, None);

                self.errors.record_success();
                // Progresso observável do passo: qualquer ferramenta de
                // verdade que rodou (meta não conta — atualizar o plano em
                // círculos não é progresso). Mudança de ESTADO, além disso,
                // esvazia a janela de repetição: reler depois de escrever é
                // conferência, não laço.
                if category != ToolCategory::Meta {
                    self.counters.passo_progrediu = true;
                }
                if !out.changed_files.is_empty()
                    || (category == ToolCategory::Execute && out.exit_code.is_none_or(|c| c == 0))
                {
                    self.repeats.note_progress();
                }
                if let Some(key) = read_key {
                    self.reads.note(&key, step_index);
                }
                for file in &out.changed_files {
                    if !self.written.contains(file) {
                        self.written.push(file.clone());
                    }
                    // O arquivo MUDOU: a leitura antiga que está no histórico
                    // ficou velha, e a releitura de conferência tem que passar.
                    self.reads.invalidate_file(file);
                }
                // Todo Execute vira registro — antes só quem tinha um campo
                // `command` nos argumentos entrava, e `test_run`/`build_run`
                // (o sinal mais valioso: "a suíte passou") eram invisíveis
                // para a verificação. A linha COMPLETA vai no display: dois
                // `cargo` diferentes não podem virar o mesmo registro.
                if category == ToolCategory::Execute {
                    self.commands.push(CommandRecord {
                        display: comando.clone().unwrap_or_else(|| name.to_string()),
                        ok: true,
                        // O código DECLARADO pela ferramenta vence o marcador
                        // textual: um log com "exit code 1" não pode derrubar
                        // um comando que saiu com 0.
                        exit_code: out
                            .exit_code
                            .or_else(|| verify::extract_exit_code(&out.content)),
                    });
                }
                if name == FOCUS_TOOL {
                    self.update_focus(&args, &out.content);
                }

                let mut resposta = out.content;
                if let Some(aviso) = self.aviso_de_reescrita(&out.changed_files) {
                    resposta.push_str("\n\n");
                    resposta.push_str(&aviso);
                }
                CallFlow::Result {
                    msg: ChatMessage::tool_result(call_id, name, resposta),
                    ok: true,
                }
            }
            Err(e) => {
                let message = e.to_model_message();
                self.sink.emit(RunEventKind::ToolResult {
                    call_id: call_id.to_string(),
                    ok: false,
                    result_preview: head_chars(&message, PREVIEW_CHARS),
                    bytes_total: 0,
                    duration_ms,
                });
                let _ = self
                    .store
                    .finish_tool_call(call_id, false, "", 0, Some(&message));
                if category == ToolCategory::Execute {
                    self.commands.push(CommandRecord {
                        display: comando.clone().unwrap_or_else(|| name.to_string()),
                        ok: false,
                        exit_code: None,
                    });
                }
                self.record_failure(call_id, name, message)
            }
        }
    }

    /// Pergunta para a pessoa e espera (o run fica pausado até a resposta).
    async fn ask_user(&mut self, call_id: &str, name: &str, args: Value, reason: &str) -> Ask {
        let rx = self.handle.register_pending(call_id);
        let _ = self
            .store
            .set_run_status(&self.run_id, RunStatus::WaitingApproval);
        self.sink.emit(RunEventKind::RunPaused {
            reason: lr_types::agent::PauseReason::WaitingApproval,
        });

        let decision = tokio::select! {
            biased;
            _ = self.handle.cancelled() => {
                self.handle.clear_pending(call_id);
                return Ask::Cancelled;
            }
            answer = tokio::time::timeout(self.config.approval_timeout, rx) => match answer {
                Ok(Ok(decision)) => decision,
                // Emissor sumiu (run derrubado): trata como cancelamento.
                Ok(Err(_)) => {
                    self.handle.clear_pending(call_id);
                    return Ask::Cancelled;
                }
                Err(_) => {
                    // Expirar não pode custar o run inteiro: a PRIMEIRA vez
                    // nega só esta chamada (o modelo recebe o motivo e pode
                    // contornar ou finalizar com o que tem); a segunda diz
                    // que não há ninguém aí e para como escalado — um run
                    // desassistido queimando passos é pior que parar.
                    self.handle.clear_pending(call_id);
                    self.counters.aprovacoes_expiradas += 1;
                    if self.counters.aprovacoes_expiradas >= 2 {
                        log::warn!("segunda confirmação expirada; parando o run");
                        return Ask::Abandoned;
                    }
                    log::warn!("confirmação de `{name}` expirou; nego a chamada e sigo");
                    let _ = self.store.set_run_status(&self.run_id, RunStatus::Running);
                    self.sink.emit(RunEventKind::RunResumed);
                    return Ask::Denied(
                        "ninguém respondeu à confirmação a tempo — a ação foi negada. \
                         Siga por um caminho que não precise dela, ou finalize em texto \
                         com o que já tem."
                            .into(),
                    );
                }
            },
        };
        self.handle.clear_pending(call_id);
        let _ = self.store.set_run_status(&self.run_id, RunStatus::Running);
        self.sink.emit(RunEventKind::RunResumed);
        log::debug!("confirmação de `{name}` resolvida ({reason})");

        match decision {
            ApprovalDecision::AllowOnce => {
                self.log_approval(call_id, name, "allowOnce", ApprovalSource::User, &args);
                Ask::Approved {
                    source: ApprovalSource::User,
                    args,
                }
            }
            ApprovalDecision::AllowAlways { scope } => {
                self.remember(name, scope, ToolPolicy::AlwaysAllow);
                self.log_approval(call_id, name, "allowAlways", ApprovalSource::User, &args);
                Ask::Approved {
                    source: ApprovalSource::User,
                    args,
                }
            }
            ApprovalDecision::AllowEdited { args_json } => {
                match serde_json::from_str::<Value>(&args_json) {
                    Ok(edited) if edited.is_object() => {
                        self.log_approval(
                            call_id,
                            name,
                            "allowEdited",
                            ApprovalSource::User,
                            &edited,
                        );
                        Ask::Approved {
                            source: ApprovalSource::User,
                            args: edited,
                        }
                    }
                    _ => Ask::Denied(
                        "Os argumentos editados não formavam um objeto JSON válido.".into(),
                    ),
                }
            }
            ApprovalDecision::Deny { reason } => {
                Ask::Denied(reason.unwrap_or_else(|| "A pessoa recusou esta ação.".to_string()))
            }
            ApprovalDecision::DenyAlways { scope } => {
                self.remember(name, scope, ToolPolicy::Never);
                Ask::Denied(format!(
                    "A pessoa recusou esta ação e desativou `{name}` para as próximas."
                ))
            }
        }
    }

    /// Grava um "sempre permitir"/"nunca" e recarrega a política do run.
    fn remember(&mut self, tool: &str, scope: PolicyScope, policy: ToolPolicy) {
        let workspace = self.workspace_str();
        let dir = match scope {
            PolicyScope::Workspace => workspace.as_deref(),
            PolicyScope::Global => None,
        };
        if let Err(e) = self.store.set_tool_permission(scope, dir, tool, policy) {
            log::warn!("não consegui guardar a permissão de `{tool}`: {e}");
        }
        self.overrides.retain(|o| o.tool_name != tool);
        self.overrides.push(PermissionOverride {
            tool_name: tool.to_string(),
            policy,
            scope,
        });
        self.policy = PolicyEngine::new(self.overrides.clone());
    }

    /// Recusa: o modelo PRECISA saber para tentar outro caminho.
    ///
    /// E não conta como estagnação: quem recusou deu informação real ao
    /// modelo — punir o passo seria punir o uso cuidadoso.
    fn denied(
        &mut self,
        call_id: &str,
        name: &str,
        reason: &str,
        source: ApprovalSource,
    ) -> CallFlow {
        self.counters.passo_teve_recusa = true;
        self.sink.emit(RunEventKind::ToolDenied {
            call_id: call_id.to_string(),
            reason: reason.to_string(),
        });
        let _ = self
            .store
            .finish_tool_call(call_id, false, "", 0, Some(reason));
        let _ = self
            .store
            .log_approval(&self.run_id, call_id, name, "deny", source, "");
        CallFlow::Result {
            msg: ChatMessage::tool_result(
                call_id,
                name,
                format!(
                    "A ação foi recusada: {reason} Não tente de novo do mesmo jeito — \
                     proponha outro caminho ou explique o que precisa."
                ),
            ),
            ok: false,
        }
    }

    /// Aviso quando o modelo entra na espiral de reescrever o mesmo arquivo,
    /// cada vez com menos conteúdo.
    ///
    /// Vem de um run real: um 9B reescreveu `app.py` doze vezes, cada versão
    /// cortada no meio de uma função, e o harness respondeu "ok" doze vezes.
    /// O modelo não tem como perceber sozinho que o próprio conteúdo está
    /// sendo cortado antes do fim — mas nós temos o tamanho das versões
    /// anteriores, e é exatamente essa a informação que falta a ele.
    fn aviso_de_reescrita(&mut self, arquivos: &[String]) -> Option<String> {
        let raiz = self.workspace.as_ref()?;
        for arquivo in arquivos {
            let Ok(meta) = std::fs::metadata(raiz.join(arquivo)) else {
                continue;
            };
            let agora = meta.len();
            let entrada = self.reescritas.entry(arquivo.clone()).or_insert((0, 0));
            entrada.0 += 1;
            let (vezes, maior) = (entrada.0, entrada.1);
            entrada.1 = maior.max(agora);
            if reescrita_encolhendo(vezes, maior, agora) {
                return Some(format!(
                    "ATENÇÃO: `{arquivo}` já foi escrito {vezes} vezes nesta execução e \
                     encolheu de {maior} para {agora} bytes — sinal de que o conteúdo está \
                     sendo cortado antes do fim. Pare de reescrever o arquivo inteiro: leia \
                     o que está lá com `fs_read` e acrescente o que falta com `fs_append`, em \
                     pedaços pequenos."
                ));
            }
        }
        None
    }

    /// Falha de ferramenta: devolve ao modelo e conta para a escalada.
    fn record_failure(&mut self, call_id: &str, name: &str, message: String) -> CallFlow {
        let msg = ChatMessage::tool_result(call_id, name, format!("ERRO: {message}"));
        // O teto TOTAL vem antes da streak: erro–sucesso–erro–sucesso nunca
        // escalava, porque qualquer sucesso zerava os "seguidos" — e zerar
        // está certo (um sucesso É informação nova); o que faltava era o
        // teto por cima, que atravessa inclusive as etapas do laço.
        self.counters.erros_totais += 1;
        if self.counters.erros_totais >= MAX_ERROS_TOTAIS {
            return CallFlow::Escalate(format!(
                "Parei depois de {} erros de ferramenta nesta execução. \
                 Revise o pedido ou me diga por onde seguir.",
                self.counters.erros_totais
            ));
        }
        if self.errors.record_error() {
            return CallFlow::Escalate(self.errors.escalation_message());
        }
        CallFlow::Result { msg, ok: false }
    }

    /// Registra na trilha uma chamada que nem chegou a rodar.
    fn trace_rejection(
        &self,
        step_id: &str,
        call_id: &str,
        name: &str,
        args_json: &str,
        spec: Option<&ToolSpec>,
        message: &str,
    ) {
        let origin = spec
            .map(|s| s.origin.clone())
            .unwrap_or(ToolOrigin::Builtin);
        let _ = self.store.create_tool_call(
            call_id,
            &self.run_id,
            Some(step_id),
            name,
            &origin_str(&origin),
            args_json,
        );
        self.sink.emit(RunEventKind::ToolRequested {
            call_id: call_id.to_string(),
            tool: name.to_string(),
            origin,
            category: spec.map(|s| s.category).unwrap_or(ToolCategory::Meta),
            tier: spec.map(|s| s.tier).unwrap_or(ToolTier::Safe),
            args_json: args_json.to_string(),
            preview: None,
            requires_approval: false,
        });
        self.sink.emit(RunEventKind::ToolResult {
            call_id: call_id.to_string(),
            ok: false,
            result_preview: head_chars(message, PREVIEW_CHARS),
            bytes_total: 0,
            duration_ms: 0,
        });
        let _ = self
            .store
            .finish_tool_call(call_id, false, "", 0, Some(message));
    }

    fn log_approval(
        &self,
        call_id: &str,
        name: &str,
        decision: &str,
        source: ApprovalSource,
        args: &Value,
    ) {
        let _ = self.store.log_approval(
            &self.run_id,
            call_id,
            name,
            decision,
            source,
            &args_hash(args),
        );
    }

    /// Tira a foto dos arquivos antes da primeira alteração do run.
    async fn take_checkpoint(&mut self, tool: &str, files: Vec<String>) {
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        let files_pedidos = files.clone();
        let store_dir = self.config.store_dir.clone();
        let label = format!("Antes de {tool}");
        let label_task = label.clone();
        let result = tokio::task::spawn_blocking(move || {
            checkpoint::snapshot_blocking(&workspace, &store_dir, &label_task, &files)
        })
        .await;

        match result {
            Ok(Ok(cp)) => {
                if files_pedidos.is_empty() {
                    self.full_snapshot = true;
                }
                self.snapshotted.extend(cp.files.iter().cloned());
                let files_json = serde_json::to_string(&cp.files).ok();
                let _ = self.store.add_checkpoint(
                    &cp.id,
                    Some(&self.run_id),
                    &self.workspace_str().unwrap_or_default(),
                    &cp.backend,
                    &cp.ref_id,
                    Some(&label),
                    files_json.as_deref(),
                );
                self.sink.emit(RunEventKind::CheckpointCreated {
                    checkpoint_id: cp.id,
                    label,
                    backend: cp.backend,
                });
            }
            Ok(Err(e)) => {
                // Sem foto o run continua, mas a pessoa precisa saber que o
                // "desfazer" não vai existir desta vez.
                log::warn!("checkpoint falhou: {e}");
                self.sink.emit(RunEventKind::RunError {
                    message: format!("Não consegui criar o ponto de restauração: {e}"),
                    retryable: true,
                });
            }
            Err(e) => log::warn!("tarefa do checkpoint falhou: {e}"),
        }
    }

    /// Plano atualizado pela ferramenta interna de todo.
    fn update_focus(&mut self, args: &Value, output: &str) {
        let md = args
            .get("todo_md")
            .or_else(|| args.get("plan"))
            .or_else(|| args.get("markdown"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| output.to_string());
        if md.trim().is_empty() {
            return;
        }
        let _ = self.store.set_run_focus(&self.run_id, &md);
        self.focus_md = Some(md.clone());
        self.sink.emit(RunEventKind::FocusUpdated { todo_md: md });
    }
}

/// Resultado da conversa com a pessoa sobre uma chamada.
enum Ask {
    Approved {
        source: ApprovalSource,
        args: Value,
    },
    Denied(String),
    Cancelled,
    /// Duas confirmações expiraram sem resposta: não há ninguém aí.
    Abandoned,
}

fn origin_str(origin: &ToolOrigin) -> String {
    match origin {
        ToolOrigin::Builtin => "builtin".to_string(),
        ToolOrigin::Mcp { server_id } => format!("mcp:{server_id}"),
    }
}

/// Impressão digital dos argumentos (auditoria — não é segredo, é rastro).
fn args_hash(args: &Value) -> String {
    let mut hash: u64 = 1469598103934665603;
    for b in args.to_string().as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    format!("{hash:016x}")
}

pub(crate) fn head_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect::<String>() + "…"
}

// ------------------------------------------------------------------ laço ---

/// Histórico do chat convertido em mensagens para o modelo.
/// Atende as chamadas de um programa do Code Mode.
///
/// É um empréstimo do runner com o passo atual em mãos: cada chamada do
/// programa vira uma chamada normal, no mesmo passo, e o que a política
/// decidir vale igual. O programa não é um caminho paralelo — é o mesmo
/// caminho, percorrido mais vezes sem passar pelo modelo.
struct DespachoDoScript<'a> {
    runner: &'a mut ToolRunner,
    step_id: String,
    step_index: u32,
    chamadas: u32,
    /// Guard-rail que estourou no meio (teto de erros, repetição).
    escalonar: Option<String>,
}

/// Teto de chamadas de um único programa.
///
/// O prazo do processo já cobre o laço infinito, mas um programa que chama
/// ferramenta dentro de um `for` sobre mil arquivos gastaria o dia. Este teto
/// é alto o bastante para o trabalho de verdade (doze arquivos de log, uma
/// varredura de pasta) e baixo o bastante para a pessoa não ficar refém.
const MAX_CHAMADAS_POR_PROGRAMA: u32 = 200;

#[async_trait::async_trait]
impl codemode::Despachante for DespachoDoScript<'_> {
    async fn chamar(&mut self, tool: &str, args: Value) -> codemode::Resposta {
        if self.runner.handle.is_cancelled() {
            return codemode::Resposta::Parar("a execução foi cancelada".into());
        }
        // Programa que roda programa: recusado. O aninhamento não acrescenta
        // nada (o programa já pode tudo) e tornaria o teto acima contornável.
        if tool == codemode::RUN_CODE {
            return codemode::Resposta::Erro(
                "`run_code` não pode ser chamada de dentro de um programa: você já está \
                 dentro de um."
                    .into(),
            );
        }
        self.chamadas += 1;
        if self.chamadas > MAX_CHAMADAS_POR_PROGRAMA {
            let motivo = format!(
                "O programa passou de {MAX_CHAMADAS_POR_PROGRAMA} chamadas de ferramenta e foi \
                 interrompido. Divida a tarefa ou filtre antes de chamar."
            );
            self.escalonar = Some(motivo.clone());
            return codemode::Resposta::Parar(motivo);
        }

        let tc = ToolCallReq {
            id: new_id("call"),
            name: tool.to_string(),
            arguments_json: args.to_string(),
        };
        // `Box::pin` porque isto é recursão assíncrona: `call` volta a passar
        // por `despachar`, e o tipo do futuro não pode se conter.
        match Box::pin(self.runner.call(&self.step_id, self.step_index, &tc)).await {
            CallFlow::Result { msg, ok } => {
                let texto = msg
                    .content
                    .as_ref()
                    .map(|c| c.as_plain_text())
                    .unwrap_or_default();
                if ok {
                    codemode::Resposta::Ok(texto)
                } else {
                    codemode::Resposta::Erro(texto)
                }
            }
            CallFlow::Cancelled => codemode::Resposta::Parar("a execução foi cancelada".into()),
            CallFlow::Escalate(motivo) => {
                self.escalonar = Some(motivo.clone());
                codemode::Resposta::Parar(motivo)
            }
        }
    }
}

/// As assinaturas do que o programa pode chamar: as ferramentas do cardápio
/// mais as peças que o próprio projeto criou.
fn menu_assinaturas(menu: &menu::MenuState, workspace: Option<&PathBuf>) -> String {
    let mut specs = menu.script_specs();
    if let Some(raiz) = workspace {
        specs.extend(lr_codemode::carregar_plugins(raiz).iter().map(|p| p.spec()));
    }
    lr_codemode::render_signatures(&specs)
}

pub fn history_from_messages(rows: &[lr_store::MessageRow], max: usize) -> Vec<ChatMessage> {
    let usable: Vec<&lr_store::MessageRow> = rows
        .iter()
        .filter(|m| matches!(m.role.as_str(), "user" | "assistant"))
        .filter(|m| !m.content.trim().is_empty())
        .collect();
    let start = usable.len().saturating_sub(max);
    usable[start..]
        .iter()
        .map(|m| match m.role.as_str() {
            "assistant" => ChatMessage::assistant(m.content.clone()),
            _ => ChatMessage::user(m.content.clone()),
        })
        .collect()
}

/// Acrescenta o pedido atual, a menos que ele já seja a última mensagem
/// (a interface grava a mensagem da pessoa antes de chamar o agente).
pub fn append_prompt(messages: &mut Vec<ChatMessage>, prompt: &str) {
    let already = messages
        .last()
        .is_some_and(|m| m.role == "user" && m.text().trim() == prompt.trim());
    if !already && !prompt.trim().is_empty() {
        messages.push(ChatMessage::user(prompt.to_string()));
    }
}

/// Dependências que o laço recebe pronto do [`crate::AgentHost`].
pub(crate) struct RunDeps {
    pub store: Arc<Store>,
    pub registry: Arc<ToolRegistry>,
    pub config: Arc<AgentConfig>,
}

/// Monta o prompt de sistema com o plano atual dentro (quando há um).
///
/// `Send + Sync` porque o run inteiro vive numa task do tokio: o fecho
/// atravessa `await`s enquanto o laço do plano roda.
pub(crate) type SystemPrompt<'a> = dyn Fn(Option<&String>) -> String + Send + Sync + 'a;

/// Motor de passos: pensar → agir → ler o resultado, até o modelo responder
/// sem pedir ferramenta.
///
/// Serve para o run inteiro (modo agente) e para UMA etapa do plano (modo
/// laço). A diferença está só no histórico que entra: no plano ele recomeça
/// vazio a cada etapa, e é isso que mantém a janela do modelo pequena.
pub(crate) struct StepEngine<'a> {
    pub client: &'a LlamaClient,
    pub sink: Arc<EventSink>,
    pub handle: Arc<RunHandle>,
    pub store: Arc<Store>,
    pub run_id: String,
    pub opts: &'a RunOptions,
    /// Cardápio vivo do run: o recorte entregue ao modelo, que muda quando
    /// ele pede mais ou quando a etapa do laço vira.
    pub menu: Arc<menu::MenuState>,
    /// Famílias habilitadas pela pessoa (a recuragem precisa delas).
    pub groups: Vec<ToolGroup>,
    /// Ferramentas em jogo neste run.
    ///
    /// É atômico e não `bool` porque pode CAIR no meio do caminho: quando o
    /// app tenta com ferramentas sem ter conseguido ler o template e o
    /// servidor recusa, o passo é refeito sem elas e o resto do run já sabe.
    pub tools_on: AtomicBool,
    /// Quantas vezes o servidor já recusou o pedido POR CAUSA das
    /// ferramentas. Vive no engine (e não no laço) para atravessar as etapas
    /// do modo laço — é a contagem que decide o desligamento definitivo.
    pub recusas_de_tools: std::sync::atomic::AtomicU32,
    pub context: ContextBudget,
}

impl StepEngine<'_> {
    fn tools_on(&self) -> bool {
        self.tools_on.load(Ordering::Relaxed)
    }
}

/// O que um trecho de laço produziu.
pub(crate) struct StepOutcome {
    pub status: RunStatus,
    /// Resposta final em texto (vazia quando o trecho não chegou lá).
    pub text: String,
    pub escalation: Option<String>,
    pub steps: u32,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

impl StepEngine<'_> {
    /// Reavalia o cardápio contra a etapa atual do plano.
    ///
    /// A instrução da etapa é a melhor pista que existe sobre o que faz falta
    /// agora — melhor do que o objetivo geral, que já ficou para trás.
    pub(crate) fn refocus_menu(&self, goal: &str) {
        if !self.tools_on() || !self.menu.refocus(goal, &self.groups) {
            return;
        }
        self.sink.emit(RunEventKind::ToolsSelected {
            available: self.menu.available(),
            active: self.menu.active_names(),
            limit: self.menu.limit(),
            requested: false,
        });
    }

    /// Roda o laço até uma resposta final, o teto de passos ou uma parada.
    ///
    /// `max_local_steps` é o teto DESTE trecho (uma etapa do plano); o
    /// `budget` é o teto do run inteiro e continua valendo por cima.
    pub(crate) async fn drive(
        &self,
        runner: &mut ToolRunner,
        messages: &mut Vec<ChatMessage>,
        budget: &mut StepBudget,
        max_local_steps: u32,
        build_system: &SystemPrompt<'_>,
    ) -> StepOutcome {
        let mut out = StepOutcome {
            status: RunStatus::Done,
            text: String::new(),
            escalation: None,
            steps: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
        };
        // O prompt já foi montado com o plano que o runner conhece: começar
        // igual evita reconstruí-lo à toa no primeiro passo.
        let mut last_focus = runner.focus_md.clone();
        let mut local: u32 = 0;
        // Os contadores de reparo moram no runner (E5): as etapas do modo
        // laço zeram a memória de contexto, mas um modelo que só anuncia não
        // pode ganhar orçamento novo a cada etapa.

        let status = 'run: loop {
            if self.handle.is_cancelled() {
                break RunStatus::Cancelled;
            }
            if local >= max_local_steps {
                break RunStatus::MaxSteps;
            }
            let Some(index) = budget.start_step() else {
                break RunStatus::MaxSteps;
            };
            local += 1;

            // O plano entrou/mudou: o prompt de sistema acompanha.
            if runner.focus_md != last_focus {
                last_focus = runner.focus_md.clone();
                messages[0] = ChatMessage::system(build_system(last_focus.as_ref()));
            }

            let step_id = new_id("step");
            let _ = self.store.create_step(&step_id, &self.run_id, index);
            self.sink.emit(RunEventKind::StepStarted {
                step_id: step_id.clone(),
                index,
            });
            runner.counters.begin_step();

            let api_tools = self.menu.api_tools();
            let mut request = chat_request(self.opts, messages, &api_tools);
            if self.context.limit().is_some() {
                compact_if_needed(
                    self.client,
                    &self.sink,
                    &self.context,
                    self.opts,
                    &api_tools,
                    messages,
                )
                .await;
                request = chat_request(self.opts, messages, &api_tools);
            }

            // O que o modelo já falou neste passo. Cancelar no meio da fala
            // descartava o `outcome` inteiro, e com ele o texto — que some da
            // conversa mesmo tendo aparecido na tela. Aqui ele fica.
            let parcial = Arc::new(std::sync::Mutex::new(String::new()));
            let outcome = {
                let sink_delta = self.sink.clone();
                let step = step_id.clone();
                let dito = parcial.clone();
                let mut on_delta = move |delta: ChatDelta| match delta {
                    ChatDelta::Text(text) => {
                        dito.lock().unwrap().push_str(&text);
                        sink_delta.emit(RunEventKind::AssistantDelta {
                            step_id: step.clone(),
                            text,
                        });
                    }
                    ChatDelta::Reasoning(text) => {
                        sink_delta.emit(RunEventKind::ReasoningDelta {
                            step_id: step.clone(),
                            text,
                        });
                    }
                    // Fragmentos de tool call viram eventos completos depois.
                    ChatDelta::ToolCall { .. } => {}
                };
                // Três destinos para um erro de stream, e a ordem importa:
                // (1) JSON de tool call inválido → devolve ao modelo com a
                //     saída (é erro DELE, e tem conserto);
                // (2) recusa do template por causa de `tools` → refaz o passo
                //     sem elas, e só desliga de vez na segunda recusa;
                // (3) o resto — rede, 5xx genérico, silêncio no prazo — é
                //     transitório: retry do MESMO pedido, com as MESMAS
                //     ferramentas. Antes, qualquer erro caía no caminho (2), e
                //     um blip de rede rebaixava o agente a chatbot em
                //     silêncio pelo resto do run.
                //
                // O retry pertence EXCLUSIVAMENTE à chamada ao modelo, antes
                // de qualquer ferramenta rodar — repetir a chamada é seguro;
                // repetir um `git commit` não seria.
                let mut tentativa: u32 = 0;
                loop {
                    let resultado = tokio::select! {
                        biased;
                        _ = self.handle.cancelled() => {
                            out.text = parcial.lock().unwrap().clone();
                            break 'run RunStatus::Cancelled;
                        }
                        r = self.client.chat_stream(&request, &mut on_delta) => r,
                    };
                    match resultado {
                        Ok(outcome) => break outcome,
                        // O servidor leu a resposta, mas não conseguiu
                        // decodificar os argumentos da chamada.
                        Err(e)
                            if e.is_bad_tool_arguments()
                                && runner.counters.jsons_quebrados < MAX_JSON_QUEBRADO =>
                        {
                            runner.counters.jsons_quebrados += 1;
                            log::warn!(
                                "tool call com JSON inválido \
                                 ({}/{MAX_JSON_QUEBRADO}): {e}",
                                runner.counters.jsons_quebrados
                            );
                            messages.push(ChatMessage::user(aviso_json_quebrado(
                                runner.counters.jsons_quebrados,
                            )));
                            self.sink.emit(RunEventKind::RunError {
                                message: "O modelo mandou uma chamada com JSON inválido; \
                                          pedi para escrever o arquivo em pedaços."
                                    .into(),
                                retryable: true,
                            });
                            continue 'run;
                        }
                        // O template não sabe renderizar `tools`: este passo
                        // segue sem elas; o desligamento só vira definitivo na
                        // segunda recusa.
                        Err(e)
                            if e.is_tools_rejection()
                                && self.tools_on()
                                && !api_tools.is_empty() =>
                        {
                            log::warn!("o servidor recusou as ferramentas: {e}");
                            let sem_tools = chat_request(self.opts, messages, &[]);
                            let retomada = tokio::select! {
                                biased;
                                _ = self.handle.cancelled() => {
                                    out.text = parcial.lock().unwrap().clone();
                                    break 'run RunStatus::Cancelled;
                                }
                                r = self.client.chat_stream(&sem_tools, &mut on_delta) => r,
                            };
                            match retomada {
                                Ok(outcome) => {
                                    let recusas =
                                        self.recusas_de_tools.fetch_add(1, Ordering::Relaxed) + 1;
                                    if recusas >= MAX_RECUSAS_DE_TOOLS {
                                        self.tools_on.store(false, Ordering::Relaxed);
                                        self.sink.emit(RunEventKind::ToolsOff {
                                            reason: ToolsOffReason::Rejected,
                                        });
                                    }
                                    break outcome;
                                }
                                Err(e2) => {
                                    out.text = parcial.lock().unwrap().clone();
                                    self.sink.emit(RunEventKind::RunError {
                                        message: format!("O modelo não respondeu: {e2}"),
                                        retryable: true,
                                    });
                                    break 'run RunStatus::Error;
                                }
                            }
                        }
                        // Transitório: tenta de novo, com recuo. O acumulador
                        // zera para a resposta final não sair dobrada; os
                        // deltas já emitidos se corrigem sozinhos quando o
                        // `assistant.message` chega com o texto final.
                        Err(e) if tentativa < MAX_TENTATIVAS_STREAM => {
                            tentativa += 1;
                            log::warn!(
                                "stream falhou (tentativa {tentativa}/{MAX_TENTATIVAS_STREAM}): {e}"
                            );
                            parcial.lock().unwrap().clear();
                            tokio::time::sleep(std::time::Duration::from_millis(
                                400 * u64::from(tentativa),
                            ))
                            .await;
                        }
                        Err(e) => {
                            out.text = parcial.lock().unwrap().clone();
                            self.sink.emit(RunEventKind::RunError {
                                message: format!("O modelo não respondeu: {e}"),
                                retryable: true,
                            });
                            break 'run RunStatus::Error;
                        }
                    }
                }
            };

            out.steps += 1;
            let (prompt_n, predicted_n) = match &outcome.timings {
                Some(t) => (t.prompt_n, t.predicted_n),
                None => (None, None),
            };
            out.prompt_tokens += prompt_n.unwrap_or(0);
            out.completion_tokens += predicted_n.unwrap_or(0);
            let _ = self.store.finish_step(
                &step_id,
                &outcome.content,
                &outcome.reasoning,
                prompt_n,
                predicted_n,
            );
            self.sink.emit(RunEventKind::AssistantMessage {
                step_id: step_id.clone(),
                content: outcome.content.clone(),
                reasoning: outcome.reasoning.clone(),
            });

            // Sem ferramentas pedidas: é a resposta final — a não ser que o
            // modelo só tenha ANUNCIADO o que ia fazer.
            if !self.tools_on() || !outcome.wants_tools() {
                // Arquivo entregue em TEXTO (o fallback pós-JSON-quebrado):
                // o harness monta a chamada de escrita com o escape certo e
                // ela passa pela política e pelo checkpoint como qualquer
                // outra. O conteúdo veio pelo stream, que não quebra.
                // Code Mode: o programa entregue como bloco de código.
                //
                // Modelo que não fala o protocolo de tool call escreve o
                // programa numa cerca e encerra o passo — e o run terminava
                // sem ter feito nada. Aqui o bloco vira a chamada `run_code`,
                // que passa pela política como qualquer outra. É o que faz um
                // modelo sem tool call nativo trabalhar de verdade.
                if self.tools_on()
                    && runner.code_menu.is_some()
                    && let Some(programa) = codemode::bloco_de_codigo(&outcome.content)
                {
                    let tc = ToolCallReq {
                        id: new_id("call"),
                        name: codemode::RUN_CODE.into(),
                        arguments_json: serde_json::json!({ "code": programa }).to_string(),
                    };
                    let como_pedido = ChatMessage::assistant_with_tool_calls(
                        outcome.content.clone(),
                        vec![lr_engine::ToolCallMsg {
                            id: tc.id.clone(),
                            kind: "function".into(),
                            function: lr_engine::FunctionCallMsg {
                                name: tc.name.clone(),
                                arguments: tc.arguments_json.clone(),
                            },
                        }],
                    );
                    messages.push(como_pedido);
                    match runner.call(&step_id, index, &tc).await {
                        CallFlow::Result { msg, .. } => {
                            messages.push(msg);
                            continue;
                        }
                        CallFlow::Cancelled => break 'run RunStatus::Cancelled,
                        CallFlow::Escalate(reason) => {
                            out.escalation = Some(reason);
                            break 'run RunStatus::Escalated;
                        }
                    }
                }
                if self.tools_on()
                    && self.menu.active_names().iter().any(|n| n == "fs_write")
                    && let Some((caminho, conteudo)) = arquivo_em_texto(&outcome.content)
                {
                    let args = serde_json::json!({
                        "path": caminho,
                        "content": conteudo,
                    });
                    let tc = ToolCallReq {
                        id: new_id("call"),
                        name: "fs_write".into(),
                        arguments_json: args.to_string(),
                    };
                    let como_pedido = ChatMessage::assistant_with_tool_calls(
                        outcome.content.clone(),
                        vec![lr_engine::ToolCallMsg {
                            id: tc.id.clone(),
                            kind: "function".into(),
                            function: lr_engine::FunctionCallMsg {
                                name: tc.name.clone(),
                                arguments: tc.arguments_json.clone(),
                            },
                        }],
                    );
                    messages.push(como_pedido);
                    match runner.call(&step_id, index, &tc).await {
                        CallFlow::Result { msg, .. } => {
                            messages.push(msg);
                            continue;
                        }
                        CallFlow::Cancelled => break 'run RunStatus::Cancelled,
                        CallFlow::Escalate(reason) => {
                            out.escalation = Some(reason);
                            break 'run RunStatus::Escalated;
                        }
                    }
                }
                let empurrao = self
                    .tools_on()
                    .then(|| cutucada_para(&outcome.content))
                    .flatten();
                if let Some(texto) = empurrao.filter(|_| runner.counters.cutucadas < MAX_CUTUCADAS)
                {
                    runner.counters.cutucadas += 1;
                    messages.push(outcome.to_assistant_message());
                    messages.push(ChatMessage::user(texto.to_string()));
                    continue;
                }
                // Pedimos duas vezes e ele continua escrevendo a chamada em
                // vez de fazê-la. Antes de encerrar um run que não fez nada,
                // rodamos o que ele escreveu — pelo caminho de sempre.
                // O `run_code` não está no cardápio ativo (ele vive à parte,
                // com as assinaturas do run na descrição), e sem ele nesta
                // lista a chamada escrita em texto era recusada por "nome
                // desconhecido" — justamente no modo feito para o modelo que
                // não emite tool call.
                let mut permitidas = self.menu.active_names();
                if runner.code_menu.is_some() {
                    permitidas.push(codemode::RUN_CODE.to_string());
                }
                if self.tools_on()
                    && let Some(tc) = chamada_escrita(&outcome.content, &permitidas)
                {
                    let como_pedido = ChatMessage::assistant_with_tool_calls(
                        outcome.content.clone(),
                        vec![lr_engine::ToolCallMsg {
                            id: tc.id.clone(),
                            kind: "function".into(),
                            function: lr_engine::FunctionCallMsg {
                                name: tc.name.clone(),
                                arguments: tc.arguments_json.clone(),
                            },
                        }],
                    );
                    messages.push(como_pedido);
                    match runner.call(&step_id, index, &tc).await {
                        CallFlow::Result { msg, .. } => {
                            messages.push(msg);
                            continue;
                        }
                        CallFlow::Cancelled => break 'run RunStatus::Cancelled,
                        CallFlow::Escalate(reason) => {
                            out.escalation = Some(reason);
                            break 'run RunStatus::Escalated;
                        }
                    }
                }
                // Escreveu uma chamada, mas com nome que não está no cardápio
                // (um `todos_update` no lugar de `todo_update`). Encerrar aqui
                // seria desistir de um modelo que estava tentando agir.
                if self.tools_on()
                    && runner.counters.nomes_errados < MAX_NOMES_ERRADOS
                    && chamada_em_texto(&outcome.content)
                {
                    runner.counters.nomes_errados += 1;
                    messages.push(outcome.to_assistant_message());
                    messages.push(ChatMessage::user(format!(
                        "Essa ferramenta não existe. Use exatamente um destes nomes: {}.",
                        self.menu.active_names().join(", ")
                    )));
                    continue;
                }
                out.text = outcome.content.clone();
                break RunStatus::Done;
            }

            messages.push(outcome.to_assistant_message());
            let lote = &outcome.tool_calls;
            for (i, tc) in lote.iter().enumerate() {
                match runner.call(&step_id, index, tc).await {
                    CallFlow::Result { msg, .. } => messages.push(msg),
                    // Parar no MEIO do lote não pode deixar chamadas sem a
                    // mensagem `role: "tool"` — é a invariante do topo do
                    // arquivo, e é o que permite RETOMAR este histórico
                    // depois sem o template perder o pareamento.
                    CallFlow::Cancelled => {
                        fecha_lote(
                            messages,
                            &lote[i..],
                            "não executada: a execução foi cancelada",
                        );
                        break 'run RunStatus::Cancelled;
                    }
                    CallFlow::Escalate(reason) => {
                        fecha_lote(messages, &lote[i..], "não executada: a execução parou aqui");
                        out.escalation = Some(reason);
                        break 'run RunStatus::Escalated;
                    }
                }
            }

            // Uma ferramenta meta fechou a etapa (ou parou para perguntar):
            // não há mais nada a decidir neste trecho.
            if runner.take_halt() {
                out.text = outcome.content.clone();
                break RunStatus::Done;
            }

            // O passo rodou ferramentas e NADA mudou? O run não pode seguir
            // queimando passos em leitura repetida até o teto — três passos
            // assim ganham a cutucada; se nem ela mudar nada, o quinto para.
            match runner.counters.end_step() {
                Some(Stall::Nudge) => {
                    messages.push(ChatMessage::user(AVISO_ESTAGNACAO.to_string()));
                }
                Some(Stall::Escalate) => {
                    out.escalation = Some(format!(
                        "O agente rodou {} passos sem produzir mudança nenhuma \
                         (nenhum arquivo alterado, nenhum comando novo) e parou \
                         para você decidir.",
                        runner.counters.sem_progresso
                    ));
                    break RunStatus::Escalated;
                }
                None => {}
            }
        };
        out.status = status;
        out
    }
}

/// Roda a execução inteira. Só retorna quando o run termina.
pub(crate) async fn execute_run(req: StartRun, handle: Arc<RunHandle>, deps: RunDeps) {
    let StartRun {
        prompt,
        history,
        memory,
        options: opts,
        endpoint,
        work_mode,
        plan: approved_plan,
    } = req;
    let sink = handle.sink();
    let run_id = handle.id.clone();
    let workspace = opts
        .workspace_dir
        .clone()
        .filter(|d| !d.is_empty())
        .map(PathBuf::from);
    let started_at = Instant::now();

    let client = LlamaClient::new(&endpoint.base_url)
        .with_optional_api_key(endpoint.api_key.clone())
        .with_dialect(endpoint.dialect)
        .with_headers(endpoint.headers.clone())
        .with_stream_deadlines(
            Some(deps.config.first_token_timeout),
            Some(deps.config.idle_timeout),
        );

    // Capacidades do modelo DESTE run — perguntadas pelo nome dele.
    //
    // Sem o `?model=`, o roteador responde por si mesmo (`role: "router"`,
    // sem `chat_template_caps`), e o harness lia isso como "o modelo não
    // suporta ferramentas". Resultado: o modo agente rodava como chat comum
    // com qualquer modelo, sem uma linha na tela dizendo por quê.
    let props = match client.props_for(Some(&opts.model)).await {
        Ok(p) if p.describes_model() => Some(p),
        Ok(_) => {
            log::warn!(
                "/props respondeu pelo roteador, não pelo modelo {}",
                opts.model
            );
            None
        }
        Err(e) => {
            log::warn!("/props indisponível ({e}); capacidades desconhecidas");
            None
        }
    };
    let quer_ferramentas = opts.mode != RunMode::Chat && work_mode != WorkMode::Chat;
    // Agent-first: sem conseguir ler o template, o app TENTA com ferramentas.
    // Errar para o lado de tentar custa uma recusa do servidor (tratada no
    // laço, que refaz o passo sem elas); errar para o lado de desistir custa
    // o recurso inteiro, e a pessoa nunca fica sabendo.
    let caps_lidas = props.is_some();
    let tools_on = quer_ferramentas && props.as_ref().is_none_or(ServerProps::supports_tools);
    if quer_ferramentas && caps_lidas && !tools_on {
        sink.emit(RunEventKind::ToolsOff {
            reason: ToolsOffReason::Unsupported,
        });
    }
    let props = props.unwrap_or_default();

    // Cardápio: o modo de trabalho manda no que o modelo PODE usar; a
    // curadoria decide o que cabe na janela DESTE modelo.
    let specs = if tools_on {
        scout::menu_for(work_mode, deps.registry.specs().await)
    } else {
        Vec::new()
    };
    let groups = ToolGroup::enabled_from_setting(
        deps.store
            .get_setting(ToolGroup::SETTING)
            .ok()
            .flatten()
            .as_deref(),
    );
    let available = specs.len();
    let mut curated = menu::curate(specs, &prompt, &groups, props.n_ctx);
    // Família desligada é escolha da pessoa: sai do alcance do agente, não
    // fica guardada para o `tools_find` ressuscitar.
    curated.rest.retain(|s| groups.contains(&menu::group_of(s)));
    let menu = Arc::new(menu::MenuState::new(curated, available));

    // Plano do run + ferramentas meta que o conduzem (vazias fora dos modos
    // que planejam).
    // Um plano já aprovado (o laço que continua um planejamento) entra
    // pronto: `run_plan` só divide o objetivo quando não há etapas.
    let plan = shared_plan(approved_plan.unwrap_or(TaskPlan {
        goal: prompt.trim().to_string(),
        ..Default::default()
    }));
    let plan_tools = PlanTools::for_mode(work_mode, plan.clone());
    if tools_on {
        for spec in plan_tools.specs() {
            menu.pin_active(spec);
        }
    }
    // A porta de saída só faz sentido quando ficou coisa de fora.
    let finder: Option<SharedTool> = menu.is_partial().then(|| {
        let tool: SharedTool = Arc::new(menu::ToolsFind::new(menu.clone(), sink.clone()));
        menu.pin_active(tool.spec());
        tool
    });

    let workspace_str = workspace.as_ref().map(|p| p.to_string_lossy().into_owned());
    let overrides: Vec<PermissionOverride> = deps
        .store
        .list_tool_permissions(workspace_str.as_deref())
        .unwrap_or_default()
        .into_iter()
        .map(|row| PermissionOverride {
            tool_name: row.tool_name,
            policy: row.policy,
            scope: row.scope,
        })
        .collect();

    // Delegar existe para poupar a janela: a investigação que gastaria vinte
    // mil tokens de leitura volta como um resumo de dez linhas. O ajudante
    // corre com a mesma política e o mesmo handle — confirmar e cancelar
    // continuam funcionando de dentro dele.
    let helpers = Arc::new(subagent::Ledger::default());
    let delegate: Option<SharedTool> = tools_on.then(|| {
        let tool: SharedTool = Arc::new(subagent::AgentDelegate::new(subagent::SubagentDeps {
            base_url: endpoint.base_url.clone(),
            api_key: endpoint.api_key.clone(),
            headers: endpoint.headers.clone(),
            dialect: endpoint.dialect,
            registry: deps.registry.clone(),
            store: deps.store.clone(),
            config: deps.config.clone(),
            handle: handle.clone(),
            sink: sink.clone(),
            opts: opts.clone(),
            workspace: workspace.clone(),
            n_ctx: props.n_ctx,
            groups: groups.clone(),
            overrides: overrides.clone(),
            mode: opts.mode,
            written: helpers.written.clone(),
            commands: helpers.commands.clone(),
            steps: helpers.steps.clone(),
            tool_calls: helpers.tool_calls.clone(),
            memory: memory.clone(),
            user_system: opts.system_prompt.clone(),
        }));
        menu.pin_active(tool.spec());
        tool
    });

    // Code Mode: o cardápio curado continua valendo, mas muda de porta —
    // vira a biblioteca do programa, e a API passa a mostrar `run_code`.
    // Sem pasta de projeto não há programa que faça sentido (nem onde
    // escrevê-lo), e sem ferramentas também não.
    let code_mode = opts.code_mode && tools_on && workspace.is_some();
    if code_mode {
        menu.usar_programa(codemode::spec_run_code(&menu_assinaturas(
            &menu,
            workspace.as_ref(),
        )));
    }

    let tools_partial = menu.is_partial();
    let tool_names: Vec<String> = menu.active_names();

    sink.emit(RunEventKind::RunStarted {
        chat_id: opts.chat_id,
        model: opts.model.clone(),
        mode: opts.mode,
        yolo: opts.yolo(),
        workspace_dir: opts.workspace_dir.clone(),
        tools: tool_names.clone(),
    });

    if tools_on {
        sink.emit(RunEventKind::ToolsSelected {
            available: menu.available(),
            active: tool_names.clone(),
            limit: menu.limit(),
            requested: false,
        });
    }

    let mut runner = ToolRunner {
        run_id: run_id.clone(),
        mode: opts.mode,
        workspace: workspace.clone(),
        sink: sink.clone(),
        registry: deps.registry.clone(),
        store: deps.store.clone(),
        handle: handle.clone(),
        config: deps.config.clone(),
        policy: PolicyEngine::new(overrides.clone()),
        overrides,
        snapshotted: Default::default(),
        full_snapshot: false,
        reads: ReadLedger::default(),
        repeats: RepeatDetector::default(),
        errors: ErrorStreak::default(),
        written: Vec::new(),
        reescritas: Default::default(),
        commands: Vec::new(),
        focus_md: None,
        tool_calls: 0,
        local_tools: if tools_on {
            let mut locais = plan_tools.tools.clone();
            locais.extend(finder.clone());
            locais.extend(delegate.clone());
            locais
        } else {
            Vec::new()
        },
        halt: plan_tools.halt.clone(),
        counters: RunCounters::default(),
        escalonar_apos_programa: None,
        code_menu: code_mode.then(|| menu.clone()),
    };

    let menu_prompt = menu.clone();
    let build_prompt = |focus: Option<&String>| {
        // As assinaturas são lidas do cardápio a cada montagem: no modo laço
        // ele é recurado por etapa, e um prompt com a lista velha mandaria o
        // modelo chamar função que não existe mais.
        let assinaturas = code_mode.then(|| menu_assinaturas(&menu_prompt, workspace.as_ref()));
        build_system_prompt(&PromptContext {
            workspace: workspace_str.as_deref(),
            focus_md: focus.map(String::as_str),
            memory: &memory,
            tools: &tool_names,
            user_system: opts.system_prompt.as_deref(),
            mode: opts.mode,
            tools_partial,
            code_signatures: assinaturas.as_deref(),
        })
    };

    let engine = StepEngine {
        client: &client,
        sink: sink.clone(),
        handle: handle.clone(),
        store: deps.store.clone(),
        run_id: run_id.clone(),
        opts: &opts,
        menu: menu.clone(),
        groups,
        tools_on: AtomicBool::new(tools_on),
        recusas_de_tools: std::sync::atomic::AtomicU32::new(0),
        context: ContextBudget::new(props.n_ctx, deps.config.context_ratio),
    };
    let window = props.n_ctx.map(WindowBudget::new).unwrap_or_default();

    let mut usage = UsageStats::default();
    let mut escalation: Option<String> = None;
    // Frase pronta do modo laço/planejamento; nos demais o resumo sai do
    // texto final.
    let mut summary_override: Option<String> = None;
    let mut final_text;

    // O teto é o que a pessoa (ou a automação) escolheu, e ponto. Antes o
    // laço o multiplicava para caber um plano de doze entregas — uma
    // automação que pedia doze passos rodava noventa e seis. Agora quem se
    // ajusta é o PLANO: com pouco orçamento, poucas entregas.
    let max_steps = opts.max_steps;
    let max_tasks = (max_steps / scout::MAX_STEPS_PER_TASK).max(1);
    let mut budget = StepBudget::new(max_steps);

    // Vive fora do despacho porque a rodada de conserto (verificação
    // reprovada no modo agente) continua a MESMA conversa.
    let mut messages_do_agente: Vec<ChatMessage> = Vec::new();
    let status = if work_mode == WorkMode::Loop {
        // Divide o objetivo e executa entrega por entrega, cada uma com
        // contexto novo.
        let plan_run = PlanRun {
            engine: &engine,
            build_system: &build_prompt,
            plan: plan.clone(),
            workspace: workspace.clone(),
            goal: prompt.clone(),
            context: String::new(),
            window,
            max_tasks,
        };
        let outcome = scout::run_plan(&plan_run, &mut runner, &mut budget).await;
        usage.steps += outcome.steps;
        usage.prompt_tokens += outcome.prompt_tokens;
        usage.completion_tokens += outcome.completion_tokens;
        final_text = outcome.summary.clone();
        summary_override = Some(outcome.summary);
        outcome.status
    } else {
        messages_do_agente = vec![ChatMessage::system(build_prompt(None))];
        if work_mode == WorkMode::Plan {
            // Segunda mensagem de sistema: a reconstrução do prompt a cada
            // passo troca só a primeira, então esta sobrevive ao plano mudar.
            messages_do_agente.push(ChatMessage::system(scout::PLAN_MODE_BRIEF));
        }
        messages_do_agente.extend(history);
        append_prompt(&mut messages_do_agente, &prompt);

        let outcome = engine
            .drive(
                &mut runner,
                &mut messages_do_agente,
                &mut budget,
                max_steps,
                &build_prompt,
            )
            .await;
        usage.steps += outcome.steps;
        usage.prompt_tokens += outcome.prompt_tokens;
        usage.completion_tokens += outcome.completion_tokens;
        final_text = outcome.text.clone();
        escalation = outcome.escalation;

        // Planejamento entrega o plano, não o trabalho.
        if work_mode == WorkMode::Plan && outcome.status == RunStatus::Done {
            let proposal =
                present_plan(&engine, &plan, &prompt, &outcome.text, window, max_tasks).await;
            if final_text.trim().is_empty() {
                final_text = proposal;
            }
        }
        outcome.status
    };

    // O que os ajudantes fizeram é trabalho do run: entra na verificação e
    // na conta de uso, senão o arquivo que um deles escreveu não é conferido
    // e o custo real fica escondido.
    helpers.merge_into(&mut runner.written, &mut runner.commands);
    usage.steps += helpers.steps();

    // Verificação do que ficou em disco — agora em QUALQUER desfecho com
    // efeito colateral: um run que estourou o teto com arquivos escritos
    // merece o relatório MAIS que um que terminou redondo.
    let mut status = status;
    let mut verificacao = matches!(
        status,
        RunStatus::Done | RunStatus::MaxSteps | RunStatus::Escalated
    )
    .then(|| verify::verify(workspace.as_deref(), &runner.written, &runner.commands))
    .flatten();

    // Reprovou no modo agente: UMA rodada de conserto, com o relatório na
    // mesa e um teto curto próprio (o orçamento global continua valendo por
    // cima). No modo laço NÃO: `scout::failure_of` já reenfileira a etapa
    // reprovada, e empilhar os dois daria 16 passos por etapa ruim.
    if work_mode == WorkMode::Agent
        && status == RunStatus::Done
        && verificacao.as_ref().is_some_and(|r| !r.passed)
        && let Some(report) = verificacao.clone()
    {
        sink.emit(RunEventKind::Verification {
            passed: report.passed,
            notes: report.notes.clone(),
        });
        messages_do_agente.push(ChatMessage::user(format!(
            "A verificação automática reprovou o resultado: {} Corrija AGORA o que ela \
             aponta — crie o que falta ou rode de novo o comando que falhou depois de \
             consertar a causa — e só então responda.",
            report.notes
        )));
        let conserto = engine
            .drive(
                &mut runner,
                &mut messages_do_agente,
                &mut budget,
                MAX_PASSOS_DE_CONSERTO,
                &build_prompt,
            )
            .await;
        usage.steps += conserto.steps;
        usage.prompt_tokens += conserto.prompt_tokens;
        usage.completion_tokens += conserto.completion_tokens;
        if !conserto.text.trim().is_empty() {
            final_text = conserto.text.clone();
        }
        escalation = escalation.or(conserto.escalation);
        status = conserto.status;
        // O resultado da rodada é FINAL: re-verifica e segue — sem segunda
        // rodada, senão vira o mesmo laço com outro nome.
        verificacao = verify::verify(workspace.as_deref(), &runner.written, &runner.commands);
    }

    if let Some(report) = &verificacao {
        sink.emit(RunEventKind::Verification {
            passed: report.passed,
            notes: report.notes.clone(),
        });
    }

    usage.tool_calls = runner.tool_calls + helpers.tool_calls();
    usage.duration_ms = started_at.elapsed().as_millis() as u64;

    // Guarda a resposta na conversa. O laço também guarda quando para no
    // meio: o motivo da parada (uma pergunta, uma etapa travada) é o que a
    // pessoa precisa ler.
    let keep_in_chat = |text: &str| {
        if !text.trim().is_empty() && opts.chat_id > 0 {
            let _ = deps.store.add_message(
                opts.chat_id,
                "assistant",
                text,
                None,
                Some(usage.completion_tokens as i64),
                Some(usage.duration_ms as i64),
                Some(&opts.model),
                // A configuração é resolvida por quem conhece o catálogo de
                // modelos; o laço só sabe o nome.
                None,
            );
        }
    };

    let summary = match status {
        RunStatus::Done => {
            keep_in_chat(&final_text);
            summary_override.unwrap_or_else(|| head_chars(final_text.trim(), 280))
        }
        RunStatus::MaxSteps => match summary_override {
            Some(plan_summary) => {
                keep_in_chat(&plan_summary);
                plan_summary
            }
            None => {
                keep_in_chat(&final_text);
                format!(
                    "Parei no limite de {} passos sem terminar a tarefa.",
                    budget.max()
                )
            }
        },
        RunStatus::Escalated => match summary_override {
            Some(plan_summary) => {
                keep_in_chat(&plan_summary);
                plan_summary
            }
            None => {
                keep_in_chat(&final_text);
                escalation.unwrap_or_else(|| "Execução interrompida.".into())
            }
        },
        // Parar ou falhar não apaga o que o agente já tinha dito. A trilha
        // guarda os passos, mas ela não sobrevive inteira a um recarregar —
        // e sem isto o texto sumia da conversa ao voltar para ela, que é
        // exatamente a queixa de "as mensagens da IA somem".
        RunStatus::Cancelled => {
            keep_in_chat(&final_text);
            "Execução cancelada.".to_string()
        }
        _ => {
            keep_in_chat(&final_text);
            "A execução falhou.".to_string()
        }
    };

    let usage_json = serde_json::to_string(&usage).unwrap_or_else(|_| "{}".into());
    let _ = deps
        .store
        .finish_run(&run_id, status, &summary, &usage_json);

    // Episódio: o que aconteceu nesta execução, para a memória consolidar
    // depois em fatos duráveis. Só vale a pena guardar o que produziu algo —
    // cancelar ou falhar de cara não ensina nada.
    if matches!(
        status,
        RunStatus::Done | RunStatus::Escalated | RunStatus::MaxSteps
    ) && usage.tool_calls > 0
    {
        let _ = deps.store.add_memory_episode(
            workspace.as_deref().and_then(|p| p.to_str()),
            (opts.chat_id > 0).then_some(opts.chat_id),
            Some(&run_id),
            &summary,
        );
    }
    sink.emit(RunEventKind::RunFinished {
        status,
        summary,
        usage,
    });
}

/// Fecha o modo planejamento: garante um plano gravado e devolve o texto que
/// vai para a conversa.
///
/// O plano fica com `approved = false` — nada executa até a pessoa aprovar.
async fn present_plan(
    engine: &StepEngine<'_>,
    plan: &SharedPlan,
    goal: &str,
    notes: &str,
    window: WindowBudget,
    // Quantas entregas cabem no teto de passos de quem for executar.
    max_tasks: u32,
) -> String {
    // O modelo pode ter escrito o plano sozinho com `plan_create`.
    if snapshot(plan).tasks.is_empty() {
        let built = scout::decompose(
            engine.client,
            &engine.opts.model,
            goal,
            window,
            notes,
            max_tasks,
        )
        .await
        .unwrap_or_else(|e| {
            log::warn!("não consegui dividir o objetivo ({e}); proponho uma entrega só");
            scout::single_task_plan(goal, window.per_task())
        });
        *crate::plan_tools::lock_plan(plan) = built;
    }

    let saved = {
        let mut current = crate::plan_tools::lock_plan(plan);
        current.approved = false;
        if current.goal.is_empty() {
            current.goal = goal.trim().to_string();
        }
        current.clone()
    };
    if let Ok(json) = serde_json::to_string(&saved) {
        let _ = engine.store.set_run_plan(&engine.run_id, &json);
    }
    let md = saved.to_markdown();
    let _ = engine.store.set_run_focus(&engine.run_id, &md);
    engine.sink.emit(RunEventKind::FocusUpdated {
        todo_md: md.clone(),
    });
    md
}

fn chat_request(opts: &RunOptions, messages: &[ChatMessage], tools: &[Value]) -> ChatRequest {
    let mut req = ChatRequest::new(&opts.model, messages.to_vec());
    if !tools.is_empty() {
        req = req.with_tools(tools.to_vec());
    }
    req.temperature = opts.temperature;
    req.top_p = opts.top_p;
    req.top_k = opts.top_k;
    req.max_tokens = opts.max_tokens;
    // Reaproveita o KV do prefixo entre passos: é o que torna um laço de 10
    // passos viável numa máquina doméstica.
    req.with_extra("cache_prompt", json!(true))
}

/// Compacta o histórico quando ele encosta no teto da janela de contexto.
/// Estimativa local de tokens (~4 caracteres por token) — o porteiro que
/// evita perguntar ao servidor a cada passo.
fn tokens_estimados(messages: &[ChatMessage], tools: &[Value]) -> u32 {
    let texto: usize = messages.iter().map(|m| m.text().len()).sum();
    let ferramentas: usize = tools.iter().map(|t| t.to_string().len()).sum();
    ((texto + ferramentas) / 4) as u32
}

/// Fração do limite a partir da qual vale PERGUNTAR ao servidor a contagem
/// exata. Abaixo disso a estimativa local basta — e perguntar custava uma
/// retokenização do histórico INTEIRO por passo (O(n²) ao longo do run).
const PORTEIRO_DA_CONTAGEM: f32 = 0.7;

async fn compact_if_needed(
    client: &LlamaClient,
    sink: &Arc<EventSink>,
    context: &ContextBudget,
    opts: &lr_types::agent::RunOptions,
    tools: &[Value],
    messages: &mut Vec<ChatMessage>,
) {
    // Porteiro barato: só pergunta ao servidor quando a estimativa local diz
    // que estamos chegando perto do limite.
    let Some(limite) = context.limit() else {
        return;
    };
    let estimado = tokens_estimados(messages, tools);
    if (estimado as f32) < limite as f32 * PORTEIRO_DA_CONTAGEM {
        return;
    }

    let request = chat_request(opts, messages, tools);
    // Servidor sem o endpoint de contagem: a estimativa decide sozinha —
    // melhor compactar por estimativa do que estourar a janela por rigor.
    let before = client.input_tokens(&request).await.unwrap_or(estimado);
    if !context.needs_compaction(before) {
        return;
    }
    let Some(plan) = plan_compaction(messages, KEEP_TAIL_MESSAGES) else {
        log::warn!("contexto cheio, mas não há histórico antigo para resumir");
        return;
    };

    let ask = ChatRequest::new(
        &opts.model,
        vec![ChatMessage::user(compaction_request(&plan))],
    );
    // O resumo pelo modelo é o caminho bom; quando ele falha (servidor caiu,
    // resposta vazia), o plano B determinístico corta o miolo com um
    // marcador — feio, mas o passo seguinte NÃO estoura a janela. Antes, a
    // falha era silenciosa e o run seguia direto para o estouro.
    let summary = match client.complete_once(&ask).await {
        Ok(o) if !o.content.trim().is_empty() => o.content,
        outro => {
            if let Err(e) = outro {
                log::warn!("não consegui resumir o histórico: {e}");
            }
            format!(
                "[{} mensagens antigas foram removidas para caber na janela; \
                 o resumo automático falhou. O que vale é o plano acima e as \
                 últimas mensagens abaixo.]",
                plan.summarized_count()
            )
        }
    };

    // O cabeçalho já é uma mensagem de sistema, então usá-la de novo é seguro.
    *messages = apply_compaction(&plan, &summary, true);
    let after = client
        .input_tokens(&chat_request(opts, messages, tools))
        .await
        .unwrap_or_else(|_| tokens_estimados(messages, tools));
    sink.emit(RunEventKind::ContextCompacted {
        tokens_before: before,
        tokens_after: after,
    });
}

#[cfg(test)]
mod tests;
