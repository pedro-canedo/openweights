//! Ponte com `lr_checkpoint`: tirar a foto antes da primeira alteração e
//! reconstruir um ponto de restauração guardado no banco.
//!
//! Duas sutilezas moram aqui:
//! 1. as operações são síncronas e mexem em disco (git, cópia) — quem chama
//!    do laço assíncrono precisa jogá-las numa thread de bloqueio;
//! 2. restaurar tem que usar o MESMO backend que gravou. Escolher de novo
//!    pelo ambiente (`engine_for`) pode devolver git para um checkpoint de
//!    cópia e a restauração falha.

use lr_checkpoint::{Checkpoint, CheckpointEngine, CheckpointError, CopySnapshot, GitShadow};
use std::path::Path;

/// Nome do arquivo que o backend de cópia grava com a lista salva.
const COPY_MANIFEST: &str = "_checkpoint.json";

/// Motor certo para um checkpoint já gravado.
fn engine_for_backend(
    backend: &str,
    workspace: &Path,
    store_dir: &Path,
) -> Box<dyn CheckpointEngine> {
    if backend == "git"
        && let Ok(git) = GitShadow::new(workspace, store_dir)
    {
        return Box::new(git);
    }
    Box::new(CopySnapshot::new(workspace, store_dir))
}

/// Tira a foto antes de uma alteração (bloqueante).
pub fn snapshot_blocking(
    workspace: &Path,
    store_dir: &Path,
    label: &str,
    files: &[String],
) -> Result<Checkpoint, CheckpointError> {
    let engine = lr_checkpoint::engine_for(workspace, store_dir)?;
    engine.snapshot(label, files)
}

/// Recompõe o [`Checkpoint`] a partir do que o banco guarda.
///
/// A lista de arquivos não volta na linha do banco; no backend de cópia ela
/// está no manifesto dentro da própria pasta do snapshot (no git ela não é
/// usada).
pub fn checkpoint_from_row(id: &str, backend: &str, ref_id: &str, label: Option<&str>) -> Checkpoint {
    let files = if backend == "copy" {
        std::fs::read_to_string(Path::new(ref_id).join(COPY_MANIFEST))
            .ok()
            .and_then(|body| serde_json::from_str::<Vec<String>>(&body).ok())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    Checkpoint {
        id: id.to_string(),
        backend: backend.to_string(),
        ref_id: ref_id.to_string(),
        label: label.unwrap_or_default().to_string(),
        files,
    }
}

/// Devolve os arquivos ao estado do checkpoint (bloqueante).
pub fn restore_blocking(
    workspace: &Path,
    store_dir: &Path,
    cp: &Checkpoint,
) -> Result<Vec<String>, CheckpointError> {
    engine_for_backend(&cp.backend, workspace, store_dir).restore(cp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_and_restore_survive_a_round_trip_through_the_database() {
        let ws = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        std::fs::write(ws.path().join("a.txt"), "original").unwrap();

        let cp = snapshot_blocking(ws.path(), store.path(), "antes", &["a.txt".into()]).unwrap();
        std::fs::write(ws.path().join("a.txt"), "estragado").unwrap();

        // Só o que a tabela `checkpoints` guarda volta para as mãos do app.
        let rebuilt = checkpoint_from_row(&cp.id, &cp.backend, &cp.ref_id, Some(&cp.label));
        restore_blocking(ws.path(), store.path(), &rebuilt).unwrap();

        assert_eq!(
            std::fs::read_to_string(ws.path().join("a.txt")).unwrap(),
            "original"
        );
    }

    #[test]
    fn copy_backend_recovers_its_file_list_from_the_manifest() {
        let ws = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        std::fs::write(ws.path().join("b.txt"), "v1").unwrap();

        let engine = CopySnapshot::new(ws.path(), store.path());
        let cp = engine.snapshot("antes", &["b.txt".into()]).unwrap();

        let rebuilt = checkpoint_from_row(&cp.id, "copy", &cp.ref_id, None);
        assert_eq!(rebuilt.files, vec!["b.txt".to_string()]);

        std::fs::write(ws.path().join("b.txt"), "v2").unwrap();
        let restored = restore_blocking(ws.path(), store.path(), &rebuilt).unwrap();
        assert_eq!(restored, vec!["b.txt".to_string()]);
        assert_eq!(
            std::fs::read_to_string(ws.path().join("b.txt")).unwrap(),
            "v1"
        );
    }
}
