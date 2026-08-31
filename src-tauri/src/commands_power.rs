//! Limite de energia da GPU: ler sempre, escrever com permissão.
//!
//! Por que isto está num app de inferência: gerar tokens é limitado pela
//! BANDA de memória da placa, não pelo quanto ela pode queimar. Cortar o
//! limite de energia de uma RTX 3090 de 350 W para 250 W custou perto de zero
//! em tokens por segundo numa medição pública — e a mesma carga numa placa de
//! geração seguinte, com quase o dobro de consumo, rendeu 2%, porque a placa
//! nova tem só 8% mais banda. Ou seja: watts a mais aqui viram calor, não
//! velocidade.
//!
//! O que o app faz e o que ele NÃO faz:
//! - **Lê** pelo NVML, sem privilégio nenhum.
//! - **Escreve** por `nvidia-smi -i <n> -pl <w>` num processo elevado — o
//!   NVML exige administrador, e um app de desktop não roda elevado. É um
//!   comando de uma linha, visível na tela antes de rodar.
//! - **Não promete persistência**: o próprio NVML documenta que o limite cai
//!   ao reiniciar a máquina ou recarregar o driver. A tela diz isso.

use crate::state::AppState;
use tauri::State;

type CmdResult<T> = Result<T, String>;

/// Estado de energia de cada placa NVIDIA (vazio fora do Windows/NVML).
#[tauri::command]
pub async fn gpu_power_status(_state: State<'_, AppState>) -> CmdResult<serde_json::Value> {
    #[cfg(windows)]
    {
        Ok(serde_json::to_value(lr_hw::power::status()).unwrap_or(serde_json::Value::Null))
    }
    #[cfg(not(windows))]
    {
        Ok(serde_json::Value::Array(Vec::new()))
    }
}

/// Aplica um limite, pedindo elevação ao sistema.
///
/// Devolve o comando que foi executado para a tela poder mostrá-lo: quem
/// aceita um pedido de administrador tem direito de saber o que autorizou.
#[tauri::command]
pub async fn gpu_power_set(
    _state: State<'_, AppState>,
    index: u32,
    watts: u32,
) -> CmdResult<String> {
    let comando = format!("nvidia-smi -i {index} -pl {watts}");
    #[cfg(windows)]
    {
        // `runas` é o verbo que dispara o UAC. Sem ele o NVML responde
        // "sem permissão" e nada acontece.
        let args = format!("-i {index} -pl {watts}");
        let mut cmd = std::process::Command::new("powershell");
        // Sem isto o pedido de elevação viria acompanhado de uma janela preta
        // piscando na cara de quem clicou — e o projeto cobra a regra num
        // teste que varre a árvore inteira.
        lr_proc::no_window_std(&mut cmd);
        let saida = cmd
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "Start-Process -FilePath 'nvidia-smi' -ArgumentList '{args}' -Verb runas -Wait -WindowStyle Hidden"
                ),
            ])
            .output()
            .map_err(|e| e.to_string())?;
        if !saida.status.success() {
            return Err(String::from_utf8_lossy(&saida.stderr).trim().to_string());
        }
        Ok(comando)
    }
    #[cfg(not(windows))]
    {
        Err(format!("rode como administrador: sudo {comando}"))
    }
}
