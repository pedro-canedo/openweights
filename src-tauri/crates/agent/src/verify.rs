//! Verificação barata do que o run afirma ter feito.
//!
//! Não usa modelo: é rápida, determinística e roda depois da resposta final.
//! A pergunta é simples — "o que ele disse que fez continua de pé?": os
//! arquivos escritos existem? algum comando terminou com erro?
//!
//! O objetivo não é auditar o trabalho, é pegar a mentira fácil (o modelo
//! afirma ter criado um arquivo que a ferramenta não criou).

use std::path::Path;

/// Um comando executado durante o run.
#[derive(Debug, Clone)]
pub struct CommandRecord {
    /// Linha como a pessoa a leria.
    pub display: String,
    /// A ferramenta terminou sem erro?
    pub ok: bool,
    /// Código de saída, quando a saída do comando o informou.
    pub exit_code: Option<i32>,
}

/// Resultado da verificação (vira o evento `verification`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    pub passed: bool,
    pub notes: String,
}

/// Confere arquivos escritos e comandos executados.
///
/// `None` quando não houve efeito colateral nenhum — nesse caso não vale
/// poluir a trilha com um evento vazio.
pub fn verify(
    workspace: Option<&Path>,
    written: &[String],
    commands: &[CommandRecord],
) -> Option<VerifyReport> {
    if written.is_empty() && commands.is_empty() {
        return None;
    }

    let mut notes: Vec<String> = Vec::new();
    let mut passed = true;

    // O MESMO comando rodado de novo supersede o registro antigo: um teste
    // que falhou e depois passou foi CONSERTADO — cobrar a falha histórica
    // condenaria exatamente o ciclo certo (rodar, corrigir, rodar de novo).
    let mut ultimos: Vec<&CommandRecord> = Vec::new();
    for cmd in commands {
        if let Some(pos) = ultimos.iter().position(|c| c.display == cmd.display) {
            ultimos[pos] = cmd;
        } else {
            ultimos.push(cmd);
        }
    }
    let commands = ultimos;

    if !written.is_empty() {
        match workspace {
            Some(root) => {
                let missing: Vec<&String> = written
                    .iter()
                    .filter(|rel| !root.join(rel).exists())
                    .collect();
                if missing.is_empty() {
                    notes.push(format!(
                        "{} arquivo(s) alterado(s) conferido(s) em disco.",
                        written.len()
                    ));
                } else {
                    passed = false;
                    notes.push(format!(
                        "não encontrei em disco: {}",
                        missing
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            None => notes.push(format!(
                "{} arquivo(s) alterado(s) fora de uma pasta de projeto — sem conferência.",
                written.len()
            )),
        }
    }

    for cmd in &commands {
        match (cmd.ok, cmd.exit_code) {
            (_, Some(0)) => notes.push(format!("`{}` terminou com código 0.", cmd.display)),
            (_, Some(code)) => {
                passed = false;
                notes.push(format!("`{}` terminou com código {code}.", cmd.display));
            }
            (false, None) => {
                passed = false;
                notes.push(format!("`{}` falhou.", cmd.display));
            }
            (true, None) => notes.push(format!("`{}` executou.", cmd.display)),
        }
    }

    if let Some(root) = workspace {
        for aviso in content_warnings(root, written) {
            notes.push(format!("aviso: {aviso}"));
        }
    }

    Some(VerifyReport {
        passed,
        notes: notes.join(" "),
    })
}

/// Comando que "passa" em qualquer máquina e não prova a entrega:
/// `node -v`, `ls`, `pwd`. Aceitá-lo como Definition of Done era o que
/// deixava a conferência verde com a pasta vazia.
pub(crate) fn trivial_check_cmd(cmd: &str) -> bool {
    let t = cmd.trim().to_lowercase();
    if t.is_empty() {
        return true;
    }
    let first = t.split_whitespace().next().unwrap_or("");
    matches!(
        first,
        "ls" | "dir" | "pwd" | "whoami" | "echo" | "true" | "date" | "clear" | "type"
    ) || t == "node -v"
        || t == "node --version"
        || t == "npm -v"
        || t == "npm --version"
        || t == "python --version"
        || t == "python -v"
        || t == "python3 --version"
        || t.starts_with("echo ")
}

/// Suspeitas baratas de arquivo truncado. São AVISOS: nenhuma reprova
/// sozinha, porque cada heurística tem exceção legítima — mas um HTML sem
/// `</html>` depois de um run que quase estourou o JSON merece uma linha.
fn content_warnings(root: &Path, written: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for rel in written {
        let caminho = root.join(rel);
        let Ok(texto) = std::fs::read_to_string(&caminho) else {
            continue; // binário ou ilegível: fora do alcance destas checagens
        };
        if texto.trim().is_empty() {
            out.push(format!("`{rel}` ficou vazio"));
            continue;
        }
        // Cerca de código aberta vale para qualquer texto.
        if texto.matches("```").count() % 2 == 1 {
            out.push(format!("`{rel}` tem cerca de código sem fechar"));
        }
        let ext = caminho
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "html" | "htm" => {
                if !texto.to_ascii_lowercase().contains("</html>") {
                    out.push(format!("`{rel}` não termina com </html>"));
                }
            }
            "rs" | "js" | "ts" | "tsx" | "jsx" | "json" | "c" | "cpp" | "java" | "css" => {
                let abre = texto.matches('{').count();
                let fecha = texto.matches('}').count();
                if abre != fecha {
                    out.push(format!(
                        "`{rel}` tem chaves desbalanceadas ({abre} abrem, {fecha} fecham)"
                    ));
                }
            }
            _ => {}
        }
    }
    out
}

/// Procura um código de saída na saída de um comando.
///
/// As ferramentas de terminal costumam terminar o texto com uma linha do tipo
/// `[código de saída: 1]`; sem essa marca, devolve `None` (nada de adivinhar).
pub fn extract_exit_code(text: &str) -> Option<i32> {
    const MARKERS: &[&str] = &[
        "código de saída",
        "codigo de saida",
        "exit code",
        "exit status",
    ];
    let lower = text.to_lowercase();
    // Reserva para ferramentas que não declaram o código no campo próprio
    // (`ToolOutput::exit_code`). Uma LINHA que começa com o marcador é o
    // formato do próprio harness e vence qualquer menção no meio do log —
    // era o `rfind` cru que pegava o "exit code 1" de dentro da saída de um
    // `cargo test` que tinha saído com 0.
    let de_linha = lower.lines().find_map(|l| {
        let l = l.trim_start();
        MARKERS
            .iter()
            .find_map(|m| l.strip_prefix(m))
            .and_then(|resto| {
                let n: String = resto
                    .chars()
                    .skip_while(|c| !c.is_ascii_digit() && *c != '-')
                    .take_while(|c| c.is_ascii_digit() || *c == '-')
                    .collect();
                n.parse().ok()
            })
    });
    if de_linha.is_some() {
        return de_linha;
    }
    let marker = MARKERS
        .iter()
        .filter_map(|m| lower.rfind(m).map(|i| i + m.len()))
        .max()?;
    let rest: String = lower[marker..]
        .chars()
        .skip_while(|c| !c.is_ascii_digit() && *c != '-')
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    rest.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_to_check_produces_no_event() {
        assert!(verify(None, &[], &[]).is_none());
    }

    #[test]
    fn written_files_must_exist() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notas.md"), "oi").unwrap();

        let ok = verify(Some(dir.path()), &["notas.md".to_string()], &[]).unwrap();
        assert!(ok.passed);
        assert!(ok.notes.contains("conferido"));

        let bad = verify(
            Some(dir.path()),
            &["notas.md".to_string(), "sumiu.md".to_string()],
            &[],
        )
        .unwrap();
        assert!(!bad.passed);
        assert!(bad.notes.contains("sumiu.md"));
    }

    #[test]
    fn a_failing_command_fails_the_verification() {
        let report = verify(
            None,
            &[],
            &[
                CommandRecord {
                    display: "cargo build".into(),
                    ok: true,
                    exit_code: Some(0),
                },
                CommandRecord {
                    display: "cargo test".into(),
                    ok: true,
                    exit_code: Some(101),
                },
            ],
        )
        .unwrap();
        assert!(!report.passed);
        assert!(report.notes.contains("código 101"));
        assert!(report.notes.contains("cargo build"));
    }

    #[test]
    fn version_and_listing_commands_are_not_proof() {
        assert!(trivial_check_cmd("node -v"));
        assert!(trivial_check_cmd("ls"));
        assert!(trivial_check_cmd("pwd"));
        assert!(trivial_check_cmd("echo ok"));
        assert!(!trivial_check_cmd("npm test"));
        assert!(!trivial_check_cmd("npx tsc --noEmit"));
    }

    /// Rodar de novo o MESMO comando supersede o registro antigo: o ciclo
    /// certo (teste vermelho → conserto → teste verde) não pode reprovar por
    /// causa da falha histórica.
    #[test]
    fn rerunning_the_same_command_supersedes_the_old_failure() {
        let dir = tempfile::tempdir().unwrap();
        let report = verify(
            Some(dir.path()),
            &[],
            &[
                CommandRecord {
                    display: "cargo test".into(),
                    ok: true,
                    exit_code: Some(101),
                },
                CommandRecord {
                    display: "cargo test".into(),
                    ok: true,
                    exit_code: Some(0),
                },
            ],
        )
        .unwrap();
        assert!(report.passed, "{}", report.notes);

        // Comandos DIFERENTES não se apagam.
        let outro = verify(
            Some(dir.path()),
            &[],
            &[
                CommandRecord {
                    display: "cargo test -p a".into(),
                    ok: true,
                    exit_code: Some(101),
                },
                CommandRecord {
                    display: "cargo test -p b".into(),
                    ok: true,
                    exit_code: Some(0),
                },
            ],
        )
        .unwrap();
        assert!(!outro.passed);
    }

    /// Suspeita de truncamento é AVISO, nunca reprova sozinha: cada
    /// heurística tem exceção legítima.
    #[test]
    fn truncation_suspicion_warns_without_failing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), "<html><body>oi</body>").unwrap();
        std::fs::write(dir.path().join("ok.rs"), "fn a() {}\n").unwrap();
        std::fs::write(dir.path().join("quebrado.rs"), "fn a() { if x {\n").unwrap();
        std::fs::write(dir.path().join("vazio.md"), "").unwrap();

        let report = verify(
            Some(dir.path()),
            &[
                "index.html".into(),
                "ok.rs".into(),
                "quebrado.rs".into(),
                "vazio.md".into(),
            ],
            &[],
        )
        .unwrap();
        assert!(report.passed, "aviso não reprova: {}", report.notes);
        assert!(report.notes.contains("</html>"), "{}", report.notes);
        assert!(report.notes.contains("desbalanceadas"), "{}", report.notes);
        assert!(report.notes.contains("vazio"), "{}", report.notes);
        assert!(!report.notes.contains("`ok.rs` tem"), "{}", report.notes);
    }

    #[test]
    fn exit_code_is_read_from_the_output_when_present() {
        assert_eq!(extract_exit_code("saída...\n[código de saída: 0]"), Some(0));
        assert_eq!(extract_exit_code("erro\n[exit code: 127]"), Some(127));
        assert_eq!(extract_exit_code("nenhuma marca aqui"), None);
        // O formato do harness põe o marcador NA PRIMEIRA LINHA e o stdout
        // depois. A linha que começa com o marcador vence qualquer menção no
        // meio do log — "valer a última" era exatamente o bug: um `cargo
        // test` que imprimia "exit code 2" no log derrubava um comando que
        // saiu com 0.
        assert_eq!(
            extract_exit_code("exit code 0\n\n[stdout]\nexit code 2 apareceu no log"),
            Some(0)
        );
        // Sem linha própria, a reserva antiga continua: vale a última menção.
        assert_eq!(
            extract_exit_code("rodei com exit code: 0 e depois exit code: 2"),
            Some(2)
        );
    }
}
