//! Atualização do app a partir dos releases do GitHub.
//!
//! O contrato com a UI é de dois passos porque a decisão é da pessoa: o app
//! só PERGUNTA se existe versão nova (barato, silencioso, na abertura) e
//! espera o clique para baixar. Baixar 20 MB sem pedir, num app que a pessoa
//! abriu para conversar com um modelo, seria roubar banda no pior momento.
//!
//! Quem valida o pacote é o próprio updater, por assinatura minisign: a chave
//! pública está no `tauri.conf.json` e a privada só existe no CI. Sem isso,
//! qualquer resposta HTTP no lugar do `latest.json` viraria instalação.

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

/// O que a UI mostra no aviso de versão nova.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    /// Versão disponível (sem o `v`), como está no `latest.json`.
    pub version: String,
    /// Versão instalada agora — a UI mostra as duas lado a lado.
    pub current: String,
    /// Notas do release, quando existirem.
    pub notes: Option<String>,
    pub date: Option<String>,
}

/// Progresso do download, para a barra da UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProgress {
    baixado: u64,
    /// Ausente quando o servidor não declara o tamanho — a UI cai para
    /// "baixando…" em vez de mostrar uma barra que mente.
    total: Option<u64>,
}

/// Existe versão nova? `None` quando já estamos na mais recente.
///
/// Erro de rede vira `Err` com a mensagem, não pânico e não `None`: dizer
/// "está atualizado" quando na verdade não deu para perguntar é o tipo de
/// mentira silenciosa que este projeto evita.
#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    let current = app.package_info().version.to_string();
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(u)) => Ok(Some(UpdateInfo {
            version: u.version.clone(),
            current,
            notes: u.body.clone(),
            date: u.date.map(|d| d.to_string()),
        })),
        Ok(None) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Baixa e instala a versão nova, avisando o progresso por evento.
///
/// Consulta de novo em vez de guardar o resultado do `update_check`: um
/// pedido HTTP a mais custa milissegundos e evita ter de manter estado vivo
/// entre dois comandos, que é onde nasceria o bug de "instalar uma versão
/// que não é mais a última".
#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "não há versão nova para instalar".to_string())?;

    let mut baixado: u64 = 0;
    let janela = app.clone();
    update
        .download_and_install(
            move |pedaco, total| {
                baixado += pedaco as u64;
                let _ = janela.emit("update://progress", UpdateProgress { baixado, total });
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;

    // No Windows o instalador NSIS assume daqui e encerra o app sozinho; no
    // macOS e no Linux o pacote já foi trocado no disco e quem reinicia somos
    // nós. `restart` não retorna.
    app.restart()
}
