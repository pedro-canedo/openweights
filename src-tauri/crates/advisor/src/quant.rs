//! Identificação e ranking de quantizações GGUF a partir do nome do arquivo.
//!
//! Ranking de qualidade (consenso da comunidade, ago/2026):
//! BF16/F16 > Q8_0 > Q6_K > UD-Q5_K_XL > Q5_K_M > UD-Q4_K_XL > Q4_K_M ≈ IQ4_XS
//! > Q3/IQ3 (degradação perceptível) > Q2/IQ2 (forte) > IQ1 (último recurso).
//! > Dinâmicas da Unsloth (UD-*) valem mais que a estática equivalente.

/// Rótulos conhecidos, do melhor para o pior. A posição inversa é o rank.
const QUALITY_ORDER: &[&str] = &[
    "F32",
    "BF16",
    "F16",
    "Q8_0",
    "UD-Q6_K_XL",
    "Q6_K",
    "UD-Q5_K_XL",
    "Q5_K_M",
    "Q5_K_S",
    "Q5_0",
    "UD-Q4_K_XL",
    "Q4_K_M",
    "IQ4_XS",
    "IQ4_NL",
    "Q4_K_S",
    "Q4_1",
    "Q4_0",
    "UD-Q3_K_XL",
    "Q3_K_L",
    "Q3_K_M",
    "IQ3_M",
    "IQ3_S",
    "Q3_K_S",
    "IQ3_XXS",
    "UD-Q2_K_XL",
    "Q2_K",
    "PQ2_0",
    "Q2_0",
    "IQ2_M",
    "IQ2_XS",
    "IQ2_XXS",
    "PTQ1_0",
    "IQ1_M",
    "IQ1_S",
    "Q1_0",
];

/// Rótulos que só o fork da PrismML abre (Bonsai 2). O nome do arquivo é a
/// única pista ANTES do download — o cabeçalho, que é a prova, só existe
/// depois (`lr_models::LocalGgufMeta::exige_prism`).
const PRISM_ONLY: &[&str] = &["PTQ1_0", "PQ2_0"];

/// O arquivo, pelo nome, pede o motor da PrismML.
pub fn requires_prism(filename: &str) -> bool {
    let upper = filename.to_uppercase();
    PRISM_ONLY.iter().any(|l| upper.contains(l))
}

/// Bits por peso aproximados (tabela oficial do HF + docs llama.cpp).
pub fn bits_per_weight(label: &str) -> Option<f32> {
    let base = label.strip_prefix("UD-").unwrap_or(label);
    let bits = match base {
        "F32" => 32.0,
        "BF16" | "F16" => 16.0,
        "Q8_0" => 8.5,
        l if l.starts_with("Q6_K") => 6.5625,
        l if l.starts_with("Q5_K") => 5.5,
        "Q5_0" | "Q5_1" => 5.5,
        l if l.starts_with("Q4_K") => 4.5,
        "IQ4_XS" => 4.25,
        "IQ4_NL" => 4.5,
        "Q4_0" | "Q4_1" => 4.5,
        l if l.starts_with("Q3_K") => 3.4375,
        "IQ3_M" | "IQ3_S" => 3.44,
        "IQ3_XXS" => 3.06,
        l if l.starts_with("Q2_K") => 2.625,
        // Ternários/binários da PrismML (Bonsai): bits declarados no card.
        "PQ2_0" | "Q2_0" => 2.13,
        "PTQ1_0" => 1.75,
        "Q1_0" => 1.13,
        "IQ2_M" | "IQ2_XS" | "IQ2_XXS" => 2.31,
        "IQ1_M" | "IQ1_S" => 1.56,
        _ => return None,
    };
    Some(bits)
}

/// Extrai o rótulo de quantização de um nome de arquivo GGUF.
/// Ex.: `Qwen3-8B-UD-Q4_K_XL.gguf` -> `UD-Q4_K_XL`.
pub fn parse_label(filename: &str) -> String {
    let upper = filename.to_uppercase();
    // Testa os rótulos mais longos primeiro para não casar prefixo
    // (Q4_K_M antes de Q4_K, IQ4_XS antes de Q4...).
    let mut candidates: Vec<&str> = QUALITY_ORDER.to_vec();
    candidates.sort_by_key(|l| std::cmp::Reverse(l.len()));
    for label in candidates {
        if upper.contains(label) {
            return label.to_string();
        }
    }
    "?".to_string()
}

