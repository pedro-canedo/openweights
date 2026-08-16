//! Cardápio de ferramentas: entregar ao modelo só o que cabe na janela.
//!
//! O catálogo passa de trinta ferramentas. Um modelo de 8B com 8k de janela
//! gasta um pedaço grande dela só lendo descrições e — pior — erra mais a
//! escolha quanto mais opções enxerga. A saída é a de um restaurante: mostrar
//! poucos pratos e ter um garçom para o resto.
//!
//! Três peças:
//! - [`group_of`] diz a que família uma ferramenta pertence; a pessoa liga e
//!   desliga famílias inteiras nas preferências;
//! - [`curate`] monta o cardápio do run — núcleo de arquivos primeiro, depois
//!   o que o objetivo pediu, até o teto que a janela aguenta ([`limit_for`]);
//! - [`MenuState`] + [`ToolsFind`] são o garçom: o que ficou de fora continua
//!   alcançável e o modelo pede pelo nome do que precisa fazer.
//!
//! No modo laço o cardápio é reavaliado a cada etapa ([`MenuState::refocus`]):
//! a instrução da etapa atual é a melhor pista que existe sobre quais
//! ferramentas fazem falta agora. O que o modelo ativou por conta própria
//! nunca sai — tirar pelas costas dele quebra o passo seguinte.
//!
//! Tudo aqui é determinístico e testável sem rede: mesma entrada, mesmo
//! cardápio.

use crate::events::EventSink;
use async_trait::async_trait;
use lr_engine::tool_specs_to_api;
use lr_tools::{Tool, ToolContext, ToolError, ToolOutput, ToolResult, arg_str};
use lr_types::agent::{RunEventKind, ToolCategory, ToolGroup, ToolOrigin, ToolSpec};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, MutexGuard};

/// Nome da porta de saída do cardápio parcial.
pub const TOOLS_FIND: &str = "tools_find";

/// Núcleo: sem isto o agente não consegue nem começar a trabalhar.
///
/// A ordem também é a ordem de corte — numa janela minúscula, `fs_read` é a
/// última a sair. `fs_glob` ficou de fora de propósito: `fs_list` e `fs_grep`
/// cobrem quase todo caso, e a vaga vale mais para a ferramenta que o
/// objetivo pediu.
const CORE: [&str; 6] = [
    "fs_read",
    "fs_list",
    "fs_grep",
    "fs_edit",
    "fs_write",
    "terminal_run",
];

/// Quantas ferramentas uma busca traz de uma vez. Mais do que isto e o
/// remédio vira a doença: o cardápio incha de novo.
const MAX_FIND: usize = 4;

/// Teto de caracteres da descrição devolvida pelo `tools_find`.
const MAX_DESC_CHARS: usize = 160;

// ------------------------------------------------------------- famílias ---

/// Família de uma ferramenta, por nome.
///
/// Ferramenta de conector é sempre [`ToolGroup::Mcp`]: o nome vem de fora e
/// não segue convenção nenhuma.
pub fn group_of(spec: &ToolSpec) -> ToolGroup {
    if matches!(spec.origin, ToolOrigin::Mcp { .. }) {
        return ToolGroup::Mcp;
    }
    match spec.name.as_str() {
        "fs_read" | "fs_write" | "fs_edit" | "fs_list" | "fs_glob" | "fs_grep" => ToolGroup::Files,
        "terminal_run" => ToolGroup::Terminal,
        "project_info" | "build_run" | "test_run" | "lint_run" | "format_run" | "code_run" => {
            ToolGroup::Code
        }
        "git_status" | "git_diff" | "git_log" | "git_add" | "git_commit" | "git_branch"
        | "git_stash" | "git_restore" => ToolGroup::Git,
        "csv_preview" | "csv_query" | "data_summary" | "sql_query" | "sql_schema" => {
            ToolGroup::Data
        }
        "web_search" | "web_fetch" | "web_download" | "http_request" => ToolGroup::Web,
        "memory_save" => ToolGroup::Memory,
        "workspace_search" => ToolGroup::Project,
        "todo_update" | "plan_create" | "plan_update" | "task_complete" | "ask_user"
        | "agent_delegate" | TOOLS_FIND => ToolGroup::Plan,
        "clipboard_read" | "clipboard_write" | "notify_user" | "open_path" => ToolGroup::Desktop,
        other => guess_group(other, spec.category),
    }
}

/// Palpite para nome que a tabela não conhece (builtin nova, ferramenta
/// renomeada). O prefixo é a pista mais forte; sem ele, sobra a categoria.
fn guess_group(name: &str, category: ToolCategory) -> ToolGroup {
    const BY_PREFIX: [(&str, ToolGroup); 8] = [
        ("git_", ToolGroup::Git),
        ("web_", ToolGroup::Web),
        ("http_", ToolGroup::Web),
        ("fs_", ToolGroup::Files),
        ("memory_", ToolGroup::Memory),
        ("csv_", ToolGroup::Data),
        ("sql_", ToolGroup::Data),
        ("data_", ToolGroup::Data),
    ];
    for (prefix, group) in BY_PREFIX {
        if name.starts_with(prefix) {
            return group;
        }
    }
    match category {
        ToolCategory::Read | ToolCategory::Edit => ToolGroup::Files,
        ToolCategory::Execute => ToolGroup::Terminal,
        ToolCategory::Network => ToolGroup::Web,
        ToolCategory::Meta => ToolGroup::Plan,
        ToolCategory::Mcp => ToolGroup::Mcp,
    }
}

