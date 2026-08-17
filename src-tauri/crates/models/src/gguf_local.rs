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
            // String: só a arquitetura importa; as outras são puladas.
            8 => {
                if key == "general.architecture" {
                    arch = Some(read_string(&mut r)?);
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
    })
}

/// Só guarda o que pode vir a interessar — o resto nem aloca.
fn guarda(valores: &mut Vec<(String, u64)>, key: &str, v: u64) {
    if key.ends_with(".block_count") || key.ends_with(".context_length") {
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
