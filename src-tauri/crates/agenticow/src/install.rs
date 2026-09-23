//! Instalação verificada do runtime: `<data>/runtimes/agenticow/<tag>/`.
//!
//! Fail-closed em cada passo — alvo sem pacote, tamanho, sha256 e identidade
//! (`runtime.json`) têm de bater com o que este binário embute. A instalação
//! é atômica (pasta nova, renomeada no fim); a versão anterior só sai depois
//! de um boot bom ([`podar`]), então um pacote ruim nunca deixa o app sem
//! nenhum.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::pins::{self, Pins};

/// Nome e formato que o `prepare-runtime.mjs` do fork grava no `runtime.json`.
pub const NOME_DO_RUNTIME: &str = "agenticow-runtime";
pub const FORMATO_DO_RUNTIME: u32 = 1;
/// O executável do runtime, relativo à raiz dele.
pub const ENTRADA: &str = "bin/agenticow-host.mjs";

#[derive(Debug, thiserror::Error)]
pub enum ErroDeInstalacao {
    #[error("agenticow-unsupported: não há pacote do AgenticOw para esta máquina ({0})")]
    Indisponivel(String),
    #[error("download do runtime falhou: {0}")]
    Download(String),
    #[error("pacote do runtime com tamanho errado: esperado {esperado}, veio {obtido}")]
    Tamanho { esperado: u64, obtido: u64 },
    #[error("verificação do runtime falhou: {0}")]
    Verificacao(String),
    #[error("o runtime baixado não é o que este app espera: {0}")]
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
            raiz: data_dir.join("runtimes").join("agenticow"),
        }
    }

    pub fn raiz(&self) -> &Path {
        &self.raiz
    }

    /// Pasta do runtime pinado por este binário.
    pub fn atual(&self) -> PathBuf {
        self.raiz.join(&pins::pins().tag)
    }

    /// O runtime pinado está instalado e é o que este app espera?
    pub fn instalado(&self) -> bool {
        let dir = self.atual();
        let Some(alvo) = pins::alvo_atual() else {
            return false;
        };
        dir.join(ENTRADA).is_file()
            && ler_identidade(&dir).is_ok_and(|id| confere(&id, pins::pins(), alvo).is_ok())
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
    pub host: String,
    pub dsh: String,
    pub target: String,
    pub entry: String,
}

pub fn ler_identidade(dir: &Path) -> Result<Identidade, ErroDeInstalacao> {
    let bytes = std::fs::read(dir.join("runtime.json"))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| ErroDeInstalacao::Identidade(format!("runtime.json ilegível: {e}")))
}

