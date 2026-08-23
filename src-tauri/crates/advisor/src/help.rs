//! O `--help` do llama-server como fonte de catálogo.
//!
//! O llama.cpp não tem saída de ajuda em JSON, mas o texto do `--help` é
//! gerado de uma tabela (`common/arg.cpp`) e sai num formato estável:
//!
//! ```text
//! ----- common params -----
//!
//! -c,    --ctx-size N                     size of the prompt context (default: 0, 0 = loaded from model)
//!                                         (env: LLAMA_ARG_CTX_SIZE)
//! ```
//!
//! Linha de flag começa na coluna 0 com `-`; a descrição começa na coluna 40
//! (na mesma linha ou na seguinte, quando os nomes são longos); continuações
//! são linhas indentadas. É disso que este módulo extrai [`HelpFlag`]s — as
//! flags "dinâmicas" do catálogo, as que ninguém curou mas que existem na
//! build pinada e por isso merecem aparecer na busca.
//!
//! Como em `devices.rs`, quem responde é o binário de verdade: rodamos o
//! `llama-server --help` da instalação e cacheamos por tag+variante, então
//! uma atualização do runtime invalida o cache sozinha.

use lr_types::flags::HelpFlag;
use std::path::{Path, PathBuf};
use std::time::Duration;

const HELP_TIMEOUT: Duration = Duration::from_secs(15);

/// Abaixo disso o parse não descreve uma build real do llama-server (que tem
/// centenas de flags) — o chamador degrada para o catálogo curado.
pub const MIN_PARSED_FLAGS: usize = 100;

#[derive(Debug, thiserror::Error)]
pub enum HelpError {
    #[error("o pacote do llama.cpp instalado não traz o `{0}`")]
    Missing(String),
    #[error("falha ao rodar o --help: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("o --help demorou demais")]
    Timeout,
    #[error("o --help rendeu só {0} flags — formato inesperado")]
    TooFew(usize),
}

/// Lê o texto do `--help` inteiro.
pub fn parse_help(texto: &str) -> Vec<HelpFlag> {
    let mut out: Vec<HelpFlag> = Vec::new();
    let mut section = String::from("common params");
    // Bloco em montagem: (nomes crus, linhas de descrição).
    let mut atual: Option<(String, Vec<String>)> = None;

    let fecha =
        |atual: &mut Option<(String, Vec<String>)>, out: &mut Vec<HelpFlag>, section: &str| {
            if let Some((nomes, desc)) = atual.take()
                && let Some(flag) = monta_flag(&nomes, &desc, section)
            {
                out.push(flag);
            }
        };

    for linha in texto.lines() {
        let aparada = linha.trim_end();
        // Cabeçalho de seção: `----- sampling params -----`.
        if let Some(titulo) = aparada
            .strip_prefix("-----")
            .and_then(|r| r.strip_suffix("-----"))
        {
            fecha(&mut atual, &mut out, &section);
            section = titulo.trim().to_string();
            continue;
        }
        // Linha de flag: coluna 0, começa com `-` seguido de letra/traço.
        let e_flag = aparada.starts_with('-')
            && aparada
                .chars()
                .nth(1)
                .map(|c| c.is_ascii_alphanumeric() || c == '-')
                .unwrap_or(false);
        if e_flag {
            fecha(&mut atual, &mut out, &section);
            // A descrição mora na coluna 40; a lacuna entre o nome curto e o
            // longo (`-c,    --ctx-size`) também tem 2+ espaços, então o que
            // separa nomes de descrição é a COLUNA, não a primeira lacuna.
            // Nomes que passam da coluna 40 empurram a descrição para as
            // linhas seguintes.
            let cabe = aparada.len() > 40
                && aparada.is_char_boundary(38)
                && aparada.is_char_boundary(40)
                && aparada[38..40] == *"  ";
            let (nomes, resto) = if cabe {
                (aparada[..40].trim_end(), aparada[40..].trim())
            } else {
                (aparada, "")
            };
            let mut desc = Vec::new();
            if !resto.is_empty() {
                desc.push(resto.to_string());
            }
            atual = Some((nomes.to_string(), desc));
            continue;
        }
        // Continuação: linha indentada dentro de um bloco aberto. Linhas em
        // branco no meio do bloco (o `--help` tem algumas) não o encerram.
        if let Some((_, desc)) = atual.as_mut() {
            let cont = aparada.trim();
            if !cont.is_empty() {
                desc.push(cont.to_string());
            }
        }
    }
    fecha(&mut atual, &mut out, &section);
    out
}

