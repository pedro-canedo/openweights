//! Descoberta do projeto: que linguagens, que gerenciador, que comandos.
//!
//! Sem isto o agente chuta. `npm test` num projeto Rust, `cargo build` num
//! projeto Go — cada chute é um passo perdido, uma mensagem de erro confusa
//! no histórico e uma chance a mais de o modelo desistir do caminho certo.
//! Ler o manifesto custa milissegundos e responde a pergunta de verdade.
//!
//! Três decisões explicam o resto do arquivo:
//!
//! **Profundidade 1.** Repositório real quase nunca é de uma linguagem só: o
//! Tauri guarda o Rust em `src-tauri/`, o monorepo guarda a API em `server/`.
//! Olhar só a raiz perderia metade dos projetos; varrer tudo custaria caro e
//! encheria a resposta de ruído (cada pacote de `node_modules` tem um
//! `package.json`). Um nível abaixo pega o caso comum e para.
//!
//! **Sem parser de TOML.** De `Cargo.toml` só precisamos saber que existe; de
//! `pyproject.toml`, se cita `pytest`, `ruff` ou `black`. Busca de texto
//! resolve os dois, e uma dependência a menos é uma superfície a menos.
//! `package.json` é a exceção: ali os *scripts* importam, e são JSON.
//!
//! **Comando estruturado, nunca linha de shell.** Cada comando é um vetor
//! `argv`. Isso não é preciosismo: o filtro de teste vem do modelo, e um
//! filtro como `x && rm -rf .` só é perigoso se alguém colar tudo numa linha
//! e entregar ao shell. Como `argv` vai direto para o sistema, o filtro é
//! sempre *um argumento*, jamais um comando.

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Teto de leitura de um manifesto (um `package.json` gerado pode ser enorme).
const MAX_MANIFEST_BYTES: usize = 512 * 1024;

/// Pastas que nunca contêm o manifesto principal do projeto.
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".idea",
    ".vscode",
    ".openweights",
    "node_modules",
    "bower_components",
    "target",
    "dist",
    "build",
    "out",
    "bin",
    "obj",
    "vendor",
    "coverage",
    ".venv",
    "venv",
    "env",
    "__pycache__",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".gradle",
    ".cache",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
];

/// Etapa do ciclo "editar, testar, corrigir" que uma ferramenta quer rodar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Task {
    Test,
    Lint,
    Format,
    Build,
}

impl Task {
    /// Nome em português para as mensagens que o modelo lê.
    pub fn label(self) -> &'static str {
        match self {
            Task::Test => "testar",
            Task::Lint => "analisar (lint)",
            Task::Format => "formatar",
            Task::Build => "compilar",
        }
    }

    /// Ferramenta que atende esta etapa (aparece nas mensagens de erro).
    pub fn tool_name(self) -> &'static str {
        match self {
            Task::Test => "test_run",
            Task::Lint => "lint_run",
            Task::Format => "format_run",
            Task::Build => "build_run",
        }
    }
}

/// Um comando pronto para rodar, já dividido em programa + argumentos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cmd {
    pub argv: Vec<String>,
}

impl Cmd {
    pub fn new<S: Into<String>>(parts: impl IntoIterator<Item = S>) -> Self {
        Self {
            argv: parts.into_iter().map(Into::into).collect(),
        }
    }

    /// Devolve o comando com mais argumentos no fim.
    pub fn plus<S: Into<String>>(mut self, parts: impl IntoIterator<Item = S>) -> Self {
        self.argv.extend(parts.into_iter().map(Into::into));
        self
    }

    pub fn program(&self) -> &str {
        self.argv.first().map(String::as_str).unwrap_or_default()
    }

    pub fn args(&self) -> &[String] {
        self.argv.get(1..).unwrap_or_default()
    }

