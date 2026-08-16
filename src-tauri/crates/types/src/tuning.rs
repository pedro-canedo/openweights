//! Como um modelo é carregado nesta máquina.
//!
//! Até aqui o app sabia guardar uma única coisa por modelo — o tamanho da
//! janela — e mandava mais nada ao motor. O resto (quantas camadas na placa,
//! que tipo de KV cache, flash attention, lote) ficava por conta do
//! llama.cpp… exceto que fixar a janela desliga o ajuste automático dele, e
//! aí ninguém escolhia. Este tipo é o pacote inteiro, num lugar só.
//!
//! **Campo ausente = deixa o llama.cpp decidir.** É a diferença entre "a
//! pessoa (ou o otimizador) escolheu isto" e "não temos opinião" — e é o que
//! permite ligar de volta o `--fit` quando não há nada fixado.

use serde::{Deserialize, Serialize};

/// Tipo do KV cache. Comprimir troca um pouco de qualidade por memória, e é
/// o botão que mais rende quando a janela é grande.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KvType {
    F16,
    Q8_0,
    Q4_0,
}

impl KvType {
    /// Valor que o llama.cpp entende (`cache-type-k`/`cache-type-v`).
    pub fn as_str(&self) -> &'static str {
        match self {
            KvType::F16 => "f16",
            KvType::Q8_0 => "q8_0",
            KvType::Q4_0 => "q4_0",
        }
    }

    /// Bytes por elemento — entra na conta de memória do KV cache.
    pub fn bytes_per_elem(&self) -> f32 {
        match self {
            KvType::F16 => 2.0,
            KvType::Q8_0 => 1.0625,
            KvType::Q4_0 => 0.5625,
        }
    }
}

/// Tipo de especulação (`--spec-type`). Acelera a geração prevendo tokens à
/// frente; quanto rende depende da máquina e do texto, então nada aqui é
/// ligado sem medir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpecType {
    /// Sem especulação.
    None,
    /// Camadas de previsão de múltiplos tokens que já vêm no próprio arquivo.
    Mtp,
    /// Repetição do próprio texto, sem segundo modelo. Rende em código e
    /// reescrita; em prosa, atrapalha.
    Ngram,
}

impl SpecType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SpecType::None => "none",
            SpecType::Mtp => "draft-mtp",
            SpecType::Ngram => "ngram-mod",
        }
    }
}

/// Como o projetor de visão é carregado.
///
/// Ele custa memória de vídeo (perto de 1 GB em modelos grandes) e só serve
/// quando há imagem na conversa. `OnDemand` resolve isso sem recarregar nada
/// na hora: o motor ganha uma **segunda entrada** do mesmo modelo, com o
/// projetor, e o roteador troca de uma para a outra quando a conversa passa a
/// ter imagem — que é exatamente o que ele já faz ao trocar de modelo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum VisionMode {
    /// Sem visão: o projetor não é carregado nem oferecido.
    Off,
    /// Carregado só quando a conversa tem imagem.
    #[default]
    OnDemand,
    /// Sempre carregado junto do modelo.
    Always,
}

/// De onde veio a configuração — a interface diz isso à pessoa, e a diferença
/// entre calculado e medido é a razão de ser do recurso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ProfileSource {
    /// A pessoa escolheu na mão.
    #[default]
    Manual,
    /// Calculado a partir do hardware e do arquivo (sem rodar o modelo).
    Recommended,
    /// Medido nesta máquina.
    Tested,
}

/// Configuração de carga de um modelo.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProfile {
    /// Janela de contexto por conversa.
    pub ctx: Option<u32>,
    /// Camadas na GPU. É **resultado**, não botão: aparece na tela como
    /// "36 de 36 camadas na placa", e quem escolhe é o otimizador.
    pub ngl: Option<u32>,
    /// Camadas de especialistas (MoE) mantidas na CPU — o que faz um modelo
    /// grande caber numa placa pequena sem virar CPU puro.
    pub ncmoe: Option<u32>,
    pub kv_k: Option<KvType>,
    pub kv_v: Option<KvType>,
    pub flash_attn: Option<bool>,
    pub batch: Option<u32>,
    pub ubatch: Option<u32>,
    pub threads: Option<u32>,
    pub spec: Option<SpecType>,
    /// Projetor de visão a carregar junto (caminho absoluto).
    pub mmproj: Option<String>,
    /// Quando carregar o projetor. Ausente = o padrão (sob demanda).
    pub vision: Option<VisionMode>,
    pub source: ProfileSource,
}

