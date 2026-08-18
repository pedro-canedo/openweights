//! Ferramentas de arquivo: ler, listar, procurar, buscar, escrever, editar e
//! acrescentar.
//!
//! São as ferramentas que o agente mais usa, então valem duas obsessões:
//!
//! 1. **A saída é para um modelo pequeno.** Linha numerada para ele conseguir
//!    citar trecho exato, caminho relativo pronto para reusar na próxima
//!    chamada, teto em tudo para não entupir o contexto com uma listagem de
//!    `node_modules`.
//! 2. **Erro é instrução.** Quando algo falha, a mensagem diz o próximo passo
//!    ("leia o arquivo de novo e copie o trecho exato") em vez de só constatar
//!    o problema. É isso que faz o agente se corrigir sozinho em vez de
//!    repetir a mesma chamada quebrada até o guard-rail cortar o run.

use crate::{
    SharedTool, Tool, ToolContext, ToolError, ToolOutput, ToolRegistry, ToolResult, arg_bool,
    arg_str, arg_str_opt, arg_u64, diff,
};
use async_trait::async_trait;
use ignore::WalkBuilder;
use ignore::overrides::OverrideBuilder;
use lr_types::agent::{ToolCategory, ToolPreview};
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Pastas que nunca entram numa listagem ou busca: são geradas, enormes e
/// nunca é delas que o usuário está falando.
const SKIP_DIRS: &[&str] = &["node_modules", "target", "dist", ".git"];

/// Quantos bytes olhamos para decidir se um arquivo é binário.
const BINARY_SNIFF_BYTES: usize = 8 * 1024;

/// Linhas devolvidas por `fs_read` quando o modelo não pede um pedaço.
const DEFAULT_READ_LINES: u64 = 400;
const MAX_READ_LINES: u64 = 2_000;

/// Teto de entradas de uma listagem.
const MAX_LIST_ENTRIES: usize = 300;
/// Profundidade padrão de `fs_list` (o suficiente para entender a estrutura).
const DEFAULT_LIST_DEPTH: u64 = 3;
/// Nunca percorremos mais que isto, mesmo só para contar.
const MAX_WALK_ENTRIES: usize = 20_000;

/// Teto de arquivos devolvidos por `fs_glob`.
const DEFAULT_GLOB_RESULTS: u64 = 200;
const MAX_GLOB_RESULTS: u64 = 1_000;

/// Teto de achados de `fs_grep`.
const DEFAULT_GREP_HITS: u64 = 100;
const MAX_GREP_HITS: u64 = 500;
/// Arquivo maior que isso não entra na busca por conteúdo.
const MAX_GREP_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// Tamanho do trecho mostrado em cada achado.
const GREP_SNIPPET_CHARS: usize = 200;

// ------------------------------------------------------------- utilidades ---

/// Um byte zero nos primeiros KB é o sinal clássico de arquivo binário.
fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(BINARY_SNIFF_BYTES).any(|b| *b == 0)
}

/// Lê um arquivo garantindo que dá para tratar como texto.
///
/// `shown` é o caminho como o modelo escreveu — é ele que volta nas mensagens
/// de erro, para o modelo reconhecer o que pediu.
fn read_text(path: &Path, shown: &str) -> ToolResult<String> {
    if path.is_dir() {
        return Err(ToolError::InvalidArgs(format!(
            "`{shown}` é uma pasta, não um arquivo — use `fs_list` para ver o conteúdo dela"
        )));
    }
    if !path.exists() {
        return Err(ToolError::NotFound(shown.to_string()));
    }
    let bytes = std::fs::read(path)?;
    if looks_binary(&bytes) {
        return Err(ToolError::Other(format!(
            "`{shown}` é um arquivo binário e não pode ser lido como texto. \
             Se precisa de informação dele, procure a documentação ou o código que o gera."
        )));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Uma pasta que nunca vale a pena percorrer.
fn is_skipped_dir(name: &str) -> bool {
    SKIP_DIRS.contains(&name)
}

/// Percorredor padrão: respeita `.gitignore` e pula as pastas geradas.
///
/// `require_git(false)` é de propósito: a pasta aberta no app costuma ter
/// `.gitignore` sem ter `.git` (projeto recém-criado, cópia baixada em zip), e
/// ignorar o arquivo nesse caso encheria a listagem de lixo de build.
fn walker(root: &Path, max_depth: Option<usize>) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false) // `.github`, `.env.example` e afins interessam.
        .git_ignore(true)
        .git_global(false)
        .require_git(false)
        .max_depth(max_depth)
        .filter_entry(|entry| {
            // A raiz nunca é filtrada: abrir um projeto chamado `dist` não
            // pode resultar numa listagem vazia.
            entry.depth() == 0 || !is_skipped_dir(&entry.file_name().to_string_lossy())
        });
    builder
}

/// Raiz do projeto, com o erro certo quando não há pasta escolhida.
fn workspace_root(ctx: &ToolContext) -> ToolResult<PathBuf> {
    ctx.resolve(".")
}

/// O caminho obrigatório `path` resolve dentro do projeto?
fn required_path_within(args: &Value, ctx: &ToolContext) -> bool {
    match arg_str(args, "path") {
        Ok(rel) => ctx.resolve(&rel).is_ok(),
        Err(_) => false,
    }
}

/// O caminho opcional `path` (ou a raiz) resolve dentro do projeto?
fn optional_path_within(args: &Value, ctx: &ToolContext) -> bool {
    let rel = arg_str_opt(args, "path").unwrap_or_else(|| ".".to_string());
    ctx.resolve(&rel).is_ok()
}

/// Caminho relativo normalizado de `path` — é ele que vai para o checkpoint.
fn risk_path(args: &Value, ctx: &ToolContext) -> Vec<String> {
    match arg_str(args, "path") {
        Ok(rel) => vec![ctx.resolve(&rel).map(|p| ctx.relativize(&p)).unwrap_or(rel)],
        Err(_) => Vec::new(),
    }
}

/// Corta um trecho no limite de caracteres sem partir UTF-8.
fn snippet(line: &str, max_chars: usize) -> String {
    let trimmed = line.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let head: String = trimmed.chars().take(max_chars).collect();
    format!("{head}…")
}

/// Diff da gravação de `content` em `path`, para a tela de confirmação.
fn write_preview(rel: &str, path: &Path, content: &str) -> ToolPreview {
    let created = !path.exists();
    let before = std::fs::read_to_string(path).unwrap_or_default();
    ToolPreview::Diff {
        path: rel.to_string(),
        unified: diff::unified(rel, &before, content),
        created,
    }
}

// ---------------------------------------------------------------- fs_read ---

/// Lê um arquivo de texto com as linhas numeradas.
pub struct FsRead;

/// `offset_lines` é o nome do schema; `offset` entra junto porque modelos
/// pequenos trocam por hábito de outras ferramentas.
fn read_offset(args: &Value) -> u64 {
    args.get("offset_lines")
        .or_else(|| args.get("offset"))
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1)
}