/// `"-c,    --ctx-size N"` + descrição → [`HelpFlag`].
fn monta_flag(nomes: &str, desc_linhas: &[String], section: &str) -> Option<HelpFlag> {
    let mut nomes_limpos: Vec<String> = Vec::new();
    let mut hint: Vec<String> = Vec::new();
    for token in nomes.split_whitespace() {
        let t = token.trim_end_matches(',');
        if t.starts_with('-') && t.len() > 1 && !t[1..].starts_with(|c: char| c.is_ascii_digit()) {
            nomes_limpos.push(t.trim_start_matches('-').to_string());
        } else if !t.is_empty() {
            hint.push(t.to_string());
        }
    }
    if nomes_limpos.is_empty() {
        return None;
    }
    // Chave canônica: o primeiro nome longo (`--x`), que é como o llama.cpp
    // moderno lista a forma preferida; sem nome longo, o primeiro que houver.
    let key = nomes
        .split_whitespace()
        .map(|t| t.trim_end_matches(','))
        .find(|t| t.starts_with("--"))
        .map(|t| t.trim_start_matches('-').to_string())
        .unwrap_or_else(|| nomes_limpos[0].clone());
    let aliases: Vec<String> = nomes_limpos.into_iter().filter(|n| *n != key).collect();

    let texto = desc_linhas.join(" ");
    // `(env: LLAMA_ARG_X)` fica fora da descrição.
    let mut env = None;
    let mut descricao = String::new();
    let mut resto = texto.as_str();
    while let Some(i) = resto.find("(env: ") {
        descricao.push_str(&resto[..i]);
        let depois = &resto[i + 6..];
        match depois.find(')') {
            Some(j) => {
                env = Some(depois[..j].trim().to_string());
                resto = &depois[j + 1..];
            }
            None => {
                resto = "";
            }
        }
    }
    descricao.push_str(resto);
    let descricao = descricao.split_whitespace().collect::<Vec<_>>().join(" ");

    // `(default: X)` — fica na descrição (o contexto ajuda), mas também sai
    // estruturado para a interface mostrar "padrão: X".
    // Tanto `(default: 0, 0 = loaded from model)` quanto `('on', 'off', or
    // 'auto', default: 'auto')` aparecem — o marcador é o `default: `, não o
    // parêntese.
    let default = descricao.find("default: ").and_then(|i| {
        let depois = &descricao[i + 9..];
        let bruto = depois[..depois.find(')').unwrap_or(depois.len())].trim();
        // `-1, -1 = infinity` → só o valor; aspas de `'auto'` caem.
        let so_valor = bruto.split(',').next()?.trim().trim_matches('\'');
        (!so_valor.is_empty()).then(|| so_valor.to_string())
    });

    Some(HelpFlag {
        key,
        aliases,
        value_hint: (!hint.is_empty()).then(|| hint.join(" ")),
        description: descricao,
        env,
        default,
        section: section.to_string(),
    })
}

#[derive(serde::Serialize, serde::Deserialize)]
struct HelpCache {
    tag: String,
    variant: String,
    flags: Vec<HelpFlag>,
}

fn cache_path(data_dir: &Path, tag: &str, variant: &str) -> PathBuf {
    data_dir.join(format!("llama-help-{tag}-{variant}.json"))
}

/// As flags do binário instalado, com cache por tag+variante.
///
/// O cache não tem invalidação por tempo de propósito: o conteúdo só muda
/// quando o runtime muda, e aí o nome do arquivo muda junto.
pub async fn help_flags_cached(
    exe: &Path,
    data_dir: &Path,
    tag: &str,
    variant: &str,
) -> Result<Vec<HelpFlag>, HelpError> {
    let cache = cache_path(data_dir, tag, variant);
    if let Ok(bruto) = std::fs::read_to_string(&cache)
        && let Ok(c) = serde_json::from_str::<HelpCache>(&bruto)
        && c.tag == tag
        && c.variant == variant
        && c.flags.len() >= MIN_PARSED_FLAGS
    {
        return Ok(c.flags);
    }

    if !exe.exists() {
        return Err(HelpError::Missing(
            exe.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "llama-server".into()),
        ));
    }
    let mut cmd = tokio::process::Command::new(exe);
    cmd.arg("--help")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // As DLLs moram ao lado do executável.
        .current_dir(exe.parent().unwrap_or(Path::new(".")))
        .kill_on_drop(true);
    lr_proc::no_window(&mut cmd);

    let saida = tokio::time::timeout(HELP_TIMEOUT, cmd.output())
        .await
        .map_err(|_| HelpError::Timeout)??;
    // O --help sai no stdout; o stderr entra junto por via das dúvidas (ruído
    // de backend não atrapalha o parser).
    let texto = format!(
        "{}\n{}",
        String::from_utf8_lossy(&saida.stdout),
        String::from_utf8_lossy(&saida.stderr)
    );
    let flags = parse_help(&texto);
    if flags.len() < MIN_PARSED_FLAGS {
        return Err(HelpError::TooFew(flags.len()));
    }
    let _ = std::fs::write(
        &cache,
        serde_json::to_string(&HelpCache {
            tag: tag.into(),
            variant: variant.into(),
            flags: flags.clone(),
        })
        .expect("HelpCache serializa"),
    );
    Ok(flags)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Recorte literal do `--help` do b10441 (capturado da release oficial).
    const RECORTE: &str = r#"----- common params -----

-h,    --help, --usage                  print usage and exit
-t,    --threads N                      number of CPU threads to use during generation (default: -1)
                                        (env: LLAMA_ARG_THREADS)