impl ModelProfile {
    /// Nada escolhido: o llama.cpp decide tudo.
    pub fn is_empty(&self) -> bool {
        *self == ModelProfile::default()
    }

    /// Identificador curto e estável desta configuração.
    ///
    /// Serve para carimbar o desempenho medido: duas respostas com a mesma
    /// chave foram geradas com a mesma configuração e podem ser comparadas.
    /// Perfil vazio não tem chave — não há configuração a atribuir.
    pub fn key(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        // FNV-1a sobre as chaves do INI (já ordenadas pela construção), que
        // é exatamente o que muda o comportamento do motor. O `source` fica
        // de fora de propósito: "recomendado" e "manual" com os mesmos
        // valores rendem igual.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for (k, v) in self.to_ini_extras() {
            for b in k.bytes().chain(b"=".iter().copied()).chain(v.bytes()) {
                hash ^= b as u64;
                hash = hash.wrapping_mul(0x100_0000_01b3);
            }
        }
        Some(format!("{hash:016x}"))
    }

    /// Chaves do INI do Router para este perfil.
    ///
    /// Devolve pares crus, e não uma entrada do preset, para o crate que
    /// conhece o formato do INI continuar sendo um só. `fit` só é desligado
    /// quando existe alguma escolha de memória — sem isso, desligar o ajuste
    /// automático do llama.cpp seria tirar a rede e não pôr nada no lugar.
    pub fn to_ini_extras(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        let mut push = |k: &str, v: String| out.push((k.to_string(), v));

        if let Some(ctx) = self.ctx {
            push("ctx-size", ctx.to_string());
        }
        if let Some(ngl) = self.ngl {
            push("n-gpu-layers", ngl.to_string());
        }
        if let Some(n) = self.ncmoe {
            push("n-cpu-moe", n.to_string());
        }
        if let Some(k) = self.kv_k {
            push("cache-type-k", k.as_str().to_string());
        }
        if let Some(v) = self.kv_v {
            push("cache-type-v", v.as_str().to_string());
        }
        if let Some(fa) = self.flash_attn {
            push("flash-attn", if fa { "on" } else { "off" }.to_string());
        }
        if let Some(b) = self.batch {
            push("batch-size", b.to_string());
        }
        if let Some(u) = self.ubatch {
            push("ubatch-size", u.to_string());
        }
        if let Some(t) = self.threads {
            push("threads", t.to_string());
        }
        if let Some(s) = self.spec
            && s != SpecType::None
        {
            push("spec-type", s.as_str().to_string());
        }
        // Só a entrada que carrega o projetor recebe a chave. Sob demanda,
        // quem a recebe é a entrada companheira, montada por quem escreve o
        // INI — esta aqui continua sendo a leve.
        if let Some(p) = &self.mmproj
            && self.vision.unwrap_or_default() == VisionMode::Always
        {
            push("mmproj", p.replace('\\', "/"));
        }

        // O `--fit` reparte a memória sozinho. Ele só sai de cena quando já
        // dissemos quanto de janela e quantas camadas queremos; caso
        // contrário, fixar só um dos dois deixaria o outro no escuro.
        if self.ctx.is_some() && self.ngl.is_some() {
            push("fit", "off".to_string());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_profile_says_nothing_to_the_engine() {
        let p = ModelProfile::default();
        assert!(p.is_empty());
        assert!(p.to_ini_extras().is_empty(), "sem escolha, sem chave");
    }

    #[test]
    fn each_choice_becomes_the_key_the_engine_knows() {
        let p = ModelProfile {
            ctx: Some(16384),
            ngl: Some(36),
            ncmoe: Some(4),
            kv_k: Some(KvType::Q8_0),
            kv_v: Some(KvType::Q8_0),
            flash_attn: Some(true),
            batch: Some(1024),
            ubatch: Some(256),
            threads: Some(8),
            spec: Some(SpecType::Ngram),
            mmproj: Some(r"C:\modelos\mmproj.gguf".into()),
            vision: Some(VisionMode::Always),
            source: ProfileSource::Recommended,
        };
        let m: std::collections::HashMap<_, _> = p.to_ini_extras().into_iter().collect();
        assert_eq!(m["ctx-size"], "16384");
        assert_eq!(m["n-gpu-layers"], "36");
        assert_eq!(m["n-cpu-moe"], "4");
        assert_eq!(m["cache-type-k"], "q8_0");
        assert_eq!(m["cache-type-v"], "q8_0");
        assert_eq!(m["flash-attn"], "on");
        assert_eq!(m["batch-size"], "1024");
        assert_eq!(m["ubatch-size"], "256");
        assert_eq!(m["threads"], "8");
        assert_eq!(m["spec-type"], "ngram-mod");
        // O parser do INI do llama.cpp trata `\` como escape.
        assert_eq!(m["mmproj"], "C:/modelos/mmproj.gguf");
        assert_eq!(m["fit"], "off");
    }

    /// Desligar o ajuste automático sem dizer quantas camadas queremos foi
    /// exatamente o que fazia o modelo não caber depois de fixar a janela.
    #[test]
    fn fit_is_only_turned_off_when_we_answered_both_questions() {
        let so_ctx = ModelProfile {
            ctx: Some(32768),
            ..Default::default()
        };
        let extras: std::collections::HashMap<_, _> = so_ctx.to_ini_extras().into_iter().collect();
        assert_eq!(extras["ctx-size"], "32768");
        assert!(!extras.contains_key("fit"), "sem camadas, o fit continua");

        let ambos = ModelProfile {
            ngl: Some(20),
            ..so_ctx
        };
        assert_eq!(
            ambos
                .to_ini_extras()
                .into_iter()
                .collect::<std::collections::HashMap<_, _>>()["fit"],
            "off"
        );
    }

    /// Sob demanda, a entrada principal continua leve: quem carrega o
    /// projetor é a companheira, e é o roteador que troca entre as duas.
    #[test]
    fn on_demand_vision_does_not_weigh_on_the_main_entry() {
        let p = ModelProfile {
            mmproj: Some("/m/mmproj.gguf".into()),
            vision: Some(VisionMode::OnDemand),
            ..Default::default()
        };
        assert!(!p.to_ini_extras().iter().any(|(k, _)| k == "mmproj"));

        let sempre = ModelProfile {
            vision: Some(VisionMode::Always),
            ..p.clone()
        };
        assert!(sempre.to_ini_extras().iter().any(|(k, _)| k == "mmproj"));

        let desligada = ModelProfile {
            vision: Some(VisionMode::Off),
            ..p
        };
        assert!(!desligada.to_ini_extras().iter().any(|(k, _)| k == "mmproj"));
    }

    #[test]
    fn no_speculation_writes_no_key() {
        let p = ModelProfile {
            spec: Some(SpecType::None),
            ..Default::default()
        };
        assert!(p.to_ini_extras().is_empty());
    }

    #[test]
    fn the_key_changes_only_when_the_engine_would_behave_differently() {
        let base = ModelProfile {
            ctx: Some(8192),
            kv_k: Some(KvType::Q8_0),
            ..Default::default()
        };
        assert_eq!(base.key(), base.clone().key());

        // Mudar de onde veio a escolha não muda como o motor roda.
        let mesma = ModelProfile {
            source: ProfileSource::Recommended,
            ..base.clone()
        };
        assert_eq!(mesma.key(), base.key());

        let outra = ModelProfile {
            ctx: Some(16384),
            ..base.clone()
        };
        assert_ne!(outra.key(), base.key());

        assert!(ModelProfile::default().key().is_none());
    }

    #[test]
    fn kv_compression_is_worth_memory() {
        assert!(KvType::Q8_0.bytes_per_elem() < KvType::F16.bytes_per_elem());
        assert!(KvType::Q4_0.bytes_per_elem() < KvType::Q8_0.bytes_per_elem());
    }

    #[test]
    fn the_profile_survives_a_json_round_trip() {
        let p = ModelProfile {
            ctx: Some(8192),
            kv_k: Some(KvType::Q4_0),
            spec: Some(SpecType::Mtp),
            source: ProfileSource::Tested,
            ..Default::default()
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"kvK\":\"q4_0\""), "{json}");
        assert!(json.contains("\"spec\":\"mtp\""), "{json}");
        assert_eq!(serde_json::from_str::<ModelProfile>(&json).unwrap(), p);
    }
}
