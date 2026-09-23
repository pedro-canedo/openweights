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
    let _app = janela.add_child(
        WebviewBuilder::new(WEBVIEW_DO_APP, WebviewUrl::default())
            // O app trata arrastar arquivo na própria UI (dragDropEnabled: false).
            .disable_drag_drop_handler()
            .auto_resize(),
        LogicalPosition::new(0.0, 0.0),
        tamanho,
    )?;
    #[cfg(target_os = "linux")]
    gtk_sobreposicao::montar(&_app);
    Ok(())
}

/// Põe uma webview filha sobre o retângulo `(x, y, largura, altura)` da
/// janela, em pixels lógicos (os do CSS).
pub fn posicionar(
    webview: &tauri::Webview,
    x: f64,
    y: f64,
    largura: f64,
    altura: f64,
) -> tauri::Result<()> {
    let (largura, altura) = (largura.max(1.0), altura.max(1.0));
    #[cfg(target_os = "linux")]
    {
        gtk_sobreposicao::posicionar(webview, x, y, largura, altura);
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        webview.set_position(LogicalPosition::new(x, y))?;
        webview.set_size(tauri::LogicalSize::new(largura, altura))
    }
}

/// No Linux o Tauri põe TODA webview da janela numa `GtkBox` vertical, com
/// expand — duas webviews dividem a janela ao meio — e o `set_bounds` do wry
/// não faz nada ali (só age num `GtkFixed` ou numa filha X11). Então a janela
/// monta a sobreposição dela: a webview do app vira o conteúdo de um
/// `GtkOverlay` (preenche tudo, acompanha o tamanho sozinha) e a filha sai da
/// caixa para uma camada por cima.
///
/// O retângulo da camada é dado pelo sinal `get-child-position`, não por
/// margens e `set_size_request`: o overlay aloca a camada pelo tamanho
/// NATURAL dela, e o da webview não encolhe junto com a área — ao diminuir a
/// janela, a filha passava por cima da barra de status do app.
#[cfg(target_os = "linux")]
mod gtk_sobreposicao {
    use gtk::prelude::*;
    use std::cell::RefCell;

    // Objetos GTK só vivem na thread principal — e é nela que o
    // `with_webview` roda.
    thread_local! {
        static SOBREPOSICAO: RefCell<Option<gtk::Overlay>> = const { RefCell::new(None) };
        /// Cada camada e o retângulo dela.
        static AREAS: RefCell<Vec<(gtk::Widget, gtk::gdk::Rectangle)>> = const { RefCell::new(Vec::new()) };
    }

    pub fn montar(app: &tauri::Webview) {
        let resultado = app.with_webview(|pw| {
            let wv = pw.inner();
            let Some(caixa) = wv.parent().and_then(|p| p.downcast::<gtk::Box>().ok()) else {
                log::warn!("webview do app fora de uma GtkBox: sem sobreposição");
                return;
            };
            let sobreposicao = gtk::Overlay::new();
            sobreposicao.connect_get_child_position(|_, filho| {
                AREAS.with(|a| a.borrow().iter().find(|(w, _)| w == filho).map(|(_, r)| *r))
            });
            caixa.remove(&wv);
            sobreposicao.add(&wv);
            caixa.pack_start(&sobreposicao, true, true, 0);
            sobreposicao.show_all();
            SOBREPOSICAO.with(|s| *s.borrow_mut() = Some(sobreposicao));
        });
        if let Err(e) = resultado {
            log::warn!("sobreposição da janela: {e}");
        }
    }

    pub fn posicionar(filha: &tauri::Webview, x: f64, y: f64, largura: f64, altura: f64) {
        let resultado = filha.with_webview(move |pw| {
            let wv: gtk::Widget = pw.inner().upcast();
            SOBREPOSICAO.with(|s| {
                let Some(sobreposicao) = s.borrow().clone() else {
                    return;
                };
                let area = gtk::gdk::Rectangle::new(
                    x.round() as i32,
                    y.round() as i32,
                    largura.round() as i32,
                    altura.round() as i32,
                );
                AREAS.with(|a| {
                    let mut a = a.borrow_mut();
                    // Camada que já saiu da janela não conta mais.
                    a.retain(|(w, _)| w.parent().is_some());
                    match a.iter_mut().find(|(w, _)| *w == wv) {
                        Some((_, r)) => *r = area,
                        None => a.push((wv.clone(), area)),
                    }
                });
                let ja_esta = wv
                    .parent()
                    .is_some_and(|p| &p == sobreposicao.upcast_ref::<gtk::Widget>());
                if !ja_esta {
                    // Recém-criada: o Tauri a empilhou na caixa da janela.
                    if let Some(pai) = wv
                        .parent()
                        .and_then(|p| p.downcast::<gtk::Container>().ok())
                    {
                        pai.remove(&wv);
                    }
                    sobreposicao.add_overlay(&wv);
                }
                sobreposicao.queue_resize();
            });
        });
        if let Err(e) = resultado {
            log::warn!("posição da webview filha: {e}");
        }
    }
}
