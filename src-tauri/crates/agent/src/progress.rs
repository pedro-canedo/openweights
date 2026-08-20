//! Resumo curto da execução em `.openweights/progress.md`.
//!
//! O modelo local esquece o que não cabe na janela. Este arquivo é a memória
//! em disco: o que já rodou, o que falta, o que travou. A próxima etapa (e o
//! próximo run) lê isto em vez de depender só do contexto.

use std::io;
use std::path::Path;

/// Caminho relativo à pasta do projeto.
pub const REL: &str = ".openweights/progress.md";

pub struct ProgressNote<'a> {
    pub run_id: &'a str,
    pub goal: &'a str,
    pub done: &'a str,
    pub files: &'a [String],
    pub next: &'a str,
    pub blockers: &'a str,
}

/// Grava (substitui) o resumo atual. A pasta só nasce quando há o que guardar.
pub fn write_progress(workspace: &Path, note: &ProgressNote<'_>) -> io::Result<()> {
    let dir = workspace.join(".openweights");
    std::fs::create_dir_all(&dir)?;
    let arquivos = if note.files.is_empty() {
        "(nenhum arquivo escrito nesta fatia)".to_string()
    } else {
        note.files
            .iter()
            .map(|f| format!("- {f}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let blockers = note.blockers.trim();
    let body = format!(
        "# Progresso do agente\n\n\
         Run: `{run}`\n\n\
         ## Pedido (recorte)\n\n{goal}\n\n\
         ## O que já executou\n\n{done}\n\n\
         ## Arquivos tocados\n\n{arquivos}\n\n\
         ## Próximo passo\n\n{next}\n\n\
         ## Bloqueios\n\n{blockers}\n",
        run = note.run_id,
        goal = clip(note.goal, 500),
        done = clip(note.done, 800),
        next = clip(note.next, 400),
        blockers = if blockers.is_empty() {
            "(nenhum)"
        } else {
            blockers
        },
    );
    std::fs::write(workspace.join(REL), body)
}

pub fn read_progress(workspace: &Path) -> Option<String> {
    std::fs::read_to_string(workspace.join(REL)).ok()
}

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_file_lands_under_openweights() {
        let dir = tempfile::tempdir().unwrap();
        write_progress(
            dir.path(),
            &ProgressNote {
                run_id: "r1",
                goal: "jogo moba",
                done: "abriu a pasta",
                files: &["src/app/page.tsx".into()],
                next: "cena 3d",
                blockers: "",
            },
        )
        .unwrap();
        let texto = read_progress(dir.path()).expect("gravou");
        assert!(texto.contains("src/app/page.tsx"));
        assert!(texto.contains("cena 3d"));
        assert!(dir.path().join(".openweights").join("progress.md").is_file());
    }
}
