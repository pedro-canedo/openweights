//! Guard-rails do laço: o que impede um modelo pequeno de girar em falso.
//!
//! Todos são deterministas e testáveis sem rede. A ordem de importância é:
//! 1. teto de passos — o run sempre termina;
//! 2. erros seguidos — devolve para a pessoa em vez de insistir;
//! 3. repetição — a mesma chamada três vezes é sintoma de laço;
//! 4. releitura — devolver o mesmo arquivo de novo só gasta contexto;
//! 5. orçamento de contexto — resume o histórico antes de estourar a janela.

use lr_engine::ChatMessage;
use serde_json::Value;
use std::collections::HashMap;

/// Erros seguidos até desistir e chamar a pessoa.
pub const DEFAULT_ERROR_LIMIT: u32 = 3;
/// Chamadas idênticas seguidas até tratar como laço.
pub const DEFAULT_REPEAT_LIMIT: u32 = 3;
/// Fração da janela de contexto que dispara a compactação.
/// O valor efetivo vem de `AgentConfig::context_ratio`; este é o padrão
/// documentado para quem configurar o host.
#[allow(dead_code)]
pub const DEFAULT_CONTEXT_RATIO: f32 = 0.8;

// -------------------------------------------------------------- passos ---

/// Teto de passos do run.
#[derive(Debug, Clone)]
pub struct StepBudget {
    max: u32,
    used: u32,
}

impl StepBudget {
    pub fn new(max_steps: u32) -> Self {
        Self {
            // Zero seria um run que não faz nada: trata como um passo.
            max: max_steps.max(1),
            used: 0,
        }
    }

    /// Reserva o próximo passo. `None` = teto atingido (`RunStatus::MaxSteps`).
    pub fn start_step(&mut self) -> Option<u32> {
        if self.used >= self.max {
            return None;
        }
        self.used += 1;
        Some(self.used)
    }

    /// Passos já consumidos (telemetria e testes).
    #[allow(dead_code)]
    pub fn used(&self) -> u32 {
        self.used
    }

    pub fn max(&self) -> u32 {
        self.max
    }
}

// ------------------------------------------------------- erros seguidos ---

/// Conta falhas consecutivas de ferramenta.
#[derive(Debug, Clone)]
pub struct ErrorStreak {
    limit: u32,
    count: u32,
}

impl Default for ErrorStreak {
    fn default() -> Self {
        Self::new(DEFAULT_ERROR_LIMIT)
    }
}

impl ErrorStreak {
    pub fn new(limit: u32) -> Self {
        Self {
            limit: limit.max(1),
            count: 0,
        }
    }

    /// Registra uma falha; `true` quando é hora de escalar para a pessoa.
    pub fn record_error(&mut self) -> bool {
        self.count += 1;
        self.count >= self.limit
    }

    pub fn record_success(&mut self) {
        self.count = 0;
    }

    /// Erros seguidos acumulados (telemetria e testes).
    #[allow(dead_code)]
    pub fn count(&self) -> u32 {
        self.count
    }

    /// Mensagem que acompanha o `RunStatus::Escalated`.
    pub fn escalation_message(&self) -> String {
        format!(
            "Parei depois de {} erros seguidos nas ferramentas. \
             Revise o pedido ou me diga por onde seguir.",
            self.count
        )
    }
}

// ----------------------------------------------------------- repetição ---

/// O que fazer com uma chamada que acabou de chegar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Repeat {
    /// Chamada nova (ou diferente da anterior).
    Fresh,
    /// Já repetida: avisa o modelo e deixa seguir.
    Warn { count: u32 },
    /// Insistiu depois do aviso: é laço.
    Escalate { count: u32 },
}

/// Quantas chamadas recentes o detector lembra. Seis ≈ dois passos: o
/// bastante para pegar o ciclo A→B→A→B, curto o bastante para uma repetição
/// legítima de dez passos atrás não virar alarme.
const REPEAT_WINDOW: usize = 6;

