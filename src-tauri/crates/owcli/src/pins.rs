//! O runtime que ESTE binário do app aceita: `pins.json`, embutido na
//! compilação. O workflow `owcli-runtime.yml` publica a release e gera o
//! arquivo; o commit que sobe o pin copia-o para cá. Sem pacote para esta
//! máquina (`assets` vazio enquanto o runtime não foi publicado), o app diz
//! que o agente não está disponível aqui.

use std::sync::OnceLock;

pub use lr_fetch::pins::{Asset, Pins, alvo_atual};

const PINS_JSON: &str = include_str!("../pins.json");

/// O prefixo das tags do runtime: `owcli-runtime-<rev8>-v<n>`.
pub const RUNTIME: &str = "owcli-runtime";

/// Os pins deste binário. Um `pins.json` malformado não chega a release: o
/// teste `os_pins_embutidos_sao_validos` barra antes.
///
/// Só em build de desenvolvimento, `OW_OWCLI_PINS` aponta para outro
/// `pins.json` (e `OW_OWCLI_URL_BASE`, em [`url`], para onde baixar): é como
/// se testa a instalação de ponta a ponta com um pacote feito na máquina,
/// antes de o workflow publicar. A release só conhece o pin embutido.
pub fn pins() -> &'static Pins {
    static PINS: OnceLock<Pins> = OnceLock::new();
    PINS.get_or_init(|| {
        #[cfg(debug_assertions)]
        if let Some(outro) = std::env::var_os("OW_OWCLI_PINS")
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|json| Pins::ler(&json).ok())
        {
            log::warn!("OwCLI: pins de desenvolvimento ({})", outro.tag);
            return outro;
        }
        Pins::ler(PINS_JSON).expect("pins.json embutido inválido")
    })
}

/// O pacote desta máquina, se a release pinada tiver um.
pub fn asset_atual() -> Option<(&'static str, &'static Asset)> {
    pins().asset_atual()
}

/// De onde baixar um pacote: a release pinada.
pub fn url(asset: &Asset) -> String {
    #[cfg(debug_assertions)]
    if let Some(base) = std::env::var_os("OW_OWCLI_URL_BASE") {
        return format!(
            "{}/{}",
            base.to_string_lossy().trim_end_matches('/'),
            asset.name
        );
    }
    pins().url(asset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_pins_embutidos_sao_validos() {
        // O embutido, não o que o `OW_OWCLI_PINS` de quem roda o teste aponte.
        assert_eq!(Pins::ler(PINS_JSON).unwrap().conferir(RUNTIME), Ok(()));
    }
}