fn read_limit(args: &Value) -> u64 {
    args.get("max_lines")
        .or_else(|| args.get("limit"))
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_READ_LINES)
        .clamp(1, MAX_READ_LINES)
}

#[async_trait]
impl Tool for FsRead {
    fn name(&self) -> &str {
        "fs_read"
    }

    fn description(&self) -> &str {
        "Lê um arquivo de texto do projeto. Devolve as linhas numeradas, para você citar trechos \
         exatos e depois editar com precisão. Leia o arquivo antes de editá-lo."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Caminho do arquivo, relativo à pasta do projeto (ex.: src/main.rs)."
                },
                "offset_lines": {
                    "type": "integer",
                    "description": "Primeira linha a devolver, começando em 1. Use para continuar a leitura de um arquivo grande."
                },
                "max_lines": {
                    "type": "integer",
                    "description": "Quantas linhas devolver no máximo. Padrão 400, teto 2000."
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Read
    }

    fn within_workspace(&self, args: &Value, ctx: &ToolContext) -> bool {
        required_path_within(args, ctx)
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let rel = arg_str(&args, "path")?;
        let path = ctx.resolve(&rel)?;
        let text = read_text(&path, &rel)?;

        let total = text.lines().count();
        if total == 0 {
            return Ok(
                ToolOutput::text(format!("{rel} — arquivo vazio (0 linhas)"))
                    .with_data(Value::String(String::new())),
            );
        }

        let offset = read_offset(&args) as usize;
        let limit = read_limit(&args) as usize;
        if offset > total {
            return Err(ToolError::InvalidArgs(format!(
                "`{rel}` tem {total} linhas, mas você pediu a partir da linha {offset}"
            )));
        }

        let start = offset - 1;
        let end = (start + limit).min(total);

        let mut body = format!("{rel} — linhas {offset}-{end} de {total}\n");
        for (i, line) in text.lines().enumerate().skip(start).take(limit) {
            let _ = writeln!(body, "{:>4}| {}", i + 1, line);
        }
        if end < total {
            let _ = writeln!(
                body,
                "[... faltam {} linhas; continue com offset_lines={}]",
                total - end,
                end + 1
            );
        }

        // Para um programa, o mesmo pedaço do arquivo sem cabeçalho e sem
        // numeração: numerar é para o modelo ler e apontar a linha, e é
        // exatamente o que atrapalha quem vai processar o conteúdo.
        let cru: String = text
            .lines()
            .skip(start)
            .take(limit)
            .collect::<Vec<_>>()
            .join("\n");

        Ok(ToolOutput::text(body)
            .with_data(Value::String(cru))
            .truncated_to(ctx.max_output_bytes))
    }
}

// ---------------------------------------------------------------- fs_list ---

/// Lista o conteúdo de uma pasta respeitando o `.gitignore`.
pub struct FsList;

#[async_trait]
impl Tool for FsList {
    fn name(&self) -> &str {
        "fs_list"
    }

    fn description(&self) -> &str {
        "Lista arquivos e pastas do projeto (respeita o .gitignore e pula node_modules, target, \
         dist e .git). Use para descobrir a estrutura antes de abrir arquivos. Os caminhos vêm \
         completos a partir da pasta do projeto — use como vieram, sem juntar com a pasta que \
         você pediu."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Pasta a listar, relativa à raiz do projeto. Omita para listar a raiz."
                },
                "max_depth": {
                    "type": "integer",
                    "description": "Quantos níveis de subpasta percorrer. Padrão 3, teto 10."
                }
            },
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Read
    }

    fn within_workspace(&self, args: &Value, ctx: &ToolContext) -> bool {
        optional_path_within(args, ctx)
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let rel = arg_str_opt(&args, "path").unwrap_or_else(|| ".".to_string());
        let root = ctx.resolve(&rel)?;
        if !root.exists() {
            return Err(ToolError::NotFound(rel));
        }
        if root.is_file() {
            return Err(ToolError::InvalidArgs(format!(
                "`{rel}` é um arquivo, não uma pasta — use `fs_read` para ver o conteúdo dele"
            )));
        }

        let depth = arg_u64(&args, "max_depth", DEFAULT_LIST_DEPTH).clamp(1, 10) as usize;

        let mut entries: Vec<String> = Vec::new();
        let mut total = 0usize;
        for item in walker(&root, Some(depth)).build().flatten() {
            if item.depth() == 0 {
                continue; // a própria pasta pedida
            }
            total += 1;
            if total > MAX_WALK_ENTRIES {
                break;
            }
            if entries.len() < MAX_LIST_ENTRIES {
                let is_dir = item.file_type().map(|t| t.is_dir()).unwrap_or(false);
                let shown = ctx.relativize(item.path());
                entries.push(if is_dir { format!("{shown}/") } else { shown });
            }
        }
        entries.sort();

        if entries.is_empty() {
            return Ok(ToolOutput::text(format!(
                "`{rel}` está vazia (ou tudo dentro dela está no .gitignore)."
            ))
            .with_data(Value::Array(Vec::new())));
        }

        let dados = Value::Array(entries.iter().map(|e| Value::String(e.clone())).collect());
        let mut body = format!("{} itens em `{rel}`:\n", total.min(MAX_WALK_ENTRIES));
        for entry in &entries {
            let _ = writeln!(body, "{entry}");
        }
        if total > entries.len() {
            let _ = writeln!(
                body,
                "[... mostrando {} de {total}; liste uma subpasta ou use max_depth menor]",
                entries.len()
            );
        }

        Ok(ToolOutput::text(body)
            .with_data(dados)
            .truncated_to(ctx.max_output_bytes))
    }
}

// ---------------------------------------------------------------- fs_glob ---

/// Encontra arquivos por padrão de nome.
pub struct FsGlob;

#[async_trait]
impl Tool for FsGlob {
    fn name(&self) -> &str {
        "fs_glob"
    }

