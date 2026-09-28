//! Os pins de um runtime pré-compilado que o app baixa: o `pins.json` que o
//! workflow do runtime publica (tag, revisão e, por alvo, nome, sha256 e
//! tamanho do pacote). Cada crate embute o seu com `include_str!`, então o
//! pin herda a assinatura do updater — a release no GitHub pode ser trocada,
//! o hash que o app aceita não.

use std::collections::BTreeMap;

use serde::Deserialize;

/// Os alvos que um runtime pode ter, no formato dos pacotes.
pub const ALVOS: [&str; 4] = ["linux-x64", "win32-x64", "darwin-arm64", "darwin-x64"];

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Pins {
    pub format: u32,
    pub repository: String,
    pub tag: String,
    pub revision: String,
    /// Alvo (`linux-x64`, `win32-x64`, `darwin-arm64`, `darwin-x64`) → pacote.
    pub assets: BTreeMap<String, Asset>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Asset {
    pub name: String,
    pub sha256: String,
    pub size: u64,
}

impl Pins {
    pub fn ler(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// O pacote desta máquina, se a release pinada tiver um.
    pub fn asset_atual(&self) -> Option<(&'static str, &Asset)> {
        let alvo = alvo_atual()?;
        self.assets.get(alvo).map(|a| (alvo, a))
    }

    /// URL de download de um pacote da release pinada.
    pub fn url(&self, asset: &Asset) -> String {
        format!(
            "https://github.com/{}/releases/download/{}/{}",
            self.repository, self.tag, asset.name
        )
    }

    /// As regras que todo pin embutido tem de cumprir, para o teste de cada
    /// crate: a tag leva o nome do runtime e os 8 primeiros caracteres da
    /// revisão (tag e revisão andam juntas), e cada pacote tem o nome, o hash
    /// e um tamanho plausíveis. `assets` vazio é válido: o runtime ainda não
    /// foi publicado e o app diz que não há pacote para esta máquina.
    pub fn conferir(&self, runtime: &str) -> Result<(), String> {
        if self.format != 1 {
            return Err(format!("format {}", self.format));
        }
        if self.repository != "pedro-canedo/openweights" {
            return Err(format!("repository {}", self.repository));
        }
        if self.revision.len() != 40 || !self.revision.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("revisão incompleta: {}", self.revision));
        }
        let esperado = format!("{runtime}-{}-v", &self.revision[..8]);
        if !self.tag.starts_with(&esperado) {
            return Err(format!("{} não bate com {}", self.tag, self.revision));
        }
        for (alvo, a) in &self.assets {
            if !ALVOS.contains(&alvo.as_str()) {
                return Err(format!("alvo {alvo}"));
            }
            let ext = if alvo == "win32-x64" { "zip" } else { "tar.gz" };
            if a.name != format!("openweights-{}-{alvo}.{ext}", self.tag) {
                return Err(format!("nome do pacote de {alvo}: {}", a.name));
            }
            if a.sha256.len() != 64
                || !a
                    .sha256
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            {
                return Err(format!("sha256 de {alvo}: {}", a.sha256));
            }
            if a.size <= 1_000_000 {
                return Err(format!("pacote de {alvo} pequeno demais: {}", a.size));
            }
        }
        Ok(())
    }
}

/// Nome do alvo para um par (os, arch) do Rust.
pub fn alvo_para(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("linux", "x86_64") => Some("linux-x64"),
        ("windows", "x86_64") => Some("win32-x64"),
        ("macos", "aarch64") => Some("darwin-arm64"),
        ("macos", "x86_64") => Some("darwin-x64"),
        _ => None,
    }
}

/// O alvo desta máquina. O app de macOS é universal: Intel ou Apple Silicon
/// se decide aqui, em tempo de execução.
pub fn alvo_atual() -> Option<&'static str> {
    alvo_para(std::env::consts::OS, std::env::consts::ARCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pins(assets: &str) -> Pins {
        Pins::ler(&format!(
            r#"{{"format":1,"repository":"pedro-canedo/openweights","tag":"owcli-runtime-34d992ac-v1","revision":"34d992ac469aa4dcf9bb41f02f959dee918bfdbb","assets":{assets}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn pin_sem_pacote_e_valido() {
        assert_eq!(pins("{}").conferir("owcli-runtime"), Ok(()));
    }

    #[test]
    fn tag_e_revisao_andam_juntas() {
        assert!(pins("{}").conferir("agenticow-runtime").is_err());
        let mut p = pins("{}");
        p.revision = "0".repeat(40);
        assert!(p.conferir("owcli-runtime").is_err());
    }

    #[test]
    fn pacote_confere_nome_hash_e_tamanho() {
        let bom = r#"{"linux-x64":{"name":"openweights-owcli-runtime-34d992ac-v1-linux-x64.tar.gz","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","size":90000000}}"#;
        assert_eq!(pins(bom).conferir("owcli-runtime"), Ok(()));
        for ruim in [
            bom.replace("linux-x64.tar.gz", "linux-x64.zip"),
            bom.replace("\"aaaa", "\"AAAA"),
            bom.replace("90000000", "10"),
            bom.replace("{\"linux-x64\"", "{\"linux-arm64\""),
        ] {
            assert!(pins(&ruim).conferir("owcli-runtime").is_err(), "{ruim}");
        }
    }

    #[test]
    fn alvos_por_sistema() {
        assert_eq!(alvo_para("linux", "x86_64"), Some("linux-x64"));
        assert_eq!(alvo_para("windows", "x86_64"), Some("win32-x64"));
        assert_eq!(alvo_para("macos", "aarch64"), Some("darwin-arm64"));
        assert_eq!(alvo_para("macos", "x86_64"), Some("darwin-x64"));
        assert_eq!(alvo_para("linux", "aarch64"), None);
        assert_eq!(alvo_para("windows", "aarch64"), None);
    }

    #[test]
    fn url_da_release() {
        let p = pins("{}");
        let a = Asset {
            name: "x.tar.gz".into(),
            sha256: String::new(),
            size: 0,
        };
        assert_eq!(
            p.url(&a),
            "https://github.com/pedro-canedo/openweights/releases/download/owcli-runtime-34d992ac-v1/x.tar.gz"
        );
    }
}
