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

/// Aproxima `a/b` por uma fração de inteiros pequenos.
///
/// Sem isto o `-ts` sai como `12233,9216`: a VRAM de uma placa raramente é um
/// número redondo, e o MDC de dois números quaisquer costuma ser 1. Como o
/// llama.cpp normaliza a razão, `4,3` diz a mesma coisa e cabe na tela.
/// Frações contínuas, com denominador limitado — o erro fica abaixo de 1%,
/// que é ruído perto da granularidade de uma camada.
pub fn ratio_pequena(a: u64, b: u64) -> (u64, u64) {
    const MAX_DEN: u64 = 16;
    if a == 0 || b == 0 {
        return (a.max(1), b.max(1));
    }
    let alvo = a as f64 / b as f64;
    let (mut melhor, mut erro) = ((a, b), f64::MAX);
    for den in 1..=MAX_DEN {
        let num = ((alvo * den as f64).round() as u64).max(1);
        let e = ((num as f64 / den as f64) - alvo).abs() / alvo;
        // `<` e não `<=`: entre dois empates fica o de denominador menor.
        if e < erro - 1e-12 {
            erro = e;
            melhor = (num, den);
        }
        if erro < 0.01 {
            break;
        }
    }
    melhor
}

/// Split a partir do que o MOTOR respondeu, não do que nós estimamos.
///
/// `devices` vem do `--list-devices` com `--rpc`: nomes e memória livre reais,
/// de ambos os lados. É o caminho preferido — `plan_split` é o retrato
/// anunciado pelo mDNS, que só existe antes de haver conexão.
pub fn plan_from_devices(devices: &[(String, u64)]) -> Option<SplitPlan> {
    let mut uteis: Vec<(String, u64)> = devices
        .iter()
        .filter(|(_, livre)| *livre > 0)
        .cloned()
        .collect();
    if uteis.len() < 2 {
        return None;
    }
    uteis.sort_by_key(|(_, livre)| std::cmp::Reverse(*livre));
    // Um host e um worker: os dois primeiros bastam, e mais que isso o `-ts`
    // não saberia distribuir sem medir cada um.
    uteis.truncate(2);
    let (primeiro, segundo) = (&uteis[0], &uteis[1]);
    let (p, s) = ratio_pequena(primeiro.1 / (1 << 20), segundo.1 / (1 << 20));
    Some(SplitPlan {
        devices: format!("{},{}", primeiro.0, segundo.0),
        tensor_split: format!("{p},{s}"),
        rpc_first: primeiro.0.starts_with("RPC"),
    })
}

/// Calcula o split. `local_device` é o pin desta máquina (`CUDA0`/`MTL0`).
pub fn plan_split(local_device: &str, local_bytes: u64, remote_bytes: u64) -> Option<SplitPlan> {
    if local_device.is_empty() || local_bytes == 0 || remote_bytes == 0 {
        return None;
    }
    // MiB para o mdc não explodir; pelo menos 1 para não zerar a fatia.
    let local_n = (local_bytes / (1 << 20)).max(1);
    let remote_n = (remote_bytes / (1 << 20)).max(1);
    let (local_p, remote_p) = ratio_pequena(local_n, remote_n);

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
    fn a_razao_cabe_na_tela() {
        // O caso real: 15,93 GiB × 0,75 = 12233 MiB, primo com 9216. Antes
        // isto virava `-ts 12233,9216` no painel.
        assert_eq!(ratio_pequena(12233, 9216), (4, 3));
        assert_eq!(ratio_pequena(18432, 12288), (3, 2));
        assert_eq!(ratio_pequena(9216, 9216), (1, 1));
        assert_eq!(ratio_pequena(0, 100), (1, 100));
    }

    #[test]
    fn o_split_medido_ganha_do_anunciado() {
        const MIB: u64 = 1 << 20;
        let plan =
            plan_from_devices(&[("CUDA0".into(), 15402 * MIB), ("RPC0".into(), 12043 * MIB)])
                .unwrap();
        assert_eq!(plan.devices, "CUDA0,RPC0");
        assert_eq!(plan.tensor_split, "9,7", "15402/12043 ≈ 1,279 ≈ 9/7");
        assert!(!plan.rpc_first);
    }

    #[test]
    fn o_motor_manda_no_nome_e_na_ordem() {
        const MIB: u64 = 1 << 20;
        // Nome que a nossa tabela de pinos não produziria, e o remoto na frente.
        let plan = plan_from_devices(&[
            ("Vulkan1".into(), 8 * 1024 * MIB),
            ("RPC0".into(), 16 * 1024 * MIB),
        ])
        .unwrap();
        assert_eq!(plan.devices, "RPC0,Vulkan1");
        assert_eq!(plan.tensor_split, "2,1");
        assert!(plan.rpc_first);
    }

    #[test]
    fn um_dispositivo_so_nao_e_cluster() {
        assert!(plan_from_devices(&[("CUDA0".into(), 1 << 30)]).is_none());
        assert!(plan_from_devices(&[("CUDA0".into(), 1 << 30), ("RPC0".into(), 0)]).is_none());
    }

    #[test]
    fn tags_must_match_exactly() {
        assert!(tags_compatible("b10441", "b10441"));
        assert!(!tags_compatible("b10441", "b10440"));
        assert!(!tags_compatible("", "b10441"));
    }
}
