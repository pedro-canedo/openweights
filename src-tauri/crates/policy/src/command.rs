//! Classificação determinística de comandos de terminal.
//!
//! Regra de ouro: **na dúvida, `Opaque`** — e `Opaque` sempre pede
//! confirmação, mesmo em modo automático. Nunca tentamos "entender" um
//! comando complicado; se ele não cai numa forma simples e reconhecível,
//! quem decide é a pessoa.
//!
//! Encadeamento (`|`, `&&`, `||`, `;`) é a exceção que a prática cobrou:
//! tratá-lo como inanalisável fazia `ls -la | head` parar o agente para pedir
//! confirmação — inclusive no modo automático, onde a pessoa pediu para não
//! ser interrompida. Como a linha continua sendo analisável **peça por
//! peça**, cada trecho é classificado sozinho e vale o mais restritivo. O que
//! esconde o comando de verdade (`eval`, `$(...)`, base64, um shell chamando
//! outro) segue `Opaque`, que é onde a regra de ouro tem valor.
//!
//! Isso protege o usuário de erros do modelo (apagar a pasta errada, rodar um
//! script que ele não leu), não é uma barreira contra um usuário mal
//! intencionado — quem aprova o comando é quem manda.

use lr_types::agent::CommandClass;

/// Comandos que só leem estado e são seguros de rodar sem perguntar.
/// Mantido curto de propósito: cada entrada aqui é uma promessa.
const READ_ONLY: &[&str] = &[
    // navegação / inspeção
    "ls", "dir", "pwd", "cd", "cat", "type", "head", "tail", "wc", "file", "stat", "tree", "find",
    "findstr", "grep", "rg", "which", "where", "echo", "date", "whoami", "hostname",
    // versões / ambiente (leitura pura)
    "node", "npm", "pnpm", "yarn", "python", "python3", "pip", "cargo", "rustc", "go", "java",
    "dotnet", "git", "docker", "kubectl",
];

/// Subcomandos de leitura para ferramentas que também têm modo de escrita.
/// (chave = programa, valores = subcomandos comprovadamente somente-leitura)
const READ_ONLY_SUBCOMMANDS: &[(&str, &[&str])] = &[
    (
        "git",
        &[
            "status",
            "log",
            "diff",
            "show",
            "branch",
            "remote",
            "config",
            "blame",
            "describe",
            "rev-parse",
            "ls-files",
            "shortlog",
            "tag",
        ],
    ),
    (
        "npm",
        &["ls", "list", "view", "outdated", "why", "root", "config"],
    ),
    ("pnpm", &["ls", "list", "why", "outdated", "root"]),
    ("yarn", &["list", "why", "info"]),
    ("cargo", &["tree", "metadata", "search", "--version", "-V"]),
    ("pip", &["list", "show", "freeze"]),
    ("docker", &["ps", "images", "logs", "inspect", "version"]),
    ("kubectl", &["get", "describe", "logs", "version"]),
    ("go", &["version", "env", "list"]),
    ("dotnet", &["--version", "--info", "--list-sdks"]),
    ("node", &["--version", "-v"]),
    ("python", &["--version", "-V"]),
    ("python3", &["--version", "-V"]),
    ("java", &["--version", "-version"]),
    ("rustc", &["--version", "-V"]),
];

/// Sinais de que o comando faz algo que não conseguimos inspecionar.
/// A presença de qualquer um força `Opaque`.
const OPAQUE_MARKERS: &[&str] = &[
    "invoke-expression",
    "iex ",
    "-encodedcommand",
    "-enc ",
    "frombase64string",
    "downloadstring",
    "start-process",
    "eval ",
    "exec ",
    "source ",
    "$(",
    "`",
    "\n",
];

/// Encadeadores que dá para analisar peça por peça, do mais longo para o mais
/// curto (senão `||` seria lido como dois `|`).
const ENCADEADORES: &[&str] = &["&&", "||", ";", "|"];

/// Programas que exigem confirmação sempre (efeito difícil de desfazer),
/// mesmo quando a linha parece simples.
const ALWAYS_ASK: &[&str] = &[
    "rm", "rmdir", "del", "erase", "format", "mkfs", "dd", "shutdown", "reboot", "reg", "regedit",
    "sc", "net", "netsh", "takeown", "icacls", "attrib", "schtasks", "wmic", "diskpart", "bcdedit",
    "sudo", "runas", "chmod", "chown",
];