    fn description(&self) -> &str {
        "Encontra arquivos por padrão de nome, com * (um trecho) e ** (qualquer subpasta). Use \
         quando souber o nome ou a extensão, mas não onde o arquivo está. Os caminhos vêm \
         completos a partir da pasta do projeto — use como vieram."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Padrão do caminho, ex.: **/*.rs, src/**/*.tsx, **/Cargo.toml. Sem barra, procura em qualquer subpasta."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Quantos arquivos devolver no máximo. Padrão 200, teto 1000."
                }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Read
    }

    fn within_workspace(&self, _args: &Value, ctx: &ToolContext) -> bool {
        // O padrão é sempre relativo à raiz: a busca não tem como sair dela.
        workspace_root(ctx).is_ok()
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let pattern = arg_str(&args, "pattern")?;
        let pattern = pattern.trim().replace('\\', "/");
        if pattern.is_empty() {
            return Err(ToolError::InvalidArgs(
                "`pattern` está vazio — mande algo como `**/*.rs`".into(),
            ));
        }
        let root = workspace_root(ctx)?;
        let max =
            arg_u64(&args, "max_results", DEFAULT_GLOB_RESULTS).clamp(1, MAX_GLOB_RESULTS) as usize;

        // Um `Override` só com padrões positivos funciona como lista branca:
        // arquivo que não casa é descartado, pasta continua sendo percorrida.
        let mut overrides = OverrideBuilder::new(&root);
        overrides.add(&pattern).map_err(|e| {
            ToolError::InvalidArgs(format!(
                "padrão `{pattern}` inválido ({e}) — use algo como `**/*.rs` ou `src/**/*.tsx`"
            ))
        })?;
        let overrides = overrides
            .build()
            .map_err(|e| ToolError::InvalidArgs(format!("padrão `{pattern}` inválido ({e})")))?;

        let mut found: Vec<String> = Vec::new();
        let mut total = 0usize;
        for item in walker(&root, None).overrides(overrides).build().flatten() {
            if item.file_type().map(|t| t.is_dir()).unwrap_or(true) {
                continue;
            }
            total += 1;
            if total > MAX_WALK_ENTRIES {
                break;
            }
            if found.len() < max {
                found.push(ctx.relativize(item.path()));
            }
        }
        found.sort();

        if found.is_empty() {
            return Ok(ToolOutput::text(format!(
                "Nenhum arquivo casou com `{pattern}`. Comece com `**/` para procurar em todas as \
                 subpastas (ex.: `**/*.rs`) ou use `fs_list` para ver a estrutura."
            ))
            .with_data(Value::Array(Vec::new())));
        }

        let dados = Value::Array(found.iter().map(|p| Value::String(p.clone())).collect());
        let mut body = format!("{total} arquivos casaram com `{pattern}`:\n");
        for path in &found {
            let _ = writeln!(body, "{path}");
        }
        if total > found.len() {
            let _ = writeln!(
                body,
                "[... mostrando {} de {total}; use um padrão mais específico]",
                found.len()
            );
        }

        Ok(ToolOutput::text(body)
            .with_data(dados)
            .truncated_to(ctx.max_output_bytes))
    }
}

// ---------------------------------------------------------------- fs_grep ---

/// Busca um texto literal dentro dos arquivos do projeto.
pub struct FsGrep;

#[async_trait]
impl Tool for FsGrep {
    fn name(&self) -> &str {
        "fs_grep"
    }

    fn description(&self) -> &str {
        "Procura um texto literal dentro dos arquivos do projeto e devolve arquivo, linha e \
         trecho. Não é expressão regular: escreva o texto exato que quer achar."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Texto exato a procurar (busca literal, sem expressão regular)."
                },
                "path": {
                    "type": "string",
                    "description": "Arquivo ou pasta onde procurar, relativo à raiz. Omita para procurar no projeto inteiro."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Quantos achados devolver no máximo. Padrão 100, teto 500."
                },
                "ignore_case": {
                    "type": "boolean",
                    "description": "true para ignorar maiúsculas e minúsculas. Padrão false."
                }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Read
    }

    fn within_workspace(&self, args: &Value, ctx: &ToolContext) -> bool {
        optional_path_within(args, ctx)
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let needle = arg_str(&args, "pattern")?;
        if needle.is_empty() {
            return Err(ToolError::InvalidArgs(
                "`pattern` está vazio — mande o texto que quer procurar".into(),
            ));
        }
        let rel = arg_str_opt(&args, "path").unwrap_or_else(|| ".".to_string());
        let root = ctx.resolve(&rel)?;
        if !root.exists() {
            return Err(ToolError::NotFound(rel));
        }
        let max = arg_u64(&args, "max_results", DEFAULT_GREP_HITS).clamp(1, MAX_GREP_HITS) as usize;
        let ignore_case = arg_bool(&args, "ignore_case", false);
        let needle_cmp = if ignore_case {
            needle.to_lowercase()
        } else {
            needle.clone()
        };

        let mut hits: Vec<String> = Vec::new();
        let mut total = 0usize;
        let mut capped = false;

        // Arquivo único não passa pelo percorredor: buscar num caminho que o
        // `.gitignore` esconde é legítimo quando a pessoa pediu por ele.
        let files: Vec<PathBuf> = if root.is_file() {
            vec![root.clone()]
        } else {
            walker(&root, None)
                .build()
                .flatten()
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .map(|e| e.path().to_path_buf())
                .take(MAX_WALK_ENTRIES)
                .collect()
        };

        'files: for file in files {
            match file.metadata() {
                Ok(meta) if meta.len() > MAX_GREP_FILE_BYTES => continue,
                Err(_) => continue,
                _ => {}
            }
            let Ok(bytes) = std::fs::read(&file) else {
                continue;
            };
            if looks_binary(&bytes) {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            let shown = ctx.relativize(&file);
            for (i, line) in text.lines().enumerate() {
                let haystack = if ignore_case {
                    line.to_lowercase()
                } else {
                    line.to_string()
                };
                if !haystack.contains(&needle_cmp) {
                    continue;
                }
                total += 1;
                if hits.len() >= max {
                    capped = true;
                    break 'files;
                }
                hits.push(format!(
                    "{shown}:{}: {}",
                    i + 1,
                    snippet(line, GREP_SNIPPET_CHARS)
                ));
            }
        }

        if hits.is_empty() {
            return Ok(ToolOutput::text(format!(
                "Nenhuma ocorrência de `{needle}` em `{rel}`. A busca é literal (sem expressão \
                 regular): confira maiúsculas e minúsculas ou tente `ignore_case: true`, ou use \
                 um trecho menor."
            ))
            .with_data(Value::Array(Vec::new())));
        }

        let mut body = if capped {
            format!("Primeiros {} achados de `{needle}`:\n", hits.len())
        } else {
            format!("{total} achados de `{needle}`:\n")
        };
        let dados = Value::Array(hits.iter().map(|h| Value::String(h.clone())).collect());
        for hit in &hits {
            let _ = writeln!(body, "{hit}");
        }
        if capped {
            let _ = writeln!(
                body,
                "[... parou em {max} achados; refine o texto ou limite a busca com `path`]"
            );
        }

        Ok(ToolOutput::text(body)
            .with_data(dados)
            .truncated_to(ctx.max_output_bytes))
    }
}

// --------------------------------------------------------------- fs_write ---

/// Cria ou substitui um arquivo inteiro.
pub struct FsWrite;

#[async_trait]
impl Tool for FsWrite {
    fn name(&self) -> &str {
        "fs_write"
    }

