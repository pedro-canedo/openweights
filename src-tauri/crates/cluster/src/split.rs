//! Ordem `-dev` e proporção `-ts` a partir das memórias anunciadas.
//!
//! O dispositivo com mais memória entra primeiro (camadas da entrada). No
//! relato 3060+Mac isso deu `RPC0,CUDA0` e `6,4` — 18 GB vs 12 GB, que em
//! inteiros reduzidos é `3,2`.

/// Plano de split para o `llama-server` do host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitPlan {
    /// `--device`, ex. `RPC0,CUDA0`.
    pub devices: String,
    /// `--tensor-split`, ex. `3,2`.
    pub tensor_split: String,
    /// O remoto (RPC0) veio primeiro.
    pub rpc_first: bool,
}

/// Tags do llama.cpp: o fio GGML muda entre builds. Diferente = recusar.
pub fn tags_compatible(local: &str, remote: &str) -> bool {
    !local.is_empty() && local == remote
}

/// Argumentos de processo do host (`ServerConfig.extra_args`).
pub fn llama_rpc_args(rpc_addr: &str, plan: &SplitPlan) -> Vec<String> {
    vec![
        "--rpc".into(),
        rpc_addr.into(),
        "--device".into(),
        plan.devices.clone(),
        "--tensor-split".into(),
        plan.tensor_split.clone(),
    ]
}

/// Calcula o split. `local_device` é o pin desta máquina (`CUDA0`/`MTL0`).
pub fn plan_split(local_device: &str, local_bytes: u64, remote_bytes: u64) -> Option<SplitPlan> {
    if local_device.is_empty() || local_bytes == 0 || remote_bytes == 0 {
        return None;
    }
    // MiB para o mdc não explodir; pelo menos 1 para não zerar a fatia.
    let local_n = (local_bytes / (1 << 20)).max(1);
    let remote_n = (remote_bytes / (1 << 20)).max(1);
    let g = gcd(local_n, remote_n);
    let local_p = (local_n / g).max(1);
    let remote_p = (remote_n / g).max(1);

    if remote_bytes >= local_bytes {
        Some(SplitPlan {
            devices: format!("RPC0,{local_device}"),
            tensor_split: format!("{remote_p},{local_p}"),
            rpc_first: true,
        })
    } else {
        Some(SplitPlan {
            devices: format!("{local_device},RPC0"),
            tensor_split: format!("{local_p},{remote_p}"),
            rpc_first: false,
        })
    }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;

    #[test]
    fn mac_18_and_3060_12_puts_rpc_first_as_three_two() {
        // Relato: -dev RPC0,CUDA0 -ts 6,4 ≡ 3,2.
        let plan = plan_split("CUDA0", 12 * GIB, 18 * GIB).unwrap();
        assert_eq!(plan.devices, "RPC0,CUDA0");
        assert_eq!(plan.tensor_split, "3,2");
        assert!(plan.rpc_first);
        let args = llama_rpc_args("192.168.1.8:50052", &plan);
        assert!(args.contains(&"--rpc".into()));
        assert!(args.contains(&"RPC0,CUDA0".into()));
        assert!(args.contains(&"3,2".into()));
    }

    #[test]
    fn when_local_has_more_vram_it_comes_first() {
        let plan = plan_split("MTL0", 18 * GIB, 9 * GIB).unwrap();
        assert_eq!(plan.devices, "MTL0,RPC0");
        assert_eq!(plan.tensor_split, "2,1");
        assert!(!plan.rpc_first);
    }

    #[test]
    fn equal_memory_puts_rpc_first() {
        // Empate: o remoto na frente, como no relato (entrada no emprestado).
        let plan = plan_split("CUDA0", 9 * GIB, 9 * GIB).unwrap();
        assert_eq!(plan.devices, "RPC0,CUDA0");
        assert_eq!(plan.tensor_split, "1,1");
    }

    #[test]
    fn missing_budget_or_device_yields_nothing() {
        assert!(plan_split("CUDA0", 0, 12 * GIB).is_none());
        assert!(plan_split("", 12 * GIB, 12 * GIB).is_none());
        assert!(plan_split("CUDA0", 12 * GIB, 0).is_none());
    }

    #[test]
    fn tags_must_match_exactly() {
        assert!(tags_compatible("b10441", "b10441"));
        assert!(!tags_compatible("b10441", "b10440"));
        assert!(!tags_compatible("", "b10441"));
    }
}