/// Desempate entre grupos: quanto menor, mais cedo entra no cardápio.
fn group_rank(group: ToolGroup) -> u8 {
    match group {
        ToolGroup::Files => 0,
        ToolGroup::Terminal => 1,
        ToolGroup::Code => 2,
        ToolGroup::Project => 3,
        ToolGroup::Git => 4,
        ToolGroup::Memory => 5,
        ToolGroup::Data => 6,
        ToolGroup::Web => 7,
        // O laço acrescenta as meta do plano por fora da curadoria; uma que
        // chegue até aqui é sobra e não pode empurrar trabalho para fora.
        ToolGroup::Plan => 8,
        // Área de transferência e afins servem ao pedido, raramente são o
        // pedido: entram quando o objetivo as chama pelo nome.
        ToolGroup::Desktop => 9,
        ToolGroup::Mcp => 10,
    }
}

// ---------------------------------------------------------------- janela ---

/// Teto de ferramentas ativas para uma janela de contexto.
///
/// Cada descrição custa de 80 a 150 tokens, então o teto segura o cardápio em
/// torno de um décimo da janela — o resto é do prompt de sistema, do histórico
/// e do trabalho de verdade.
pub fn limit_for(n_ctx: Option<u32>) -> usize {
    match n_ctx {
        // Sem `n_ctx` o servidor ainda não respondeu: um meio-termo que não
        // estoura 8k nem desperdiça 32k.
        None => 14,
        Some(n) if n <= 8_192 => 8,
        Some(n) if n <= 16_384 => 12,
        Some(n) if n <= 32_768 => 16,
        Some(n) if n <= 65_536 => 24,
        // Janela grande: o custo das descrições deixa de importar.
        Some(_) => usize::MAX,
    }
}

// ------------------------------------------------------------ pontuação ---

/// Palavras que ligam uma ferramenta ao que foi pedido, em português e em
/// inglês.
///
/// A chave é um radical já sem acento e em minúsculas: "compil" pega
/// "compilar", "compilando" e "compile". Radical que aparece por acidente
/// dentro de outra palavra fica de fora — "log" pegaria "lógica", então o que
/// vale é "git log".
const KEYWORDS: &[(&str, &[&str])] = &[
    (
        "clipboard_read",
        &[
            "area de transferencia",
            "clipboard",
            "copiei",
            "colei",
            "ctrl+v",
        ],
    ),
    (
        "clipboard_write",
        &[
            "copie",
            "copiar",
            "area de transferencia",
            "clipboard",
            "ctrl+c",
        ],
    ),
    (
        "notify_user",
        &["avise", "avisar", "notific", "me chame", "quando terminar"],
    ),
    (
        "open_path",
        &[
            "abra",
            "abrir",
            "mostre o arquivo",
            "no navegador",
            "visualiz",
        ],
    ),
    (
        "project_info",
        &[
            "que projeto",
            "stack",
            "dependencia",
            "package.json",
            "cargo.toml",
            "linguagem",
            "estrutura do projeto",
        ],
    ),
    (
        "build_run",
        &[
            "compil",
            "build",
            "erro de compilacao",
            "nao builda",
            "cargo build",
        ],
    ),
    (
        "test_run",
        &["test", "suite", "falh", "cobertura", "pytest", "quebrou"],
    ),
    (
        "lint_run",
        &[
            "lint",
            "clippy",
            "eslint",
            "warning",
            "aviso do compilador",
            "analise estatica",
        ],
    ),
    (
        "format_run",
        &["format", "fmt", "prettier", "indenta", "estilo do codigo"],
    ),
    (
        "code_run",
        &[
            "rodar um script",
            "executar o script",
            "snippet",
            "interpretador",
            "trecho de codigo",
        ],
    ),
    (
        "workspace_search",
        &[
            "onde fica",
            "onde esta",
            "encontre no projeto",
            "busca semantica",
            "em que arquivo",
            "procure no projeto",
        ],
    ),
    (
        "git_status",
        &[
            "git",
            "status",
            "alteracoes",
            "mudancas",
            "nao commitad",
            "working tree",
        ],
    ),
    (
        "git_diff",
        &[
            "diff",
            "git",
            "o que mudou",
            "alteracoes",
            "comparar versoes",
        ],
    ),
    (
        "git_log",
        &[
            "git log",
            "historico de commits",
            "ultimos commits",
            "quem alterou",
            "blame",
            "git",
        ],
    ),
    (
        "git_add",
        &[
            "git",
            "stage",
            "commit",
            "adicionar ao commit",
            "preparar o commit",
        ],
    ),
    (
        "git_commit",
        &[
            "commit",
            "commitar",
            "versionar",
            "git",
            "mensagem de commit",
        ],
    ),
    (
        "git_branch",
        &["branch", "ramo", "checkout", "git", "trocar de ramo"],
    ),
    (
        "git_stash",
        &["stash", "guardar as alteracoes", "git", "engavetar"],
    ),
    (
        "git_restore",
        &["restaurar", "descartar", "reverter", "desfazer", "git"],
    ),
    (
        "csv_preview",
        &[
            "csv",
            "planilha",
            "coluna",
            "linhas",
            "cabecalho",
            "primeiras linhas",
        ],
    ),
    (
        "csv_query",
        &["csv", "planilha", "coluna", "linhas", "filtrar", "agrupar"],
    ),
    (
        "data_summary",
        &[
            "csv",
            "planilha",
            "coluna",
            "estatistica",
            "media",
            "resumo dos dados",
            "distribuicao",
        ],
    ),
    (
        "sql_query",
        &[
            "sql",
            "banco",
            "sqlite",
            "tabela",
            "query",
            "consulta no banco",
        ],
    ),
    (
        "sql_schema",
        &["sql", "banco", "sqlite", "tabela", "schema", "esquema"],
    ),
    (
        "web_search",
        &[
            "pesquis",
            "busca na internet",
            "na internet",
            "google",
            "procure na web",
            "search the web",
            "documentacao online",
        ],
    ),
    (
        "web_fetch",
        &[
            "url",
            "site",
            "pagina web",
            "link",
            "abra o endereco",
            "conteudo da pagina",
            "http",
        ],
    ),
    (
        "web_download",
        &[
            "baixar",
            "baixe",
            "download",
            "salvar o arquivo da internet",
        ],
    ),
    (
        "http_request",
        &[
            "endpoint",
            "requisicao",
            "curl",
            "chamada http",
            "api rest",
            "webhook",
            "http",
        ],
    ),
    (
        "memory_save",
        &["lembre", "memoria", "anote", "guarde isso", "nao esqueca"],
    ),
    (
        "todo_update",
        &[
            "lista de tarefas",
            "todo list",
            "checklist",
            "organizar os passos",
        ],
    ),
    ("plan_create", &["plano", "planejar", "dividir em etapas"]),
    (
        "plan_update",
        &["plano", "atualizar o plano", "etapa concluida"],
    ),
];

