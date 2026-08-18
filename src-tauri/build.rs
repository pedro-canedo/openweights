use std::io::Write;

fn main() {
    chave_de_voz();
    tauri_build::build()
}

/// Embute a chave da voz neutra (Deepgram) fornecida pelo CI.
///
/// Duas coisas, ditas sem rodeio:
///
/// 1. A chave **não** entra no repositório. Ela chega por variável de
///    ambiente (`OW_DEEPGRAM_KEY`, um secret do GitHub) só na hora de gerar
///    o instalador. Compilando sem ela, o app usa a voz do sistema e ninguém
///    fica sem funcionalidade.
/// 2. Guardar assim **não é segredo de verdade**. Chave que roda no
///    computador de outra pessoa é chave que aquela pessoa pode extrair —
///    do binário ou olhando o próprio tráfego. O XOR abaixo só impede que um
///    `strings` no executável a entregue de bandeja. Proteção real exigiria
///    um servidor nosso intermediando a chamada, que hoje não existe.
fn chave_de_voz() {
    println!("cargo:rerun-if-env-changed=OW_DEEPGRAM_KEY");
    let chave = std::env::var("OW_DEEPGRAM_KEY").unwrap_or_default();

    let cifrada: Vec<String> = chave
        .bytes()
        .enumerate()
        .map(|(i, b)| format!("{}", b ^ MASCARA[i % MASCARA.len()]))
        .collect();

    let destino = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("voz.rs");
    let mut f = std::fs::File::create(&destino).expect("criar voz.rs");
    writeln!(
        f,
        "pub const CHAVE_CIFRADA: &[u8] = &[{}];\npub const MASCARA: &[u8] = &{MASCARA:?};",
        cifrada.join(", ")
    )
    .expect("escrever voz.rs");
}

/// Máscara do XOR. Fixa de propósito: o objetivo é não aparecer em texto
/// puro, não resistir a quem abre o binário com paciência.
const MASCARA: [u8; 8] = [0x5b, 0xa7, 0x1e, 0xc4, 0x39, 0x86, 0x62, 0xf0];