/// Resultado da análise de uma linha de comando.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandAnalysis {
    pub class: CommandClass,
    /// Primeiro token (o programa em si), normalizado em minúsculas.
    pub program: String,
    /// Motivo legível — vai para a UI quando pedimos confirmação.
    pub reason: String,
}

/// Analisa uma linha de comando e devolve sua classificação.
///
/// Não executa nada, não expande variáveis, não resolve caminhos: é uma
/// análise sintática conservadora.
pub fn classify(command: &str) -> CommandAnalysis {
    let trimmed = command.trim();
    let lower = trimmed.to_lowercase();

    if trimmed.is_empty() {
        return CommandAnalysis {
            class: CommandClass::Opaque,
            program: String::new(),
            reason: "comando vazio".into(),
        };
    }

    // Encadeamentos, redireções, substituição de comando e execução dinâmica
    // deixam de ser analisáveis linha a linha.
    if let Some(marker) = OPAQUE_MARKERS.iter().find(|m| lower.contains(**m)) {
        return CommandAnalysis {
            class: CommandClass::Opaque,
            program: first_token(&lower),
            reason: format!(
                "contém `{}` — não dá para analisar por completo",
                marker.trim()
            ),
        };
    }

    // Encadeamento: cada trecho é um comando analisável, e vale o pior deles.
    let partes = separar(trimmed);
    if partes.len() > 1 {
        return combinar(&partes);
    }

    let tokens = tokenize(trimmed);
    let program = tokens
        .first()
        .map(|t| normalize_program(t))
        .unwrap_or_default();

    if program.is_empty() {
        return CommandAnalysis {
            class: CommandClass::Opaque,
            program,
            reason: "não foi possível identificar o programa".into(),
        };
    }

    if ALWAYS_ASK.contains(&program.as_str()) {
        return CommandAnalysis {
            class: CommandClass::Mutating,
            reason: format!("`{program}` pode alterar o sistema de forma difícil de desfazer"),
            program,
        };
    }

    // Shells só são analisáveis quando chamados sem script (`-c`, `/c`...).
    if matches!(
        program.as_str(),
        "cmd" | "powershell" | "pwsh" | "bash" | "sh" | "zsh" | "wsl"
    ) {
        return CommandAnalysis {
            class: CommandClass::Opaque,
            reason: format!("`{program}` executa outro comando por dentro"),
            program,
        };
    }

    let sub = tokens.get(1).map(|s| s.to_lowercase());
    if let Some(subs) = READ_ONLY_SUBCOMMANDS
        .iter()
        .find(|(p, _)| *p == program)
        .map(|(_, s)| *s)
    {
        return match sub.as_deref() {
            Some(s) if subs.contains(&s) => CommandAnalysis {
                class: CommandClass::ReadOnly,
                reason: format!("`{program} {s}` só lê informações"),
                program,
            },
            _ => CommandAnalysis {
                class: CommandClass::Mutating,
                reason: format!("`{program}` pode alterar arquivos ou dependências"),
                program,
            },
        };
    }

    if READ_ONLY.contains(&program.as_str()) {
        // `ls > lista.txt` lê o diretório mas ESCREVE um arquivo: continua
        // analisável (e por isso não é `Opaque`), só não é mais leitura.
        if escreve_em_arquivo(trimmed) {
            return CommandAnalysis {
                class: CommandClass::Mutating,
                reason: format!("`{program}` grava a saída em um arquivo"),
                program,
            };
        }
        return CommandAnalysis {
            class: CommandClass::ReadOnly,
            reason: format!("`{program}` só lê informações"),
            program,
        };
    }

    CommandAnalysis {
        class: CommandClass::Mutating,
        reason: format!("`{program}` não está na lista de comandos só de leitura"),
        program,
    }
}

/// Divide respeitando aspas simples/duplas (sem interpretar escapes).
/// A linha precisa de um shell para fazer o que está escrito?
///
/// Pergunta DIFERENTE de "dá para analisar". Antes as duas andavam juntas
/// (`Opaque` significava as duas coisas), e separar a classificação sem
/// separar isto faria `ls | head` rodar sem shell — o `|` viraria argumento
/// do `ls`. Aqui a resposta é sintática: metacaractere fora de aspas.
pub fn needs_shell(command: &str) -> bool {
    const META: &[char] = &[
        '|', '&', ';', '<', '>', '$', '`', '*', '?', '(', ')', '{', '}', '\n',
    ];
    let mut quote: Option<char> = None;
    for c in command.chars() {
        match (quote, c) {
            (Some(q), x) if x == q => quote = None,
            (Some(_), _) => {}
            (None, x @ ('"' | '\'')) => quote = Some(x),
            (None, x) if META.contains(&x) => return true,
            (None, _) => {}
        }
    }
    // Embutidos do shell não existem como programa.
    matches!(
        first_token(&command.to_lowercase()).as_str(),
        "exit" | "cd" | "export" | "set" | "unset" | "alias" | "type"
    )
}

