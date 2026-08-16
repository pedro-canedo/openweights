//! O lado do app do contrato `lr_desktop::DesktopHost`.
//!
//! As ferramentas de desktop moram num crate que não conhece o Tauri — é o
//! que permite testá-las sem tela, com um host falso. Quem sabe mesmo falar
//! com o sistema é aqui: os plugins oficiais resolvem as diferenças entre
//! Windows, macOS e Linux (notificação com identidade do app, área de
//! transferência por servidor gráfico, abrir no aplicativo padrão).

use lr_desktop::DesktopHost;
use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

pub struct TauriDesktop {
    app: AppHandle,
}

impl TauriDesktop {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl DesktopHost for TauriDesktop {
    fn clipboard_read(&self) -> Result<String, String> {
        self.app
            .clipboard()
            .read_text()
            .map_err(|e| format!("não consegui ler a área de transferência: {e}"))
    }

    fn clipboard_write(&self, text: &str) -> Result<(), String> {
        self.app
            .clipboard()
            .write_text(text.to_string())
            .map_err(|e| format!("não consegui escrever na área de transferência: {e}"))
    }

    fn notify(&self, title: &str, body: &str) -> Result<(), String> {
        self.app
            .notification()
            .builder()
            .title(title)
            .body(body)
            .show()
            .map_err(|e| format!("não consegui mostrar o aviso: {e}"))
    }

    fn open(&self, target: &str) -> Result<(), String> {
        // A validação (esquema aceito, caminho dentro do projeto, extensão
        // que não é programa) já aconteceu no crate; aqui só sobra escolher
        // entre link e arquivo.
        let baixo = target.to_ascii_lowercase();
        let opener = self.app.opener();
        if baixo.starts_with("http://") || baixo.starts_with("https://") {
            opener
                .open_url(target, None::<&str>)
                .map_err(|e| format!("não consegui abrir o link: {e}"))
        } else {
            opener
                .open_path(target, None::<&str>)
                .map_err(|e| format!("não consegui abrir: {e}"))
        }
    }
}

/// Aviso do sistema disparado pelo próprio app (não pelo agente).
///
/// Automação que termina de madrugada não tem para quem falar na tela; este
/// é o caminho de sair da janela e chegar na pessoa.
pub fn notify(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        log::debug!("aviso do sistema indisponível: {e}");
    }
}
