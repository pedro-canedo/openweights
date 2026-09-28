//! Instalação verificada do runtime: `<data>/runtimes/owcli/<tag>/`.
//!
//! Fail-closed em cada passo — alvo sem pacote, tamanho, sha256 e identidade
//! (`runtime.json`) têm de bater com o que este binário embute. A instalação
//! é atômica (pasta nova, renomeada no fim), e só a versão pinada fica: as
//! outras saem em [`podar`], chamada quando nenhuma sessão do OwCLI pode
//! estar usando uma delas.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::pins::{self, Pins};

/// Nome e formato que o workflow `owcli-runtime.yml` grava no `runtime.json`.
pub const NOME_DO_RUNTIME: &str = "owcli-runtime";
pub const FORMATO_DO_RUNTIME: u32 = 1;

/// O executável do runtime num alvo, relativo à raiz dele. Tem de se chamar
/// `owcli`: com outro nome o lançador do fork não entra no modo OwCLI.
pub fn entrada_para(alvo: &str) -> &'static str {
    if alvo == "win32-x64" {
        "bin/owcli.exe"
    } else {
        "bin/owcli"
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ErroDeInstalacao {
    #[error("owcli-unsupported: não há pacote do OwCLI para esta máquina ({0})")]
    Indisponivel(String),
    #[error("download do OwCLI falhou: {0}")]
    Download(String),
    #[error("pacote do OwCLI com tamanho errado: esperado {esperado}, veio {obtido}")]
    Tamanho { esperado: u64, obtido: u64 },
    #[error("verificação do OwCLI falhou: {0}")]
    Verificacao(String),
    #[error("o OwCLI baixado não é o que este app espera: {0}")]
    Identidade(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Onde o runtime mora.
#[derive(Clone, Debug)]
pub struct Layout {
    raiz: PathBuf,
}

impl Layout {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            raiz: data_dir.join("runtimes").join("owcli"),
        }
    }

    pub fn raiz(&self) -> &Path {
        &self.raiz
    }

    /// Pasta do runtime pinado por este binário.
    pub fn atual(&self) -> PathBuf {
        self.raiz.join(&pins::pins().tag)
    }

    /// O executável do runtime pinado, se estiver instalado e for o que este
    /// app espera.
    pub fn executavel(&self) -> Option<PathBuf> {
        let alvo = pins::alvo_atual()?;
        let dir = self.atual();
        let exe = dir.join(entrada_para(alvo));
        (exe.is_file()
            && ler_identidade(&dir).is_ok_and(|id| confere(&id, pins::pins(), alvo).is_ok()))
        .then_some(exe)
    }
}

/// O `runtime.json` do pacote.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identidade {
    pub name: String,
    pub format: u32,
    pub revision: String,
    pub upstream_tag: String,
    pub target: String,
    pub entry: String,
}

pub fn ler_identidade(dir: &Path) -> Result<Identidade, ErroDeInstalacao> {
    let bytes = std::fs::read(dir.join("runtime.json"))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| ErroDeInstalacao::Identidade(format!("runtime.json ilegível: {e}")))
}

/// A identidade confere com os pins e o alvo? Campo a campo, fail-closed.
pub fn confere(id: &Identidade, pins: &Pins, alvo: &str) -> Result<(), ErroDeInstalacao> {
    let erro = |campo: &str, esperado: &str, obtido: &str| {
        Err(ErroDeInstalacao::Identidade(format!(
            "{campo}: esperado {esperado}, veio {obtido}"
        )))
    };
    if id.name != NOME_DO_RUNTIME {
        return erro("name", NOME_DO_RUNTIME, &id.name);
    }
    if id.format != FORMATO_DO_RUNTIME {
        return erro(
            "format",
            &FORMATO_DO_RUNTIME.to_string(),
            &id.format.to_string(),
        );
    }
    if id.revision != pins.revision {
        return erro("revision", &pins.revision, &id.revision);
    }
    if id.target != alvo {
        return erro("target", alvo, &id.target);
    }
    if id.entry != entrada_para(alvo) {
        return erro("entry", entrada_para(alvo), &id.entry);
    }
    Ok(())
}

