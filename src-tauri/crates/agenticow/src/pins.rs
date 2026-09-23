//! O runtime que ESTE binário do app aceita: `pins.json`, embutido na
//! compilação. O workflow `agenticow-runtime.yml` publica a release e gera o
//! arquivo (sha256 e tamanho por alvo); o commit que sobe o pin copia-o para
//! cá. Como o pin vai dentro do binário, ele herda a assinatura do updater — a
//! release no GitHub pode ser trocada, o hash que o app aceita não.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

const PINS_JSON: &str = include_str!("../pins.json");

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

/// Os pins deste binário. Um `pins.json` malformado não compila na prática:
/// o teste `os_pins_embutidos_sao_validos` barra antes do release.
pub fn pins() -> &'static Pins {
    static PINS: OnceLock<Pins> = OnceLock::new();
    PINS.get_or_init(|| serde_json::from_str(PINS_JSON).expect("pins.json embutido inválido"))
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

/// O pacote desta máquina, se a release publicada tiver um.
pub fn asset_atual() -> Option<(&'static str, &'static Asset)> {
    let alvo = alvo_atual()?;
    pins().assets.get(alvo).map(|a| (alvo, a))
}

/// URL de download de um pacote da release pinada.
pub fn url(asset: &Asset) -> String {
    let p = pins();
    format!(
        "https://github.com/{}/releases/download/{}/{}",
        p.repository, p.tag, asset.name
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_pins_embutidos_sao_validos() {
        let p = pins();
        assert_eq!(p.format, 1);
        assert_eq!(p.repository, "pedro-canedo/openweights");
        assert_eq!(p.revision.len(), 40, "revisão completa do fork");
        assert!(p.revision.chars().all(|c| c.is_ascii_hexdigit()));
        // A tag carrega os 8 primeiros caracteres da revisão: tag e revisão andam juntas.
        let esperado = format!("agenticow-runtime-{}-v", &p.revision[..8]);
        assert!(
            p.tag.starts_with(&esperado),
            "{} não bate com {}",
            p.tag,
            p.revision
        );
        for (alvo, a) in &p.assets {
            assert!(
                ["linux-x64", "win32-x64", "darwin-arm64", "darwin-x64"].contains(&alvo.as_str()),
                "alvo {alvo}"
            );
            let ext = if alvo == "win32-x64" { "zip" } else { "tar.gz" };
            assert_eq!(a.name, format!("openweights-{}-{alvo}.{ext}", p.tag));
            assert_eq!(a.sha256.len(), 64);
            assert!(
                a.sha256
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            );
            assert!(
                a.size > 1_000_000,
                "pacote de {alvo} pequeno demais: {}",
                a.size
            );
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
        let a = Asset {
            name: "x.tar.gz".into(),
            sha256: String::new(),
            size: 0,
        };
        assert_eq!(
            url(&a),
            format!(
                "https://github.com/pedro-canedo/openweights/releases/download/{}/x.tar.gz",
                pins().tag
            )
        );
    }
}