/// Detecta chamada repetida (nome + argumentos) numa JANELA recente.
///
/// A versão anterior só comparava com a chamada imediatamente anterior — o
/// ciclo A→B→A→B, que é como um modelo pequeno gira em falso de verdade,
/// passava batido. A janela pega o ciclo; e qualquer PROGRESSO real (arquivo
/// mudou, comando rodou) a esvazia, porque repetir uma leitura depois de uma
/// mudança de estado é conferência, não laço.
#[derive(Debug, Clone)]
pub struct RepeatDetector {
    limit: u32,
    window: std::collections::VecDeque<String>,
}

impl Default for RepeatDetector {
    fn default() -> Self {
        Self::new(DEFAULT_REPEAT_LIMIT)
    }
}

impl RepeatDetector {
    pub fn new(limit: u32) -> Self {
        Self {
            limit: limit.max(2),
            window: std::collections::VecDeque::new(),
        }
    }

    pub fn observe(&mut self, tool: &str, args: &Value) -> Repeat {
        let key = canonical_call(tool, args);
        let count = 1 + self.window.iter().filter(|k| **k == key).count() as u32;
        self.window.push_back(key);
        if self.window.len() > REPEAT_WINDOW {
            self.window.pop_front();
        }
        if count == 1 {
            Repeat::Fresh
        } else if count >= self.limit {
            Repeat::Escalate { count }
        } else {
            Repeat::Warn { count }
        }
    }

    /// O estado do projeto mudou: repetir uma leitura virou conferência.
    pub fn note_progress(&mut self) {
        self.window.clear();
    }

    /// Aviso injetado como resultado de ferramenta quando há repetição.
    pub fn warning_for(tool: &str) -> String {
        format!(
            "Você já chamou `{tool}` com estes mesmos argumentos. \
             Repetir não vai mudar o resultado: use o que já está no histórico, \
             mude os argumentos ou responda em texto com o que descobriu."
        )
    }

    pub fn escalation_message(tool: &str) -> String {
        format!("O agente insistiu na mesma chamada de `{tool}` e parou para você decidir.")
    }
}

/// Forma canônica de uma chamada: nome + JSON com chaves ordenadas.
///
/// Sem isso, `{"a":1,"b":2}` e `{"b":2,"a":1}` passariam por chamadas
/// diferentes e o laço escaparia do detector.
pub fn canonical_call(tool: &str, args: &Value) -> String {
    format!("{tool}({})", canonical_json(args))
}

fn canonical_json(v: &Value) -> String {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let body = keys
                .iter()
                .map(|k| format!("{}:{}", k, canonical_json(&map[*k])))
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{body}}}")
        }
        Value::Array(items) => {
            let body = items
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",");
            format!("[{body}]")
        }
        Value::String(s) => format!("{s:?}"),
        other => other.to_string(),
    }
}

// ------------------------------------------------------------- releitura ---

/// Ferramentas cuja leitura vale a pena deduplicar dentro de um run.
///
/// `fs_grep` e `git_status` ficam DE FORA de propósito: o resultado deles
/// muda legitimamente a cada edição, e bloquear a repetição só ensinaria o
/// modelo a trabalhar às cegas.
const READ_TOOLS: &[&str] = &["fs_read", "fs_list", "fs_glob"];

/// Lembra o que já foi lido no run para não devolver o mesmo conteúdo duas
/// vezes (é o maior desperdício de contexto num modelo pequeno).
#[derive(Debug, Clone, Default)]
pub struct ReadLedger {
    seen: HashMap<String, u32>,
}