/// Quebra a linha nos encadeadores, respeitando aspas.
///
/// `grep "a | b" arquivo` é UM comando: o `|` dentro das aspas é texto.
fn separar(cmd: &str) -> Vec<String> {
    let mut partes = Vec::new();
    let mut atual = String::new();
    let mut quote: Option<char> = None;
    let bytes: Vec<char> = cmd.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match (quote, c) {
            (Some(q), x) if x == q => {
                quote = None;
                atual.push(x);
                i += 1;
            }
            (Some(_), x) => {
                atual.push(x);
                i += 1;
            }
            (None, x @ ('"' | '\'')) => {
                quote = Some(x);
                atual.push(x);
                i += 1;
            }
            (None, _) => {
                let resto: String = bytes[i..].iter().collect();
                match ENCADEADORES.iter().find(|e| resto.starts_with(**e)) {
                    // `2>&1` traz um `&` que não encadeia nada.
                    Some(sep) if !(*sep == "&&" && resto.starts_with("&&1")) => {
                        partes.push(std::mem::take(&mut atual));
                        i += sep.len();
                    }
                    _ => {
                        atual.push(c);
                        i += 1;
                    }
                }
            }
        }
    }
    partes.push(atual);
    partes
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// O pior trecho manda: uma linha é tão segura quanto sua pior peça.
fn combinar(partes: &[String]) -> CommandAnalysis {
    let mut pior = CommandAnalysis {
        class: CommandClass::ReadOnly,
        program: String::new(),
        reason: String::new(),
    };
    for parte in partes {
        let a = classify(parte);
        let sobe = match (pior.class, a.class) {
            (CommandClass::Opaque, _) => false,
            (_, CommandClass::Opaque) => true,
            (CommandClass::ReadOnly, CommandClass::Mutating) => true,
            _ => pior.program.is_empty(),
        };
        if sobe {
            pior = a;
        }
    }
    let programas: Vec<&str> = partes
        .iter()
        .map(|p| p.split_whitespace().next().unwrap_or(""))
        .collect();
    pior.reason = format!(
        "encadeamento de {} comandos ({}); o mais sensível é: {}",
        partes.len(),
        programas.join(", "),
        pior.reason
    );
    pior
}

/// A linha manda a saída para um arquivo? (`>` fora de aspas, e não `2>&1`.)
fn escreve_em_arquivo(cmd: &str) -> bool {
    let mut quote: Option<char> = None;
    let chars: Vec<char> = cmd.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        match (quote, c) {
            (Some(q), x) if x == q => quote = None,
            (Some(_), _) => {}
            (None, x @ ('"' | '\'')) => quote = Some(x),
            (None, '>') => {
                // `>&1`, `>&2`: redireciona para outro descritor, não arquivo.
                if chars.get(i + 1) != Some(&'&') {
                    return true;
                }
            }
            (None, _) => {}
        }
    }
    false
}

fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in s.chars() {
        match (quote, ch) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, c @ ('"' | '\'')) => quote = Some(c),
            (None, c) if c.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `C:\Program Files\git\git.exe` → `git`; `./scripts/build.sh` → `build.sh`.
fn normalize_program(token: &str) -> String {
    let base = token
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(token)
        .to_lowercase();
    base.strip_suffix(".exe")
        .or_else(|| base.strip_suffix(".cmd"))
        .or_else(|| base.strip_suffix(".bat"))
        .unwrap_or(&base)
        .to_string()
}

