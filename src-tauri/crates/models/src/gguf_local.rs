//! Leitura do CABEÇALHO de um GGUF local — sem carregar tensor nenhum.
//!
//! Existe por causa de um bug caro: o advisor gravava `n-gpu-layers` a partir
//! de uma tabela de chute por faixa de parâmetros (20–40B → "48 camadas"), e
//! com `fit = off` o chute virava lei. Um Qwen3.8-27B tem 65 camadas; as 17
//! que sobravam iam para a CPU e a geração caía de 23 para 4 tok/s — sem
//! erro em lugar nenhum. O número real está no arquivo, nos primeiros
//! kilobytes, e ler custa menos de um milissegundo.
//!
//! O parser é deliberadamente defensivo: qualquer coisa fora do esperado
//! devolve `None` no campo (nunca erro, nunca pânico) — quem consome trata
//! ausência como "não sei", que é sempre mais seguro que um chute.

use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// O que o cabeçalho diz sobre o modelo — só o que as decisões de carga usam.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalGgufMeta {
    /// Número REAL de camadas (`{arch}.block_count`) — é o `ngl` de carga
    /// inteira, sem chute.
    pub n_layers: Option<u32>,
    /// Janela de treino (`{arch}.context_length`): pedir além disso degrada a
    /// resposta em silêncio.
    pub context_length: Option<u32>,
    /// Especialistas MoE (`{arch}.expert_count`) — presente e > 0 quer dizer
    /// "Mixture of Experts", que é o que habilita `n-cpu-moe`.
    pub n_experts: Option<u32>,
    /// Especialistas que DISPARAM por token (`{arch}.expert_used_count`).
    ///
    /// É a razão de um MoE caber onde não deveria: o arquivo tem 256
    /// especialistas, mas cada token acorda oito. O resto fica parado — e
    /// peso parado pode morar na RAM do sistema.
    pub n_experts_used: Option<u32>,
    /// Cabeças de atenção KV (`{arch}.attention.head_count_kv`). Entra direto
    /// na conta do KV cache, que até aqui era chutada por faixa de parâmetros.
    pub n_kv_heads: Option<u32>,
    /// Cabeças de atenção (`{arch}.attention.head_count`).
    pub n_heads: Option<u32>,
    /// Dimensão do embedding (`{arch}.embedding_length`) — de onde sai o
    /// `head_dim` quando o arquivo não o declara.
    pub embedding_length: Option<u32>,
    /// Tamanho da chave por cabeça (`{arch}.attention.key_length`), quando
    /// declarado. É o `head_dim` sem intermediários.
    pub key_length: Option<u32>,
    /// Tamanho do valor por cabeça (`{arch}.attention.value_length`).
    pub value_length: Option<u32>,
    /// Dimensão interna de UM especialista
    /// (`{arch}.expert_feed_forward_length`) — com o número de especialistas,
    /// é o que diz que fatia do arquivo pode sair da placa.
    pub expert_ffn_length: Option<u32>,
    /// Dimensão interna do especialista COMPARTILHADO
    /// (`{arch}.expert_shared_feed_forward_length`), que dispara em todo
    /// token e por isso fica na placa.
    pub expert_shared_ffn_length: Option<u32>,
    /// Dimensão interna do FFN denso (`{arch}.feed_forward_length`).
    pub ffn_length: Option<u32>,
    /// Camadas de previsão multi-token (`{arch}.nextn_predict_layers`) — a
    /// cabeça MTP dos GGUF que suportam `--spec-type draft-mtp`. Ausente é
    /// "não sei", não "não tem": arquiteturas novas podem usar outra chave, e
    /// a interface nunca deve bloquear por isso.
    pub nextn_layers: Option<u32>,
    /// Níveis de esforço de raciocínio que o template ACEITA, na ordem em
    /// que ele os lista.
    ///
    /// Não é preferência nem chute: o template do Qwen3.8 recusa qualquer
    /// outro valor com `raise_exception`, e o llama.cpp devolve erro 500. Os
    /// nomes saem da própria linha que faz essa validação, então o seletor da
    /// interface oferece exatamente o que o arquivo aceita — nem um a mais,
    /// nem um a menos.
    pub reasoning_efforts: Vec<String>,
    /// O chat template aceita `enable_thinking` — isto é, o raciocínio do
    /// modelo pode ser LIGADO E DESLIGADO por quem chama, via
    /// `chat_template_kwargs`.
    ///
    /// É a única maneira honesta de saber: não existe chave de metadado que
    /// diga "sou um modelo de raciocínio", e adivinhar pelo nome erra nos
    /// dois sentidos. O template é o que o llama.cpp de fato executa, então
    /// se ele lê a variável, o botão funciona.
    pub thinking_toggle: bool,
}

