//! Leitura da tabela SMBIOS para descobrir a MEMÓRIA — velocidade e canais.
//!
//! Existe por causa do teto que nenhuma flag move. Quando os especialistas de
//! um modelo MoE moram na RAM do sistema, tokens por segundo deixa de ser um
//! número de GPU e vira um número de banda de memória: uma DDR4-3200 em dois
//! canais entrega ~51 GB/s, contra centenas na VRAM da placa. Sem esse número
//! na tela, a pessoa fica ajustando flags contra uma parede que o app sabia
//! onde estava.
//!
//! O sistema operacional já tem a resposta: a tabela SMBIOS descreve cada
//! módulo instalado. Este módulo é só o **parser**, puro e testável; quem
//! consegue os bytes é código específico de plataforma.
//!
//! Formato (SMBIOS 3.x, seção "Memory Device", tipo 17): cada estrutura tem
//! um cabeçalho de 4 bytes (tipo, tamanho, handle), a área formatada, e
//! depois um conjunto de strings terminado por dois NUL seguidos. Campos que
//! interessam, em deslocamentos a partir do início da estrutura:
//!
//! | offset | campo |
//! |---|---|
//! | 0x0A | largura de dados, em bits (64 num DIMM comum) |
//! | 0x0C | tamanho (0 = slot vazio) |
//! | 0x11 | índice da string do "bank locator" (é onde o canal aparece) |
//! | 0x15 | velocidade nominal, em MT/s |
//! | 0x20 | velocidade CONFIGURADA, em MT/s (é a que vale) |

/// Um módulo de memória instalado, como a tabela o descreve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryDevice {
    /// Velocidade em MT/s: a configurada quando existe, senão a nominal.
    pub speed_mts: Option<u32>,
    /// Largura de dados em bits (64 num DIMM comum).
    pub data_width_bits: Option<u32>,
    /// Texto do "bank locator" — em placas de desktop é onde o canal aparece
    /// ("P0 CHANNEL A").
    pub bank_locator: Option<String>,
    /// Texto do "device locator" ("DIMM A1") — o segundo lugar onde o canal
    /// costuma estar escrito.
    pub device_locator: Option<String>,
}

/// O que a tabela diz sobre a memória desta máquina.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MemoryTopology {
    pub speed_mts: Option<u32>,
    pub channels: Option<u32>,
    pub bandwidth_bytes_s: Option<u64>,
}

/// Percorre as estruturas SMBIOS e devolve os módulos INSTALADOS (tipo 17
/// com tamanho diferente de zero).
///
/// Defensivo por decisão: qualquer coisa fora do esperado encerra a leitura
/// com o que já deu para entender, em vez de falhar. Um campo que não existe
/// nesta versão da tabela vira `None`, que é sempre mais seguro que um chute.
pub fn parse_memory_devices(tabela: &[u8]) -> Vec<MemoryDevice> {
    let mut out = Vec::new();
    let mut i = 0usize;
    // Um teto: tabela corrompida não pode nos prender num laço.
    for _ in 0..512 {
        if i + 4 > tabela.len() {
            break;
        }
        let tipo = tabela[i];
        // O byte 1 é o tamanho da ÁREA FORMATADA, cabeçalho incluído.
        let formatado = tabela[i + 1] as usize;
        if formatado < 4 || i + formatado > tabela.len() {
            break;
        }
        // Fim da área formatada; daqui até dois NUL seguidos vêm as strings.
        let inicio_strings = i + formatado;
        let (strings, fim) = le_strings(tabela, inicio_strings);

        if tipo == 17 {
            let campo_u16 = |off: usize| -> Option<u16> {
                if off + 2 <= formatado {
                    Some(u16::from_le_bytes([tabela[i + off], tabela[i + off + 1]]))
                } else {
                    None
                }
            };
            let campo_str = |off: usize| -> Option<String> {
                let idx = if off < formatado {
                    tabela[i + off] as usize
                } else {
                    0
                };
                if idx == 0 {
                    None
                } else {
                    strings.get(idx - 1).cloned()
                }
            };
            let tamanho = campo_u16(0x0C).unwrap_or(0);
            if tamanho != 0 {
                // A configurada (2.7+) é a que a placa de fato usa; a nominal
                // é o que o módulo sabe fazer. Preferir a primeira evita
                // prometer 3600 numa máquina rodando a 2133.
                let speed = campo_u16(0x20)
                    .filter(|v| *v > 0)
                    .or_else(|| campo_u16(0x15).filter(|v| *v > 0))
                    .map(u32::from);
                out.push(MemoryDevice {
                    speed_mts: speed,
                    data_width_bits: campo_u16(0x0A)
                        .filter(|v| *v > 0 && *v != 0xFFFF)
                        .map(u32::from),
                    bank_locator: campo_str(0x11),
                    device_locator: campo_str(0x10),
                });
            }
        }

        if fim <= i {
            break;
        }
        i = fim;
        if tipo == 127 {
            break;
        }
    }
    out
}

