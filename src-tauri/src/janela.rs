//! A janela principal, criada em código (e não pelo `tauri.conf.json`): uma
//! janela nua com a interface do app numa webview filha que acompanha o
//! tamanho dela.
//!
//! É a montagem que o Tauri usa para várias webviews na mesma janela — e é o
//! que deixa a UI do AgenticOw entrar noutra webview filha, sobre a área de
//! conteúdo, em primeira parte (ver `commands_agenticow`). Os rótulos ficam
//! `main` para a janela e para a webview do app: a capability `default`, os
//! eventos de janela e a permissão de microfone do WebView2 continuam os mesmos.

use tauri::webview::WebviewBuilder;
use tauri::window::WindowBuilder;
use tauri::{LogicalPosition, WebviewUrl};

/// Rótulo da janela principal.
pub const JANELA: &str = "main";
/// Rótulo da webview que carrega a interface do app.
pub const WEBVIEW_DO_APP: &str = "main";

pub fn criar(app: &tauri::App) -> tauri::Result<()> {
    let janela = WindowBuilder::new(app, JANELA)
        .title("OpenWeights")
        .inner_size(1280.0, 820.0)
        .min_inner_size(980.0, 640.0)
        .build()?;
    let tamanho = janela
        .inner_size()?
        .to_logical::<f64>(janela.scale_factor()?);
    janela.add_child(
        WebviewBuilder::new(WEBVIEW_DO_APP, WebviewUrl::default())
            // O app trata arrastar arquivo na própria UI (dragDropEnabled: false).
            .disable_drag_drop_handler()
            .auto_resize(),
        LogicalPosition::new(0.0, 0.0),
        tamanho,
    )?;
    Ok(())
}