    fn description(&self) -> &str {
        "Cria um arquivo ou substitui todo o conteúdo de um existente (as pastas do caminho são \
         criadas). Para mudar só um trecho de um arquivo grande, prefira `fs_edit`."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Caminho do arquivo, relativo à pasta do projeto (ex.: src/notas.md)."
                },
                "content": {
                    "type": "string",
                    "description": "Conteúdo completo e final do arquivo. Substitui tudo que existia antes. Acima de ~60 linhas (~4 KB), NÃO mande tudo aqui: o JSON da chamada quebra. Escreva só o começo e continue com fs_append."
                }
            },
            "required": ["path", "content"],
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Edit
    }

    fn within_workspace(&self, args: &Value, ctx: &ToolContext) -> bool {
        required_path_within(args, ctx)
    }

    fn files_at_risk(&self, args: &Value, ctx: &ToolContext) -> Vec<String> {
        risk_path(args, ctx)
    }

    async fn preview(&self, args: &Value, ctx: &ToolContext) -> Option<ToolPreview> {
        let rel = arg_str(args, "path").ok()?;
        let content = arg_str(args, "content").ok()?;
        let path = ctx.resolve(&rel).ok()?;
        Some(write_preview(&rel, &path, &content))
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let rel = arg_str(&args, "path")?;
        let content = arg_str(&args, "content")?;
        let path = ctx.resolve(&rel)?;

        if path.is_dir() {
            return Err(ToolError::InvalidArgs(format!(
                "`{rel}` é uma pasta — escolha um caminho de arquivo"
            )));
        }

        let existed = path.exists();
        if existed && std::fs::read_to_string(&path).ok().as_deref() == Some(content.as_str()) {
            return Ok(ToolOutput::text(format!(
                "`{rel}` já estava exatamente com esse conteúdo; nada foi alterado."
            )));
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &content)?;

        let lines = content.lines().count();
        let bytes = content.len();
        let verb = if existed { "substituído" } else { "criado" };
        Ok(ToolOutput::text(format!(
            "Arquivo {verb}: `{rel}` ({lines} linhas, {bytes} bytes)."
        ))
        .with_changed(vec![ctx.relativize(&path)]))
    }
}

// ---------------------------------------------------------------- fs_edit ---

/// Substitui um trecho exato dentro de um arquivo.
pub struct FsEdit;

/// Aplica a substituição, com as mensagens que ensinam o modelo a acertar.
fn apply_edit(
    rel: &str,
    before: &str,
    old_text: &str,
    new_text: &str,
    replace_all: bool,
) -> ToolResult<(String, usize)> {
    if old_text.is_empty() {
        return Err(ToolError::InvalidArgs(
            "`old_text` está vazio — copie o trecho exato que quer trocar".into(),
        ));
    }
    if old_text == new_text {
        return Err(ToolError::Other(format!(
            "`old_text` e `new_text` são iguais, então não há nada a mudar em `{rel}`."
        )));
    }

    let count = before.matches(old_text).count();
    if count == 0 {
        return Err(ToolError::Other(format!(
            "O texto de `old_text` não foi encontrado em `{rel}`; leia o arquivo de novo com \
             `fs_read` e copie o trecho exato, com os mesmos espaços e quebras de linha."
        )));
    }
    if count > 1 && !replace_all {
        return Err(ToolError::Other(format!(
            "O texto de `old_text` aparece {count} vezes em `{rel}`; inclua mais contexto ao redor \
             para deixar o trecho único, ou passe `replace_all: true` para trocar todas."
        )));
    }

    let after = if replace_all {
        before.replace(old_text, new_text)
    } else {
        before.replacen(old_text, new_text, 1)
    };
    let applied = if replace_all { count } else { 1 };
    Ok((after, applied))
}

#[async_trait]
impl Tool for FsEdit {
    fn name(&self) -> &str {
        "fs_edit"
    }

    fn description(&self) -> &str {
        "Troca um trecho exato de um arquivo por outro. O `old_text` precisa aparecer igualzinho \
         no arquivo (leia com `fs_read` antes e copie o trecho)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Caminho do arquivo, relativo à pasta do projeto."
                },
                "old_text": {
                    "type": "string",
                    "description": "Trecho atual do arquivo, copiado exatamente (mesmos espaços e quebras de linha). Inclua contexto suficiente para ele ser único."
                },
                "new_text": {
                    "type": "string",
                    "description": "Texto que entra no lugar. Use string vazia para apagar o trecho."
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "true troca todas as ocorrências. Padrão false, que exige que o trecho seja único."
                }
            },
            "required": ["path", "old_text", "new_text"],
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Edit
    }

    fn within_workspace(&self, args: &Value, ctx: &ToolContext) -> bool {
        required_path_within(args, ctx)
    }

    fn files_at_risk(&self, args: &Value, ctx: &ToolContext) -> Vec<String> {
        risk_path(args, ctx)
    }

    async fn preview(&self, args: &Value, ctx: &ToolContext) -> Option<ToolPreview> {
        let rel = arg_str(args, "path").ok()?;
        let old_text = arg_str(args, "old_text").ok()?;
        let new_text = args
            .get("new_text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let replace_all = arg_bool(args, "replace_all", false);
        let path = ctx.resolve(&rel).ok()?;
        let before = read_text(&path, &rel).ok()?;

        // A prévia é honesta: se a edição não se aplica, a tela diz isso em
        // vez de mostrar um diff vazio que ninguém entende.
        match apply_edit(&rel, &before, &old_text, new_text, replace_all) {
            Ok((after, _)) => Some(ToolPreview::Diff {
                path: rel.clone(),
                unified: diff::unified(&rel, &before, &after),
                created: false,
            }),
            Err(e) => Some(ToolPreview::Text {
                body: e.to_model_message(),
            }),
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let rel = arg_str(&args, "path")?;
        let old_text = arg_str(&args, "old_text")?;
        let new_text = args
            .get("new_text")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                ToolError::InvalidArgs("faltou `new_text` (use \"\" para apagar o trecho)".into())
            })?;
        let replace_all = arg_bool(&args, "replace_all", false);

        let path = ctx.resolve(&rel)?;
        let before = read_text(&path, &rel)?;
        let (after, applied) = apply_edit(&rel, &before, &old_text, &new_text, replace_all)?;

        std::fs::write(&path, &after)?;

        let plural = if applied == 1 {
            "1 ocorrência trocada".to_string()
        } else {
            format!("{applied} ocorrências trocadas")
        };
        Ok(
            ToolOutput::text(format!("Arquivo editado: `{rel}` ({plural})."))
                .with_changed(vec![ctx.relativize(&path)]),
        )
    }
}

// -------------------------------------------------------------- fs_append ---