/// Progresso da instalação, para a tela.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum EventoDeInstalacao {
    Progress {
        received_bytes: u64,
        total_bytes: u64,
    },
    Verifying,
    Extracting,
    Installed,
}

/// Baixa, verifica e instala o runtime pinado. Devolve o executável.
pub async fn instalar(
    layout: &Layout,
    on_event: &(dyn Fn(EventoDeInstalacao) + Send + Sync),
) -> Result<PathBuf, ErroDeInstalacao> {
    let (alvo, asset) = pins::asset_atual().ok_or_else(|| {
        ErroDeInstalacao::Indisponivel(format!(
            "{}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
    })?;
    std::fs::create_dir_all(layout.raiz())?;
    let sessao = lr_fetch::Session::new(layout.raiz())?;
    let cliente = lr_fetch::client("OpenWeights-owcli")
        .map_err(|e| ErroDeInstalacao::Download(e.to_string()))?;

    let pacote = sessao.path().join("download.part");
    let progresso = |recebidos, total| {
        on_event(EventoDeInstalacao::Progress {
            received_bytes: recebidos,
            total_bytes: total,
        })
    };
    lr_fetch::download_to(&cliente, &pins::url(asset), &pacote, &progresso)
        .await
        .map_err(|e| ErroDeInstalacao::Download(e.to_string()))?;

    on_event(EventoDeInstalacao::Verifying);
    let obtido = std::fs::metadata(&pacote)?.len();
    if obtido != asset.size {
        return Err(ErroDeInstalacao::Tamanho {
            esperado: asset.size,
            obtido,
        });
    }
    lr_fetch::verify_sha256_strict(&pacote, &asset.name, &asset.sha256)
        .await
        .map_err(|e| ErroDeInstalacao::Verificacao(e.to_string()))?;

    on_event(EventoDeInstalacao::Extracting);
    let extraido = sessao.path().join("extraido");
    lr_fetch::extract_archive_async(pacote, asset.name.clone(), extraido.clone()).await?;
    let raiz = lr_fetch::find_dir_containing(&extraido, "runtime.json")
        .ok_or_else(|| ErroDeInstalacao::Identidade("o pacote não tem runtime.json".into()))?;
    confere(&ler_identidade(&raiz)?, pins::pins(), alvo)?;
    let entrada = entrada_para(alvo);
    if !raiz.join(entrada).is_file() {
        return Err(ErroDeInstalacao::Identidade(format!(
            "o pacote não tem {entrada}"
        )));
    }

    let destino = layout.atual();
    lr_fetch::install_atomically(&raiz, &destino)?;
    on_event(EventoDeInstalacao::Installed);
    Ok(destino.join(entrada))
}

/// Remove versões que não são a pinada. Chamar quando nenhuma sessão do
/// OwCLI está aberta: o `auth.command` de uma sessão aponta para o executável
/// dela, e no Windows um `.exe` em uso nem sai. Devolve quantas pastas saíram.
pub fn podar(layout: &Layout) -> usize {
    let atual = pins::pins().tag.as_str();
    let Ok(entradas) = std::fs::read_dir(layout.raiz()) else {
        return 0;
    };
    let mut removidas = 0;
    for entrada in entradas.flatten() {
        let nome = entrada.file_name();
        let nome = nome.to_string_lossy();
        if nome == atual || nome.starts_with('.') || !entrada.path().is_dir() {
            continue;
        }
        match lr_fetch::remove_dir_all_retrying(&entrada.path()) {
            Ok(()) => removidas += 1,
            Err(e) => log::warn!("não consegui remover o OwCLI antigo {nome}: {e}"),
        }
    }
    removidas
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pins_de_teste() -> Pins {
        Pins::ler(
            r#"{"format":1,"repository":"pedro-canedo/openweights","tag":"owcli-runtime-34d992ac-v1","revision":"34d992ac469aa4dcf9bb41f02f959dee918bfdbb","assets":{}}"#,
        )
        .unwrap()
    }

    fn identidade(alvo: &str) -> Identidade {
        Identidade {
            name: NOME_DO_RUNTIME.into(),
            format: 1,
            revision: "34d992ac469aa4dcf9bb41f02f959dee918bfdbb".into(),
            upstream_tag: "rust-v0.157.1".into(),
            target: alvo.into(),
            entry: entrada_para(alvo).into(),
        }
    }

    #[test]
    fn a_identidade_do_workflow_confere() {
        // O formato que o owcli-runtime.yml grava, com campos a mais.
        let json = r#"{"name":"owcli-runtime","format":1,"revision":"34d992ac469aa4dcf9bb41f02f959dee918bfdbb","upstreamTag":"rust-v0.157.1","target":"win32-x64","entry":"bin/owcli.exe","bytes":280000000}"#;
        let id: Identidade = serde_json::from_str(json).unwrap();
        assert!(confere(&id, &pins_de_teste(), "win32-x64").is_ok());
    }

    #[test]
    fn identidade_estranha_e_recusada() {
        let p = pins_de_teste();
        let mut outra_revisao = identidade("linux-x64");
        outra_revisao.revision = "0".repeat(40);
        assert!(confere(&outra_revisao, &p, "linux-x64").is_err());
        assert!(
            confere(&identidade("linux-x64"), &p, "darwin-arm64").is_err(),
            "alvo trocado"
        );
        let mut outro_nome = identidade("linux-x64");
        outro_nome.name = "agenticow-runtime".into();
        assert!(confere(&outro_nome, &p, "linux-x64").is_err());
        let mut codex = identidade("linux-x64");
        codex.entry = "bin/codex".into();
        assert!(
            confere(&codex, &p, "linux-x64").is_err(),
            "com outro nome o lançador não vira OwCLI"
        );
    }

    #[test]
    fn a_entrada_do_windows_tem_exe() {
        assert_eq!(entrada_para("win32-x64"), "bin/owcli.exe");
        assert_eq!(entrada_para("linux-x64"), "bin/owcli");
        assert_eq!(entrada_para("darwin-arm64"), "bin/owcli");
    }

    #[test]
    fn podar_so_tira_versoes_que_nao_sao_a_pinada() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = Layout::new(tmp.path());
        std::fs::create_dir_all(layout.atual().join("bin")).unwrap();
        std::fs::create_dir_all(layout.raiz().join("owcli-runtime-00000000-v1")).unwrap();
        std::fs::create_dir_all(layout.raiz().join(".tmp").join("job-1")).unwrap();
        assert_eq!(podar(&layout), 1);
        assert!(layout.atual().is_dir());
        assert!(layout.raiz().join(".tmp").is_dir());
        assert!(!layout.raiz().join("owcli-runtime-00000000-v1").exists());
    }

    #[test]
    fn instalado_exige_entrada_e_identidade() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = Layout::new(tmp.path());
        assert_eq!(layout.executavel(), None);
        let Some(alvo) = pins::alvo_atual() else {
            return;
        };
        let dir = layout.atual();
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join(entrada_para(alvo)), "binário").unwrap();
        assert_eq!(layout.executavel(), None, "sem runtime.json");
        let mut id = identidade(alvo);
        id.revision = pins::pins().revision.clone();
        std::fs::write(
            dir.join("runtime.json"),
            serde_json::to_string(&id).unwrap(),
        )
        .unwrap();
        assert_eq!(layout.executavel(), Some(dir.join(entrada_para(alvo))));
    }

    #[tokio::test]
    async fn sem_pacote_para_a_maquina_a_instalacao_diz_por_que() {
        if pins::asset_atual().is_some() {
            return; // pin publicado: o caso não se aplica.
        }
        let tmp = tempfile::tempdir().unwrap();
        let erro = instalar(&Layout::new(tmp.path()), &|_| {})
            .await
            .unwrap_err();
        assert!(matches!(erro, ErroDeInstalacao::Indisponivel(_)), "{erro}");
        assert!(erro.to_string().starts_with("owcli-unsupported"));
    }
}