fn first_token(s: &str) -> String {
    s.split_whitespace()
        .next()
        .map(normalize_program)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(cmd: &str) -> CommandClass {
        classify(cmd).class
    }

    #[test]
    fn plain_read_only_commands() {
        for cmd in [
            "ls -la",
            "dir",
            "cat README.md",
            "git status",
            "git log --oneline -5",
        ] {
            assert_eq!(class(cmd), CommandClass::ReadOnly, "{cmd}");
        }
    }

    #[test]
    fn mutating_commands() {
        for cmd in [
            "npm install",
            "cargo build",
            "git commit -m x",
            "git push",
            "mkdir nova",
        ] {
            assert_eq!(class(cmd), CommandClass::Mutating, "{cmd}");
        }
    }

    #[test]
    fn destructive_programs_are_never_read_only() {
        for cmd in [
            "rm -rf pasta",
            "del arquivo.txt",
            "reg add HKLM\\x",
            "sudo apt install x",
        ] {
            assert_eq!(class(cmd), CommandClass::Mutating, "{cmd}");
        }
    }

    #[test]
    fn dynamic_execution_is_opaque() {
        for cmd in [
            "powershell -EncodedCommand aGk=",
            "Invoke-Expression $x",
            "ls $(whoami)",
            "echo `date`",
            // Um shell no meio do encadeamento esconde o comando de verdade.
            "echo oi | sh",
            "cat script.sh | bash",
        ] {
            assert_eq!(class(cmd), CommandClass::Opaque, "{cmd}");
        }
    }

    /// Encadeamento é analisável peça por peça, e vale a pior peça.
    ///
    /// Tratar todo `|` como inanalisável custava caro na prática: `ls | head`
    /// parava o agente para pedir confirmação até no modo automático, onde a
    /// pessoa pediu explicitamente para não ser interrompida.
    #[test]
    fn a_chain_is_as_safe_as_its_worst_link() {
        assert_eq!(class("ls -la . | head -20"), CommandClass::ReadOnly);
        assert_eq!(class("git status | grep modified"), CommandClass::ReadOnly);
        assert_eq!(class("cargo test 2>&1 | tail -30"), CommandClass::Mutating);
        assert_eq!(class("git status && rm x"), CommandClass::Mutating);
        assert_eq!(class("ls; npm install"), CommandClass::Mutating);
        // Aspas não encadeiam nada: é um comando só.
        let uma_so = classify(r#"grep "a | b" arquivo.txt"#);
        assert_eq!(uma_so.class, CommandClass::ReadOnly);
        assert_eq!(uma_so.program, "grep");
    }

    /// Redirecionar a saída para um arquivo é escrita — analisável, e por
    /// isso não é `Opaque`, mas também não é leitura.
    #[test]
    fn writing_to_a_file_is_not_read_only() {
        assert_eq!(class("cat a > b"), CommandClass::Mutating);
        assert_eq!(class("ls > lista.txt"), CommandClass::Mutating);
        // `2>&1` redireciona descritor, não arquivo.
        assert_eq!(class("git status 2>&1"), CommandClass::ReadOnly);
    }

    #[test]
    fn shells_are_opaque_even_alone() {
        assert_eq!(class("bash"), CommandClass::Opaque);
        assert_eq!(class("cmd /c dir"), CommandClass::Opaque);
    }

    #[test]
    fn program_path_is_normalized() {
        let a = classify(r"C:\tools\Git\bin\git.exe status");
        assert_eq!(a.program, "git");
        assert_eq!(a.class, CommandClass::ReadOnly);

        // Caminho com espaço e sem aspas é ambíguo de verdade: o primeiro
        // token não é o programa. Ficar conservador é o comportamento certo.
        let b = classify(r"C:\Program Files\Git\bin\git.exe status");
        assert_eq!(b.class, CommandClass::Mutating);

        // Com aspas, volta a ser analisável.
        let c = classify(r#""C:\Program Files\Git\bin\git.exe" status"#);
        assert_eq!(c.program, "git");
        assert_eq!(c.class, CommandClass::ReadOnly);
    }

    #[test]
    fn quoted_paths_do_not_split_the_program() {
        let a = classify("\"C:\\meus arquivos\\tool.exe\" --version");
        assert_eq!(a.program, "tool");
        assert_eq!(a.class, CommandClass::Mutating);
    }

    #[test]
    fn empty_or_weird_is_opaque() {
        assert_eq!(class(""), CommandClass::Opaque);
        assert_eq!(class("   "), CommandClass::Opaque);
    }

    #[test]
    fn subcommand_gate_is_per_program() {
        // `npm ls` lê, `npm run` não.
        assert_eq!(class("npm ls"), CommandClass::ReadOnly);
        assert_eq!(class("npm run build"), CommandClass::Mutating);
        // Sem subcomando, ferramenta com modos vira Mutating (conservador).
        assert_eq!(class("git"), CommandClass::Mutating);
    }

    #[test]
    fn reason_is_human_readable() {
        assert!(classify("git status").reason.contains("git status"));
        let cadeia = classify("ls && rm x").reason;
        assert!(cadeia.contains("encadeamento"), "{cadeia}");
        assert!(cadeia.contains("rm"), "{cadeia}");
    }
}