/// Lê o bloco de strings que segue a área formatada. Devolve as strings e o
/// deslocamento da próxima estrutura.
fn le_strings(tabela: &[u8], inicio: usize) -> (Vec<String>, usize) {
    let mut strings = Vec::new();
    let mut i = inicio;
    // Bloco vazio é codificado como dois NUL.
    if i + 1 < tabela.len() && tabela[i] == 0 && tabela[i + 1] == 0 {
        return (strings, i + 2);
    }
    let mut atual = Vec::new();
    while i < tabela.len() {
        let b = tabela[i];
        i += 1;
        if b == 0 {
            if atual.is_empty() {
                // Segundo NUL seguido: fim do bloco.
                return (strings, i);
            }
            strings.push(String::from_utf8_lossy(&atual).trim().to_string());
            atual.clear();
        } else {
            atual.push(b);
        }
    }
    (strings, i)
}

/// Do que a tabela diz para o número que a tela mostra.
///
/// Canais são contados por NOME, não por número de módulos: dois pentes no
/// mesmo canal não dobram a banda, e é exatamente esse o erro que faria a
/// tela prometer o dobro do que a máquina entrega. Quando nenhum dos textos
/// nomeia um canal, o app diz que não sabe em vez de chutar.
pub fn topology(devices: &[MemoryDevice]) -> MemoryTopology {
    if devices.is_empty() {
        return MemoryTopology::default();
    }
    // A mais lenta manda: o controlador roda todos os módulos no mesmo passo.
    let speed = devices.iter().filter_map(|d| d.speed_mts).min();
    let largura = devices
        .iter()
        .filter_map(|d| d.data_width_bits)
        .min()
        .unwrap_or(64);

    let mut canais: Vec<String> = devices
        .iter()
        .filter_map(|d| {
            canal(d.bank_locator.as_deref()).or_else(|| canal(d.device_locator.as_deref()))
        })
        .collect();
    canais.sort();
    canais.dedup();
    let channels = (!canais.is_empty()).then_some(canais.len() as u32);

    let bandwidth_bytes_s = match (speed, channels) {
        (Some(s), Some(c)) => {
            Some(u64::from(s) * 1_000_000 * u64::from(largura / 8) * u64::from(c))
        }
        _ => None,
    };
    MemoryTopology {
        speed_mts: speed,
        channels,
        bandwidth_bytes_s,
    }
}