/// Tira o acento de uma letra latina comum: quem escreve com pressa não
/// acentua, e "compilacao" tem que casar com "compilação".
fn deaccent(c: char) -> char {
    match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        other => other,
    }
}

/// Minúsculas, sem acento e com espaços colapsados — a forma em que as
/// chaves da tabela são escritas.
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    for c in text.chars().flat_map(char::to_lowercase).map(deaccent) {
        if c.is_whitespace() {
            pending_space = !out.is_empty();
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(c);
        }
    }
    out
}

/// Quanto uma ferramenta combina com o texto (já normalizado).
fn goal_score(name: &str, goal: &str) -> u32 {
    if let Some((_, keys)) = KEYWORDS.iter().find(|(tool, _)| *tool == name) {
        return keys.iter().filter(|k| goal.contains(**k)).count() as u32;
    }
    // Sem verbete na tabela (conector MCP, builtin nova): o próprio nome é a
    // única pista — `github__create_issue` casa com "abra uma issue".
    name.split(|c: char| !c.is_alphanumeric())
        .filter(|part| part.len() >= 4)
        .filter(|part| goal.contains(&normalize(part)))
        .count() as u32
}

// ----------------------------------------------------------- curadoria ---

/// O cardápio de um run, já dividido.
#[derive(Debug, Clone)]
pub struct Curated {
    /// O que vai para o modelo, na ordem final.
    pub active: Vec<ToolSpec>,
    /// O que ficou de fora — `tools_find` procura aqui.
    pub rest: Vec<ToolSpec>,
    pub limit: usize,
}

/// Escolhe o cardápio de um run.
///
/// `goal` é o objetivo da pessoa ou a instrução da etapa atual; `enabled` são
/// as famílias que a pessoa deixou ligadas (lista vazia = nenhuma; passe
/// [`ToolGroup::ALL`] para tudo).
///
/// A ordem de preenchimento, até o teto: núcleo, depois o que o objetivo
/// pontuou, depois o resto por família e nome. `tools_find` entra sempre e
/// **não** consome vaga — se consumisse, a curadoria estaria pagando pela
/// própria escapatória.
pub fn curate(
    specs: Vec<ToolSpec>,
    goal: &str,
    enabled: &[ToolGroup],
    n_ctx: Option<u32>,
) -> Curated {
    curate_with(specs, goal, enabled, limit_for(n_ctx), &[])
}

/// Curadoria com teto explícito e nomes fixados.
///
/// Fixado é o que o modelo pediu por `tools_find`: entra como núcleo e nunca
/// sai, mesmo que estoure o teto e mesmo que a família esteja desligada — ele
/// pediu, e sumir com a ferramenta faz o passo seguinte falhar.
fn curate_with(
    specs: Vec<ToolSpec>,
    goal: &str,
    enabled: &[ToolGroup],
    limit: usize,
    pinned: &[String],
) -> Curated {
    let goal = normalize(goal);
    let (mut pool, mut rest): (Vec<ToolSpec>, Vec<ToolSpec>) = specs
        .into_iter()
        .partition(|s| enabled.contains(&group_of(s)));

    let escape = take_named(&mut pool, TOOLS_FIND);
    let mut active: Vec<ToolSpec> = Vec::new();

    for name in CORE {
        if active.len() >= limit {
            break;
        }
        if let Some(spec) = take_named(&mut pool, name) {
            active.push(spec);
        }
    }

    for name in pinned {
        if let Some(spec) = take_named(&mut pool, name).or_else(|| take_named(&mut rest, name)) {
            active.push(spec);
        }
    }

    // Chave de ordenação: o que o objetivo pediu primeiro, depois a família
    // mais útil, depois o nome. Nenhum empate sobra ao acaso — dois runs com
    // a mesma entrada precisam gerar o mesmo cardápio.
    let mut ranked: Vec<(u32, u8, ToolSpec)> = pool
        .into_iter()
        .map(|spec| {
            (
                goal_score(&spec.name, &goal),
                group_rank(group_of(&spec)),
                spec,
            )
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.name.cmp(&b.2.name))
    });

    let slots = limit.saturating_sub(active.len()).min(ranked.len());
    let out = ranked.split_off(slots);
    active.extend(ranked.into_iter().map(|(_, _, spec)| spec));
    active.extend(escape);
    rest.extend(out.into_iter().map(|(_, _, spec)| spec));

    Curated {
        active,
        rest,
        limit,
    }
}

