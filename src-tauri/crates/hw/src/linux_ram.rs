//! A tabela SMBIOS no Linux, quando o kernel deixa lê-la.
//!
//! `/sys/firmware/dmi/tables/DMI` é a tabela crua, sem o cabeçalho que o
//! Windows acrescenta. Costuma ser 0400 (só root), e é por isso que aqui
//! ausência não é erro: o app segue sem o número da banda, como segue sem
//! qualquer outro metadado que a máquina não entrega.

use crate::smbios::{self, MemoryTopology};

pub fn topology() -> MemoryTopology {
    match std::fs::read("/sys/firmware/dmi/tables/DMI") {
        Ok(bytes) => smbios::topology(&smbios::parse_memory_devices(&bytes)),
        Err(e) => {
            log::debug!("SMBIOS indisponível ({e}); seguindo sem a banda de memória");
            MemoryTopology::default()
        }
    }
}