    /// Linha legível para a tela de confirmação e para o resumo.
    ///
    /// É só para leitura humana: quem executa recebe `argv`, então as aspas
    /// aqui não têm poder nenhum de mudar o que roda.
    pub fn display(&self) -> String {
        self.argv
            .iter()
            .map(|part| {
                if part.contains(' ') {
                    format!("\"{part}\"")
                } else {
                    part.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Como um filtro de teste entra na linha de comando de cada runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FilterStyle {
    /// `cargo test soma`
    Positional,
    /// `npm run test -- soma` (o `--` repassa ao runner por baixo do script)
    AfterDoubleDash,
    /// `pytest -k soma`
    Flag(&'static str),
    /// `mvn -Dtest=soma`
    Glued(&'static str),
}

impl FilterStyle {
    /// Aplica o filtro ao comando, sempre como argumento separado.
    pub fn apply(self, cmd: Cmd, filter: &str) -> Cmd {
        match self {
            FilterStyle::Positional => cmd.plus([filter]),
            FilterStyle::AfterDoubleDash => cmd.plus(["--", filter]),
            FilterStyle::Flag(flag) => cmd.plus([flag, filter]),
            FilterStyle::Glued(prefix) => cmd.plus([format!("{prefix}={filter}")]),
        }
    }
}

/// Uma linguagem/gerenciador encontrado numa pasta do projeto.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stack {
    /// `rust`, `node`, `python`, `go`, `java`, `dotnet`, `php`, `ruby`.
    pub language: String,
    /// `cargo`, `npm`, `pnpm`, `uv`, `go`, `maven`…
    pub manager: String,
    /// Pasta relativa à raiz (`""` = a própria raiz).
    pub dir: String,
    /// Manifesto que identificou a stack, relativo à raiz.
    pub manifest: String,
    /// Scripts declarados (npm/composer), quando houver.
    pub scripts: BTreeMap<String, String>,
    pub test: Option<Cmd>,
    pub test_filter: FilterStyle,
    pub lint: Option<Cmd>,
    /// Variante do lint que reescreve arquivos, quando a ferramenta tem uma.
    pub lint_fix: Option<Cmd>,
    /// Formatador em modo escrita.
    pub format: Option<Cmd>,
    /// Formatador em modo conferência — é ele que diz o que *mudaria*.
    pub format_check: Option<Cmd>,
    pub build: Option<Cmd>,
    /// Extensões que o formatador desta stack costuma reescrever.
    pub source_ext: Vec<String>,
}

impl Stack {
    fn new(language: &str, manager: &str, dir: &str, manifest_name: &str) -> Self {
        Self {
            language: language.to_string(),
            manager: manager.to_string(),
            dir: dir.to_string(),
            manifest: join_rel(dir, manifest_name),
            scripts: BTreeMap::new(),
            test: None,
            test_filter: FilterStyle::Positional,
            lint: None,
            lint_fix: None,
            format: None,
            format_check: None,
            build: None,
            source_ext: Vec::new(),
        }
    }

    /// Comando desta stack para a etapa pedida.
    pub fn command(&self, task: Task) -> Option<&Cmd> {
        match task {
            Task::Test => self.test.as_ref(),
            Task::Lint => self.lint.as_ref(),
            Task::Format => self.format.as_ref(),
            Task::Build => self.build.as_ref(),
        }
    }

    /// Como a pasta desta stack aparece nas mensagens.
    pub fn where_label(&self) -> String {
        if self.dir.is_empty() {
            "raiz do projeto".to_string()
        } else {
            self.dir.clone()
        }
    }

    /// Profundidade da pasta (0 = raiz) — usada para ordenar as stacks.
    fn depth(&self) -> usize {
        if self.dir.is_empty() {
            0
        } else {
            self.dir.matches('/').count() + 1
        }
    }
}

/// Tudo que descobrimos sobre o projeto.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub root: PathBuf,
    /// Ordenadas: a primeira é a principal.
    pub stacks: Vec<Stack>,
}

impl Project {
    pub fn primary(&self) -> Option<&Stack> {
        self.stacks.first()
    }

    pub fn is_empty(&self) -> bool {
        self.stacks.is_empty()
    }

    /// Linguagens encontradas, sem repetição e na ordem de importância.
    pub fn languages(&self) -> Vec<String> {
        let mut seen = BTreeSet::new();
        self.stacks
            .iter()
            .filter(|s| seen.insert(s.language.clone()))
            .map(|s| s.language.clone())
            .collect()
    }

    /// Escolhe a stack que atende a etapa pedida.
    ///
    /// Com `dir`, obedece: a pessoa (ou o modelo) sabe onde quer rodar. Sem
    /// `dir`, devolve a primeira stack que **tem** aquele comando — não a
    /// principal. É o que faz um projeto Tauri funcionar: a raiz é `node` sem
    /// script de teste, e `test_run` cai sozinho no `cargo test` de
    /// `src-tauri/` em vez de reclamar que não há testes.
    pub fn pick(&self, task: Task, dir: Option<&str>) -> Option<&Stack> {
        match dir {
            Some(want) => {
                let want = normalize_dir(want);
                self.stacks.iter().find(|s| s.dir == want)
            }
            None => self
                .stacks
                .iter()
                .find(|s| s.command(task).is_some())
                .or_else(|| self.primary()),
        }
    }

    /// Pastas conhecidas, para sugerir `dir` numa mensagem de erro.
    pub fn dirs(&self) -> Vec<String> {
        self.stacks
            .iter()
            .map(|s| {
                if s.dir.is_empty() {
                    ".".to_string()
                } else {
                    s.dir.clone()
                }
            })
            .collect()
    }
}

/// Lê a pasta e devolve o que o projeto usa.
pub fn detect(root: &Path) -> Project {
    let mut stacks = Vec::new();
    scan_dir(root, "", &mut stacks);

    for sub in subdirs(root) {
        scan_dir(&root.join(&sub), &sub, &mut stacks);
    }

    // Mais raso primeiro; empate resolvido por uma ordem fixa, para a resposta
    // ser sempre a mesma no mesmo projeto.
    stacks.sort_by_key(|s| (s.depth(), rank(&s.language), s.dir.clone()));

    Project {
        root: root.to_path_buf(),
        stacks,
    }
}

/// Ordem de desempate entre stacks na mesma profundidade.
///
/// Em repositório misto a camada interpretada costuma ser a casca do produto
/// (o app, o site) e a compilada mora numa subpasta — por isso `node` e
/// `python` vêm antes. De todo jeito `pick` prefere quem *tem* o comando, e
/// `project_info` mostra todas, então esta ordem só decide o desempate.
fn rank(language: &str) -> u8 {
    match language {
        "node" => 0,
        "python" => 1,
        "rust" => 2,
        "go" => 3,
        "dotnet" => 4,
        "java" => 5,
        "php" => 6,
        "ruby" => 7,
        _ => 8,
    }
}

/// Subpastas candidatas a conter um manifesto (um nível, sem as ignoradas).
fn subdirs(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str()))
        .collect();
    out.sort();
    out
}

fn scan_dir(path: &Path, rel: &str, out: &mut Vec<Stack>) {
    let dir = Dir::read(path);
    if dir.names.is_empty() {
        return;
    }
    out.extend(node_stack(&dir, rel));
    out.extend(rust_stack(&dir, rel));
    out.extend(python_stack(&dir, rel));
    out.extend(go_stack(&dir, rel));
    out.extend(java_stack(&dir, rel));
    out.extend(dotnet_stack(&dir, rel));
    out.extend(php_stack(&dir, rel));
    out.extend(ruby_stack(&dir, rel));
}

// --------------------------------------------------------------- node/js ---

fn node_stack(dir: &Dir, rel: &str) -> Option<Stack> {
    let raw = dir.text("package.json")?;
    // `package.json` quebrado ainda identifica um projeto Node; seguimos sem
    // os scripts em vez de fingir que a pasta não é Node.
    let pkg: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);

    let scripts = string_map(pkg.get("scripts"));
    let deps = dep_names(&pkg);
    let pm = package_manager(&pkg, dir);
    let exec = exec_prefix(&pm);

    let mut stack = Stack::new("node", &pm, rel, "package.json");
    stack.source_ext = str_vec(&[
        "js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts", "json", "css", "scss", "less",
        "html", "vue", "svelte", "md", "yml", "yaml",
    ]);

    // Testes: o script do projeto manda, desde que não seja o esqueleto que o
    // `npm init` deixa (`echo "Error: no test specified" && exit 1`).
    let script_test = scripts
        .get("test")
        .filter(|body| !body.contains("no test specified"));
    if script_test.is_some() {
        stack.test = Some(run_script(&pm, "test"));
        stack.test_filter = FilterStyle::AfterDoubleDash;
    } else if deps.contains("vitest") {
        stack.test = Some(Cmd::new(exec.clone()).plus(["vitest", "run"]));
    } else if deps.contains("jest") {
        stack.test = Some(Cmd::new(exec.clone()).plus(["jest", "--ci"]));
    } else if deps.contains("mocha") {
        stack.test = Some(Cmd::new(exec.clone()).plus(["mocha"]));
    }

    // Lint.
    let eslint_config = dir.has_prefix(".eslintrc") || dir.has_prefix("eslint.config.");
    if let Some(script) = scripts.get("lint") {
        stack.lint = Some(run_script(&pm, "lint"));
        // `npm run lint -- --fix` só faz sentido se o script for um linter que
        // aceita `--fix`; para um script qualquer o repasse pode virar erro.
        if ["eslint", "biome", "oxlint", "ruff"]
            .iter()
            .any(|t| script.contains(t))
        {
            stack.lint_fix = Some(run_script(&pm, "lint").plus(["--", "--fix"]));
        }
    } else if eslint_config || deps.contains("eslint") {
        stack.lint = Some(Cmd::new(exec.clone()).plus(["eslint", "."]));
        stack.lint_fix = Some(Cmd::new(exec.clone()).plus(["eslint", ".", "--fix"]));
    } else if deps.contains("@biomejs/biome") || dir.has("biome.json") {
        stack.lint = Some(Cmd::new(exec.clone()).plus(["biome", "lint", "."]));
        stack.lint_fix = Some(Cmd::new(exec.clone()).plus(["biome", "lint", "--write", "."]));
    }

    // Formatação: preferimos chamar o formatador direto, porque precisamos das
    // DUAS variantes (conferir e escrever) e um script só entrega uma.
    let prettier = deps.contains("prettier")
        || dir.has_prefix(".prettierrc")
        || dir.has_prefix("prettier.config.");
    if prettier {
        stack.format = Some(Cmd::new(exec.clone()).plus(["prettier", "--write", "."]));
        stack.format_check = Some(Cmd::new(exec.clone()).plus(["prettier", "--check", "."]));
    } else if deps.contains("@biomejs/biome") || dir.has("biome.json") {
        stack.format = Some(Cmd::new(exec.clone()).plus(["biome", "format", "--write", "."]));
        stack.format_check = Some(Cmd::new(exec.clone()).plus(["biome", "format", "."]));
    } else if let Some(name) = ["format", "fmt"].iter().find(|n| scripts.contains_key(**n)) {
        stack.format = Some(run_script(&pm, name));
    }

    // Build.
    if scripts.contains_key("build") {
        stack.build = Some(run_script(&pm, "build"));
    } else if dir.has("tsconfig.json") {
        stack.build = Some(Cmd::new(exec).plus(["tsc", "--noEmit"]));
    }

    stack.scripts = scripts;
    Some(stack)
}

/// Gerenciador do projeto Node: campo `packageManager`, senão o lockfile.
fn package_manager(pkg: &serde_json::Value, dir: &Dir) -> String {
    if let Some(declared) = pkg.get("packageManager").and_then(|v| v.as_str()) {
        let name = declared.split('@').next().unwrap_or_default().trim();
        if ["npm", "pnpm", "yarn", "bun"].contains(&name) {
            return name.to_string();
        }
    }
    for (lock, pm) in [
        ("pnpm-lock.yaml", "pnpm"),
        ("yarn.lock", "yarn"),
        ("bun.lockb", "bun"),
        ("bun.lock", "bun"),
        ("package-lock.json", "npm"),
    ] {
        if dir.has(lock) {
            return pm.to_string();
        }
    }
    "npm".to_string()
}

/// Como cada gerenciador roda um binário instalado no projeto.
fn exec_prefix(pm: &str) -> Vec<String> {
    match pm {
        "pnpm" => str_vec(&["pnpm", "exec"]),
        "yarn" => str_vec(&["yarn", "exec"]),
        "bun" => str_vec(&["bunx"]),
        _ => str_vec(&["npx"]),
    }
}

fn run_script(pm: &str, name: &str) -> Cmd {
    Cmd::new([pm, "run", name])
}

/// Nomes de `dependencies` + `devDependencies`.
fn dep_names(pkg: &serde_json::Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for field in ["dependencies", "devDependencies", "peerDependencies"] {
        if let Some(map) = pkg.get(field).and_then(|v| v.as_object()) {
            out.extend(map.keys().cloned());
        }
    }
    out
}

// ------------------------------------------------------------------ rust ---

fn rust_stack(dir: &Dir, rel: &str) -> Option<Stack> {
    if !dir.has("Cargo.toml") {
        return None;
    }
    let mut stack = Stack::new("rust", "cargo", rel, "Cargo.toml");
    stack.source_ext = str_vec(&["rs"]);
    stack.test = Some(Cmd::new(["cargo", "test"]));
    stack.test_filter = FilterStyle::Positional;
    stack.lint = Some(Cmd::new(["cargo", "clippy", "--all-targets"]));
    // `cargo clippy --fix` exige árvore de git limpa e reescreve código: fica
    // fora do automático de propósito. A mensagem do `lint_run` explica.
    stack.format = Some(Cmd::new(["cargo", "fmt"]));
    stack.format_check = Some(Cmd::new(["cargo", "fmt", "--check"]));
    stack.build = Some(Cmd::new(["cargo", "build"]));
    Some(stack)
}

// ---------------------------------------------------------------- python ---

/// Nome do interpretador padrão de cada sistema (com plano B em `exec`).
pub fn python_program() -> &'static str {
    if cfg!(windows) { "python" } else { "python3" }
}

fn python_stack(dir: &Dir, rel: &str) -> Option<Stack> {
    let manifest = ["pyproject.toml", "requirements.txt", "setup.py", "Pipfile"]
        .into_iter()
        .find(|name| dir.has(name))?;

    let config = [
        "pyproject.toml",
        "setup.cfg",
        "tox.ini",
        "pytest.ini",
        "requirements.txt",
        "requirements-dev.txt",
        "ruff.toml",
        ".ruff.toml",
    ]
    .iter()
    .filter_map(|name| dir.text(name))
    .collect::<Vec<_>>()
    .join("\n")
    .to_lowercase();

    let (manager, runner) = if dir.has("uv.lock") {
        ("uv", str_vec(&["uv", "run"]))
    } else if dir.has("poetry.lock") || config.contains("[tool.poetry]") {
        ("poetry", str_vec(&["poetry", "run"]))
    } else if dir.has("Pipfile") {
        ("pipenv", str_vec(&["pipenv", "run"]))
    } else {
        ("pip", Vec::new())
    };

    /// Monta `uv run <tool>` / `poetry run <tool>` ou `python -m <tool>`.
    fn tool(runner: &[String], name: &str) -> Cmd {
        if runner.is_empty() {
            Cmd::new([python_program(), "-m", name])
        } else {
            Cmd::new(runner.to_vec()).plus([name])
        }
    }

    let mut stack = Stack::new("python", manager, rel, manifest);
    stack.source_ext = str_vec(&["py", "pyi"]);

    let has_pytest = config.contains("pytest")
        || dir.has("pytest.ini")
        || dir.has("conftest.py")
        || dir.is_dir("tests")
        || dir.is_dir("test");
    stack.test = Some(if has_pytest {
        tool(&runner, "pytest")
    } else {
        tool(&runner, "unittest").plus(["discover"])
    });
    stack.test_filter = FilterStyle::Flag("-k");

    if config.contains("ruff") || dir.has("ruff.toml") || dir.has(".ruff.toml") {
        stack.lint = Some(tool(&runner, "ruff").plus(["check", "."]));
        stack.lint_fix = Some(tool(&runner, "ruff").plus(["check", ".", "--fix"]));
        stack.format = Some(tool(&runner, "ruff").plus(["format", "."]));
        stack.format_check = Some(tool(&runner, "ruff").plus(["format", "--check", "."]));
    } else if config.contains("flake8") || dir.has(".flake8") {
        stack.lint = Some(tool(&runner, "flake8"));
    }

    if stack.format.is_none() && config.contains("black") {
        stack.format = Some(tool(&runner, "black").plus(["."]));
        stack.format_check = Some(tool(&runner, "black").plus(["--check", "."]));
    }

    // Python não tem etapa de compilação: `build_run` explica em vez de
    // inventar um comando que não ajuda ninguém.
    Some(stack)
}

// -------------------------------------------------------------------- go ---

fn go_stack(dir: &Dir, rel: &str) -> Option<Stack> {
    if !dir.has("go.mod") {
        return None;
    }
    let mut stack = Stack::new("go", "go", rel, "go.mod");
    stack.source_ext = str_vec(&["go"]);
    stack.test = Some(Cmd::new(["go", "test", "./..."]));
    stack.test_filter = FilterStyle::Flag("-run");
    if dir.has_prefix(".golangci") {
        stack.lint = Some(Cmd::new(["golangci-lint", "run"]));
        stack.lint_fix = Some(Cmd::new(["golangci-lint", "run", "--fix"]));
    } else {
        stack.lint = Some(Cmd::new(["go", "vet", "./..."]));
    }
    stack.format = Some(Cmd::new(["gofmt", "-w", "."]));
    // `gofmt -l` lista exatamente os arquivos fora do padrão — é o modo
    // conferência mais direto que existe entre todos os formatadores.
    stack.format_check = Some(Cmd::new(["gofmt", "-l", "."]));
    stack.build = Some(Cmd::new(["go", "build", "./..."]));
    Some(stack)
}

// ------------------------------------------------------------------ java ---

fn java_stack(dir: &Dir, rel: &str) -> Option<Stack> {
    if dir.has("pom.xml") {
        let mut stack = Stack::new("java", "maven", rel, "pom.xml");
        stack.source_ext = str_vec(&["java"]);
        stack.test = Some(Cmd::new(["mvn", "-B", "test"]));
        stack.test_filter = FilterStyle::Glued("-Dtest");
        stack.build = Some(Cmd::new(["mvn", "-B", "compile"]));
        return Some(stack);
    }

    let manifest = ["build.gradle", "build.gradle.kts"]
        .into_iter()
        .find(|name| dir.has(name))?;
    // O wrapper do repositório usa a versão certa do Gradle; o `gradle` do
    // sistema pode nem existir.
    let gradle = if dir.has("gradlew") {
        if cfg!(windows) {
            "gradlew.bat"
        } else {
            "./gradlew"
        }
    } else {
        "gradle"
    };
    let mut stack = Stack::new("java", "gradle", rel, manifest);
    stack.source_ext = str_vec(&["java", "kt", "kts"]);
    stack.test = Some(Cmd::new([gradle, "test"]));
    stack.test_filter = FilterStyle::Flag("--tests");
    // `assemble` compila sem rodar a suíte — é o que "compilar" quer dizer.
    stack.build = Some(Cmd::new([gradle, "assemble"]));
    Some(stack)
}

// ---------------------------------------------------------------- dotnet ---

fn dotnet_stack(dir: &Dir, rel: &str) -> Option<Stack> {
    let manifest = dir
        .find_ext("sln")
        .or_else(|| dir.find_ext("csproj"))
        .or_else(|| dir.find_ext("fsproj"))?;
    let mut stack = Stack::new("dotnet", "dotnet", rel, &manifest);
    stack.source_ext = str_vec(&["cs", "fs", "vb"]);
    stack.test = Some(Cmd::new(["dotnet", "test"]));
    stack.test_filter = FilterStyle::Flag("--filter");
    stack.format = Some(Cmd::new(["dotnet", "format"]));
    stack.format_check = Some(Cmd::new(["dotnet", "format", "--verify-no-changes"]));
    stack.build = Some(Cmd::new(["dotnet", "build"]));
    Some(stack)
}

// ------------------------------------------------------------------- php ---

fn php_stack(dir: &Dir, rel: &str) -> Option<Stack> {
    let raw = dir.text("composer.json")?;
    let json: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
    let scripts = string_map(json.get("scripts"));

    let mut stack = Stack::new("php", "composer", rel, "composer.json");
    stack.source_ext = str_vec(&["php"]);
    if scripts.contains_key("test") {
        stack.test = Some(Cmd::new(["composer", "test"]));
    } else if dir.has_prefix("phpunit.xml") {
        stack.test = Some(Cmd::new(["composer", "exec", "--", "phpunit"]));
    }
    stack.test_filter = FilterStyle::Flag("--filter");
    if scripts.contains_key("lint") {
        stack.lint = Some(Cmd::new(["composer", "lint"]));
    }
    for name in ["format", "cs-fix", "fmt"] {
        if scripts.contains_key(name) {
            stack.format = Some(Cmd::new(["composer", name]));
            break;
        }
    }
    if scripts.contains_key("build") {
        stack.build = Some(Cmd::new(["composer", "build"]));
    }
    stack.scripts = scripts;
    Some(stack)
}

// ------------------------------------------------------------------ ruby ---

fn ruby_stack(dir: &Dir, rel: &str) -> Option<Stack> {
    if !dir.has("Gemfile") {
        return None;
    }
    let mut stack = Stack::new("ruby", "bundler", rel, "Gemfile");
    stack.source_ext = str_vec(&["rb"]);
    if dir.is_dir("spec") {
        stack.test = Some(Cmd::new(["bundle", "exec", "rspec"]));
        stack.test_filter = FilterStyle::Flag("-e");
    } else if dir.has("Rakefile") {
        stack.test = Some(Cmd::new(["bundle", "exec", "rake", "test"]));
    }
    if dir.has_prefix(".rubocop") {
        stack.lint = Some(Cmd::new(["bundle", "exec", "rubocop"]));
        stack.lint_fix = Some(Cmd::new(["bundle", "exec", "rubocop", "-a"]));
    }
    Some(stack)
}

// --------------------------------------------------------------- suporte ---

/// Uma pasta já lida: os nomes de uma vez, o conteúdo sob demanda.
struct Dir {
    path: PathBuf,
    names: Vec<String>,
}

impl Dir {
    fn read(path: &Path) -> Self {
        let mut names: Vec<String> = std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        Self {
            path: path.to_path_buf(),
            names,
        }
    }