/// A identidade confere com os pins e o alvo? Byte a byte, fail-closed.
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
    if id.entry != ENTRADA {
        return erro("entry", ENTRADA, &id.entry);
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

/// Baixa, verifica e instala o runtime pinado. Devolve a pasta instalada.
pub async fn instalar(
    layout: &Layout,
    on_event: &(dyn Fn(EventoDeInstalacao) + Send + Sync),
) -> Result<PathBuf, ErroDeInstalacao> {
    let alvo = pins::alvo_atual().ok_or_else(|| {
        ErroDeInstalacao::Indisponivel(format!(
            "{}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
    })?;
    let asset = pins::pins()
        .assets
        .get(alvo)
        .ok_or_else(|| ErroDeInstalacao::Indisponivel(alvo.to_string()))?;
    std::fs::create_dir_all(layout.raiz())?;
    let sessao = lr_fetch::Session::new(layout.raiz())?;
    let cliente = lr_fetch::client("OpenWeights-agenticow")
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
    if !raiz.join(ENTRADA).is_file() {
        return Err(ErroDeInstalacao::Identidade(format!(
            "o pacote não tem {ENTRADA}"
        )));
    }

    let destino = layout.atual();
    lr_fetch::install_atomically(&raiz, &destino)?;
    on_event(EventoDeInstalacao::Installed);
    Ok(destino)
}

/// Remove versões que não são a pinada. Chamar só depois de um boot bom da
/// atual: é o que garante que um pacote ruim nunca deixa o app sem nenhum.
/// Devolve quantas pastas saíram.
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
            Err(e) => log::warn!("não consegui remover o runtime antigo {nome}: {e}"),
        }
    }
    removidas
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pins_de_teste() -> Pins {
        Pins {
            format: 1,
            repository: "pedro-canedo/openweights".into(),
            tag: "agenticow-runtime-758a924f-v1".into(),
            revision: "758a924fdb1d7e18b0b98682469f93516353fc57".into(),
            assets: Default::default(),
        }
    }

    fn identidade() -> Identidade {
        Identidade {
            name: NOME_DO_RUNTIME.into(),
            format: 1,
            revision: "758a924fdb1d7e18b0b98682469f93516353fc57".into(),
            upstream_tag: "dsh-v0.1.5-rc.3".into(),
            host: "0.1.0".into(),
            dsh: "0.1.5-rc.3".into(),
            target: "linux-x64".into(),
            entry: ENTRADA.into(),
        }
    }

    #[test]
    fn a_identidade_do_prepare_runtime_confere() {
        // O mesmo formato que o prepare-runtime.mjs do fork escreve.
        let json = r#"{"name":"agenticow-runtime","format":1,"revision":"758a924fdb1d7e18b0b98682469f93516353fc57","upstreamTag":"dsh-v0.1.5-rc.3","host":"0.1.0","dsh":"0.1.5-rc.3","target":"linux-x64","entry":"bin/agenticow-host.mjs","buildNode":"v22.20.0","buildNodeAbi":"127","files":11976,"bytes":122897134,"longestPath":166}"#;
        let id: Identidade = serde_json::from_str(json).unwrap();
        assert!(confere(&id, &pins_de_teste(), "linux-x64").is_ok());
    }

    #[test]
    fn identidade_estranha_e_recusada() {
        let p = pins_de_teste();
        let mut outra_revisao = identidade();
        outra_revisao.revision = "0".repeat(40);
        assert!(confere(&outra_revisao, &p, "linux-x64").is_err());
        assert!(
            confere(&identidade(), &p, "win32-x64").is_err(),
            "alvo trocado"
        );
        let mut outro_nome = identidade();
        outro_nome.name = "outro-runtime".into();
        assert!(confere(&outro_nome, &p, "linux-x64").is_err());
        let mut outra_entrada = identidade();
        outra_entrada.entry = "bin/outro.mjs".into();
        assert!(confere(&outra_entrada, &p, "linux-x64").is_err());
    }

    #[test]
    fn podar_so_tira_versoes_que_nao_sao_a_pinada() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = Layout::new(tmp.path());
        std::fs::create_dir_all(layout.atual().join("bin")).unwrap();
        std::fs::create_dir_all(layout.raiz().join("agenticow-runtime-00000000-v1")).unwrap();
        std::fs::create_dir_all(layout.raiz().join(".tmp").join("job-1")).unwrap();
        assert_eq!(podar(&layout), 1);
        assert!(layout.atual().is_dir());
        assert!(layout.raiz().join(".tmp").is_dir());
        assert!(!layout.raiz().join("agenticow-runtime-00000000-v1").exists());
    }

    #[test]
    fn instalado_exige_entrada_e_identidade() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = Layout::new(tmp.path());
        assert!(!layout.instalado());
        let dir = layout.atual();
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join(ENTRADA), "// host").unwrap();
        assert!(!layout.instalado(), "sem runtime.json");
        let Some(alvo) = pins::alvo_atual() else {
            return;
        };
        let mut id = identidade();
        id.target = alvo.into();
        id.revision = pins::pins().revision.clone();
        std::fs::write(
            dir.join("runtime.json"),
            serde_json::to_string(&id).unwrap(),
        )
        .unwrap();
        assert!(layout.instalado());
    }
}