-c,    --ctx-size N                     size of the prompt context (default: 0, 0 = loaded from model)
                                        (env: LLAMA_ARG_CTX_SIZE)
-fa,   --flash-attn [on|off|auto]       set Flash Attention use ('on', 'off', or 'auto', default: 'auto')
                                        (env: LLAMA_ARG_FLASH_ATTN)
--rope-scaling {none,linear,yarn}       RoPE frequency scaling method, defaults to linear unless specified by
                                        the model
                                        (env: LLAMA_ARG_ROPE_SCALING_TYPE)
--swa-full                              use full-size SWA cache (default: false)
                                        (env: LLAMA_ARG_SWA_FULL)
-lm,   --load-mode MODE                 model loading mode (default: auto)
                                        - auto: mmap, unless a device does not support it
                                        - dio: use DirectIO if available

                                        (env: LLAMA_ARG_LOAD_MODE)

----- speculative params -----

--spec-draft-model, -md, --model-draft FNAME
                                        draft model for speculative decoding (default: unused)
                                        (env: LLAMA_ARG_SPEC_DRAFT_MODEL)
--spec-draft-n-max N                    number of tokens to draft for speculative decoding (default: 3)
                                        (env: LLAMA_ARG_SPEC_DRAFT_MAX)

----- example-specific params -----

--jinja, --no-jinja                     whether to use jinja template engine for chat (default: enabled)
                                        (env: LLAMA_ARG_JINJA)
-to,   --timeout N                      server read/write timeout in seconds (default: 3600)
"#;

    #[test]
    fn the_snippet_parses_flag_by_flag() {
        let flags = parse_help(RECORTE);
        let por_chave = |k: &str| flags.iter().find(|f| f.key == k).unwrap();

        let ctx = por_chave("ctx-size");
        assert_eq!(ctx.aliases, vec!["c"]);
        assert_eq!(ctx.value_hint.as_deref(), Some("N"));
        assert_eq!(ctx.env.as_deref(), Some("LLAMA_ARG_CTX_SIZE"));
        assert_eq!(ctx.default.as_deref(), Some("0"));
        assert_eq!(ctx.section, "common params");

        let fa = por_chave("flash-attn");
        assert_eq!(fa.value_hint.as_deref(), Some("[on|off|auto]"));
        assert_eq!(fa.default.as_deref(), Some("auto"), "aspas caem");

        // Descrição que continua na linha seguinte vira um texto só.
        let rope = por_chave("rope-scaling");
        assert!(rope.description.ends_with("specified by the model"));

        // Bloco com linha em branco no meio não é cortado.
        let lm = por_chave("load-mode");
        assert_eq!(lm.env.as_deref(), Some("LLAMA_ARG_LOAD_MODE"));
        assert!(lm.description.contains("DirectIO"));

        // Nomes longos demais empurram a descrição para a linha de baixo.
        let draft = por_chave("spec-draft-model");
        assert_eq!(draft.aliases, vec!["md", "model-draft"]);
        assert_eq!(draft.value_hint.as_deref(), Some("FNAME"));
        assert!(draft.description.starts_with("draft model"));

        // Par ligado/desligado vira uma flag com o par como alias.
        let jinja = por_chave("jinja");
        assert_eq!(jinja.aliases, vec!["no-jinja"]);
        assert!(jinja.value_hint.is_none());

        let spec = por_chave("spec-draft-n-max");
        assert_eq!(spec.section, "speculative params");
        assert_eq!(spec.default.as_deref(), Some("3"));

        assert_eq!(por_chave("timeout").section, "example-specific params");
    }

    #[test]
    fn a_bool_flag_has_no_hint_and_infers_bool() {
        let flags = parse_help(RECORTE);
        let swa = flags.iter().find(|f| f.key == "swa-full").unwrap();
        assert!(swa.value_hint.is_none());
        assert_eq!(swa.infer_kind(), lr_types::flags::FlagKind::Bool);
    }

    /// A fixture completa, capturada do binário oficial do b10441. Se o pin
    /// de release mudar, recapture-a e deixe este teste avisar o que mudou.
    #[test]
    fn the_pinned_build_help_parses_whole() {
        let texto = include_str!("../tests/fixtures/llama-server-help-b10441.txt");
        let flags = parse_help(texto);
        assert!(
            flags.len() >= 150,
            "só {} flags — o formato do --help mudou?",
            flags.len()
        );
        for chave in [
            "ctx-size",
            "n-gpu-layers",
            "spec-type",
            "spec-draft-n-max",
            "spec-draft-p-min",
            "cache-type-k",
            "models-dir",
            "models-max",
            "jinja",
            "chat-template",
            "webui",
        ] {
            assert!(
                flags
                    .iter()
                    .any(|f| f.key == chave || f.aliases.iter().any(|a| a == chave)),
                "flag {chave} sumiu do parse"
            );
        }
        // Nenhuma chave vazia ou com traço sobrando.
        assert!(
            flags
                .iter()
                .all(|f| !f.key.is_empty() && !f.key.starts_with('-'))
        );
    }
}