    /// Comparação sem diferenciar maiúsculas: o Windows não diferencia, e um
    /// `cargo.toml` minúsculo não deveria esconder o projeto.
    fn has(&self, name: &str) -> bool {
        self.names.iter().any(|n| n.eq_ignore_ascii_case(name))
    }

    fn has_prefix(&self, prefix: &str) -> bool {
        let prefix = prefix.to_lowercase();
        self.names
            .iter()
            .any(|n| n.to_lowercase().starts_with(&prefix))
    }

    fn find_ext(&self, ext: &str) -> Option<String> {
        let suffix = format!(".{}", ext.to_lowercase());
        self.names
            .iter()
            .find(|n| n.to_lowercase().ends_with(&suffix))
            .cloned()
    }

    fn is_dir(&self, name: &str) -> bool {
        self.has(name) && self.path.join(name).is_dir()
    }

    fn text(&self, name: &str) -> Option<String> {
        let real = self.names.iter().find(|n| n.eq_ignore_ascii_case(name))?;
        let path = self.path.join(real);
        if std::fs::metadata(&path).ok()?.len() as usize > MAX_MANIFEST_BYTES {
            return None;
        }
        std::fs::read_to_string(path).ok()
    }
}

/// Mapa de strings de um objeto JSON (valores não-texto viram texto).
fn string_map(value: Option<&serde_json::Value>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Some(map) = value.and_then(|v| v.as_object()) {
        for (k, v) in map {
            let text = match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Array(items) => items
                    .iter()
                    .filter_map(|i| i.as_str())
                    .collect::<Vec<_>>()
                    .join(" && "),
                other => other.to_string(),
            };
            out.insert(k.clone(), text);
        }
    }
    out
}