/// Acrescenta conteúdo ao fim de um arquivo — a saída para arquivo grande.
///
/// Nasceu de um run real: um modelo local (Qwen3.8-27B) tentou escrever uma
/// landing page inteira (7 KB de HTML) dentro dos argumentos de UMA chamada
/// `fs_write`, errou o escape no meio da string e o llama.cpp recusou o passo
/// com HTTP 500 ("Failed to parse tool call arguments as JSON … missing
/// closing quote"). A saída conhecida é escrever em pedaços — mas a
/// ferramenta certa para isso não existia: `fs_edit` obriga a repetir
/// `old_text`, o que DOBRA o tamanho do JSON exatamente onde o modelo pequeno
/// erra o escape. Daí o `fs_append`: base curta com `fs_write`, resto em
/// pedaços pequenos, um por chamada.
pub struct FsAppend;

/// Contagem de linhas com plural correto — "1 linha" ou "N linhas".
fn lines_label(n: usize) -> String {
    if n == 1 {
        "1 linha".to_string()
    } else {
        format!("{n} linhas")
    }
}

#[async_trait]
impl Tool for FsAppend {
    fn name(&self) -> &str {
        "fs_append"
    }

    fn description(&self) -> &str {
        "Acrescenta texto ao FIM de um arquivo (cria o arquivo e as pastas do caminho se não \
         existirem). É a ferramenta para arquivo grande: escreva a base curta com `fs_write` e \
         acrescente o resto em pedaços de até ~60 linhas (~4 KB) por chamada — pedaço grande \
         demais quebra o JSON da chamada. Se o arquivo não termina em quebra de linha e o texto \
         novo não começa com uma, uma quebra é inserida entre os dois."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Caminho do arquivo, relativo à pasta do projeto (ex.: site/index.html)."
                },
                "content": {
                    "type": "string",
                    "description": "Texto a acrescentar ao fim do arquivo. Mande pedaços de até ~60 linhas (~4 KB) por chamada e chame de novo para o pedaço seguinte; não repita o que já está no arquivo. Se o arquivo não terminar em quebra de linha e este texto não começar com uma, uma quebra de linha é inserida entre os dois."
                }
            },
            "required": ["path", "content"],
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Edit
    }

    fn within_workspace(&self, args: &Value, ctx: &ToolContext) -> bool {
        required_path_within(args, ctx)
    }

    fn files_at_risk(&self, args: &Value, ctx: &ToolContext) -> Vec<String> {
        risk_path(args, ctx)
    }

    async fn preview(&self, args: &Value, ctx: &ToolContext) -> Option<ToolPreview> {
        let rel = arg_str(args, "path").ok()?;
        let content = arg_str(args, "content").ok()?;
        let path = ctx.resolve(&rel).ok()?;
        let before = std::fs::read_to_string(&path).unwrap_or_default();
        let seam = !before.is_empty() && !before.ends_with('\n') && !content.starts_with('\n');
        let after = if seam {
            format!("{before}\n{content}")
        } else {
            format!("{before}{content}")
        };
        Some(write_preview(&rel, &path, &after))
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let rel = arg_str(&args, "path")?;
        let content = arg_str(&args, "content")?;
        if content.is_empty() {
            return Err(ToolError::InvalidArgs(
                "`content` está vazio — mande o trecho a acrescentar".into(),
            ));
        }
        let path = ctx.resolve(&rel)?;

        if path.is_dir() {
            return Err(ToolError::InvalidArgs(format!(
                "`{rel}` é uma pasta — escolha um caminho de arquivo"
            )));
        }

        // O que já existe é lido só para inspeção (binário? termina em `\n`?
        // quantas linhas?); a gravação é um append de verdade, para nunca
        // reescrever bytes que não são nossos.
        let before = if path.exists() {
            let bytes = std::fs::read(&path)?;
            if looks_binary(&bytes) {
                return Err(ToolError::Other(format!(
                    "`{rel}` é um arquivo binário e não pode receber texto. \
                     Escolha um arquivo de texto ou crie um caminho novo."
                )));
            }
            bytes
        } else {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            Vec::new()
        };

        // Costura de linha: modelo pequeno esquece a quebra e o resultado
        // seriam duas linhas coladas. Só entra quando falta dos dois lados.
        let seam =
            !before.is_empty() && before.last() != Some(&b'\n') && !content.starts_with('\n');
        let piece = if seam {
            format!("\n{content}")
        } else {
            content.clone()
        };

        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        file.write_all(piece.as_bytes())?;

        let added = content.lines().count();
        let total_text = format!("{}{piece}", String::from_utf8_lossy(&before));
        let total_lines = total_text.lines().count();
        let total_bytes = before.len() + piece.len();
        Ok(ToolOutput::text(format!(
            "Acrescentei {} a `{rel}` (agora {}, {total_bytes} bytes).",
            lines_label(added),
            lines_label(total_lines),
        ))
        .with_changed(vec![ctx.relativize(&path)]))
    }
}

// -------------------------------------------------------------- catálogo ---

