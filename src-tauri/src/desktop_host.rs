//! Avisos do sistema disparados pelo próprio app.
//!
//! O plugin oficial resolve as diferenças entre Windows, macOS e Linux
//! (notificação com a identidade do app em cada sistema). Já morou aqui o
//! host completo das ferramentas de desktop do agente; com o modo agente
//! fora do app, sobrou o que o resto usa: avisar a pessoa.

use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

/// Aviso do sistema disparado pelo próprio app.
///
/// O cluster usa para dar notícia de pareamento e empréstimo de GPU quando
/// ninguém está olhando a janela; este é o caminho de sair da tela e chegar
/// na pessoa.
pub fn notify(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        log::debug!("aviso do sistema indisponível: {e}");
    }
}
