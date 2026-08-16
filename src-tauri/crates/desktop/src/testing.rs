//! Host de mentira e bancada dos testes.
//!
//! O [`FakeHost`] cumpre o contrato do [`DesktopHost`] em memória: guarda o
//! texto "copiado", registra os avisos mostrados e os alvos abertos. É o que
//! permite a suíte rodar em CI sem tela — e, mais importante, é o que permite
//! **provar as recusas**: um teste de `open_path` que recusa `.exe` só vale se
//! puder afirmar que nada chegou ao host.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use lr_tools::ToolContext;

use crate::DesktopHost;

#[derive(Default)]
struct Inner {
    clipboard: String,
    notifications: Vec<(String, String)>,
    opened: Vec<String>,
}

#[derive(Default)]
pub(crate) struct FakeHost {
    inner: Mutex<Inner>,
    /// Quando presente, toda chamada falha com esta mensagem — é assim que
    /// testamos o caminho "o sistema recusou" sem quebrar nada de verdade.
    fail: Option<String>,
}

impl FakeHost {
    /// Host que começa com algo já copiado.
    pub(crate) fn with_clipboard(text: &str) -> Self {
        let host = Self::default();
        host.lock().clipboard = text.to_string();
        host
    }

    /// Host em que toda operação do sistema falha.
    pub(crate) fn failing(message: &str) -> Self {
        Self {
            fail: Some(message.to_string()),
            ..Self::default()
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("estado do host de teste")
    }

    fn check(&self) -> Result<(), String> {
        match &self.fail {
            Some(msg) => Err(msg.clone()),
            None => Ok(()),
        }
    }

    pub(crate) fn clipboard(&self) -> String {
        self.lock().clipboard.clone()
    }

    pub(crate) fn notifications(&self) -> Vec<(String, String)> {
        self.lock().notifications.clone()
    }

    pub(crate) fn opened(&self) -> Vec<String> {
        self.lock().opened.clone()
    }
}

impl DesktopHost for FakeHost {
    fn clipboard_read(&self) -> Result<String, String> {
        self.check()?;
        Ok(self.clipboard())
    }

    fn clipboard_write(&self, text: &str) -> Result<(), String> {
        self.check()?;
        self.lock().clipboard = text.to_string();
        Ok(())
    }

    fn notify(&self, title: &str, body: &str) -> Result<(), String> {
        self.check()?;
        self.lock()
            .notifications
            .push((title.to_string(), body.to_string()));
        Ok(())
    }

    fn open(&self, target: &str) -> Result<(), String> {
        self.check()?;
        self.lock().opened.push(target.to_string());
        Ok(())
    }
}

/// Bancada de teste: host de mentira + pasta de projeto de verdade.
///
/// A pasta é real (e temporária) porque `ToolContext::resolve` resolve links
/// simbólicos do que existe — sem arquivo no disco, a metade mais importante
/// da validação de `open_path` não seria exercitada.
pub(crate) struct Bench {
    pub host: Arc<FakeHost>,
    pub ctx: ToolContext,
    dir: tempfile::TempDir,
}

impl Bench {
    pub(crate) fn new() -> Self {
        Self::with_host(FakeHost::default())
    }

    pub(crate) fn with_host(host: FakeHost) -> Self {
        let dir = tempfile::tempdir().expect("pasta temporária");
        let ctx = ToolContext::new(Some(dir.path().to_path_buf()), "teste");
        Self {
            host: Arc::new(host),
            ctx,
            dir,
        }
    }

    /// O host como o construtor das ferramentas o recebe.
    pub(crate) fn shared(&self) -> Arc<dyn DesktopHost> {
        self.host.clone()
    }

    /// Cria um arquivo dentro do projeto (com as pastas que faltarem).
    pub(crate) fn write(&self, rel: &str, body: &str) -> PathBuf {
        let path = self.dir.path().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("criar pasta");
        }
        std::fs::write(&path, body).expect("escrever arquivo");
        path
    }
}
