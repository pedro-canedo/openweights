//! Alguma sessão está rodando o executável de uma versão do runtime?
//!
//! O `auth.command` de uma sessão do OwCLI é o caminho absoluto do executável
//! dela, chamado de novo a cada minuto. Um terminal do sistema aberto antes de
//! uma atualização ainda roda a versão velha: apagar a pasta dela mataria o
//! token da sessão no meio do trabalho. Por isso a poda deixa em paz o que está
//! em uso. No Windows o sinal é o `.exe` não abrir para escrita.

use std::path::Path;

/// Algum processo tem o executável dentro de `pasta`?
pub fn em_uso(pasta: &Path) -> bool {
    if exe_em_uso(pasta) {
        return true;
    }
    let pasta = std::fs::canonicalize(pasta).unwrap_or_else(|_| pasta.to_path_buf());
    executaveis_em_execucao()
        .iter()
        .any(|e| e.starts_with(&pasta))
}

#[cfg(target_os = "linux")]
fn executaveis_em_execucao() -> Vec<std::path::PathBuf> {
    let Ok(procs) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    procs
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .bytes()
                .all(|b| b.is_ascii_digit())
        })
        // `readlink` falha para processo de outro usuário: ele não é nosso.
        .filter_map(|e| std::fs::read_link(e.path().join("exe")).ok())
        .collect()
}

#[cfg(target_os = "macos")]
fn executaveis_em_execucao() -> Vec<std::path::PathBuf> {
    let mut cmd = std::process::Command::new("ps");
    cmd.args(["-axo", "comm="])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    lr_proc::host_env_std(&mut cmd);
    lr_proc::no_window_std(&mut cmd);
    cmd.output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| std::path::PathBuf::from(l.trim()))
                // `comm` de um processo aberto pelo nome vem sem pasta.
                .filter(|p| p.is_absolute())
                // Uma sessão aberta pelo link em ~/.local/bin aparece com o
                // caminho do link: o que conta é o executável de verdade.
                .map(|p| std::fs::canonicalize(&p).unwrap_or(p))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn executaveis_em_execucao() -> Vec<std::path::PathBuf> {
    Vec::new()
}

/// No Windows não há lista barata de executáveis: um `.exe` em uso não abre
/// para escrita (violação de compartilhamento), e é esse o sinal. Abrir sem
/// criar nem truncar não altera o arquivo.
#[cfg(windows)]
fn exe_em_uso(pasta: &Path) -> bool {
    let exe = pasta.join("bin").join("owcli.exe");
    match std::fs::OpenOptions::new().write(true).open(&exe) {
        Ok(_) => false,
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

#[cfg(not(windows))]
fn exe_em_uso(_: &Path) -> bool {
    false
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn uma_pasta_so_esta_em_uso_enquanto_um_processo_roda_de_dentro_dela() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("v1/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let sleep = ["/usr/bin/sleep", "/bin/sleep"]
            .into_iter()
            .find(|p| Path::new(p).is_file())
            .expect("sleep");
        std::fs::copy(sleep, bin.join("owcli")).unwrap();
        assert!(!em_uso(&tmp.path().join("v1")));

        let mut filho = std::process::Command::new(bin.join("owcli"))
            .arg("30")
            .spawn()
            .unwrap();
        assert!(
            em_uso(&tmp.path().join("v1")),
            "o processo roda de dentro dela"
        );
        assert!(!em_uso(&tmp.path().join("v2")), "outra pasta não");
        filho.kill().unwrap();
        filho.wait().unwrap();
        assert!(!em_uso(&tmp.path().join("v1")));
    }
}
