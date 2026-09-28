//! Comandos dos terminais embutidos (tela do OwCLI).
//!
//! O processo e os bytes são do `lr_pty`; aqui fica a ponte com a webview:
//!
//! - a saída vai por um `Channel` com bytes crus (`InvokeResponseBody::Raw`) —
//!   nunca `Channel<Vec<u8>>`, que o serde transformaria num array JSON de
//!   números. Cada mensagem é `[offset: u64 LE][truncado: u8][dados]`;
//! - título, pasta, "precisa de você" e saída vão pelo evento `terminal`
//!   (emitido pelo `AppState`), que a lista de sessões escuta mesmo com a
//!   tela fechada;
//! - todo comando é `async`: um comando síncrono roda na thread principal, e
//!   escrever num pseudoterminal cheio pode esperar.

use std::ffi::OsString;
use std::path::PathBuf;

use serde::Deserialize;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Manager, State};

use crate::state::AppState;

type CmdResult<T> = Result<T, String>;

/// A tela que recebe a saída de uma sessão.
struct CanalDaTela(Channel<InvokeResponseBody>);

impl lr_pty::Destino for CanalDaTela {
    fn enviar(&self, bloco: lr_pty::Bloco) -> bool {
        let mut msg = Vec::with_capacity(9 + bloco.dados.len());
        msg.extend_from_slice(&bloco.offset.to_le_bytes());
        msg.push(u8::from(bloco.truncado));
        msg.extend_from_slice(&bloco.dados);
        self.0.send(InvokeResponseBody::Raw(msg)).is_ok()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PedidoDeShell {
    /// Pasta inicial; sem ela, a pasta pessoal.
    pub pasta: Option<String>,
    pub colunas: u16,
    pub linhas: u16,
}

/// O shell de quem usa a máquina, como login shell: é o perfil que põe o
/// Homebrew, o nvm e o pyenv no `PATH`.
#[cfg(unix)]
fn shell_padrao() -> (OsString, Vec<OsString>) {
    let shell = lr_proc::host_var("SHELL")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| OsString::from("/bin/bash"));
    (shell, vec![OsString::from("-l")])
}

/// PowerShell 7, senão o do Windows, senão o cmd.
#[cfg(windows)]
fn shell_padrao() -> (OsString, Vec<OsString>) {
    let caminhos = lr_proc::host_var("PATH").unwrap_or_default();
    for exe in ["pwsh.exe", "powershell.exe"] {
        if std::env::split_paths(&caminhos).any(|dir| dir.join(exe).is_file()) {
            return (OsString::from(exe), vec![OsString::from("-NoLogo")]);
        }
    }
    (OsString::from("cmd.exe"), Vec::new())
}

fn nome_do_programa(programa: &OsString) -> String {
    let caminho = PathBuf::from(programa);
    caminho
        .file_stem()
        .unwrap_or(caminho.as_os_str())
        .to_string_lossy()
        .into_owned()
}

fn pasta_ou_pessoal(app: &AppHandle, pasta: Option<String>) -> CmdResult<PathBuf> {
    match pasta.filter(|p| !p.trim().is_empty()) {
        Some(p) => {
            let p = PathBuf::from(p);
            if p.is_dir() {
                Ok(p)
            } else {
                Err(format!("a pasta {} não existe", p.display()))
            }
        }
        None => app.path().home_dir().map_err(|e| e.to_string()),
    }
}

#[tauri::command]
pub async fn terminal_abrir_shell(
    app: AppHandle,
    state: State<'_, AppState>,
    pedido: PedidoDeShell,
) -> CmdResult<lr_pty::SessaoId> {
    let pasta = pasta_ou_pessoal(&app, pedido.pasta)?;
    let (programa, args) = shell_padrao();
    let titulo = nome_do_programa(&programa);
    let gerente = state.terminais.clone();
    tauri::async_runtime::spawn_blocking(move || {
        gerente.abrir(lr_pty::Pedido {
            programa,
            args,
            pasta,
            env: vec![
                ("TERM_PROGRAM".into(), "OpenWeights".into()),
                (
                    "TERM_PROGRAM_VERSION".into(),
                    env!("CARGO_PKG_VERSION").into(),
                ),
            ],
            colunas: pedido.colunas,
            linhas: pedido.linhas,
            titulo,
            tipo: lr_pty::Tipo::Shell,
        })
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("não foi possível abrir o terminal: {e}"))
}

/// Liga a tela à sessão a partir do byte `desde` (`None`: tudo o que o anel
/// guarda). `false` se a sessão não existe mais.
#[tauri::command]
pub async fn terminal_anexar(
    state: State<'_, AppState>,
    id: lr_pty::SessaoId,
    desde: Option<u64>,
    canal: Channel<InvokeResponseBody>,
) -> CmdResult<bool> {
    Ok(state
        .terminais
        .anexar(id, desde, Box::new(CanalDaTela(canal))))
}

#[tauri::command]
pub async fn terminal_desanexar(state: State<'_, AppState>, id: lr_pty::SessaoId) -> CmdResult<()> {
    state.terminais.desanexar(id);
    Ok(())
}

#[tauri::command]
pub async fn terminal_escrever(
    state: State<'_, AppState>,
    id: lr_pty::SessaoId,
    dados: String,
) -> CmdResult<()> {
    let gerente = state.terminais.clone();
    tauri::async_runtime::spawn_blocking(move || gerente.escrever(id, dados.as_bytes()))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn terminal_redimensionar(
    state: State<'_, AppState>,
    id: lr_pty::SessaoId,
    colunas: u16,
    linhas: u16,
) -> CmdResult<()> {
    state.terminais.redimensionar(id, colunas, linhas);
    Ok(())
}

/// A tela desenhou até o byte `ate` (controle de fluxo).
#[tauri::command]
pub async fn terminal_confirmar(
    state: State<'_, AppState>,
    id: lr_pty::SessaoId,
    ate: u64,
) -> CmdResult<()> {
    state.terminais.confirmar(id, ate);
    Ok(())
}

#[tauri::command]
pub async fn terminal_fechar(state: State<'_, AppState>, id: lr_pty::SessaoId) -> CmdResult<()> {
    state.terminais.fechar(id);
    Ok(())
}

#[tauri::command]
pub async fn terminal_visto(state: State<'_, AppState>, id: lr_pty::SessaoId) -> CmdResult<()> {
    state.terminais.visto(id);
    Ok(())
}

#[tauri::command]
pub async fn terminal_listar(state: State<'_, AppState>) -> CmdResult<Vec<lr_pty::Resumo>> {
    Ok(state.terminais.listar())
}