fn str_vec(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn join_rel(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// Normaliza o `dir` que veio do modelo (`./src-tauri`, `src-tauri\`, `.`).
pub fn normalize_dir(dir: &str) -> String {
    let cleaned = dir.trim().replace('\\', "/");
    let cleaned = cleaned.trim_matches('/');
    match cleaned.strip_prefix("./").unwrap_or(cleaned) {
        "." | "" => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn project(files: &[(&str, &str)]) -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, body) in files {
            let full = dir.path().join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, body).unwrap();
        }
        dir
    }

    #[test]
    fn empty_folder_has_no_stack() {
        let dir = tempfile::tempdir().unwrap();
        let info = detect(dir.path());
        assert!(info.is_empty());
        assert!(info.primary().is_none());
        assert!(info.pick(Task::Test, None).is_none());
    }

    #[test]
    fn node_project_reads_scripts_and_package_manager() {
        let dir = project(&[
            (
                "package.json",
                r#"{"scripts":{"test":"vitest run","build":"vite build","lint":"eslint ."}}"#,
            ),
            ("pnpm-lock.yaml", ""),
        ]);
        let info = detect(dir.path());
        let stack = info.primary().unwrap();
        assert_eq!(stack.language, "node");
        assert_eq!(stack.manager, "pnpm", "o lockfile decide o gerenciador");
        assert_eq!(stack.test.as_ref().unwrap().display(), "pnpm run test");
        assert_eq!(stack.build.as_ref().unwrap().display(), "pnpm run build");
        assert_eq!(stack.lint.as_ref().unwrap().display(), "pnpm run lint");
        // O script é um eslint: dá para pedir correção automática.
        assert_eq!(
            stack.lint_fix.as_ref().unwrap().display(),
            "pnpm run lint -- --fix"
        );
        assert_eq!(stack.scripts.len(), 3);
    }

    #[test]
    fn package_manager_field_wins_over_lockfile() {
        let dir = project(&[
            ("package.json", r#"{"packageManager":"yarn@4.1.0"}"#),
            ("package-lock.json", "{}"),
        ]);
        let stack = detect(dir.path()).stacks.remove(0);
        assert_eq!(stack.manager, "yarn");
    }

    #[test]
    fn npm_placeholder_test_script_is_not_a_test_command() {
        let dir = project(&[(
            "package.json",
            r#"{"scripts":{"test":"echo \"Error: no test specified\" && exit 1"}}"#,
        )]);
        let stack = detect(dir.path()).stacks.remove(0);
        assert!(
            stack.test.is_none(),
            "o esqueleto do npm init não é uma suíte de testes"
        );
    }

    #[test]
    fn node_without_scripts_falls_back_to_the_installed_runner() {
        let dir = project(&[(
            "package.json",
            r#"{"devDependencies":{"jest":"^29","prettier":"^3"}}"#,
        )]);
        let stack = detect(dir.path()).stacks.remove(0);
        assert_eq!(stack.test.as_ref().unwrap().display(), "npx jest --ci");
        assert_eq!(
            stack.format_check.as_ref().unwrap().display(),
            "npx prettier --check ."
        );
        assert_eq!(
            stack.format.as_ref().unwrap().display(),
            "npx prettier --write ."
        );
    }

    #[test]
    fn broken_package_json_still_identifies_node() {
        let dir = project(&[("package.json", "{ isto não é json")]);
        let stack = detect(dir.path()).stacks.remove(0);
        assert_eq!(stack.language, "node");
        assert!(stack.scripts.is_empty());
    }

    #[test]
    fn rust_project_suggests_the_cargo_commands() {
        let dir = project(&[("Cargo.toml", "[package]\nname = \"x\"\n")]);
        let stack = detect(dir.path()).stacks.remove(0);
        assert_eq!(stack.language, "rust");
        assert_eq!(stack.test.as_ref().unwrap().display(), "cargo test");
        assert_eq!(
            stack.lint.as_ref().unwrap().display(),
            "cargo clippy --all-targets"
        );
        assert_eq!(
            stack.format_check.as_ref().unwrap().display(),
            "cargo fmt --check"
        );
        assert_eq!(stack.build.as_ref().unwrap().display(), "cargo build");
    }

    #[test]
    fn pyproject_with_ruff_and_pytest_is_fully_detected() {
        let dir = project(&[(
            "pyproject.toml",
            "[project]\nname=\"x\"\n[tool.ruff]\nline-length=100\n[tool.pytest.ini_options]\n",
        )]);
        let stack = detect(dir.path()).stacks.remove(0);
        assert_eq!(stack.language, "python");
        assert_eq!(stack.manager, "pip");
        let py = python_program();
        assert_eq!(
            stack.test.as_ref().unwrap().display(),
            format!("{py} -m pytest")
        );
        assert_eq!(
            stack.lint.as_ref().unwrap().display(),
            format!("{py} -m ruff check .")
        );
        assert_eq!(
            stack.format_check.as_ref().unwrap().display(),
            format!("{py} -m ruff format --check .")
        );
        assert!(stack.build.is_none(), "python não tem etapa de compilação");
    }

    #[test]
    fn uv_lock_switches_python_to_uv_run() {
        let dir = project(&[
            (
                "pyproject.toml",
                "[project]\nname=\"x\"\ndependencies=[\"pytest\"]\n",
            ),
            ("uv.lock", ""),
        ]);
        let stack = detect(dir.path()).stacks.remove(0);
        assert_eq!(stack.manager, "uv");
        assert_eq!(stack.test.as_ref().unwrap().display(), "uv run pytest");
    }

    #[test]
    fn requirements_only_project_defaults_to_unittest() {
        let dir = project(&[("requirements.txt", "requests==2.31.0\n")]);
        let stack = detect(dir.path()).stacks.remove(0);
        assert_eq!(stack.language, "python");
        let py = python_program();
        assert_eq!(
            stack.test.as_ref().unwrap().display(),
            format!("{py} -m unittest discover"),
            "sem sinal de pytest, unittest é o que existe garantido"
        );
    }

    #[test]
    fn tests_folder_alone_is_evidence_of_pytest() {
        let dir = project(&[
            ("requirements.txt", "requests\n"),
            ("tests/test_a.py", "def test_a():\n    assert True\n"),
        ]);
        let stack = detect(dir.path()).stacks.remove(0);
        assert!(stack.test.as_ref().unwrap().display().contains("pytest"));
    }

    #[test]
    fn go_module_gets_test_vet_and_gofmt() {
        let dir = project(&[("go.mod", "module exemplo\n\ngo 1.22\n")]);
        let stack = detect(dir.path()).stacks.remove(0);
        assert_eq!(stack.language, "go");
        assert_eq!(stack.test.as_ref().unwrap().display(), "go test ./...");
        assert_eq!(stack.lint.as_ref().unwrap().display(), "go vet ./...");
        assert_eq!(stack.format_check.as_ref().unwrap().display(), "gofmt -l .");
        assert_eq!(stack.build.as_ref().unwrap().display(), "go build ./...");
        assert_eq!(stack.test_filter, FilterStyle::Flag("-run"));
    }

    #[test]
    fn maven_and_gradle_and_dotnet_are_recognized() {
        let maven = project(&[("pom.xml", "<project/>")]);
        let stack = detect(maven.path()).stacks.remove(0);
        assert_eq!(stack.manager, "maven");
        assert_eq!(stack.test.as_ref().unwrap().display(), "mvn -B test");

        let gradle = project(&[("build.gradle.kts", ""), ("gradlew", "#!/bin/sh\n")]);
        let stack = detect(gradle.path()).stacks.remove(0);
        assert_eq!(stack.manager, "gradle");
        assert!(
            stack.test.as_ref().unwrap().program().contains("gradlew"),
            "o wrapper do repositório vem antes do gradle do sistema"
        );

        let net = project(&[("App.csproj", "<Project/>")]);
        let stack = detect(net.path()).stacks.remove(0);
        assert_eq!(stack.language, "dotnet");
        assert_eq!(stack.test.as_ref().unwrap().display(), "dotnet test");
        assert_eq!(
            stack.format_check.as_ref().unwrap().display(),
            "dotnet format --verify-no-changes"
        );
    }

    #[test]
    fn composer_and_gemfile_are_recognized() {
        let php = project(&[("composer.json", r#"{"scripts":{"test":"phpunit"}}"#)]);
        let stack = detect(php.path()).stacks.remove(0);
        assert_eq!(stack.language, "php");
        assert_eq!(stack.test.as_ref().unwrap().display(), "composer test");

        let ruby = project(&[("Gemfile", "source 'x'\n"), ("spec/a_spec.rb", "")]);
        let stack = detect(ruby.path()).stacks.remove(0);
        assert_eq!(stack.language, "ruby");
        assert_eq!(stack.test.as_ref().unwrap().display(), "bundle exec rspec");
    }

    #[test]
    fn mixed_project_finds_the_nested_stack_too() {
        // Layout Tauri: Node na raiz, Rust em src-tauri/.
        let dir = project(&[
            ("package.json", r#"{"scripts":{"build":"vite build"}}"#),
            ("src-tauri/Cargo.toml", "[package]\nname=\"app\"\n"),
            ("node_modules/pacote/package.json", r#"{"name":"pacote"}"#),
            ("target/debug/Cargo.toml", "[package]\nname=\"lixo\"\n"),
        ]);
        let info = detect(dir.path());
        assert_eq!(info.stacks.len(), 2, "stacks: {:?}", info.dirs());
        assert_eq!(info.primary().unwrap().language, "node");
        assert_eq!(info.primary().unwrap().dir, "");
        let rust = &info.stacks[1];
        assert_eq!(rust.language, "rust");
        assert_eq!(rust.dir, "src-tauri");
        assert_eq!(rust.manifest, "src-tauri/Cargo.toml");
        assert_eq!(info.languages(), vec!["node", "rust"]);
    }

    #[test]
    fn pick_prefers_whoever_actually_has_the_command() {
        // A raiz é Node sem script de teste; quem testa é o Rust de dentro.
        let dir = project(&[
            ("package.json", r#"{"scripts":{"build":"vite build"}}"#),
            ("src-tauri/Cargo.toml", "[package]\nname=\"app\"\n"),
        ]);
        let info = detect(dir.path());
        let chosen = info.pick(Task::Test, None).unwrap();
        assert_eq!(chosen.language, "rust");
        assert_eq!(chosen.dir, "src-tauri");
        // Build existe na raiz: continua sendo o Node.
        assert_eq!(info.pick(Task::Build, None).unwrap().language, "node");
    }

    #[test]
    fn pick_obeys_an_explicit_dir_in_any_notation() {
        let dir = project(&[
            ("package.json", "{}"),
            ("src-tauri/Cargo.toml", "[package]\nname=\"app\"\n"),
        ]);
        let info = detect(dir.path());
        for spelling in ["src-tauri", "./src-tauri", "src-tauri/", "src-tauri\\"] {
            let stack = info.pick(Task::Test, Some(spelling)).unwrap();
            assert_eq!(stack.language, "rust", "grafia: {spelling}");
        }
        assert_eq!(info.pick(Task::Test, Some(".")).unwrap().language, "node");
        assert!(info.pick(Task::Test, Some("nao-existe")).is_none());
    }

    #[test]
    fn filter_style_always_produces_a_separate_argument() {
        let base = Cmd::new(["cargo", "test"]);
        // O filtro vem do modelo: nunca pode virar sintaxe de shell.
        let hostile = "x && rm -rf .";
        let cmd = FilterStyle::Positional.apply(base.clone(), hostile);
        assert_eq!(cmd.argv, vec!["cargo", "test", hostile]);
        assert_eq!(
            FilterStyle::Flag("-k").apply(base.clone(), "soma").argv,
            vec!["cargo", "test", "-k", "soma"]
        );
        assert_eq!(
            FilterStyle::AfterDoubleDash
                .apply(base.clone(), "soma")
                .argv,
            vec!["cargo", "test", "--", "soma"]
        );
        assert_eq!(
            FilterStyle::Glued("-Dtest").apply(base, "Soma").argv,
            vec!["cargo", "test", "-Dtest=Soma"]
        );
    }

    #[test]
    fn display_quotes_arguments_with_spaces() {
        let cmd = Cmd::new(["pytest", "-k"]).plus(["soma e resto"]);
        assert_eq!(cmd.display(), "pytest -k \"soma e resto\"");
        assert_eq!(cmd.program(), "pytest");
        assert_eq!(cmd.args(), ["-k", "soma e resto"]);
    }

    #[test]
    fn normalize_dir_accepts_the_shapes_a_model_writes() {
        assert_eq!(normalize_dir("."), "");
        assert_eq!(normalize_dir("./src"), "src");
        assert_eq!(normalize_dir("src\\a"), "src/a");
        assert_eq!(normalize_dir(" src/ "), "src");
    }
}