impl ReadLedger {
    /// Chave de deduplicação de uma chamada de leitura, se ela for uma.
    pub fn key_for(tool: &str, args: &Value) -> Option<String> {
        if !READ_TOOLS.contains(&tool) {
            return None;
        }
        // `fs_glob` procura por padrão, não por caminho; `fs_list` sem `path`
        // lista a raiz — a chave precisa cobrir os dois.
        let path = match tool {
            "fs_glob" => args.get("pattern").and_then(Value::as_str)?,
            "fs_list" => args.get("path").and_then(Value::as_str).unwrap_or("."),
            _ => args.get("path").and_then(Value::as_str)?,
        };
        // Uma leitura parcial não substitui a anterior. O schema de `fs_read`
        // usa `offset_lines`/`max_lines`, mas modelos pequenos costumam
        // mandar `offset`/`limit` — as duas grafias contam como a mesma faixa.
        let offset = args.get("offset_lines").or_else(|| args.get("offset"));
        let limit = args.get("max_lines").or_else(|| args.get("limit"));
        let range = match (offset, limit) {
            (None, None) => String::new(),
            (a, b) => format!(
                "#{}..{}",
                a.and_then(Value::as_u64).unwrap_or(0),
                b.and_then(Value::as_u64).unwrap_or(0)
            ),
        };
        Some(format!("{tool}\x1f{}\x1f{range}", path.trim()))
    }

    /// Em que passo esta leitura já aconteceu, se aconteceu.
    pub fn seen(&self, key: &str) -> Option<u32> {
        self.seen.get(key).copied()
    }

    /// Registra a leitura; devolve o passo em que ela já havia acontecido.
    pub fn note(&mut self, key: &str, step: u32) -> Option<u32> {
        match self.seen.get(key) {
            Some(previous) => Some(*previous),
            None => {
                self.seen.insert(key.to_string(), step);
                None
            }
        }
    }

    /// Esquece as leituras de um arquivo que acabou de MUDAR.
    ///
    /// Sem isto, o ciclo mais comum de um modelo pequeno — ler, editar,
    /// **reler para conferir** — recebia "você já leu no passo N", que vira
    /// mentira no instante em que a edição acontece. O harness estava
    /// ensinando o modelo a não conferir o próprio trabalho.
    pub fn invalidate_file(&mut self, path: &str) {
        let alvo = path.trim();
        self.seen.retain(|key, _| {
            let mut partes = key.split('\x1f');
            let tool = partes.next().unwrap_or("");
            let key_path = partes.next();
            // Listagens e globs enxergam a árvore inteira: QUALQUER mudança
            // de arquivo os deixa velhos. A leitura de arquivo só envelhece
            // quando o próprio arquivo muda.
            if tool == "fs_list" || tool == "fs_glob" {
                return false;
            }
            key_path != Some(alvo)
        });
    }

    /// Resposta devolvida ao modelo no lugar do conteúdo repetido.
    pub fn duplicate_message(path: &str, step: u32) -> String {
        format!(
            "Você já leu `{path}` no passo {step} — o conteúdo está no histórico desta \
             conversa. Use-o em vez de ler de novo; se precisar de outra parte do \
             arquivo, peça um trecho diferente."
        )
    }
}

// -------------------------------------------------- orçamento de contexto ---

/// Decide quando o histórico precisa encolher.
#[derive(Debug, Clone)]
pub struct ContextBudget {
    n_ctx: Option<u32>,
    ratio: f32,
}

impl ContextBudget {
    pub fn new(n_ctx: Option<u32>, ratio: f32) -> Self {
        Self {
            n_ctx: n_ctx.filter(|n| *n > 0),
            ratio: ratio.clamp(0.3, 0.95),
        }
    }

    /// Teto de tokens de entrada antes de compactar.
    pub fn limit(&self) -> Option<u32> {
        self.n_ctx.map(|n| (n as f32 * self.ratio) as u32)
    }

    pub fn window(&self) -> Option<u32> {
        self.n_ctx
    }

    pub fn needs_compaction(&self, input_tokens: u32) -> bool {
        self.limit().is_some_and(|limit| input_tokens >= limit)
    }
}

/// Teto em caracteres de UMA mensagem, dado o `n_ctx`.
///
/// Uma entrega que come metade da janela impede a compactação (não há
/// "miolo" para resumir) e deixa o modelo local sem espaço para pensar.
/// Cortar a mensagem GRANDE cedo é o que a fração 80/90% sozinha não faz.
pub fn max_chars_per_message(n_ctx: u32) -> usize {
    ((n_ctx as f32 * 0.20) as usize).saturating_mul(4).max(2_000)
}