/// Rank de qualidade: maior = melhor. Desconhecido fica no fundo.
pub fn quality_rank(label: &str) -> usize {
    QUALITY_ORDER
        .iter()
        .position(|l| *l == label)
        .map(|p| QUALITY_ORDER.len() - p)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_filenames() {
        assert_eq!(parse_label("Qwen3-8B-Q4_K_M.gguf"), "Q4_K_M");
        assert_eq!(parse_label("model-UD-Q4_K_XL.gguf"), "UD-Q4_K_XL");
        assert_eq!(parse_label("gemma-3-27b-it-IQ4_XS.gguf"), "IQ4_XS");
        assert_eq!(parse_label("llama-bf16.gguf"), "BF16");
        assert_eq!(parse_label("model-q8_0.gguf"), "Q8_0");
        assert_eq!(parse_label("weird-name.gguf"), "?");
    }

    /// Os arquivos da PrismML ganham rótulo em vez de "?", e só os dois
    /// formatos privados do fork pedem o motor dela — `Q2_g64` e `Q1_0` são
    /// do llama.cpp oficial.
    #[test]
    fn prism_labels_are_recognised_and_only_the_fork_formats_require_prism() {
        assert_eq!(parse_label("Ternary-Bonsai-2-27B-PTQ1_0.gguf"), "PTQ1_0");
        assert_eq!(parse_label("Ternary-Bonsai-27B-PQ2_0.gguf"), "PQ2_0");
        assert_eq!(parse_label("Ternary-Bonsai-27B-Q2_0.gguf"), "Q2_0");
        assert_eq!(parse_label("Bonsai-8B-Q1_0.gguf"), "Q1_0");
        assert!(requires_prism("Ternary-Bonsai-2-27B-ptq1_0.gguf"));
        assert!(requires_prism("Ternary-Bonsai-27B-PQ2_0.gguf"));
        assert!(!requires_prism("Ternary-Bonsai-27B-Q2_g64.gguf"));
        assert!(!requires_prism("Ternary-Bonsai-27B-Q2_0.gguf"));
        assert!(!requires_prism("Qwen3-8B-Q4_K_M.gguf"));
        assert!(quality_rank("Q2_K") > quality_rank("PQ2_0"));
        assert!(quality_rank("PQ2_0") > quality_rank("PTQ1_0"));
        assert!(quality_rank("PTQ1_0") > quality_rank("IQ1_S"));
        assert_eq!(bits_per_weight("PTQ1_0"), Some(1.75));
        assert_eq!(bits_per_weight("PQ2_0"), Some(2.13));
    }

    #[test]
    fn ud_beats_static_equivalent() {
        assert!(quality_rank("UD-Q4_K_XL") > quality_rank("Q4_K_M"));
        assert!(quality_rank("Q4_K_M") > quality_rank("IQ4_XS"));
    }

    #[test]
    fn quality_order_is_monotonic_where_expected() {
        assert!(quality_rank("Q8_0") > quality_rank("Q6_K"));
        assert!(quality_rank("Q6_K") > quality_rank("Q5_K_M"));
        assert!(quality_rank("Q5_K_M") > quality_rank("Q4_K_M"));
        assert!(quality_rank("Q4_K_M") > quality_rank("Q3_K_M"));
        assert!(quality_rank("Q3_K_M") > quality_rank("Q2_K"));
        assert!(quality_rank("Q2_K") > quality_rank("IQ1_S"));
        assert!(quality_rank("?") == 0);
    }

    #[test]
    fn bits_table_sane() {
        assert_eq!(bits_per_weight("Q4_K_M"), Some(4.5));
        assert_eq!(bits_per_weight("UD-Q4_K_XL"), Some(4.5));
        assert_eq!(bits_per_weight("BF16"), Some(16.0));
        assert_eq!(bits_per_weight("?"), None);
    }
}
