//! Encerrar a árvore de um terminal.
//!
//! O `kill(-pid)` do [`lr_proc::kill_process_tree`] não basta aqui: o
//! portable-pty põe o filho como líder de uma SESSÃO nova, e um shell com
//! controle de jobs coloca cada pipeline num grupo de processos próprio. Um
//! `npm run dev &` iniciado no shell está na sessão, mas não no grupo do
//! shell. Por isso, no Unix, quem mora na sessão é achado pelo id da sessão
//! (que é o pid do líder).
//!
//! No Windows o filho do ConPTY vai para um Job Object com
//! `KILL_ON_JOB_CLOSE` na abertura; encerrar é terminar o job.

#[cfg(unix)]
pub(crate) fn sinalizar_grupo(pid: u32, sinal: i32) {
    // O líder da sessão também lidera o próprio grupo (pgid == pid).
    unsafe {
        libc::killpg(pid as libc::pid_t, sinal);
    }
}

/// Todos os processos cuja sessão é `sid`.
#[cfg(target_os = "linux")]
pub(crate) fn membros_da_sessao(sid: u32) -> Vec<u32> {
    let Ok(entradas) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entradas
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter(|&pid| sessao_de(pid) == Some(sid))
        .collect()
}

/// Campo `session` de `/proc/<pid>/stat`. O nome do programa (campo 2) vem
/// entre parênteses e pode ter espaço e parêntese: conta-se depois do último `)`.
#[cfg(target_os = "linux")]
fn sessao_de(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let depois = &stat[stat.rfind(')')? + 1..];
    // estado ppid pgrp session …
    depois.split_whitespace().nth(3)?.parse().ok()
}

#[cfg(target_os = "macos")]
pub(crate) fn membros_da_sessao(sid: u32) -> Vec<u32> {
    let mut cmd = std::process::Command::new("/bin/ps");
    cmd.args(["-ax", "-o", "pid="]);
    lr_proc::host_env_std(&mut cmd);
    lr_proc::no_window_std(&mut cmd);
    let Ok(saida) = cmd.output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&saida.stdout)
        .split_whitespace()
        .filter_map(|p| p.parse::<u32>().ok())
        .filter(|&pid| unsafe { libc::getsid(pid as libc::pid_t) } == sid as libc::pid_t)
        .collect()
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
pub(crate) fn membros_da_sessao(_sid: u32) -> Vec<u32> {
    Vec::new()
}

/// SIGKILL em todo mundo da sessão, líder incluído.
#[cfg(unix)]
pub(crate) fn matar_sessao(sid: u32) {
    for pid in membros_da_sessao(sid) {
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
    }
    sinalizar_grupo(sid, libc::SIGKILL);
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn a_propria_sessao_contem_o_proprio_processo() {
        let eu = std::process::id();
        let sid = sessao_de(eu).expect("o /proc/self/stat tem sessão");
        assert!(membros_da_sessao(sid).contains(&eu));
    }
}