/// Encolhe mensagens de usuário/assistente que sozinhas estouram o teto.
/// Devolve quantos caracteres foram cortados.
pub fn shrink_oversized(messages: &mut [ChatMessage], max_chars: usize) -> usize {
    let mut saved = 0usize;
    for m in messages.iter_mut() {
        if m.role != "user" && m.role != "assistant" {
            continue;
        }
        if !m.tool_calls.is_empty() {
            continue;
        }
        let t = m.text();
        let n = t.chars().count();
        if n <= max_chars {
            continue;
        }
        let clipped = clip_keep_structure(&t, max_chars);
        saved += n.saturating_sub(clipped.chars().count());
        *m = if m.role == "user" {
            ChatMessage::user(clipped)
        } else {
            ChatMessage::assistant(clipped)
        };
    }
    saved
}

fn clip_keep_structure(text: &str, max: usize) -> String {
    let n = text.chars().count();
    if n <= max {
        return text.to_string();
    }
    let head = (max * 7) / 10;
    let tail = (max * 2) / 10;
    let start: String = text.chars().take(head).collect();
    let end: String = text
        .chars()
        .skip(n.saturating_sub(tail))
        .collect();
    format!(
        "{start}\n\n[... {omitido} caracteres omitidos para caber na janela; \
         o recorte vivo está em `.openweights/progress.md` ...]\n\n{end}",
        omitido = n.saturating_sub(head + tail)
    )
}

/// Recorte do histórico para a compactação.
#[derive(Debug, Clone, PartialEq)]
pub struct CompactionPlan {
    /// O que abre o histórico compactado: o prompt de sistema e, se ela tiver
    /// ficado para trás, a última mensagem da pessoa.
    pub head: Vec<ChatMessage>,
    /// O miolo antigo, que vira um resumo.
    pub summarize: Vec<ChatMessage>,
    /// A cauda preservada: os passos mais recentes, íntegros.
    pub tail: Vec<ChatMessage>,
}

impl CompactionPlan {
    /// Quantas mensagens o resumo substitui (para o plano B dizer o número).
    pub fn summarized_count(&self) -> usize {
        self.summarize.len()
    }
}

/// Separa o histórico em "resumir" e "preservar".
///
/// Regras que não podem ser quebradas:
/// - o prompt de sistema fica inteiro;
/// - a última mensagem da pessoa fica (é o pedido em andamento) — na cauda,
///   se for recente, ou fixada logo depois do prompt, se for antiga;
/// - a cauda nunca começa com `role: "tool"` — um resultado de ferramenta
///   solto, sem o turno do assistente que o pediu, quebra o template.
///
/// `None` quando não há o que ganhar (histórico já curto).
pub fn plan_compaction(messages: &[ChatMessage], keep_tail: usize) -> Option<CompactionPlan> {
    let head_len = messages.iter().take_while(|m| m.role == "system").count();
    let body = &messages[head_len..];
    if body.len() <= keep_tail + 1 {
        return None;
    }

    let mut start = body.len().saturating_sub(keep_tail);
    // Nunca comece a cauda num resultado de ferramenta órfão: recua até o
    // turno do assistente que fez o pedido.
    while start > 0 && body[start].role == "tool" {
        start -= 1;
    }
    if start == 0 {
        return None;
    }

    let old = &body[..start];
    let pinned = old.iter().rposition(|m| m.role == "user");
    let mut head = messages[..head_len].to_vec();
    let mut summarize = Vec::with_capacity(old.len());
    for (i, m) in old.iter().enumerate() {
        if Some(i) == pinned {
            head.push(m.clone());
        } else {
            summarize.push(m.clone());
        }
    }
    if summarize.is_empty() {
        return None;
    }

    Some(CompactionPlan {
        head,
        summarize,
        tail: body[start..].to_vec(),
    })
}