/// Registro com as nove ferramentas nativas do agente.
///
/// É o cardápio padrão do app: arquivos (ler, listar, procurar, buscar,
/// escrever, editar, acrescentar), terminal e plano. Conectores MCP entram
/// depois, por [`ToolRegistry::add_provider`].
pub fn builtin_registry() -> ToolRegistry {
    let tools: Vec<SharedTool> = vec![
        Arc::new(FsRead),
        Arc::new(FsList),
        Arc::new(FsGlob),
        Arc::new(FsGrep),
        Arc::new(FsWrite),
        Arc::new(FsEdit),
        Arc::new(FsAppend),
        Arc::new(crate::terminal::TerminalRun),
        Arc::new(crate::todo::TodoUpdate),
    ];
    let mut registry = ToolRegistry::new();
    for tool in tools {
        registry.register(tool);
    }
    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Projeto de mentira com alguns arquivos, para os testes.
    fn project() -> (TempDir, ToolContext) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/pacote")).unwrap();
        std::fs::write(root.join("README.md"), "# Projeto\n\nlinha dois\n").unwrap();
        std::fs::write(
            root.join("src/main.rs"),
            "fn main() {\n    println!(\"oi\");\n}\n",
        )
        .unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn soma() {}\n").unwrap();
        std::fs::write(
            root.join("node_modules/pacote/index.js"),
            "module.exports\n",
        )
        .unwrap();
        let ctx = ToolContext::new(Some(root.to_path_buf()), "call-1");
        (dir, ctx)
    }

    async fn run(tool: &dyn Tool, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        tool.execute(args, ctx).await
    }

    // ------------------------------------------------------------- fs_read ---

    #[tokio::test]
    async fn read_numbers_the_lines() {
        let (_d, ctx) = project();
        let out = run(&FsRead, json!({"path": "README.md"}), &ctx)
            .await
            .unwrap();
        assert!(out.content.contains("   1| # Projeto"), "{}", out.content);
        assert!(out.content.contains("   3| linha dois"), "{}", out.content);
        assert!(out.content.contains("de 3"), "cabeçalho: {}", out.content);
    }

    #[tokio::test]
    async fn read_refuses_paths_outside_the_project() {
        let (_d, ctx) = project();
        for bad in ["../fora.txt", "/etc/passwd", "src/../../fora.txt"] {
            let args = json!({ "path": bad });
            let err = run(&FsRead, args.clone(), &ctx).await.unwrap_err();
            assert!(
                matches!(err, ToolError::OutsideWorkspace(_)),
                "deveria recusar {bad}: {err:?}"
            );
            assert!(
                !FsRead.within_workspace(&args, &ctx),
                "a política precisa saber que {bad} escapa"
            );
        }
    }

    #[tokio::test]
    async fn read_refuses_binary_files() {
        let (dir, ctx) = project();
        std::fs::write(dir.path().join("icone.png"), [0x89, b'P', 0x00, 0x01]).unwrap();
        let err = run(&FsRead, json!({"path": "icone.png"}), &ctx)
            .await
            .unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("binário"), "{msg}");
        assert!(msg.contains("icone.png"), "{msg}");
    }

    #[tokio::test]
    async fn read_pages_through_a_long_file() {
        let (dir, ctx) = project();
        let big: String = (1..=50).map(|i| format!("linha {i}\n")).collect();
        std::fs::write(dir.path().join("grande.txt"), big).unwrap();

        let out = run(
            &FsRead,
            json!({"path": "grande.txt", "offset_lines": 10, "max_lines": 5}),
            &ctx,
        )
        .await
        .unwrap();
        assert!(out.content.contains("  10| linha 10"), "{}", out.content);
        assert!(out.content.contains("  14| linha 14"), "{}", out.content);
        assert!(!out.content.contains("| linha 15"), "{}", out.content);
        assert!(
            out.content.contains("offset_lines=15"),
            "deveria ensinar a continuar: {}",
            out.content
        );
    }

    #[tokio::test]
    async fn read_missing_file_says_what_to_do() {
        let (_d, ctx) = project();
        let err = run(&FsRead, json!({"path": "sumiu.txt"}), &ctx)
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)));
        assert!(err.to_model_message().contains("Liste a pasta"));
    }

    #[tokio::test]
    async fn read_on_a_directory_points_to_fs_list() {
        let (_d, ctx) = project();
        let err = run(&FsRead, json!({"path": "src"}), &ctx)
            .await
            .unwrap_err();
        assert!(err.to_model_message().contains("fs_list"));
    }

    // ------------------------------------------------------------- fs_list ---

    #[tokio::test]
    async fn list_marks_directories_with_a_slash() {
        let (_d, ctx) = project();
        let out = run(&FsList, json!({}), &ctx).await.unwrap();
        assert!(out.content.contains("src/"), "{}", out.content);
        assert!(out.content.contains("README.md"), "{}", out.content);
        assert!(
            !out.content.contains("README.md/"),
            "arquivo não leva barra: {}",
            out.content
        );
    }

    #[tokio::test]
    async fn list_skips_generated_folders() {
        let (_d, ctx) = project();
        let out = run(&FsList, json!({}), &ctx).await.unwrap();
        assert!(
            !out.content.contains("node_modules"),
            "não deveria listar dependências: {}",
            out.content
        );
    }

    #[tokio::test]
    async fn list_respects_gitignore_even_without_a_git_folder() {
        let (dir, ctx) = project();
        std::fs::write(dir.path().join(".gitignore"), "segredo.txt\n").unwrap();
        std::fs::write(dir.path().join("segredo.txt"), "x").unwrap();
        let out = run(&FsList, json!({}), &ctx).await.unwrap();
        assert!(!out.content.contains("segredo.txt"), "{}", out.content);
        assert!(out.content.contains(".gitignore"), "{}", out.content);
    }

    #[tokio::test]
    async fn list_caps_the_number_of_entries() {
        let (dir, ctx) = project();
        for i in 0..(MAX_LIST_ENTRIES + 40) {
            std::fs::write(dir.path().join(format!("f{i}.txt")), "x").unwrap();
        }
        let out = run(&FsList, json!({}), &ctx).await.unwrap();
        assert!(out.content.contains("mostrando"), "{}", out.content);
        let listed = out.content.lines().filter(|l| l.ends_with(".txt")).count();
        assert!(listed <= MAX_LIST_ENTRIES, "listou {listed}");
    }

    #[tokio::test]
    async fn list_on_a_file_points_to_fs_read() {
        let (_d, ctx) = project();
        let err = run(&FsList, json!({"path": "README.md"}), &ctx)
            .await
            .unwrap_err();
        assert!(err.to_model_message().contains("fs_read"));
    }

    // ------------------------------------------------------------- fs_glob ---

    #[tokio::test]
    async fn glob_finds_files_in_any_subfolder() {
        let (_d, ctx) = project();
        let out = run(&FsGlob, json!({"pattern": "**/*.rs"}), &ctx)
            .await
            .unwrap();
        assert!(out.content.contains("src/main.rs"), "{}", out.content);
        assert!(out.content.contains("src/lib.rs"), "{}", out.content);
        assert!(!out.content.contains("README.md"), "{}", out.content);
    }

    #[tokio::test]
    async fn glob_limits_the_number_of_results() {
        let (dir, ctx) = project();
        for i in 0..20 {
            std::fs::write(dir.path().join(format!("t{i}.md")), "x").unwrap();
        }
        let out = run(&FsGlob, json!({"pattern": "*.md", "max_results": 5}), &ctx)
            .await
            .unwrap();
        let listed = out.content.lines().filter(|l| l.ends_with(".md")).count();
        assert_eq!(listed, 5, "{}", out.content);
        assert!(out.content.contains("mostrando 5"), "{}", out.content);
    }

    #[tokio::test]
    async fn glob_without_matches_teaches_the_syntax() {
        let (_d, ctx) = project();
        let out = run(&FsGlob, json!({"pattern": "**/*.kt"}), &ctx)
            .await
            .unwrap();
        assert!(out.content.contains("Nenhum arquivo"), "{}", out.content);
        assert!(out.content.contains("**/"), "{}", out.content);
    }

    // ------------------------------------------------------------- fs_grep ---

    #[tokio::test]
    async fn grep_reports_file_line_and_snippet() {
        let (_d, ctx) = project();
        let out = run(&FsGrep, json!({"pattern": "println"}), &ctx)
            .await
            .unwrap();
        assert!(
            out.content.contains("src/main.rs:2: println!(\"oi\");"),
            "{}",
            out.content
        );
    }

    #[tokio::test]
    async fn grep_can_ignore_case() {
        let (_d, ctx) = project();
        let strict = run(&FsGrep, json!({"pattern": "PROJETO"}), &ctx)
            .await
            .unwrap();
        assert!(
            strict.content.contains("Nenhuma ocorrência"),
            "{}",
            strict.content
        );

        let loose = run(
            &FsGrep,
            json!({"pattern": "PROJETO", "ignore_case": true}),
            &ctx,
        )
        .await
        .unwrap();
        assert!(loose.content.contains("README.md:1"), "{}", loose.content);
    }

    #[tokio::test]
    async fn grep_stops_at_the_result_cap() {
        let (dir, ctx) = project();
        let many: String = (0..50).map(|_| "achado aqui\n").collect();
        std::fs::write(dir.path().join("muitos.txt"), many).unwrap();
        let out = run(
            &FsGrep,
            json!({"pattern": "achado", "max_results": 10, "path": "muitos.txt"}),
            &ctx,
        )
        .await
        .unwrap();
        let lines = out
            .content
            .lines()
            .filter(|l| l.starts_with("muitos.txt:"))
            .count();
        assert_eq!(lines, 10, "{}", out.content);
        assert!(out.content.contains("parou em 10"), "{}", out.content);
    }

    #[tokio::test]
    async fn grep_ignores_binary_files() {
        let (dir, ctx) = project();
        let mut blob = b"achado".to_vec();
        blob.push(0);
        std::fs::write(dir.path().join("dados.bin"), blob).unwrap();
        std::fs::write(dir.path().join("texto.txt"), "achado\n").unwrap();
        let out = run(&FsGrep, json!({"pattern": "achado"}), &ctx)
            .await
            .unwrap();
        assert!(out.content.contains("texto.txt:1"), "{}", out.content);
        assert!(!out.content.contains("dados.bin"), "{}", out.content);
    }

    #[tokio::test]
    async fn grep_without_hits_explains_it_is_literal() {
        let (_d, ctx) = project();
        let out = run(&FsGrep, json!({"pattern": "f.*n"}), &ctx)
            .await
            .unwrap();
        assert!(out.content.contains("literal"), "{}", out.content);
        assert!(out.content.contains("ignore_case"), "{}", out.content);
    }

    // ------------------------------------------------------------ fs_write ---

    #[tokio::test]
    async fn write_creates_parent_directories() {
        let (dir, ctx) = project();
        let out = run(
            &FsWrite,
            json!({"path": "docs/guia/inicio.md", "content": "# Início\n"}),
            &ctx,
        )
        .await
        .unwrap();
        let written = dir.path().join("docs/guia/inicio.md");
        assert!(written.exists(), "arquivo não foi criado");
        assert_eq!(std::fs::read_to_string(written).unwrap(), "# Início\n");
        assert_eq!(out.changed_files, vec!["docs/guia/inicio.md".to_string()]);
        assert!(out.content.contains("criado"), "{}", out.content);
    }

    #[tokio::test]
    async fn write_preview_of_a_new_file_is_a_creation_diff() {
        let (_d, ctx) = project();
        let args = json!({"path": "novo.txt", "content": "linha\n"});
        let preview = FsWrite.preview(&args, &ctx).await.unwrap();
        match preview {
            ToolPreview::Diff {
                path,
                unified,
                created,
            } => {
                assert_eq!(path, "novo.txt");
                assert!(created, "arquivo novo deveria marcar created");
                assert!(unified.contains("+linha"), "{unified}");
            }
            other => panic!("esperava diff, veio {other:?}"),
        }
    }

    #[tokio::test]
    async fn write_preview_of_an_existing_file_shows_the_change() {
        let (_d, ctx) = project();
        let args = json!({"path": "src/lib.rs", "content": "pub fn soma() { 1; }\n"});
        let preview = FsWrite.preview(&args, &ctx).await.unwrap();
        match preview {
            ToolPreview::Diff {
                unified, created, ..
            } => {
                assert!(!created, "arquivo existente não é criação");
                assert!(unified.contains("-pub fn soma() {}"), "{unified}");
                assert!(unified.contains("+pub fn soma() { 1; }"), "{unified}");
            }
            other => panic!("esperava diff, veio {other:?}"),
        }
    }

    #[tokio::test]
    async fn write_with_the_same_content_does_nothing() {
        let (_d, ctx) = project();
        let out = run(
            &FsWrite,
            json!({"path": "src/lib.rs", "content": "pub fn soma() {}\n"}),
            &ctx,
        )
        .await
        .unwrap();
        assert!(out.content.contains("nada foi alterado"), "{}", out.content);
        assert!(out.changed_files.is_empty(), "não houve mudança");
    }

    #[tokio::test]
    async fn write_declares_the_file_at_risk() {
        let (_d, ctx) = project();
        let args = json!({"path": "src/main.rs", "content": "x"});
        assert_eq!(FsWrite.files_at_risk(&args, &ctx), vec!["src/main.rs"]);
        assert!(FsWrite.within_workspace(&args, &ctx));

        let escape = json!({"path": "../fora.txt", "content": "x"});
        assert!(!FsWrite.within_workspace(&escape, &ctx));
    }

    // ------------------------------------------------------------- fs_edit ---

    #[tokio::test]
    async fn edit_replaces_a_unique_snippet() {
        let (dir, ctx) = project();
        let out = run(
            &FsEdit,
            json!({"path": "src/lib.rs", "old_text": "soma", "new_text": "subtrai"}),
            &ctx,
        )
        .await
        .unwrap();
        let text = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
        assert_eq!(text, "pub fn subtrai() {}\n");
        assert_eq!(out.changed_files, vec!["src/lib.rs".to_string()]);
        assert!(out.content.contains("1 ocorrência"), "{}", out.content);
    }

    #[tokio::test]
    async fn edit_with_missing_text_tells_the_model_to_reread() {
        let (_d, ctx) = project();
        let err = run(
            &FsEdit,
            json!({"path": "src/lib.rs", "old_text": "não existe", "new_text": "x"}),
            &ctx,
        )
        .await
        .unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("não foi encontrado"), "{msg}");
        assert!(msg.contains("fs_read"), "deveria mandar reler: {msg}");
        assert!(msg.contains("exato"), "{msg}");
    }

    #[tokio::test]
    async fn edit_with_several_matches_asks_for_more_context() {
        let (dir, ctx) = project();
        std::fs::write(dir.path().join("rep.txt"), "alvo\nalvo\nalvo\n").unwrap();
        let err = run(
            &FsEdit,
            json!({"path": "rep.txt", "old_text": "alvo", "new_text": "novo"}),
            &ctx,
        )
        .await
        .unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("3 vezes"), "{msg}");
        assert!(msg.contains("replace_all"), "{msg}");
        // Nada pode ter sido gravado no meio do caminho.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("rep.txt")).unwrap(),
            "alvo\nalvo\nalvo\n"
        );
    }

    #[tokio::test]
    async fn edit_replace_all_changes_every_occurrence() {
        let (dir, ctx) = project();
        std::fs::write(dir.path().join("rep.txt"), "alvo\nalvo\n").unwrap();
        let out = run(
            &FsEdit,
            json!({"path": "rep.txt", "old_text": "alvo", "new_text": "novo", "replace_all": true}),
            &ctx,
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("rep.txt")).unwrap(),
            "novo\nnovo\n"
        );
        assert!(out.content.contains("2 ocorrências"), "{}", out.content);
    }

    #[tokio::test]
    async fn edit_preview_shows_the_diff() {
        let (_d, ctx) = project();
        let args = json!({"path": "src/lib.rs", "old_text": "soma", "new_text": "subtrai"});
        match FsEdit.preview(&args, &ctx).await.unwrap() {
            ToolPreview::Diff {
                path,
                unified,
                created,
            } => {
                assert_eq!(path, "src/lib.rs");
                assert!(!created);
                assert!(unified.contains("+pub fn subtrai() {}"), "{unified}");
            }
            other => panic!("esperava diff, veio {other:?}"),
        }
    }

    #[tokio::test]
    async fn edit_preview_is_honest_when_it_cannot_apply() {
        let (_d, ctx) = project();
        let args = json!({"path": "src/lib.rs", "old_text": "inexistente", "new_text": "x"});
        match FsEdit.preview(&args, &ctx).await.unwrap() {
            ToolPreview::Text { body } => assert!(body.contains("não foi encontrado"), "{body}"),
            other => panic!("esperava texto explicando a falha, veio {other:?}"),
        }
    }

    #[tokio::test]
    async fn edit_needs_a_workspace() {
        let ctx = ToolContext::new(None, "call-1");
        let args = json!({"path": "a.txt", "old_text": "a", "new_text": "b"});
        assert!(!FsEdit.within_workspace(&args, &ctx));
        assert!(run(&FsEdit, args, &ctx).await.is_err());
    }

    // ----------------------------------------------------------- fs_append ---

    #[tokio::test]
    async fn appends_to_an_existing_file() {
        let (dir, ctx) = project();
        let out = run(
            &FsAppend,
            json!({"path": "README.md", "content": "linha três\n"}),
            &ctx,
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("README.md")).unwrap(),
            "# Projeto\n\nlinha dois\nlinha três\n"
        );
        assert_eq!(out.changed_files, vec!["README.md".to_string()]);
        assert!(
            out.content.contains("Acrescentei 1 linha a `README.md`"),
            "{}",
            out.content
        );
        assert!(out.content.contains("agora 4 linhas"), "{}", out.content);
    }

    #[tokio::test]
    async fn creates_the_file_and_folders_when_missing() {
        let (dir, ctx) = project();
        let out = run(
            &FsAppend,
            json!({"path": "site/partes/rodape.html", "content": "<footer>fim</footer>\n"}),
            &ctx,
        )
        .await
        .unwrap();
        let written = dir.path().join("site/partes/rodape.html");
        assert!(written.exists(), "arquivo não foi criado");
        assert_eq!(
            std::fs::read_to_string(written).unwrap(),
            "<footer>fim</footer>\n"
        );
        assert_eq!(
            out.changed_files,
            vec!["site/partes/rodape.html".to_string()]
        );
    }

    #[tokio::test]
    async fn a_newline_seam_is_added_only_when_needed() {
        // Os quatro cruzamentos: arquivo com/sem `\n` final × pedaço com/sem
        // `\n` inicial. A costura só entra quando falta dos dois lados.
        let cases = [
            ("a", "b", "a\nb"),
            ("a\n", "b", "a\nb"),
            ("a", "\nb", "a\nb"),
            ("a\n", "\nb", "a\n\nb"),
        ];
        for (before, piece, expected) in cases {
            let (dir, ctx) = project();
            std::fs::write(dir.path().join("alvo.txt"), before).unwrap();
            run(
                &FsAppend,
                json!({"path": "alvo.txt", "content": piece}),
                &ctx,
            )
            .await
            .unwrap();
            assert_eq!(
                std::fs::read_to_string(dir.path().join("alvo.txt")).unwrap(),
                expected,
                "antes {before:?} + pedaço {piece:?}"
            );
        }
    }

    #[tokio::test]
    async fn refuses_a_path_outside_the_project() {
        let (_d, ctx) = project();
        for bad in ["../fora.txt", "/etc/passwd", "src/../../fora.txt"] {
            let args = json!({ "path": bad, "content": "x" });
            let err = run(&FsAppend, args.clone(), &ctx).await.unwrap_err();
            assert!(
                matches!(err, ToolError::OutsideWorkspace(_)),
                "deveria recusar {bad}: {err:?}"
            );
            assert!(
                !FsAppend.within_workspace(&args, &ctx),
                "a política precisa saber que {bad} escapa"
            );
        }

        // Caminho legítimo entra no checkpoint antes da primeira alteração.
        let args = json!({"path": "src/main.rs", "content": "x"});
        assert_eq!(FsAppend.files_at_risk(&args, &ctx), vec!["src/main.rs"]);
    }

    #[tokio::test]
    async fn refuses_appending_to_a_binary_file() {
        let (dir, ctx) = project();
        std::fs::write(dir.path().join("icone.png"), [0x89, b'P', 0x00, 0x01]).unwrap();
        let err = run(
            &FsAppend,
            json!({"path": "icone.png", "content": "texto"}),
            &ctx,
        )
        .await
        .unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("binário"), "{msg}");
        assert!(msg.contains("icone.png"), "{msg}");
    }

    // ------------------------------------------------------------ catálogo ---

    #[tokio::test]
    async fn builtin_registry_exposes_the_nine_tools() {
        let registry = builtin_registry();
        let mut names = registry.builtin_names();
        names.sort();
        assert_eq!(
            names,
            vec![
                "fs_append",
                "fs_edit",
                "fs_glob",
                "fs_grep",
                "fs_list",
                "fs_read",
                "fs_write",
                "terminal_run",
                "todo_update",
            ]
        );
        // O catálogo que vai para o modelo tem de estar completo.
        for spec in registry.specs().await {
            assert!(!spec.description.is_empty(), "{} sem descrição", spec.name);
            assert_eq!(spec.parameters["type"], "object", "{}", spec.name);
        }
    }

    #[tokio::test]
    async fn read_only_tools_are_marked_as_such() {
        let registry = builtin_registry();
        for spec in registry.specs().await {
            let expected = matches!(
                spec.name.as_str(),
                "terminal_run" | "fs_write" | "fs_edit" | "fs_append"
            );
            assert_eq!(spec.read_only, !expected, "read_only de {}", spec.name);
        }
    }

    #[test]
    fn binary_sniffing_only_looks_at_the_head() {
        assert!(looks_binary(&[b'a', 0, b'b']));
        assert!(!looks_binary("texto normal".as_bytes()));
        // Zero depois da janela de inspeção não conta.
        let mut late = vec![b'a'; BINARY_SNIFF_BYTES];
        late.push(0);
        assert!(!looks_binary(&late));
    }

    #[test]
    fn snippet_cuts_without_breaking_utf8() {
        let cut = snippet("  çãõ é acentuado demais  ", 5);
        assert_eq!(cut, "çãõ é…");
        assert_eq!(snippet("  curto  ", 50), "curto");
    }
}