/// Teto de pares chave/valor lidos. Um GGUF normal tem dezenas; um arquivo
/// corrompido não pode nos prender num laço.
const MAX_KV: u64 = 4096;
/// Teto de bytes para uma string ou array do cabeçalho.
const MAX_STR: u64 = 1 << 20;

/// Lê `block_count` e `context_length` do cabeçalho de um GGUF.
pub fn read_local_meta(path: &Path) -> LocalGgufMeta {
    parse(path).unwrap_or_default()
}

fn parse(path: &Path) -> Option<LocalGgufMeta> {
    let mut r = BufReader::new(std::fs::File::open(path).ok()?);

    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).ok()?;
    if &magic != b"GGUF" {
        return None;
    }
    let _version = read_u32(&mut r)?;
    let _n_tensors = read_u64(&mut r)?;
    let n_kv = read_u64(&mut r)?.min(MAX_KV);

    let mut arch: Option<String> = None;
    let mut thinking_toggle = false;
    let mut reasoning_efforts: Vec<String> = Vec::new();
    let mut valores: Vec<(String, u64)> = Vec::new();

    for _ in 0..n_kv {
        let key = read_string(&mut r)?;
        let tipo = read_u32(&mut r)?;
        match tipo {
            // Inteiros: são os únicos valores que interessam.
            0 | 1 => {
                let v = read_bytes_as_u64(&mut r, 1)?;
                guarda(&mut valores, &key, v);
            }
            2 | 3 => {
                let v = read_bytes_as_u64(&mut r, 2)?;
                guarda(&mut valores, &key, v);
            }
            4 | 5 => {
                let v = read_bytes_as_u64(&mut r, 4)?;
                guarda(&mut valores, &key, v);
            }
            10 | 11 => {
                let v = read_bytes_as_u64(&mut r, 8)?;
                guarda(&mut valores, &key, v);
            }
            6 => {
                r.seek(SeekFrom::Current(4)).ok()?;
            }
            12 => {
                r.seek(SeekFrom::Current(8)).ok()?;
            }
            7 => {
                r.seek(SeekFrom::Current(1)).ok()?;
            }
            // String: a arquitetura e o chat template importam; o resto é
            // pulado. O template chega a dezenas de KB, então dele fica só
            // a resposta de uma pergunta — nunca o texto.
            8 => {
                if key == "general.architecture" {
                    arch = Some(read_string(&mut r)?);
                } else if key == "tokenizer.chat_template" {
                    let tpl = read_string(&mut r)?;
                    thinking_toggle = tpl.contains("enable_thinking");
                    reasoning_efforts = niveis_de_esforco(&tpl);
                } else {
                    skip_string(&mut r)?;
                }
            }
            // Array: pular por inteiro (tokenizer mora aqui, e é enorme).
            9 => skip_array(&mut r)?,
            _ => return None,
        }
    }

    let arch = arch?;
    let acha = |sufixo: &str| {
        valores
            .iter()
            .find(|(k, _)| *k == format!("{arch}.{sufixo}"))
            .and_then(|(_, v)| u32::try_from(*v).ok())
            .filter(|v| *v > 0)
    };
    Some(LocalGgufMeta {
        n_layers: acha("block_count"),
        context_length: acha("context_length"),
        n_experts: acha("expert_count"),
        n_experts_used: acha("expert_used_count"),
        n_kv_heads: acha("attention.head_count_kv"),
        n_heads: acha("attention.head_count"),
        embedding_length: acha("embedding_length"),
        key_length: acha("attention.key_length"),
        value_length: acha("attention.value_length"),
        expert_ffn_length: acha("expert_feed_forward_length"),
        expert_shared_ffn_length: acha("expert_shared_feed_forward_length"),
        ffn_length: acha("feed_forward_length"),
        nextn_layers: acha("nextn_predict_layers"),
        thinking_toggle,
        reasoning_efforts,
    })
}