/// Texto entregue ao modelo auxiliar que escreve o resumo.
pub fn compaction_request(plan: &CompactionPlan) -> String {
    let mut body = String::new();
    for m in &plan.summarize {
        let text = m.text();
        let line = match m.role.as_str() {
            "tool" => format!(
                "[resultado de {}] {}",
                m.name.as_deref().unwrap_or("ferramenta"),
                text
            ),
            role => format!("[{role}] {text}"),
        };
        // Cada trecho entra cortado: o resumo não pode custar mais que o
        // histórico que ele substitui.
        body.push_str(&truncate_chars(&line, 600));
        body.push('\n');
    }
    format!(
        "Resuma o trecho abaixo de uma sessão de trabalho em até 10 tópicos curtos. \
         Preserve: o que foi pedido, arquivos lidos/alterados, comandos executados e \
         seus resultados, decisões tomadas e o que ficou pendente. Não invente nada.\n\n\
         {body}"
    )
}

/// Monta o histórico já compactado.
pub fn apply_compaction(
    plan: &CompactionPlan,
    summary: &str,
    system_role_ok: bool,
) -> Vec<ChatMessage> {
    let text = format!(
        "Resumo do que já aconteceu nesta execução (o histórico completo foi \
         encurtado para caber no contexto):\n{}",
        summary.trim()
    );
    let mut out = plan.head.clone();
    out.push(if system_role_ok {
        ChatMessage::system(text)
    } else {
        ChatMessage::user(text)
    });
    out.extend(plan.tail.iter().cloned());
    out
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str(" […]");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_engine::{FunctionCallMsg, ToolCallMsg};
    use serde_json::json;

    #[test]
    fn step_budget_stops_at_the_ceiling() {
        let mut b = StepBudget::new(3);
        assert_eq!(b.start_step(), Some(1));
        assert_eq!(b.start_step(), Some(2));
        assert_eq!(b.start_step(), Some(3));
        assert_eq!(b.start_step(), None, "quarto passo não pode existir");
        assert_eq!(b.used(), 3);
        // Teto zero vindo da UI ainda deixa um passo acontecer.
        assert_eq!(StepBudget::new(0).start_step(), Some(1));
    }

    #[test]
    fn three_consecutive_errors_escalate_but_a_success_resets() {
        let mut s = ErrorStreak::default();
        assert!(!s.record_error());
        assert!(!s.record_error());
        s.record_success();
        assert_eq!(s.count(), 0, "sucesso zera a sequência");
        assert!(!s.record_error());
        assert!(!s.record_error());
        assert!(s.record_error(), "o terceiro erro seguido escala");
        assert!(s.escalation_message().contains('3'));
    }

    #[test]
    fn repeated_identical_calls_warn_then_escalate() {
        let mut d = RepeatDetector::default();
        let args = json!({"path": "a.md"});
        assert_eq!(d.observe("fs_read", &args), Repeat::Fresh);
        assert_eq!(d.observe("fs_read", &args), Repeat::Warn { count: 2 });
        assert_eq!(d.observe("fs_read", &args), Repeat::Escalate { count: 3 });
        // Mudar de alvo recomeça a contagem.
        assert_eq!(
            d.observe("fs_read", &json!({"path": "b.md"})),
            Repeat::Fresh
        );
    }

    #[test]
    fn key_order_does_not_hide_a_repetition() {
        let mut d = RepeatDetector::default();
        let a = json!({"path": "a.md", "limit": 10});
        let b = json!({"limit": 10, "path": "a.md"});
        assert_eq!(d.observe("fs_read", &a), Repeat::Fresh);
        assert_eq!(d.observe("fs_read", &b), Repeat::Warn { count: 2 });
        // Argumento realmente diferente não conta como repetição.
        assert_eq!(
            d.observe("fs_read", &json!({"path": "a.md", "limit": 20})),
            Repeat::Fresh
        );
    }

    #[test]
    fn reading_the_same_file_twice_is_answered_from_the_ledger() {
        let mut led = ReadLedger::default();
        let args = json!({"path": "src/main.rs"});
        let key = ReadLedger::key_for("fs_read", &args).unwrap();
        assert_eq!(led.seen(&key), None);
        assert_eq!(led.note(&key, 2), None, "primeira leitura passa");
        assert_eq!(led.seen(&key), Some(2));
        assert_eq!(led.note(&key, 5), Some(2), "a segunda aponta o passo 2");

        // Ferramenta que não é leitura de arquivo fica fora do mecanismo.
        assert!(ReadLedger::key_for("terminal_run", &json!({"command": "ls"})).is_none());
        // Trecho diferente do mesmo arquivo é outra leitura.
        let partial =
            ReadLedger::key_for("fs_read", &json!({"path": "src/main.rs", "offset": 100})).unwrap();
        assert_ne!(key, partial);
        assert!(ReadLedger::duplicate_message("src/main.rs", 2).contains("passo 2"));
    }

    /// O ciclo A→B→A→B é como um modelo pequeno gira em falso DE VERDADE —
    /// e o detector antigo, que só comparava com a chamada anterior, deixava
    /// passar.
    #[test]
    fn an_alternating_cycle_is_caught_by_the_window() {
        let mut d = RepeatDetector::default();
        let a = json!({"path": "a.md"});
        let b = json!({"path": "b.md"});
        assert_eq!(d.observe("fs_read", &a), Repeat::Fresh);
        assert_eq!(d.observe("fs_read", &b), Repeat::Fresh);
        assert_eq!(d.observe("fs_read", &a), Repeat::Warn { count: 2 });
        assert_eq!(d.observe("fs_read", &b), Repeat::Warn { count: 2 });
        assert_eq!(d.observe("fs_read", &a), Repeat::Escalate { count: 3 });
    }

    /// Progresso real (arquivo mudou, comando rodou) esvazia a janela:
    /// repetir uma leitura depois de uma mudança é conferência, não laço.
    #[test]
    fn progress_clears_the_repeat_window() {
        let mut d = RepeatDetector::default();
        let a = json!({"path": "a.md"});
        assert_eq!(d.observe("fs_read", &a), Repeat::Fresh);
        assert_eq!(d.observe("fs_read", &a), Repeat::Warn { count: 2 });
        d.note_progress();
        assert_eq!(d.observe("fs_read", &a), Repeat::Fresh);
    }

    /// Listagens e globs também gastam janela — e envelhecem com QUALQUER
    /// escrita, porque enxergam a árvore inteira.
    #[test]
    fn listing_and_globbing_age_with_any_write() {
        let mut led = ReadLedger::default();
        let lista = ReadLedger::key_for("fs_list", &json!({})).unwrap();
        let glob = ReadLedger::key_for("fs_glob", &json!({"pattern": "**/*.rs"})).unwrap();
        let leitura = ReadLedger::key_for("fs_read", &json!({"path": "a.md"})).unwrap();
        led.note(&lista, 1);
        led.note(&glob, 2);
        led.note(&leitura, 3);

        // Uma escrita em QUALQUER arquivo derruba lista e glob; a leitura de
        // um arquivo que não mudou continua deduplicada.
        led.invalidate_file("outro/arquivo.rs");
        assert_eq!(led.seen(&lista), None);
        assert_eq!(led.seen(&glob), None);
        assert_eq!(led.seen(&leitura), Some(3));

        // `fs_grep` fica fora do mecanismo: resultado muda a cada edição.
        assert!(ReadLedger::key_for("fs_grep", &json!({"pattern": "x"})).is_none());
    }

    /// O ciclo ler → editar → reler para conferir. A releitura DEPOIS da
    /// edição tem que passar: o conteúdo do histórico ficou velho, e apontar
    /// para ele seria ensinar o modelo a não conferir o próprio trabalho.
    #[test]
    fn editing_a_file_lets_the_model_read_it_again() {
        let mut led = ReadLedger::default();
        let key = ReadLedger::key_for("fs_read", &json!({"path": "app.py"})).unwrap();
        let parcial =
            ReadLedger::key_for("fs_read", &json!({"path": "app.py", "offset": 10})).unwrap();
        let outra = ReadLedger::key_for("fs_read", &json!({"path": "outro.py"})).unwrap();
        led.note(&key, 1);
        led.note(&parcial, 2);
        led.note(&outra, 3);

        led.invalidate_file("app.py");

        assert_eq!(led.seen(&key), None, "a edição invalida a leitura inteira");
        assert_eq!(led.seen(&parcial), None, "e as leituras parciais também");
        assert_eq!(led.seen(&outra), Some(3), "arquivo que não mudou continua");
    }

    #[test]
    fn context_budget_triggers_at_eighty_percent() {
        let b = ContextBudget::new(Some(1000), DEFAULT_CONTEXT_RATIO);
        assert_eq!(b.limit(), Some(800));
        assert!(!b.needs_compaction(799));
        assert!(b.needs_compaction(800));
        // Sem `n_ctx` conhecido não há o que orçar (nunca compacta às cegas).
        assert!(!ContextBudget::new(None, 0.8).needs_compaction(999_999));
    }

    #[test]
    fn a_giant_user_message_is_clipped_before_it_eats_the_window() {
        let mut msgs = vec![
            ChatMessage::system("sys"),
            ChatMessage::user("X".repeat(20_000)),
        ];
        let saved = shrink_oversized(&mut msgs, 4_000);
        assert!(saved > 10_000);
        assert!(msgs[1].text().chars().count() < 5_000);
        assert!(msgs[1].text().contains("omitidos"));
        assert_eq!(msgs[0].text(), "sys", "o prompt de sistema fica");
    }

    fn tool_step(id: &str) -> (ChatMessage, ChatMessage) {
        let call = ToolCallMsg {
            id: id.into(),
            kind: "function".into(),
            function: FunctionCallMsg {
                name: "fs_read".into(),
                arguments: "{}".into(),
            },
        };
        (
            ChatMessage::assistant_with_tool_calls("", vec![call]),
            ChatMessage::tool_result(id, "fs_read", "conteúdo"),
        )
    }

    #[test]
    fn compaction_keeps_system_last_user_and_the_recent_tail() {
        let mut msgs = vec![
            ChatMessage::system("prompt do harness"),
            ChatMessage::user("tarefa original"),
        ];
        for i in 0..6 {
            let (a, t) = tool_step(&format!("c{i}"));
            msgs.push(a);
            msgs.push(t);
        }
        msgs.push(ChatMessage::user("agora faça outra coisa"));
        let (a, t) = tool_step("last");
        msgs.push(a);
        msgs.push(t);

        let plan = plan_compaction(&msgs, 4).expect("há o que compactar");
        assert_eq!(plan.head[0].role, "system");
        assert_eq!(
            plan.head[1].text(),
            "tarefa original",
            "o pedido antigo é fixado no topo em vez de virar resumo"
        );
        assert!(!plan.summarize.is_empty());
        assert!(plan.summarize.iter().all(|m| m.role != "user"));
        assert_ne!(
            plan.tail.first().map(|m| m.role.as_str()),
            Some("tool"),
            "a cauda não pode começar num resultado órfão"
        );
        assert!(
            plan.tail
                .iter()
                .any(|m| m.text() == "agora faça outra coisa"),
            "a última mensagem da pessoa fica"
        );

        let compacted = apply_compaction(&plan, "fez isto e aquilo", true);
        assert_eq!(compacted[0].role, "system");
        assert_eq!(compacted[1].role, "user");
        assert_eq!(compacted[2].role, "system");
        assert!(compacted[2].text().contains("fez isto e aquilo"));
        assert!(compacted.len() < msgs.len());
        assert!(compaction_request(&plan).contains("Resuma"));
    }

    #[test]
    fn compaction_is_skipped_when_the_history_is_short() {
        let msgs = vec![
            ChatMessage::system("prompt"),
            ChatMessage::user("oi"),
            ChatMessage::assistant("olá"),
        ];
        assert!(plan_compaction(&msgs, 4).is_none());
    }

    #[test]
    fn compaction_without_system_role_uses_a_user_turn() {
        let mut msgs = vec![ChatMessage::user("tarefa")];
        for i in 0..8 {
            msgs.push(ChatMessage::assistant(format!("passo {i}")));
        }
        let plan = plan_compaction(&msgs, 2).unwrap();
        let out = apply_compaction(&plan, "resumo", false);
        assert!(out.iter().all(|m| m.role != "system"));
        assert_eq!(out[0].role, "user");
    }
}
