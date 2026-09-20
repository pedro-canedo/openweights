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

use std::collections::BTreeSet;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// `GGML_TYPE_COUNT` do llama.cpp oficial que o app fixa (`lr_runtime::PINNED_TAG`,
/// b10441): qualquer tensor com tipo igual ou acima disto é desconhecido para
/// aquele binário e faz o carregamento falhar com "unknown type". Acompanha o
/// pino — ao promover a tag, conferir `ggml/include/ggml.h` da release.
pub const GGML_TYPE_COUNT_STOCK: u32 = 43;
/// Tipos privados do fork da PrismML (branch `prism`): ternário de 2 bits
/// com Hadamard e ternário empacotado de ~1,75 bit, os dois do Bonsai 2.
pub const GGML_TYPE_PQ2_0: u32 = 142;
pub const GGML_TYPE_PTQ1_0: u32 = 143;

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
    /// Os tipos ggml de TODOS os tensores (ids numéricos, sem repetição).
    ///
    /// É o que diz se o binário oficial consegue abrir o arquivo: o nome do
    /// arquivo pode mentir, o cabeçalho não. Vazio quando a tabela de
    /// tensores não pôde ser lida — os campos acima continuam válidos.
    pub tensor_types: BTreeSet<u32>,
}

impl LocalGgufMeta {
    /// O arquivo usa um tipo de tensor que o llama.cpp oficial não conhece
    /// e que só o fork da PrismML carrega (Bonsai 2: `PQ2_0`, `PTQ1_0`).
    pub fn exige_prism(&self) -> bool {
        self.tensor_types
            .iter()
            .any(|t| *t >= GGML_TYPE_COUNT_STOCK)
    }

    /// Os tipos que o binário oficial não conhece, com nome quando o app
    /// sabe qual é (`PQ2_0`, `PTQ1_0`) e o id numérico quando não sabe — é
    /// o que uma mensagem de erro consegue dizer de útil.
    pub fn tipos_fora_do_oficial(&self) -> Vec<String> {
        self.tensor_types
            .iter()
            .filter(|t| **t >= GGML_TYPE_COUNT_STOCK)
            .map(|t| match *t {
                GGML_TYPE_PQ2_0 => "PQ2_0".to_string(),
                GGML_TYPE_PTQ1_0 => "PTQ1_0".to_string(),
                outro => format!("tipo {outro}"),
            })
            .collect()
    }
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
    let n_tensors = read_u64(&mut r)?;
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
    // A tabela de tensores vem logo depois dos KVs. Ela é opcional para o
    // resto do cabeçalho: uma tabela que não dá para ler deixa os campos de
    // geometria valendo e só o conjunto de tipos vazio.
    let tensor_types = tipos_de_tensor(&mut r, n_tensors).unwrap_or_default();
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
        tensor_types,
    })
}

/// Teto de tensores da tabela. Um modelo grande tem alguns milhares; um
/// arquivo corrompido não pode nos prender num laço.
const MAX_TENSORS: u64 = 100_000;

/// Os tipos ggml da tabela de tensores, lidos a partir da posição atual (o
/// leitor tem de estar logo depois do último KV).
fn tipos_de_tensor<R: Read>(r: &mut R, n_tensors: u64) -> Option<BTreeSet<u32>> {
    if n_tensors > MAX_TENSORS {
        return None;
    }
    let mut tipos = BTreeSet::new();
    for _ in 0..n_tensors {
        let (_, _, kind, _) = le_tensor_info(r)?;
        tipos.insert(kind);
    }
    Some(tipos)
}

