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

    for cmd in commands {
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

    Some(VerifyReport {
        passed,
        notes: notes.join(" "),
    })
}

/// Procura um código de saída na saída de um comando.
///
/// As ferramentas de terminal costumam terminar o texto com uma linha do tipo
/// `[código de saída: 1]`; sem essa marca, devolve `None` (nada de adivinhar).
pub fn extract_exit_code(text: &str) -> Option<i32> {
    const MARKERS: &[&str] = &["código de saída", "codigo de saida", "exit code", "exit status"];
    let lower = text.to_lowercase();
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

        let ok = verify(
            Some(dir.path()),
            &["notas.md".to_string()],
            &[],
        )
        .unwrap();
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
    fn exit_code_is_read_from_the_output_when_present() {
        assert_eq!(extract_exit_code("saída...\n[código de saída: 0]"), Some(0));
        assert_eq!(extract_exit_code("erro\n[exit code: 127]"), Some(127));
        assert_eq!(extract_exit_code("nenhuma marca aqui"), None);
        // Duas marcas: vale a última (a do comando que acabou de rodar).
        assert_eq!(
            extract_exit_code("exit code: 0\n...\nexit code: 2"),
            Some(2)
        );
    }
}