/// Retira do vetor a ferramenta com este nome, se existir.
fn take_named(specs: &mut Vec<ToolSpec>, name: &str) -> Option<ToolSpec> {
    specs
        .iter()
        .position(|s| s.name == name)
        .map(|i| specs.remove(i))
}

// ----------------------------------------------------------- cardápio vivo ---

struct Menu {
    active: Vec<ToolSpec>,
    rest: Vec<ToolSpec>,
    /// Nomes que o modelo ativou por `tools_find` — imunes à recuragem.
    pinned: Vec<String>,
    /// Formato da API já pronto. Refeito só quando o cardápio muda, porque é
    /// serializado a cada passo do laço.
    api: Option<Vec<Value>>,
}

/// O cardápio vivo de um run, compartilhado entre o laço e o `tools_find`.
///
/// Um cadeado só cobre o cardápio inteiro (ativas, fora e cache): não existe
/// leitura válida em que o cache esteja fora de sincronia com a lista.
pub struct MenuState {
    menu: Mutex<Menu>,
    limit: usize,
    available: usize,
}

impl MenuState {
    pub fn new(curated: Curated, available: usize) -> Self {
        Self {
            menu: Mutex::new(Menu {
                active: curated.active,
                rest: curated.rest,
                pinned: Vec::new(),
                api: None,
            }),
            limit: curated.limit,
            available,
        }
    }

    /// Pega o cardápio mesmo depois de um pânico: seguir com ele é melhor do
    /// que derrubar o run.
    fn lock(&self) -> MutexGuard<'_, Menu> {
        self.menu.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Formato da API OpenAI, pronto para o request.
    pub fn api_tools(&self) -> Vec<Value> {
        let mut menu = self.lock();
        if menu.api.is_none() {
            let api = tool_specs_to_api(&menu.active);
            menu.api = Some(api);
        }
        menu.api.clone().unwrap_or_default()
    }

    pub fn active_names(&self) -> Vec<String> {
        self.lock().active.iter().map(|s| s.name.clone()).collect()
    }

    pub fn limit(&self) -> u32 {
        u32::try_from(self.limit).unwrap_or(u32::MAX)
    }

    pub fn available(&self) -> u32 {
        u32::try_from(self.available).unwrap_or(u32::MAX)
    }

    /// Sobrou ferramenta fora do cardápio? É o que decide se vale falar de
    /// `tools_find` para o modelo.
    pub fn is_partial(&self) -> bool {
        !self.lock().rest.is_empty()
    }

    /// Acrescenta uma ferramenta que é DO RUN, não do catálogo: as meta do
    /// plano e o próprio `tools_find`.
    ///
    /// Elas entram fixadas e fora do teto. Perder `task_complete` no meio de
    /// um laço deixaria a etapa sem como terminar, e perder `tools_find`
    /// tiraria a única saída de um cardápio recortado.
    pub fn pin_active(&self, spec: ToolSpec) {
        let mut menu = self.lock();
        if menu.active.iter().any(|s| s.name == spec.name) {
            return;
        }
        menu.pinned.push(spec.name.clone());
        menu.active.push(spec);
        menu.api = None;
    }

    /// Move de `rest` para `active` o que casar; devolve os nomes movidos.
    ///
    /// Não respeita o teto de propósito: o modelo pediu de forma explícita, e
    /// `tools_find` já traz poucas de cada vez.
    pub fn activate(&self, names: &[String]) -> Vec<String> {
        let mut menu = self.lock();
        let mut moved = Vec::new();
        for wanted in names {
            let wanted = normalize(wanted);
            let Some(i) = menu.rest.iter().position(|s| normalize(&s.name) == wanted) else {
                continue;
            };
            let spec = menu.rest.remove(i);
            moved.push(spec.name.clone());
            menu.pinned.push(spec.name.clone());
            menu.active.push(spec);
        }
        if !moved.is_empty() {
            menu.api = None;
        }
        moved
    }

    /// Reavalia o cardápio contra o texto da etapa atual (modo laço).
    /// Devolve `true` quando algo mudou — o chamador emite o evento.
    pub fn refocus(&self, goal: &str, enabled: &[ToolGroup]) -> bool {
        let mut menu = self.lock();
        let before: Vec<String> = menu.active.iter().map(|s| s.name.clone()).collect();
        let mut all = std::mem::take(&mut menu.active);
        all.append(&mut menu.rest);

        let pinned = std::mem::take(&mut menu.pinned);
        let fresh = curate_with(all, goal, enabled, self.limit, &pinned);
        menu.pinned = pinned;
        let changed = fresh.active.len() != before.len()
            || fresh
                .active
                .iter()
                .zip(&before)
                .any(|(spec, old)| spec.name != *old);
        menu.active = fresh.active;
        menu.rest = fresh.rest;
        if changed {
            menu.api = None;
        }
        changed
    }