/// Uma entrada da tabela de tensores: nome, dimensões, tipo ggml e offset.
fn le_tensor_info<R: Read>(r: &mut R) -> Option<(String, Vec<u64>, u32, u64)> {
    let name = read_string(r)?;
    let nd = read_u32(r)?;
    if !(1..=4).contains(&nd) {
        return None;
    }
    let mut dims = Vec::with_capacity(nd as usize);
    for _ in 0..nd {
        dims.push(read_u64(r)?);
    }
    let kind = read_u32(r)?;
    let offset = read_u64(r)?;
    Some((name, dims, kind, offset))
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

/// Upper bound for one cached expert across every routed tensor in a shard.
/// Uses file spans (including alignment), so new quantization types do not
/// require a guessed bytes/parameter table. None means unverified geometry.
pub fn expert_slot_bytes(path: &Path, experts: u32) -> Option<u64> {
    if experts == 0 {
        return None;
    }
    let mut r = BufReader::new(std::fs::File::open(path).ok()?);
    let size = r.get_ref().metadata().ok()?.len();
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).ok()?;
    if &magic != b"GGUF" || !matches!(read_u32(&mut r)?, 2 | 3) {
        return None;
    }
    let tensors = read_u64(&mut r)?;
    let kv = read_u64(&mut r)?;
    if tensors > MAX_TENSORS || kv > MAX_KV {
        return None;
    }
    let mut alignment = 32u64;
    for _ in 0..kv {
        let key = read_string(&mut r)?;
        let kind = read_u32(&mut r)?;
        if key == "general.alignment" && kind == 4 {
            alignment = read_u32(&mut r)? as u64;
            continue;
        }
        let bytes = match kind {
            0 | 1 | 7 => 1,
            2 | 3 => 2,
            4..=6 => 4,
            10..=12 => 8,
            8 => {
                skip_string(&mut r)?;
                continue;
            }
            9 => {
                skip_array(&mut r)?;
                continue;
            }
            _ => return None,
        };
        r.seek(SeekFrom::Current(bytes)).ok()?;
    }
    if alignment == 0 || alignment > 1024 * 1024 {
        return None;
    }
    let mut spans = Vec::new();
    for _ in 0..tensors {
        let (name, dims, _kind, offset) = le_tensor_info(&mut r)?;
        let nd = dims.len() as u32;
        let routed = name.contains("_exps.weight") || name.contains("_chexps.weight");
        // GGUF keeps the tensor dimensions in model order, which differs
        // between exporters.  The expert axis is the dimension declared by
        // the file's own expert count; requiring it to be present avoids
        // guessing a layout while still accepting both common orderings.
        if routed && (nd != 3 || !dims.contains(&(experts as u64))) {
            return None;
        }
        spans.push((offset, routed));
    }
    let pos = r.stream_position().ok()?;
    let data = pos.div_ceil(alignment).checked_mul(alignment)?;
    let end = size.checked_sub(data)?;
    spans.sort_unstable_by_key(|s| s.0);
    spans.push((end, false));
    let mut total = 0u64;
    for pair in spans.windows(2) {
        let bytes = pair[1].0.checked_sub(pair[0].0)?;
        if pair[0].1 {
            total = total.checked_add(bytes.div_ceil(experts as u64).checked_add(512)?)?;
        }
    }
    Some(total)
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
        gguf_com_tensores(pairs, &[])
    }

    /// Um tensor da tabela: nome, dimensões e tipo ggml. Os offsets são
    /// sequenciais e fictícios — só o cabeçalho existe.
    type Tensor = (&'static str, Vec<u64>, u32);

    fn gguf_com_tensores(pairs: &[(&str, KV)], tensores: &[Tensor]) -> Vec<u8> {
        let mut out = gguf_cabecalho(pairs, tensores.len() as u64);
        for (i, (nome, dims, kind)) in tensores.iter().enumerate() {
            out.extend_from_slice(&(nome.len() as u64).to_le_bytes());
            out.extend_from_slice(nome.as_bytes());
            out.extend_from_slice(&(dims.len() as u32).to_le_bytes());
            for d in dims {
                out.extend_from_slice(&d.to_le_bytes());
            }
            out.extend_from_slice(&kind.to_le_bytes());
            out.extend_from_slice(&((i as u64) * 4096).to_le_bytes());
        }
        out
    }

    /// Só o cabeçalho e os KVs, anunciando `n_tensores` na contagem — para
    /// os testes de tabela ausente ou absurda.
    fn gguf_cabecalho(pairs: &[(&str, KV)], n_tensores: u64) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"GGUF");
        out.extend_from_slice(&3u32.to_le_bytes());
        out.extend_from_slice(&n_tensores.to_le_bytes());
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
    fn qwen(tensores: &[Tensor]) -> Vec<u8> {
        gguf_com_tensores(
            &[
                ("general.architecture", KV::Str("qwen3")),
                ("qwen3.block_count", KV::U32(64)),
            ],
            tensores,
        )
    }

    /// Tipos que o binário oficial conhece (F32 = 0, Q4_K = 12) não pedem
    /// fork; a geometria continua sendo lida junto.
    #[test]
    fn stock_tensor_types_do_not_require_the_prism_fork() {
        let f = escreve(&qwen(&[
            ("token_embd.weight", vec![5120, 248_320], 0),
            ("blk.0.attn_q.weight", vec![5120, 5120], 12),
        ]));
        let meta = read_local_meta(f.path());
        assert_eq!(meta.n_layers, Some(64));
        assert_eq!(meta.tensor_types, BTreeSet::from([0, 12]));
        assert!(!meta.exige_prism());
    }

    /// Um único tensor `PQ2_0` ou `PTQ1_0` basta: o llama.cpp oficial
    /// recusa o arquivo inteiro como "unknown type".
    #[test]
    fn a_bonsai_2_tensor_type_requires_the_prism_fork() {
        let f = escreve(&qwen(&[
            ("token_embd.weight", vec![5120, 248_320], 12),
            ("blk.0.ffn_up.weight", vec![5120, 17_408], GGML_TYPE_PQ2_0),
        ]));
        assert!(read_local_meta(f.path()).exige_prism());

        let f = escreve(&qwen(&[(
            "blk.0.ffn_up.weight",
            vec![5120, 17_408],
            GGML_TYPE_PTQ1_0,
        )]));
        let meta = read_local_meta(f.path());
        assert!(meta.exige_prism());
        assert_eq!(meta.tensor_types, BTreeSet::from([GGML_TYPE_PTQ1_0]));
        assert_eq!(meta.tipos_fora_do_oficial(), vec!["PTQ1_0"]);
        let desconhecido = LocalGgufMeta {
            tensor_types: BTreeSet::from([12, 150]),
            ..Default::default()
        };
        assert_eq!(desconhecido.tipos_fora_do_oficial(), vec!["tipo 150"]);
    }

    /// Tabela anunciada mas cortada: a geometria dos KVs fica valendo, e o
    /// conjunto de tipos volta vazio — nunca um chute e nunca um pânico.
    #[test]
    fn a_truncated_tensor_table_keeps_the_kv_fields() {
        let mut bytes = qwen(&[("blk.0.attn_q.weight", vec![5120, 5120], 12)]);
        bytes.truncate(bytes.len() - 6);
        let f = escreve(&bytes);
        let meta = read_local_meta(f.path());
        assert_eq!(meta.n_layers, Some(64));
        assert!(meta.tensor_types.is_empty());
        assert!(!meta.exige_prism());
    }

    /// Uma contagem absurda de tensores é ignorada sem tentar ler nada.
    #[test]
    fn an_absurd_tensor_count_is_ignored_without_reading_the_table() {
        let bytes = gguf_cabecalho(
            &[
                ("general.architecture", KV::Str("qwen3")),
                ("qwen3.block_count", KV::U32(64)),
            ],
            200_000,
        );
        let meta = read_local_meta(escreve(&bytes).path());
        assert_eq!(meta.n_layers, Some(64));
        assert!(meta.tensor_types.is_empty());
    }

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
