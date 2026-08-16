//! Apoio aos testes deste crate.
//!
//! Metade das ferramentas daqui só faz sentido com um interpretador ou um
//! compilador instalado, e a máquina de CI de cada pessoa tem um conjunto
//! diferente. A regra que seguimos: **teste que depende de programa externo
//! detecta a ausência e sai avisando, nunca falha vermelho** — um `node` que
//! não existe não é bug do nosso código, e um vermelho por isso ensina a
//! ignorar a suíte.

/// O programa existe no PATH?
///
/// Procura na mão em vez de rodar `which`/`where`: um `spawn` a mais por teste
/// é lento e, ironicamente, também dependeria de um programa externo.
pub fn program_exists(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    // No Windows o executável tem extensão; no resto, o nome é o nome.
    let names: Vec<String> = if cfg!(windows) {
        ["", ".exe", ".cmd", ".bat"]
            .iter()
            .map(|ext| format!("{program}{ext}"))
            .collect()
    } else {
        vec![program.to_string()]
    };

    std::env::split_paths(&path).any(|dir| names.iter().any(|name| dir.join(name).is_file()))
}

/// Anuncia que o teste foi pulado e por quê (aparece com `cargo test -- --nocapture`).
pub fn skip(program: &str) -> bool {
    eprintln!("pulando: `{program}` não está instalado nesta máquina");
    true
}

/// Nome de pacote único para um projeto Rust de teste.
///
/// Os testes daqui rodam `cargo` de verdade em pastas temporárias, e todos
/// herdam o mesmo `CARGO_TARGET_DIR` do ambiente. Dois projetos com o **mesmo
/// nome e versão** podem então reaproveitar o artefato um do outro: o teste
/// que espera suíte vermelha acaba lendo o binário verde do vizinho, e a falha
/// aparece de forma intermitente — o pior tipo de teste instável. Nome único
/// por projeto elimina a colisão na origem.
pub fn unique_package(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or_default();
    format!("{prefix}_{}_{n}_{nanos}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_program_that_surely_exists_and_not_one_that_does_not() {
        // `cargo` está rodando esta suíte agora mesmo.
        assert!(program_exists("cargo"), "cargo deveria estar no PATH");
        assert!(!program_exists("programa-que-nunca-existiu-xyz"));
    }
}