    /// Busca em `rest` por texto (nome + descrição, sem acento nem caixa).
    pub fn search(&self, query: &str, max: usize) -> Vec<ToolSpec> {
        let query = normalize(query);
        let terms = terms_of(&query);
        let menu = self.lock();
        let mut hits: Vec<(u32, u8, ToolSpec)> = menu
            .rest
            .iter()
            .filter_map(|spec| {
                // A tabela de palavras-chave vale o dobro do texto solto: ela
                // foi escrita para o jeito que a pessoa pede a tarefa.
                let score = goal_score(&spec.name, &query) * 2 + text_score(spec, &terms);
                if score == 0 {
                    None
                } else {
                    Some((score, group_rank(group_of(spec)), spec.clone()))
                }
            })
            .collect();
        hits.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.name.cmp(&b.2.name))
        });
        hits.into_iter()
            .take(max)
            .map(|(_, _, spec)| spec)
            .collect()
    }
}

/// Palavras que não distinguem ferramenta nenhuma.
const STOPWORDS: [&str; 22] = [
    "para",
    "como",
    "quero",
    "preciso",
    "quais",
    "alguma",
    "alguns",
    "tenho",
    "fazer",
    "isso",
    "essa",
    "esse",
    "esta",
    "este",
    "onde",
    "ferramenta",
    "ferramentas",
    "that",
    "need",
    "want",
    "tool",
    "tools",
];

/// Radicais de uma busca: a palavra inteira erra por flexão ("commitar" não
/// está em "git_commit"), então cinco letras bastam.
fn terms_of(query: &str) -> Vec<String> {
    let mut terms: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 3 && !STOPWORDS.contains(t))
        .map(|t| t.chars().take(5).collect())
        .collect();
    terms.sort();
    terms.dedup();
    terms
}

/// Quantos radicais aparecem no nome e na descrição (nome vale dobrado).
fn text_score(spec: &ToolSpec, terms: &[String]) -> u32 {
    let name = normalize(&spec.name);
    let description = normalize(&spec.description);
    let in_name = terms.iter().filter(|t| name.contains(t.as_str())).count() as u32;
    let in_description = terms
        .iter()
        .filter(|t| description.contains(t.as_str()))
        .count() as u32;
    in_name * 2 + in_description
}

// ---------------------------------------------------------- tools_find ---

/// A porta de saída do cardápio parcial.
pub struct ToolsFind {
    menu: Arc<MenuState>,
    sink: Arc<EventSink>,
}

impl ToolsFind {
    pub fn new(menu: Arc<MenuState>, sink: Arc<EventSink>) -> Self {
        Self { menu, sink }
    }
}

#[async_trait]
impl Tool for ToolsFind {
    fn name(&self) -> &str {
        TOOLS_FIND
    }

    fn description(&self) -> &str {
        "Procura ferramentas que não estão no seu cardápio atual e as ativa. A lista que você vê \
         é parcial: use isto quando precisar fazer algo (rodar testes, usar git, ler um CSV, \
         buscar na internet) e não encontrar a ferramenta na lista. Descreva a tarefa em poucas \
         palavras; o que for encontrado já fica disponível no próximo passo."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "O que você precisa fazer, em poucas palavras (ex.: \"rodar os testes\", \"commitar as alterações\", \"ler um csv\")."
                }
            },
            "required": ["query"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Meta
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let query = arg_str(&args, "query")?;
        let query = query.trim();
        if query.is_empty() {
            return Err(ToolError::InvalidArgs(
                "diga em `query` o que você precisa fazer".into(),
            ));
        }

        let found = self.menu.search(query, MAX_FIND);
        let names: Vec<String> = found.iter().map(|s| s.name.clone()).collect();
        let moved = self.menu.activate(&names);
        if moved.is_empty() {
            return Ok(ToolOutput::text(format!(
                "Nenhuma ferramenta nova para «{query}». As que você já tem estão na lista."
            )));
        }

        // Só o que ACABOU de entrar: a trilha mostra o pedido do modelo, não
        // o cardápio inteiro de novo.
        self.sink.emit(RunEventKind::ToolsSelected {
            available: self.menu.available(),
            active: moved.clone(),
            limit: self.menu.limit(),
            requested: true,
        });

        let mut out = String::from("Estas ferramentas passaram a existir para você:\n");
        for spec in found.iter().filter(|s| moved.contains(&s.name)) {
            let line = one_line(&spec.description, MAX_DESC_CHARS);
            out.push_str(&format!("- {}: {line}\n", spec.name));
        }
        out.push_str("Chame a que precisar no próximo passo.");
        Ok(ToolOutput::text(out))
    }
}

