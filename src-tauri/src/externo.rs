//! Abrir link e pasta no programa padrão do sistema.
//!
//! No Linux o `tauri-plugin-opener` chama o `xdg-open` que o bundler do Tauri
//! põe DENTRO do AppImage, herdando o ambiente do pacote: o script cai no
//! `gio` do sistema, que morre contra a glib embutida, depois no navegador e
//! no gerenciador de arquivos, que morrem do mesmo jeito — e o spawn devolve
//! Ok, então nada abre e nenhum erro aparece. Aqui o `xdg-open` é o do
//! sistema, com o ambiente do sistema (`lr_proc::host_env_std`). Nos outros
//! sistemas o plugin continua sendo o caminho.
//!
//! Terceiros que criam processo e o `lr_proc` não alcança: o `open` (via
//! plugin), trocado por este módulo; o fallback zenity do rfd, trocado pelo
//! seletor próprio em `workspace.rs`; e o `restart` do Tauri depois de uma
//! atualização, cujo resto (entradas do mount antigo) o `lr_proc` poda.

use tauri::AppHandle;
#[cfg(not(target_os = "linux"))]
use tauri_plugin_opener::OpenerExt;

/// Os esquemas que o `opener:default` já liberava (`allow-default-urls`).
/// Nada de `file:` nem `javascript:` vindo da interface.
const ESQUEMAS: [&str; 4] = ["http://", "https://", "mailto:", "tel:"];

fn esquema_permitido(url: &str) -> bool {
    let minusculo = url.trim_start().to_ascii_lowercase();
    ESQUEMAS.iter().any(|e| minusculo.starts_with(e))
}

/// Abre `url` no navegador (ou cliente de e-mail) padrão.
pub fn abrir_url(app: &AppHandle, url: &str) -> Result<(), String> {
    if !esquema_permitido(url) {
        return Err(format!("esquema não permitido: {url}"));
    }
    #[cfg(target_os = "linux")]
    {
        // Nada de cair no plugin aqui: no AppImage ele acharia o `xdg-open`
        // de dentro do pacote e devolveria Ok sem abrir nada.
        let _ = app;
        abrir_no_sistema(std::ffi::OsStr::new(url)).map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "linux"))]
    {
        app.opener()
            .open_url(url, None::<&str>)
            .map_err(|e| e.to_string())
    }
}

/// Abre `alvo` (URL ou pasta) com o programa padrão, sem esperar por ele.
///
/// A mesma lista do crate `open`, que o plugin usa — `xdg-open`, `gio open`,
/// `gnome-open`, `kde-open` —, cada um com o ambiente do sistema; o próximo
/// só é tentado quando o anterior não existe. Grupo de processos próprio,
/// para um Ctrl+C no terminal que abriu o app não levar junto o navegador; o
/// status é colhido numa thread, senão o filho vira zumbi até o app fechar.
#[cfg(target_os = "linux")]
pub fn abrir_no_sistema(alvo: &std::ffi::OsStr) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt as _;
    use std::process::Stdio;

    const PROGRAMAS: [(&str, &[&str]); 4] = [
        ("xdg-open", &[]),
        ("gio", &["open"]),
        ("gnome-open", &[]),
        ("kde-open", &["--"]),
    ];
    let mut ultimo = std::io::Error::from(std::io::ErrorKind::NotFound);
    for (programa, args) in PROGRAMAS {
        let mut cmd = std::process::Command::new(programa);
        lr_proc::no_window_std(&mut cmd);
        lr_proc::host_env_std(&mut cmd);
        cmd.args(args)
            .arg(alvo)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        match cmd.spawn() {
            Ok(mut filho) => {
                std::thread::spawn(move || {
                    let _ = filho.wait();
                });
                return Ok(());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => ultimo = e,
            Err(e) => return Err(e),
        }
    }
    Err(ultimo)
}

/// O `openUrl` da interface: links externos, "ver no Hub", página de release.
#[tauri::command]
pub fn open_external(app: AppHandle, url: String) -> Result<(), String> {
    abrir_url(&app, &url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_schemes_the_opener_already_allowed_get_through() {
        for ok in [
            "https://huggingface.co/x/y",
            "http://127.0.0.1:20128",
            "HTTPS://EXEMPLO.COM",
            "mailto:alguem@exemplo.com",
            "tel:+5511999999999",
        ] {
            assert!(esquema_permitido(ok), "{ok}");
        }
        for recusado in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "/home/pedro",
            "ftp://exemplo.com",
            "",
        ] {
            assert!(!esquema_permitido(recusado), "{recusado}");
        }
    }
}
