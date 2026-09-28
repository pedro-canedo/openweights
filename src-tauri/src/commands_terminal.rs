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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PedidoDeOwcli {
    pub pasta: Option<String>,
    pub colunas: u16,
    pub linhas: u16,
    /// `on-request` (padrão), `untrusted` ou `never`.
    pub aprovacao: Option<String>,
    /// `workspace-write` (padrão), `read-only` ou `danger-full-access`.
    pub sandbox: Option<String>,
    /// Id de uma conversa gravada para continuar (`owcli resume <id>`).
    pub retomar: Option<String>,
    /// O modelo, com o prefixo da fonte (`local:…`). Sem ele, vale o da
    /// config do OwCLI ou o padrão do app.
    pub modelo: Option<String>,
}

/// Abre o agente OwCLI numa sessão: liga o gateway e o `openweights.json`
/// (primeira vez) e sobe a TUI na pasta escolhida — nova, ou continuando uma
/// conversa gravada.
#[tauri::command]
pub async fn terminal_abrir_owcli(
    app: AppHandle,
    state: State<'_, AppState>,
    pedido: PedidoDeOwcli,
) -> CmdResult<lr_pty::SessaoId> {
    let exe = crate::commands_owcli::binario()?;
    crate::commands_owcli::ativar(&app).await;
    let casa = crate::commands_owcli::casa(&app).ok_or("sem pasta pessoal")?;
    let retomar = match pedido.retomar {
        Some(id) if crate::owcli_historico::id_valido(&id) => Some(id),
        Some(_) => return Err("conversa inválida".to_string()),
        None => None,
    };
    // A pasta de uma conversa antiga pode ter sido apagada ou movida: aí ela
    // continua na pasta pessoal, em vez de não abrir.
    let pasta = match pasta_ou_pessoal(&app, pedido.pasta) {
        Err(_) if retomar.is_some() => pasta_ou_pessoal(&app, None)?,
        outra => outra?,
    };
    let aprovacao = match pedido.aprovacao.as_deref() {
        Some(a @ ("untrusted" | "never" | "on-request")) => a.to_string(),
        _ => "on-request".to_string(),
    };
    let sandbox = match pedido.sandbox.as_deref() {
        Some(s @ ("read-only" | "danger-full-access" | "workspace-write")) => s.to_string(),
        _ => "workspace-write".to_string(),
    };
    let mut args: Vec<OsString> = Vec::new();
    if let Some(id) = retomar {
        args.extend(["resume".into(), id.into()]);
    }
    match pedido.modelo {
        Some(m) if crate::commands_owcli::modelo_valido(&m) => {
            args.extend(["--model".into(), m.into()]);
        }
        Some(_) => return Err("modelo inválido".to_string()),
        None => {}
    }
    args.extend::<[OsString; 6]>([
        "--cd".into(),
        pasta.clone().into_os_string(),
        "--ask-for-approval".into(),
        aprovacao.into(),
        "--sandbox".into(),
        sandbox.into(),
    ]);
    let gerente = state.terminais.clone();
    tauri::async_runtime::spawn_blocking(move || {
        gerente.abrir(lr_pty::Pedido {
            programa: exe.into_os_string(),
            args,
            pasta,
            env: vec![
                // A casa explícita também faz um build de desenvolvimento
                // (binário `codex`) entrar no modo OwCLI.
                ("OWCLI_HOME".into(), casa.into_os_string()),
                ("TERM_PROGRAM".into(), "OpenWeights".into()),
                // O xterm.js não fala o protocolo de teclado do kitty; sem
                // isto a TUI espera a resposta da sondagem na abertura.
                ("CODEX_TUI_DISABLE_KEYBOARD_ENHANCEMENT".into(), "1".into()),
            ],
            colunas: pedido.colunas,
            linhas: pedido.linhas,
            titulo: "OwCLI".to_string(),
            tipo: lr_pty::Tipo::OwCli,
        })
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("não foi possível abrir o OwCLI: {e}"))
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

// ------------------------------------------------- área de transferência ---

/// A área de transferência do sistema, para copiar e colar nos terminais.
///
/// No WebKitGTK a API do navegador não é confiável para ler, e colar num
/// terminal é ler. A instância fica viva: no X11 quem copiou continua dono do
/// conteúdo e tem de estar lá para entregá-lo a quem colar.
fn area_de_transferencia() -> CmdResult<std::sync::MutexGuard<'static, arboard::Clipboard>> {
    static AREA: std::sync::OnceLock<Result<std::sync::Mutex<arboard::Clipboard>, String>> =
        std::sync::OnceLock::new();
    let area = AREA
        .get_or_init(|| {
            arboard::Clipboard::new()
                .map(std::sync::Mutex::new)
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| format!("área de transferência indisponível: {e}"))?;
    Ok(area.lock().unwrap_or_else(|e| e.into_inner()))
}

#[tauri::command]
pub async fn area_de_transferencia_ler() -> CmdResult<String> {
    tauri::async_runtime::spawn_blocking(|| {
        // Vazia ou com imagem: nada a colar, não é erro.
        Ok(area_de_transferencia()?.get_text().unwrap_or_default())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn area_de_transferencia_escrever(texto: String) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        area_de_transferencia()?
            .set_text(texto)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