/// Junta o texto em uma linha e corta no teto (sem partir UTF-8).
fn one_line(text: &str, max: usize) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() <= max {
        return joined;
    }
    joined.chars().take(max).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_types::agent::{RunEvent, ToolTier};
    use std::sync::Mutex as StdMutex;

    fn category_of(name: &str) -> ToolCategory {
        match name {
            "fs_write" | "fs_edit" => ToolCategory::Edit,
            "terminal_run" | "build_run" | "test_run" | "lint_run" | "format_run" | "code_run" => {
                ToolCategory::Execute
            }
            "web_search" | "web_fetch" | "web_download" | "http_request" => ToolCategory::Network,
            "todo_update" | TOOLS_FIND => ToolCategory::Meta,
            _ => ToolCategory::Read,
        }
    }

    fn spec(name: &str, description: &str) -> ToolSpec {
        let category = category_of(name);
        ToolSpec {
            name: name.into(),
            description: description.into(),
            parameters: json!({ "type": "object", "properties": {} }),
            category,
            tier: ToolTier::Safe,
            origin: ToolOrigin::Builtin,
            read_only: matches!(category, ToolCategory::Read | ToolCategory::Meta),
        }
    }

    /// O catálogo do app, com as descrições no espírito das de verdade.
    fn catalog() -> Vec<ToolSpec> {
        [
            ("fs_read", "Lê o conteúdo de um arquivo do projeto."),
            ("fs_write", "Escreve um arquivo do projeto."),
            ("fs_edit", "Troca um trecho exato dentro de um arquivo."),
            ("fs_list", "Lista arquivos e pastas."),
            ("fs_glob", "Encontra arquivos por padrão de nome."),
            ("fs_grep", "Procura um texto dentro dos arquivos."),
            ("terminal_run", "Roda um comando no terminal do projeto."),
            (
                "project_info",
                "Diz que projeto é este e quais são as dependências.",
            ),
            ("build_run", "Compila o projeto e devolve os erros."),
            ("test_run", "Roda a suíte de testes e devolve as falhas."),
            ("lint_run", "Roda o analisador estático."),
            ("format_run", "Formata o código."),
            ("code_run", "Executa um trecho de código avulso."),
            ("git_status", "Mostra o estado do repositório git."),
            ("git_diff", "Mostra o diff das alterações."),
            ("git_log", "Mostra o histórico de commits."),
            ("git_add", "Prepara arquivos para o próximo commit."),
            (
                "git_commit",
                "Grava um commit com as alterações preparadas.",
            ),
            ("git_branch", "Cria e troca de branch."),
            ("git_stash", "Guarda as alterações de lado."),
            ("git_restore", "Desfaz alterações de um arquivo."),
            ("csv_preview", "Mostra as primeiras linhas de um CSV."),
            ("csv_query", "Filtra e agrupa linhas de um CSV."),
            ("data_summary", "Resume estatísticas de uma tabela."),
            ("sql_query", "Roda uma consulta SQL num banco SQLite."),
            ("sql_schema", "Mostra as tabelas e colunas do banco."),
            ("web_search", "Pesquisa na internet."),
            ("web_fetch", "Baixa o conteúdo de uma página."),
            ("web_download", "Baixa um arquivo da internet."),
            ("http_request", "Faz uma requisição HTTP a um endpoint."),
            ("memory_save", "Guarda um fato na memória do projeto."),
            (
                "workspace_search",
                "Busca por significado no projeto indexado.",
            ),
            ("todo_update", "Atualiza a lista de tarefas do run."),
            (TOOLS_FIND, "Procura ferramentas fora do cardápio."),
        ]
        .into_iter()
        .map(|(name, description)| spec(name, description))
        .collect()
    }

    fn names(specs: &[ToolSpec]) -> Vec<String> {
        specs.iter().map(|s| s.name.clone()).collect()
    }

    fn has(specs: &[ToolSpec], name: &str) -> bool {
        specs.iter().any(|s| s.name == name)
    }

    /// Ativas sem contar a porta de saída, que não consome vaga.
    fn budgeted(curated: &Curated) -> usize {
        curated
            .active
            .iter()
            .filter(|s| s.name != TOOLS_FIND)
            .count()
    }

    #[test]
    fn the_window_size_decides_how_many_tools_fit() {
        assert_eq!(limit_for(None), 14);
        assert_eq!(limit_for(Some(4_096)), 8);
        assert_eq!(limit_for(Some(8_192)), 8);
        assert_eq!(limit_for(Some(16_384)), 12);
        assert_eq!(limit_for(Some(32_768)), 16);
        assert_eq!(limit_for(Some(65_536)), 24);
        assert_eq!(limit_for(Some(131_072)), usize::MAX);
    }

    #[test]
    fn a_small_window_keeps_only_the_core() {
        let curated = curate(catalog(), "faça o que der", &ToolGroup::ALL, Some(8_192));

        assert_eq!(curated.limit, 8);
        assert_eq!(budgeted(&curated), 8);
        for core in CORE {
            assert!(
                has(&curated.active, core),
                "{core} tem que estar no cardápio"
            );
        }
        assert!(
            has(&curated.active, TOOLS_FIND),
            "a porta de saída entra sem gastar vaga"
        );
        assert!(has(&curated.rest, "web_search"));
    }

    #[test]
    fn a_broken_build_and_failing_tests_win_the_last_slots() {
        let curated = curate(
            catalog(),
            "descubra por que não compila, corrija e rode os testes",
            &ToolGroup::ALL,
            Some(8_192),
        );

        assert_eq!(curated.limit, 8);
        assert_eq!(budgeted(&curated), 8);
        for core in CORE {
            assert!(
                has(&curated.active, core),
                "{core} tem que estar no cardápio"
            );
        }
        assert!(has(&curated.active, "build_run"));
        assert!(has(&curated.active, "test_run"));
        assert!(
            !has(&curated.active, "fs_glob"),
            "a vaga vale mais para o que o objetivo pediu"
        );
    }

    #[test]
    fn a_testing_goal_brings_the_test_tool_in_a_tight_window() {
        let curated = curate(
            catalog(),
            "rode os testes e conserte o que falhar",
            &ToolGroup::ALL,
            Some(12_288),
        );

        assert_eq!(curated.limit, 12);
        assert!(has(&curated.active, "test_run"));
        assert!(!has(&curated.active, "web_search"));
        assert!(!has(&curated.active, "sql_query"));
        assert!(has(&curated.rest, "web_search"));
    }

    #[test]
    fn a_commit_goal_brings_the_git_tools() {
        let curated = curate(
            catalog(),
            "commite as alterações",
            &ToolGroup::ALL,
            Some(12_288),
        );

        assert!(has(&curated.active, "git_commit"));
        assert!(has(&curated.active, "git_status"));
    }

    #[test]
    fn a_disabled_group_never_reaches_the_menu() {
        let enabled = [
            ToolGroup::Terminal,
            ToolGroup::Code,
            ToolGroup::Git,
            ToolGroup::Plan,
        ];
        let curated = curate(catalog(), "leia o arquivo README", &enabled, None);

        assert!(
            !has(&curated.active, "fs_read"),
            "núcleo de família desligada continua desligado"
        );
        assert!(has(&curated.rest, "fs_read"));
        assert!(has(&curated.active, "terminal_run"));
        assert!(
            curated
                .active
                .iter()
                .all(|s| enabled.contains(&group_of(s))),
            "nada de família desligada no cardápio"
        );
    }

    #[test]
    fn curation_is_deterministic_regardless_of_the_input_order() {
        let goal = "rode os testes, conserte e commite";
        let first = curate(catalog(), goal, &ToolGroup::ALL, Some(12_288));

        let mut shuffled = catalog();
        shuffled.reverse();
        shuffled.rotate_left(7);
        let second = curate(shuffled, goal, &ToolGroup::ALL, Some(12_288));

        assert_eq!(names(&first.active), names(&second.active));
        let mut a = names(&first.rest);
        let mut b = names(&second.rest);
        a.sort();
        b.sort();
        assert_eq!(a, b);
    }

    #[test]
    fn a_large_window_keeps_every_enabled_tool() {
        let total = catalog().len();
        let curated = curate(catalog(), "faça tudo", &ToolGroup::ALL, Some(131_072));

        assert_eq!(curated.limit, usize::MAX);
        assert_eq!(curated.active.len(), total);
        assert!(curated.rest.is_empty());
    }

    #[test]
    fn a_default_window_does_not_cut_a_small_catalog() {
        let few = vec![
            spec("fs_read", "Lê um arquivo."),
            spec("fs_list", "Lista arquivos."),
            spec("git_status", "Estado do repositório."),
        ];
        let curated = curate(few, "olhe o repositório", &ToolGroup::ALL, None);

        assert_eq!(curated.active.len(), 3);
        assert!(curated.rest.is_empty());
    }

    #[test]
    fn tools_of_a_connector_always_belong_to_the_connector_group() {
        let mut mcp = spec("fs_read", "Lê um arquivo pelo conector.");
        mcp.origin = ToolOrigin::Mcp {
            server_id: "github".into(),
        };
        assert_eq!(group_of(&mcp), ToolGroup::Mcp);
    }

    #[test]
    fn an_unknown_name_falls_back_to_the_prefix_and_then_the_category() {
        assert_eq!(
            group_of(&spec("git_worktree", "Nova do git.")),
            ToolGroup::Git
        );
        assert_eq!(
            group_of(&spec("memory_load", "Lê a memória.")),
            ToolGroup::Memory
        );
        assert_eq!(group_of(&spec("csv_join", "Junta CSVs.")), ToolGroup::Data);

        let mut unknown = spec("deploy_app", "Publica o app.");
        unknown.category = ToolCategory::Execute;
        assert_eq!(group_of(&unknown), ToolGroup::Terminal);

        let mut networked = spec("ping_host", "Testa a rede.");
        networked.category = ToolCategory::Network;
        assert_eq!(group_of(&networked), ToolGroup::Web);
    }

    #[test]
    fn activate_moves_a_tool_from_the_rest_and_the_api_reflects_it() {
        let curated = curate(catalog(), "leia o projeto", &ToolGroup::ALL, Some(8_192));
        let total = curated.active.len() + curated.rest.len();
        let menu = MenuState::new(curated, total);

        assert!(!menu.active_names().iter().any(|n| n == "git_commit"));
        let before = menu.api_tools().len();

        let moved = menu.activate(&["git_commit".into(), "nao_existe".into()]);
        assert_eq!(moved, vec!["git_commit".to_string()]);

        let after = menu.api_tools();
        assert_eq!(after.len(), before + 1);
        assert!(
            after.iter().any(|t| t["function"]["name"] == "git_commit"),
            "o cache do formato da API tem que ser refeito"
        );
        assert_eq!(menu.available(), total as u32);
        assert_eq!(menu.limit(), 8);
    }

    #[test]
    fn search_ignores_what_is_already_in_the_menu() {
        let curated = curate(catalog(), "leia o projeto", &ToolGroup::ALL, Some(8_192));
        let total = curated.active.len() + curated.rest.len();
        let menu = MenuState::new(curated, total);

        let found = menu.search("preciso commitar no git", 4);
        assert!(found.iter().any(|s| s.name == "git_commit"));

        menu.activate(&["git_commit".into()]);
        let again = menu.search("preciso commitar no git", 4);
        assert!(
            !again.iter().any(|s| s.name == "git_commit"),
            "o que já está no cardápio não volta na busca"
        );
    }

    #[test]
    fn refocus_follows_the_current_step() {
        let curated = curate(catalog(), "rode os testes", &ToolGroup::ALL, Some(8_192));
        let total = curated.active.len() + curated.rest.len();
        let menu = MenuState::new(curated, total);
        assert!(menu.active_names().iter().any(|n| n == "test_run"));

        assert!(menu.refocus("leia o csv de vendas e resuma as colunas", &ToolGroup::ALL));
        let now = menu.active_names();
        assert!(now.iter().any(|n| n == "csv_preview"), "{now:?}");
        assert!(!now.iter().any(|n| n == "test_run"));
        assert!(
            !menu.refocus("leia o csv de vendas e resuma as colunas", &ToolGroup::ALL),
            "sem mudança, sem evento"
        );
    }

    #[test]
    fn refocus_keeps_what_the_model_asked_for() {
        let curated = curate(catalog(), "rode os testes", &ToolGroup::ALL, Some(8_192));
        let total = curated.active.len() + curated.rest.len();
        let menu = MenuState::new(curated, total);

        assert_eq!(menu.activate(&["git_commit".into()]), vec!["git_commit"]);
        menu.refocus("leia o csv de vendas e resuma as colunas", &ToolGroup::ALL);

        assert!(
            menu.active_names().iter().any(|n| n == "git_commit"),
            "o que o modelo ativou não sai pelas costas dele"
        );
    }

    /// As ferramentas do run (meta do plano, `tools_find`) não são parte do
    /// catálogo e não podem sumir numa recuragem — sem `task_complete` a
    /// etapa do laço não tem como terminar.
    #[test]
    fn tools_that_belong_to_the_run_never_leave_the_menu() {
        let curated = curate(catalog(), "rode os testes", &ToolGroup::ALL, Some(8_192));
        let total = curated.active.len() + curated.rest.len();
        let menu = MenuState::new(curated, total);

        menu.pin_active(spec("task_complete", "Marca a etapa atual como concluída."));
        assert!(menu.active_names().iter().any(|n| n == "task_complete"));

        // Segunda vez não duplica.
        menu.pin_active(spec("task_complete", "Marca a etapa atual como concluída."));
        assert_eq!(
            menu.active_names()
                .iter()
                .filter(|n| *n == "task_complete")
                .count(),
            1
        );

        menu.refocus("commite as alterações", &ToolGroup::ALL);
        assert!(
            menu.active_names().iter().any(|n| n == "task_complete"),
            "a meta do plano some e a etapa fica sem como terminar"
        );
    }

    #[test]
    fn a_full_menu_knows_it_is_not_partial() {
        let poucas = vec![
            spec("fs_read", "Lê um arquivo."),
            spec("fs_list", "Lista uma pasta."),
        ];
        let curated = curate(poucas, "leia o readme", &ToolGroup::ALL, None);
        let menu = MenuState::new(curated, 2);
        assert!(!menu.is_partial(), "nada ficou de fora");

        let curated = curate(catalog(), "leia o readme", &ToolGroup::ALL, Some(8_192));
        let total = curated.active.len() + curated.rest.len();
        assert!(MenuState::new(curated, total).is_partial());
    }

    fn collector() -> (crate::events::EventCallback, Arc<StdMutex<Vec<RunEvent>>>) {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let mine = seen.clone();
        let cb: crate::events::EventCallback = Arc::new(move |ev| mine.lock().unwrap().push(ev));
        (cb, seen)
    }

    fn find_tool(goal: &str) -> (ToolsFind, Arc<MenuState>, Arc<StdMutex<Vec<RunEvent>>>) {
        let curated = curate(catalog(), goal, &ToolGroup::ALL, Some(8_192));
        let total = curated.active.len() + curated.rest.len();
        let menu = Arc::new(MenuState::new(curated, total));
        let (cb, seen) = collector();
        let sink = Arc::new(EventSink::new("r1", None).with_listener(cb));
        (ToolsFind::new(menu.clone(), sink), menu, seen)
    }

    #[tokio::test]
    async fn tools_find_answers_a_question_in_portuguese() {
        let (tool, menu, seen) = find_tool("leia os arquivos do projeto");
        let ctx = ToolContext::new(None, "c1");

        let out = tool
            .execute(
                json!({ "query": "preciso commitar as mudanças no git" }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(out.content.contains("git_commit"), "{}", out.content);
        assert!(menu.active_names().iter().any(|n| n == "git_commit"));

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        match &seen[0].event {
            RunEventKind::ToolsSelected {
                requested,
                active,
                limit,
                available,
            } => {
                assert!(requested);
                assert_eq!(*limit, 8);
                assert_eq!(*available, catalog().len() as u32);
                assert!(active.contains(&"git_commit".to_string()), "{active:?}");
                assert!(
                    active.len() <= MAX_FIND,
                    "o evento traz só o que entrou agora: {active:?}"
                );
            }
            other => panic!("evento inesperado: {other:?}"),
        }
    }

    #[tokio::test]
    async fn tools_find_says_when_there_is_nothing_new() {
        let (tool, _menu, seen) = find_tool("leia os arquivos do projeto");
        let ctx = ToolContext::new(None, "c1");

        let out = tool
            .execute(json!({ "query": "zzzz qqqq wwww" }), &ctx)
            .await
            .unwrap();

        assert!(out.content.contains("Nenhuma ferramenta nova"));
        assert!(seen.lock().unwrap().is_empty(), "nada mudou, nada a emitir");
    }

    #[tokio::test]
    async fn tools_find_refuses_an_empty_question() {
        let (tool, _menu, _seen) = find_tool("leia os arquivos");
        let ctx = ToolContext::new(None, "c1");
        assert!(tool.execute(json!({ "query": "  " }), &ctx).await.is_err());
    }
}
