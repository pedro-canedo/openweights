//! No WebView2 o microfone é negado por omissão. Sem este handler a
//! Web Speech API falha com `not-allowed` mesmo com o microfone ligado
//! nas configurações do Windows.

use tauri::Manager;

pub fn allow_microphone(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        log::warn!("janela principal ausente; microfone não será liberado");
        return;
    };
    if let Err(e) = window.with_webview(|webview| {
        grant(webview);
    }) {
        log::warn!("falha ao configurar permissão de microfone: {e}");
    }
}

fn grant(webview: tauri::webview::PlatformWebview) {
    use webview2_com::{
        Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_PERMISSION_KIND_MICROPHONE, COREWEBVIEW2_PERMISSION_STATE_ALLOW,
        },
        PermissionRequestedEventHandler,
    };

    unsafe {
        let Ok(core) = webview.controller().CoreWebView2() else {
            log::warn!("WebView2: CoreWebView2 indisponível para o microfone");
            return;
        };
        let handler = PermissionRequestedEventHandler::create(Box::new(|_sender, args| {
            if let Some(args) = args {
                let mut kind = Default::default();
                args.PermissionKind(&mut kind)?;
                if kind == COREWEBVIEW2_PERMISSION_KIND_MICROPHONE {
                    args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW)?;
                }
            }
            Ok(())
        }));
        let mut token = 0i64;
        if let Err(e) = core.add_PermissionRequested(&handler, &mut token) {
            log::warn!("WebView2: não registrou o handler de microfone: {e}");
        } else {
            log::info!("WebView2: microfone permitido para ditado");
        }
    }
}