/// Só guarda o que pode vir a interessar — o resto nem aloca.
fn guarda(valores: &mut Vec<(String, u64)>, key: &str, v: u64) {
    if key.ends_with(".block_count")
        || key.ends_with(".context_length")
        || key.ends_with(".expert_count")
        || key.ends_with(".expert_used_count")
        || key.ends_with(".attention.head_count_kv")
        || key.ends_with(".attention.head_count")
        || key.ends_with(".attention.key_length")
        || key.ends_with(".attention.value_length")
        || key.ends_with(".embedding_length")
        || key.ends_with(".expert_feed_forward_length")
        || key.ends_with(".expert_shared_feed_forward_length")
        || key.ends_with(".feed_forward_length")
        || key.ends_with(".nextn_predict_layers")
    {
        valores.push((key.to_string(), v));
    }
}

fn read_u32<R: Read>(r: &mut R) -> Option<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b).ok()?;
    Some(u32::from_le_bytes(b))
}

fn read_u64<R: Read>(r: &mut R) -> Option<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b).ok()?;
    Some(u64::from_le_bytes(b))
}

fn read_bytes_as_u64<R: Read>(r: &mut R, n: usize) -> Option<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b[..n]).ok()?;
    Some(u64::from_le_bytes(b))
}

fn read_string<R: Read>(r: &mut R) -> Option<String> {
    let len = read_u64(r)?;
    if len > MAX_STR {
        return None;
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

fn skip_string<R: Read + Seek>(r: &mut R) -> Option<()> {
    let len = read_u64(r)?;
    if len > MAX_STR {
        return None;
    }
    r.seek(SeekFrom::Current(len as i64)).ok()?;
    Some(())
}

fn skip_array<R: Read + Seek>(r: &mut R) -> Option<()> {
    let tipo = read_u32(r)?;
    let n = read_u64(r)?;
    let fixo: u64 = match tipo {
        0 | 1 | 7 => 1,
        2 | 3 => 2,
        4..=6 => 4,
        10..=12 => 8,
        8 => {
            // Array de strings: pular uma a uma.
            for _ in 0..n.min(10_000_000) {
                skip_string(r)?;
            }
            return Some(());
        }
        _ => return None,
    };
    let total = n.checked_mul(fixo)?;
    r.seek(SeekFrom::Current(i64::try_from(total).ok()?)).ok()?;
    Some(())
}

/// Os níveis de esforço que o chat template aceita.
///
/// A fonte é a linha em que o próprio template recusa o que não conhece —
/// no Qwen3.8, `resolved_reasoning_effort not in ('xhigh', 'medium', 'low')`.
/// Ler dali é o oposto de adivinhar: um nível que passe por aqui é um nível
/// que o modelo aceita, e a interface não oferece nada que dê erro 500.
///
/// Template sem essa validação devolve lista vazia — aí o app fica no que
/// sabe (ligado/desligado), em vez de inventar nomes.
fn niveis_de_esforco(tpl: &str) -> Vec<String> {
    let Some(i) = tpl.find("reasoning_effort not in") else {
        return Vec::new();
    };
    let resto = &tpl[i..];
    let Some(a) = resto.find('(') else {
        return Vec::new();
    };
    let Some(b) = resto[a..].find(')') else {
        return Vec::new();
    };
    resto[a + 1..a + b]
        .split(',')
        .filter_map(|p| {
            let n = p.trim().trim_matches('\'').trim_matches('"').trim();
            // Só nomes simples: o que vier com espaço ou vazio não é nível.
            (!n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
                .then(|| n.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Monta um GGUF sintético só de cabeçalho, no formato v3.
    fn gguf(pairs: &[(&str, KV)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"GGUF");
        out.extend_from_slice(&3u32.to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes()); // tensores
        out.extend_from_slice(&(pairs.len() as u64).to_le_bytes());
        for (key, val) in pairs {
            out.extend_from_slice(&(key.len() as u64).to_le_bytes());
            out.extend_from_slice(key.as_bytes());
            match val {
                KV::U32(v) => {
                    out.extend_from_slice(&4u32.to_le_bytes());
                    out.extend_from_slice(&v.to_le_bytes());
                }
                KV::Str(s) => {
                    out.extend_from_slice(&8u32.to_le_bytes());
                    out.extend_from_slice(&(s.len() as u64).to_le_bytes());
                    out.extend_from_slice(s.as_bytes());
                }
                KV::ArrU32(items) => {
                    out.extend_from_slice(&9u32.to_le_bytes());
                    out.extend_from_slice(&4u32.to_le_bytes());
                    out.extend_from_slice(&(items.len() as u64).to_le_bytes());
                    for i in items {
                        out.extend_from_slice(&i.to_le_bytes());
                    }
                }
            }
        }
        out
    }

    enum KV {
        U32(u32),
        Str(&'static str),
        ArrU32(Vec<u32>),
    }

    fn escreve(bytes: &[u8]) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        f
    }

    /// O caso que motivou o módulo: o arquivo diz 65 camadas, e é o 65 que
    /// tem de chegar ao advisor — não o "48" da tabela de chute.
    #[test]
    fn the_real_layer_count_comes_from_the_file() {
        let f = escreve(&gguf(&[
            ("general.architecture", KV::Str("qwen35")),
            ("qwen35.block_count", KV::U32(65)),
            ("qwen35.context_length", KV::U32(262_144)),
            // Um array no meio não pode atrapalhar o parse.
            ("tokenizer.ggml.token_ids", KV::ArrU32(vec![1, 2, 3])),
        ]));
        let meta = read_local_meta(f.path());
        assert_eq!(meta.n_layers, Some(65));
        assert_eq!(meta.context_length, Some(262_144));
        assert_eq!(meta.n_experts, None, "denso: sem especialistas");
        assert_eq!(meta.nextn_layers, None, "sem cabeça MTP declarada");
    }

    /// MoE e cabeça MTP saem do mesmo cabeçalho — são os fatos que ligam os
    /// badges de `n-cpu-moe` e `draft-mtp` na tela de configuração.
    #[test]
    fn moe_and_mtp_head_are_read_when_declared() {
        let f = escreve(&gguf(&[
            ("general.architecture", KV::Str("qwen36moe")),
            ("qwen36moe.block_count", KV::U32(48)),
            ("qwen36moe.expert_count", KV::U32(128)),
            ("qwen36moe.nextn_predict_layers", KV::U32(1)),
        ]));
        let meta = read_local_meta(f.path());
        assert_eq!(meta.n_experts, Some(128));
        assert_eq!(meta.nextn_layers, Some(1));
    }

    /// A geometria que faz a conta de memória parar de ser chute: quantos
    /// especialistas disparam por token, quantas cabeças de KV existem e que
    /// fatia do arquivo é especialista roteado.
    #[test]
    fn the_geometry_that_decides_what_can_leave_the_card() {
        let f = escreve(&gguf(&[
            ("general.architecture", KV::Str("qwen36moe")),
            ("qwen36moe.block_count", KV::U32(48)),
            ("qwen36moe.expert_count", KV::U32(128)),
            ("qwen36moe.expert_used_count", KV::U32(8)),
            ("qwen36moe.attention.head_count", KV::U32(32)),
            ("qwen36moe.attention.head_count_kv", KV::U32(4)),
            ("qwen36moe.attention.key_length", KV::U32(128)),
            ("qwen36moe.attention.value_length", KV::U32(128)),
            ("qwen36moe.embedding_length", KV::U32(4096)),
            ("qwen36moe.expert_feed_forward_length", KV::U32(768)),
            ("qwen36moe.expert_shared_feed_forward_length", KV::U32(512)),
            ("qwen36moe.feed_forward_length", KV::U32(12288)),
        ]));
        let meta = read_local_meta(f.path());
        assert_eq!(meta.n_experts_used, Some(8), "8 de 128 disparam por token");
        assert_eq!(meta.n_kv_heads, Some(4));
        assert_eq!(meta.n_heads, Some(32));
        assert_eq!(meta.key_length, Some(128));
        assert_eq!(meta.value_length, Some(128));
        assert_eq!(meta.embedding_length, Some(4096));
        assert_eq!(meta.expert_ffn_length, Some(768));
        assert_eq!(meta.expert_shared_ffn_length, Some(512));
        assert_eq!(meta.ffn_length, Some(12288));
    }

    /// O botão de raciocínio do harness sai daqui: o template do Qwen3 lê
    /// `enable_thinking`, e é isso — não o nome do modelo — que diz se
    /// desligar o raciocínio é possível.
    #[test]
    fn a_thinking_toggle_is_read_from_the_chat_template() {
        let com = escreve(&gguf(&[
            ("general.architecture", KV::Str("qwen35")),
            ("qwen35.block_count", KV::U32(65)),
            (
                "tokenizer.chat_template",
                KV::Str("{%- if enable_thinking %}<think>{%- endif %}"),
            ),
        ]));
        assert!(read_local_meta(com.path()).thinking_toggle);

        // Template sem a variável: o modelo pensa (ou não) sozinho, e o app
        // não pode oferecer um botão que não faria nada.
        let sem = escreve(&gguf(&[
            ("general.architecture", KV::Str("llama")),
            ("llama.block_count", KV::U32(32)),
            (
                "tokenizer.chat_template",
                KV::Str("{{ bos_token }}{% for m in messages %}{{ m.content }}{% endfor %}"),
            ),
        ]));
        assert!(!read_local_meta(sem.path()).thinking_toggle);

        // Sem template nenhum é "não" — ausência nunca vira promessa.
        let nada = escreve(&gguf(&[
            ("general.architecture", KV::Str("llama")),
            ("llama.block_count", KV::U32(32)),
        ]));
        assert!(!read_local_meta(nada.path()).thinking_toggle);
    }

    /// Os níveis saem da linha em que o template RECUSA o que não conhece —
    /// a mesma que faz o llama.cpp devolver 500 para um valor inventado.
    #[test]
    fn the_effort_levels_come_from_the_templates_own_validation() {
        let tpl = "{%- if resolved_reasoning_effort not in ('xhigh', 'medium', 'low') %}\
                   {{- raise_exception('Unexpected reasoning effort') }}{%- endif %}";
        let f = escreve(&gguf(&[
            ("general.architecture", KV::Str("qwen35")),
            ("qwen35.block_count", KV::U32(65)),
            ("tokenizer.chat_template", KV::Str(tpl)),
        ]));
        let meta = read_local_meta(f.path());
        assert_eq!(meta.reasoning_efforts, vec!["xhigh", "medium", "low"]);
    }

    /// Template sem a validação não ganha níveis inventados: o app fica no
    /// que sabe (ligado/desligado) em vez de oferecer um valor que dá erro.
    #[test]
    fn a_template_without_that_line_offers_no_levels() {
        let f = escreve(&gguf(&[
            ("general.architecture", KV::Str("qwen35")),
            ("qwen35.block_count", KV::U32(65)),
            (
                "tokenizer.chat_template",
                KV::Str("{%- if enable_thinking %}<think>{%- endif %}"),
            ),
        ]));
        let meta = read_local_meta(f.path());
        assert!(meta.thinking_toggle, "o interruptor continua sendo lido");
        assert!(meta.reasoning_efforts.is_empty());
    }

    /// Lixo, arquivo vazio e magic errado devolvem "não sei" — nunca pânico.
    #[test]
    fn garbage_yields_unknown_not_a_panic() {
        let vazio = escreve(b"");
        assert_eq!(read_local_meta(vazio.path()), LocalGgufMeta::default());
        let errado = escreve(b"NOPE1234567890");
        assert_eq!(read_local_meta(errado.path()), LocalGgufMeta::default());
        let sem_arch = escreve(&gguf(&[("qwen35.block_count", KV::U32(65))]));
        assert_eq!(read_local_meta(sem_arch.path()), LocalGgufMeta::default());
        assert_eq!(
            read_local_meta(std::path::Path::new("/nao/existe.gguf")),
            LocalGgufMeta::default()
        );
    }
}
