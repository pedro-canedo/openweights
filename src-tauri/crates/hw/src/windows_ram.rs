//! A tabela SMBIOS pelo caminho mais curto do Windows.
//!
//! `GetSystemFirmwareTable` é chamada Win32 pura: não precisa de COM, não
//! precisa de WMI e não spawna PowerShell — coisas que custariam centenas de
//! milissegundos no boot do app para responder uma pergunta que o firmware já
//! respondeu. O parser fica em [`crate::smbios`], que é puro e testável.

use crate::smbios::{self, MemoryTopology};
use windows::Win32::System::SystemInformation::{FIRMWARE_TABLE_PROVIDER, GetSystemFirmwareTable};

/// Assinatura do provedor "Raw SMBIOS", em big-endian como a API espera.
const RSMB: FIRMWARE_TABLE_PROVIDER = FIRMWARE_TABLE_PROVIDER(u32::from_be_bytes(*b"RSMB"));

/// Cabeçalho `RawSMBIOSData` que o Windows põe antes da tabela: método de
/// chamada, versão maior, versão menor, revisão do DMI e o comprimento.
const CABECALHO: usize = 8;

pub fn topology() -> MemoryTopology {
    let bytes = match tabela_bruta() {
        Some(b) => b,
        None => return MemoryTopology::default(),
    };
    smbios::topology(&smbios::parse_memory_devices(&bytes))
}

fn tabela_bruta() -> Option<Vec<u8>> {
    unsafe {
        let tamanho = GetSystemFirmwareTable(RSMB, 0, None);
        if tamanho == 0 {
            return None;
        }
        let mut buf = vec![0u8; tamanho as usize];
        let escrito = GetSystemFirmwareTable(RSMB, 0, Some(&mut buf)) as usize;
        if escrito == 0 || escrito > buf.len() || escrito <= CABECALHO {
            return None;
        }
        buf.truncate(escrito);
        Some(buf.split_off(CABECALHO))
    }
}
