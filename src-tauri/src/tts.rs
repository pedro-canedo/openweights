//! Voz neutra para "ouvir a resposta", com a voz do sistema como rede.
//!
//! A fala do próprio sistema (`speechSynthesis`) continua sendo o caminho
//! padrão e o plano B: ela não custa rede, não manda nada para lugar nenhum
//! e existe em qualquer máquina. O que muda é que, quando dá, o app usa uma
//! voz melhor — a do Deepgram — e cai para a do sistema em qualquer tropeço:
//! sem chave embutida, sem internet, idioma não atendido, erro do serviço.
//!
//! **O texto vai para um serviço externo.** É a única parte do app que faz
//! isso sem a pessoa pedir explicitamente, então vale ser literal: só o que
//! ela mandou ler sai daqui, e só quando ela clica em ouvir.
//!
//! Sobre o idioma: as vozes Aura-2 são de inglês. Ler português com voz
//! inglesa sai pior que a voz do sistema, então qualquer coisa que não seja
//! inglês nem chega a virar requisição.

use std::time::Duration;

mod embutido {
    include!(concat!(env!("OUT_DIR"), "/voz.rs"));
}

const MODELO: &str = "aura-2-luna-en";
/// Teto de texto por chamada. Resposta longa demais vira conta alta e espera
/// longa; acima disso a voz do sistema atende melhor.
const MAX_CARACTERES: usize = 1800;

/// A chave em claro, ou `None` quando o build não recebeu nenhuma.
fn chave() -> Option<String> {
    if embutido::CHAVE_CIFRADA.is_empty() {
        return None;
    }
    let bytes: Vec<u8> = embutido::CHAVE_CIFRADA
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ embutido::MASCARA[i % embutido::MASCARA.len()])
        .collect();
    String::from_utf8(bytes).ok().filter(|s| !s.is_empty())
}

/// O idioma tem voz neutra? Só o inglês, hoje.
fn atendido(lang: &str) -> bool {
    lang.to_ascii_lowercase().starts_with("en")
}

/// Gera o áudio da fala. Erro aqui não é falha do app: a UI usa a voz do
/// sistema e segue. Por isso a mensagem é curta e serve só para o log.
#[tauri::command]
pub async fn tts_speak(text: String, lang: String) -> Result<tauri::ipc::Response, String> {
    if !atendido(&lang) {
        return Err(format!("sem voz neutra para `{lang}`"));
    }
    let texto = text.trim();
    if texto.is_empty() {
        return Err("nada para ler".into());
    }
    if texto.chars().count() > MAX_CARACTERES {
        return Err("texto longo demais para a voz neutra".into());
    }
    let chave = chave().ok_or("esta compilação não tem chave de voz")?;

    let resposta = reqwest::Client::new()
        .post(format!("https://api.deepgram.com/v1/speak?model={MODELO}"))
        .header("Authorization", format!("Token {chave}"))
        .header("Content-Type", "text/plain")
        .timeout(Duration::from_secs(30))
        .body(texto.to_string())
        .send()
        .await
        .map_err(|e| format!("não deu para falar com o serviço de voz: {e}"))?;

    let status = resposta.status();
    let bytes = resposta
        .bytes()
        .await
        .map_err(|e| format!("resposta de voz truncada: {e}"))?;
    if !status.is_success() {
        // O corpo do erro do Deepgram é JSON curto; cabe no log e ajuda a
        // distinguir chave inválida de cota estourada.
        let corpo = String::from_utf8_lossy(&bytes);
        return Err(format!("serviço de voz recusou ({status}): {corpo:.200}"));
    }

    Ok(tauri::ipc::Response::new(bytes.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Português não vira requisição: voz inglesa lendo português é pior que
    /// a voz do sistema, que é justamente o plano B.
    #[test]
    fn only_english_gets_the_neutral_voice() {
        assert!(atendido("en"));
        assert!(atendido("en-US"));
        assert!(!atendido("pt-BR"));
        assert!(!atendido("es"));
    }

    /// Sem chave no build, a função diz "não tenho" em vez de montar uma
    /// requisição que o serviço recusaria.
    #[test]
    fn a_build_without_a_key_reports_it_instead_of_calling_out() {
        if embutido::CHAVE_CIFRADA.is_empty() {
            assert!(chave().is_none());
        } else {
            // Num build com chave, o que se afirma é que ela volta inteira:
            // o XOR tem de ser reversível byte a byte.
            let k = chave().expect("chave embutida");
            assert_eq!(k.len(), embutido::CHAVE_CIFRADA.len());
            assert!(k.chars().all(|c| c.is_ascii_graphic()));
        }
    }
}