/// O identificador do canal escrito num texto de localização.
///
/// Dois formatos cobrem o que as placas escrevem: "P0 CHANNEL A" (bank
/// locator, comum em desktop) e "DIMM A1"/"ChannelA-DIMM0" (device locator).
fn canal(texto: Option<&str>) -> Option<String> {
    let t = texto?.to_ascii_uppercase();
    if let Some(pos) = t.find("CHANNEL") {
        let resto = t[pos + "CHANNEL".len()..].trim_start_matches(['-', '_', ' ']);
        let id: String = resto
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        if !id.is_empty() {
            return Some(id);
        }
    }
    if let Some(resto) = t.strip_prefix("DIMM") {
        let resto = resto.trim_start();
        let c = resto.chars().next()?;
        if c.is_ascii_alphabetic() {
            return Some(c.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monta uma estrutura tipo 17 com os campos que o parser lê.
    fn memory_device(size_mb: u16, speed: u16, configured: u16, bank: &str) -> Vec<u8> {
        // Área formatada de 0x54 bytes (SMBIOS 3.3), zerada e preenchida.
        let mut s = vec![0u8; 0x54];
        s[0] = 17;
        s[1] = 0x54;
        s[0x0A..0x0C].copy_from_slice(&64u16.to_le_bytes()); // largura de dados
        s[0x0C..0x0E].copy_from_slice(&size_mb.to_le_bytes());
        s[0x10] = 1; // device locator -> string 1
        s[0x11] = 2; // bank locator -> string 2
        s[0x15..0x17].copy_from_slice(&speed.to_le_bytes());
        s[0x20..0x22].copy_from_slice(&configured.to_le_bytes());
        s.extend_from_slice(b"DIMM 0\0");
        s.extend_from_slice(bank.as_bytes());
        s.push(0);
        s.push(0); // fim do bloco de strings
        s
    }

    fn fim_da_tabela() -> Vec<u8> {
        vec![127, 4, 0, 0, 0, 0]
    }

    #[test]
    fn the_table_becomes_modules_with_speed_and_bank() {
        let mut t = memory_device(16384, 3200, 3200, "P0 CHANNEL A");
        t.extend(memory_device(16384, 3200, 3200, "P0 CHANNEL B"));
        t.extend(fim_da_tabela());

        let devs = parse_memory_devices(&t);
        assert_eq!(devs.len(), 2, "dois módulos instalados");
        assert_eq!(devs[0].speed_mts, Some(3200));
        assert_eq!(devs[0].data_width_bits, Some(64));
        assert_eq!(devs[0].bank_locator.as_deref(), Some("P0 CHANNEL A"));
    }

    /// O número que a tela mostra, na máquina do vídeo e na do projeto:
    /// DDR4-3200 em dois canais = 3200 × 8 bytes × 2 ≈ 51 GB/s.
    #[test]
    fn two_channels_of_ddr4_3200_are_fifty_one_gigabytes_per_second() {
        let mut t = memory_device(16384, 3200, 3200, "P0 CHANNEL A");
        t.extend(memory_device(16384, 3200, 3200, "P0 CHANNEL B"));
        t.extend(fim_da_tabela());

        let topo = topology(&parse_memory_devices(&t));
        assert_eq!(topo.speed_mts, Some(3200));
        assert_eq!(topo.channels, Some(2));
        assert_eq!(topo.bandwidth_bytes_s, Some(51_200_000_000));
    }

    /// Quatro pentes em dois canais entregam a banda de DOIS canais. Contar
    /// módulos daria 102 GB/s numa máquina que faz 51.
    #[test]
    fn four_modules_in_two_channels_are_still_two_channels() {
        let mut t = memory_device(16384, 3200, 3200, "P0 CHANNEL A");
        t.extend(memory_device(16384, 3200, 3200, "P0 CHANNEL A"));
        t.extend(memory_device(16384, 3200, 3200, "P0 CHANNEL B"));
        t.extend(memory_device(16384, 3200, 3200, "P0 CHANNEL B"));
        t.extend(fim_da_tabela());

        let topo = topology(&parse_memory_devices(&t));
        assert_eq!(topo.channels, Some(2));
        assert_eq!(topo.bandwidth_bytes_s, Some(51_200_000_000));
    }

    /// A velocidade CONFIGURADA vence a nominal: prometer 3600 numa máquina
    /// rodando a 2133 seria pior do que não dizer nada.
    #[test]
    fn the_configured_speed_wins_over_the_rated_one() {
        let mut t = memory_device(16384, 3600, 2133, "P0 CHANNEL A");
        t.extend(fim_da_tabela());
        assert_eq!(parse_memory_devices(&t)[0].speed_mts, Some(2133));
    }

    /// Slot vazio não é módulo.
    #[test]
    fn an_empty_slot_is_not_a_module() {
        let mut t = memory_device(0, 3200, 3200, "P0 CHANNEL B");
        t.extend(fim_da_tabela());
        assert!(parse_memory_devices(&t).is_empty());
    }

    /// Sem nome de canal em lugar nenhum, o app diz que não sabe — em vez de
    /// inventar uma banda a partir do número de pentes.
    #[test]
    fn without_a_channel_name_we_say_we_do_not_know() {
        let mut s = vec![0u8; 0x54];
        s[0] = 17;
        s[1] = 0x54;
        s[0x0A..0x0C].copy_from_slice(&64u16.to_le_bytes());
        s[0x0C..0x0E].copy_from_slice(&16384u16.to_le_bytes());
        s[0x15..0x17].copy_from_slice(&3200u16.to_le_bytes());
        s.push(0);
        s.push(0);
        s.extend(fim_da_tabela());

        let topo = topology(&parse_memory_devices(&s));
        assert_eq!(topo.speed_mts, Some(3200), "a velocidade ainda é um fato");
        assert_eq!(topo.channels, None);
        assert_eq!(topo.bandwidth_bytes_s, None, "sem canais, sem banda");
    }
}
