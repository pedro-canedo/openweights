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

/// Como o arquivo do modelo é levado para a memória (`--load-mode`).
///
/// Substitui `--mmap`/`--no-mmap`/`--mlock`, que o llama.cpp marca como
/// obsoletas desde a b10441 — as três decidiam a mesma coisa por três portas.
///
/// A escolha que importa é [`LoadMode::None`]: ela lê o arquivo inteiro para
/// a RAM antes de responder, o que rende quando sobra memória **e cobra caro
/// quando não sobra**. Com o modelo maior que a RAM — o caso normal quando os
/// especialistas de um MoE moram fora da placa — o mapeamento é o que faz o
/// trabalho, porque a page cache guarda as páginas quentes e descarta o
/// resto. Desligá-lo ali troca leitura sob demanda por swap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LoadMode {
    /// O llama.cpp decide (mapeia, salvo em dispositivo que não suporta).
    Auto,
    /// Sem mapeamento: carrega tudo para a RAM antes de servir.
    None,
    /// Mapeia o arquivo.
    Mmap,
    /// Sem mapeamento e travado na RAM.
    Mlock,
    /// Mapeia e trava as páginas na RAM — o que impede o núcleo de despejar
    /// os especialistas depois de horas ociosos e fazer o primeiro token
    /// custar segundos.
    MmapMlock,
    /// Leitura direta, sem passar pela cache do sistema, quando o disco e o
    /// sistema de arquivos suportam.
    Dio,
}

impl LoadMode {
    /// Valor que o llama.cpp entende.
    pub fn as_str(&self) -> &'static str {
        match self {
            LoadMode::Auto => "auto",
            LoadMode::None => "none",
            LoadMode::Mmap => "mmap",
            LoadMode::Mlock => "mlock",
            LoadMode::MmapMlock => "mmap+mlock",
            LoadMode::Dio => "dio",
        }
    }
}

/// Tipo de especulação (`--spec-type`). Acelera a geração prevendo tokens à
/// frente; quanto rende depende da máquina e do texto, então nada aqui é
/// ligado sem medir.
///
/// A ordem das variantes é o critério de ordenação de um [`SpecSet`] — mexer
/// nela muda a chave de identidade de perfis já medidos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpecType {
    /// Sem especulação.
    None,
    /// Modelo de rascunho externo, o esquema clássico.
    DraftSimple,
    /// Cabeça EAGLE-3 treinada junto do modelo.
    DraftEagle3,
    /// Camadas de previsão de múltiplos tokens que já vêm no próprio arquivo.
    ///
    /// O alias existe porque perfis gravados antes da lista diziam `"mtp"`, e
    /// um perfil que não desserializa SOME em silêncio (o store faz `.ok()`).
    #[serde(alias = "mtp")]
    DraftMtp,
    /// Rascunho paralelo DFlash.
    DraftDflash,
    /// Rascunho paralelo dSpark.
    DraftDspark,
    /// n-grama simples sobre o texto já visto.
    NgramSimple,
    /// n-grama com mapa por chave.
    NgramMapK,
    /// n-grama com mapa por chave, quatro valores.
    NgramMapK4v,
    /// Repetição do próprio texto, sem segundo modelo. Rende em código e
    /// reescrita; em prosa, atrapalha. Alias `"ngram"` pelo mesmo motivo do
    /// `"mtp"` acima.
    #[serde(alias = "ngram")]
    NgramMod,
    /// n-grama com cache em arquivo.
    NgramCache,
}

impl SpecType {
    /// O valor que o llama.cpp lê. Os três primeiros nomes são CONTRATO: a
    /// `key()` de um perfil é um hash sobre estes bytes, e mudá-los desligaria
    /// o histórico de desempenho de toda máquina que já mediu.
    pub fn as_str(&self) -> &'static str {
        match self {
            SpecType::None => "none",
            SpecType::DraftSimple => "draft-simple",
            SpecType::DraftEagle3 => "draft-eagle3",
            SpecType::DraftMtp => "draft-mtp",
            SpecType::DraftDflash => "draft-dflash",
            SpecType::DraftDspark => "draft-dspark",
            SpecType::NgramSimple => "ngram-simple",
            SpecType::NgramMapK => "ngram-map-k",
            SpecType::NgramMapK4v => "ngram-map-k4v",
            SpecType::NgramMod => "ngram-mod",
            SpecType::NgramCache => "ngram-cache",
        }
    }
}

/// Os tipos de especulação ligados ao mesmo tempo.
///
/// O `--spec-type` do llama.cpp aceita uma LISTA, e os tipos são
/// complementares: um rascunho (MTP, EAGLE) adivinha texto novo, enquanto o
/// n-grama adivinha o que já está escrito no prompt — que é o dia inteiro de
/// um agente de código reescrevendo arquivos que ele mesmo acabou de ler.
/// Escolher um só era jogar fora metade do ganho.
///
/// Serializa SEMPRE como lista; aceita ler a string solta que as versões
/// antigas gravaram.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SpecSet(Vec<SpecType>);

impl SpecSet {
    /// Normaliza: sem repetidos, sem `None` acompanhado, e em ordem estável.
    ///
    /// Ordenar é o que faz `[Mtp, Ngram]` e `[Ngram, Mtp]` serem a MESMA
    /// configuração — e portanto a mesma linha do histórico de desempenho. Se
    /// algum dia se comprovar que o motor trata a ordem como prioridade, isso
    /// vira uma decisão explícita, com interface própria.
    pub fn new(tipos: impl IntoIterator<Item = SpecType>) -> Self {
        let mut v: Vec<SpecType> = tipos.into_iter().collect();
        v.sort_unstable();
        v.dedup();
        if v.len() > 1 {
            v.retain(|t| *t != SpecType::None);
        }
        Self(v)
    }

    /// Nada ligado: vazio ou apenas `None`.
    pub fn is_off(&self) -> bool {
        self.0.is_empty() || self.0 == [SpecType::None]
    }

    /// O valor do `spec-type` no INI.
    pub fn as_ini_value(&self) -> String {
        self.0
            .iter()
            .map(|t| t.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }

    pub fn iter(&self) -> impl Iterator<Item = SpecType> + '_ {
        self.0.iter().copied()
    }

    pub fn contains(&self, t: SpecType) -> bool {
        self.0.contains(&t)
    }
}

impl From<SpecType> for SpecSet {
    fn from(t: SpecType) -> Self {
        Self::new([t])
    }
}

impl<'de> Deserialize<'de> for SpecSet {
    /// Aceita as duas formas: a string solta dos perfis já gravados
    /// (`"mtp"`) e a lista de hoje (`["draftMtp","ngramMod"]`).
    ///
    /// Isto NÃO é conveniência: `Store::model_profile` desserializa com
    /// `.ok()`, então um formato recusado não vira erro — vira um perfil que
    /// desaparece sem uma linha de log, levando junto a configuração de quem
    /// atualizou.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Forma {
            Um(SpecType),
            Muitos(Vec<SpecType>),
        }
        Ok(match Forma::deserialize(d)? {
            Forma::Um(t) => SpecSet::new([t]),
            Forma::Muitos(v) => SpecSet::new(v),
        })
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
    pub spec: Option<SpecSet>,
    /// Quantos tokens a especulação rascunha por passada
    /// (`spec-draft-n-max`; 3 é o padrão do llama.cpp, 2–4 é o equilíbrio —
    /// acima disso a taxa de rejeição e a VRAM cobram o ganho).
    #[serde(default)]
    pub spec_draft_n_max: Option<u32>,
    /// Mínimo de tokens rascunhados (`spec-draft-n-min`).
    #[serde(default)]
    pub spec_draft_n_min: Option<u32>,
    /// Probabilidade mínima para aceitar um rascunho (`spec-draft-p-min`) —
    /// 0.75 corta os ciclos desperdiçados em contexto longo.
    #[serde(default)]
    pub spec_draft_p_min: Option<f32>,
    /// Modelo de rascunho externo (`spec-draft-model`, caminho GGUF). Com um
    /// caminho aqui e nenhum outro tipo escolhido, a especulação vira
    /// `draft-simple` — o tipo clássico de segundo modelo.
    #[serde(default)]
    pub spec_draft_model: Option<String>,
    /// Projetor de visão a carregar junto (caminho absoluto).
    pub mmproj: Option<String>,
    /// Quando carregar o projetor. Ausente = o padrão (sob demanda).
    pub vision: Option<VisionMode>,
    /// Manter o KV cache na GPU. `false` grava `no-kv-offload`.
    pub kv_offload: Option<bool>,
    /// Como o arquivo vai para a memória (`load-mode`). É o campo vivo;
    /// `mmap`/`mlock` abaixo só continuam existindo para ler perfis gravados
    /// antes da b10441 e são convertidos na emissão.
    #[serde(default)]
    pub load_mode: Option<LoadMode>,
    /// **Obsoleto** — use [`Self::load_mode`]. Mantido para desserializar
    /// perfis antigos: `false` virava `no-mmap`.
    pub mmap: Option<bool>,
    /// **Obsoleto** — use [`Self::load_mode`]. Mantido para desserializar
    /// perfis antigos: `true` virava `mlock`.
    pub mlock: Option<bool>,
    /// Slots paralelos no llama-server (`parallel`).
    pub parallel: Option<u32>,
    /// Qualquer outra flag do catálogo (`lr_types::flags`) que não tem campo
    /// tipado: pares `chave = valor` que seguem para a seção do modelo no
    /// INI. A validação acontece antes de chegar aqui; na emissão, campo
    /// tipado vence extra de mesma chave e a denylist é filtrada de novo por
    /// segurança.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extras: Vec<(String, String)>,
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

    /// O `load-mode` que este perfil de fato pede — já convertendo os campos
    /// obsoletos.
    ///
    /// A conversão é literal, e não uma opinião nova: `no-mmap` sozinho era
    /// "carregue tudo para a RAM" (`none`); `mlock` sozinho era "mapeie e
    /// trave" (`mmap+mlock`); os dois juntos eram "não mapeie e trave"
    /// (`mlock`). `mmap = true` continua não emitindo nada — era o padrão
    /// antes e continua sendo o `auto` agora, e forçar `mmap` mudaria o
    /// comportamento em dispositivo que não suporta mapeamento.
    pub fn effective_load_mode(&self) -> Option<LoadMode> {
        if self.load_mode.is_some() {
            return self.load_mode;
        }
        match (self.mmap, self.mlock) {
            (Some(false), Some(true)) => Some(LoadMode::Mlock),
            (Some(false), _) => Some(LoadMode::None),
            (_, Some(true)) => Some(LoadMode::MmapMlock),
            _ => None,
        }
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
        if let Some(n) = self.parallel {
            push("parallel", n.to_string());
        }
        if let Some(false) = self.kv_offload {
            push("no-kv-offload", "1".to_string());
        }
        if let Some(m) = self.effective_load_mode() {
            push("load-mode", m.as_str().to_string());
        }
        // A especulação liga por escolha explícita OU pela presença de um
        // modelo de rascunho (que sozinho significa o tipo clássico
        // `draft-simple`). `spec: Some(None)` é "sem especulação" dito com
        // todas as letras — aí nem o rascunho liga nada.
        let spec_ativa = match &self.spec {
            Some(s) if s.is_off() => false,
            Some(s) => {
                push("spec-type", s.as_ini_value());
                true
            }
            None if self.spec_draft_model.is_some() => {
                push("spec-type", "draft-simple".to_string());
                true
            }
            None => false,
        };
        if spec_ativa {
            if let Some(p) = &self.spec_draft_model {
                push("spec-draft-model", p.replace('\\', "/"));
            }
            if let Some(n) = self.spec_draft_n_max {
                push("spec-draft-n-max", n.to_string());
            }
            if let Some(n) = self.spec_draft_n_min {
                push("spec-draft-n-min", n.to_string());
            }
            if let Some(p) = self.spec_draft_p_min {
                push("spec-draft-p-min", p.to_string());
            }
        }
        // Só a entrada que carrega o projetor recebe a chave. Sob demanda,
        // quem a recebe é a entrada companheira, montada por quem escreve o
        // INI — esta aqui continua sendo a leve.
        if let Some(p) = &self.mmproj
            && self.vision.unwrap_or_default() == VisionMode::Always
        {
            push("mmproj", p.replace('\\', "/"));
        }

        // Extras entram depois dos campos tipados e antes do `fit`, com duas
        // guardas de profundidade: campo tipado vence extra de mesma chave, e
        // chave gerenciada pelo app não passa nem se a validação da borda
        // falhou.
        for (k, v) in &self.extras {
            let chave = crate::flags::normalize_key(k);
            if out.iter().any(|(existente, _)| *existente == chave) {
                continue;
            }
            if crate::flags::managed_keys().contains(&chave.as_str()) {
                continue;
            }
            out.push((chave, v.clone()));
        }

        // O `--fit` reparte a memória sozinho. Ele só sai de cena quando já
        // dissemos quanto de janela e quantas camadas queremos; caso
        // contrário, fixar só um dos dois deixaria o outro no escuro. Um
        // extra explícito de `fit` (ligar de volta, por exemplo) tem a
        // palavra final.
        if self.ctx.is_some() && self.ngl.is_some() && !out.iter().any(|(k, _)| k == "fit") {
            out.push(("fit".to_string(), "off".to_string()));
        }
        out
    }

    /// As chaves de INI que os campos tipados deste perfil emitem — o que a
    /// validação de extras precisa saber para acusar duplicata e enxergar
    /// dependências satisfeitas (ex.: `spec-type` vindo do campo `spec`).
    pub fn typed_ini_keys(&self) -> Vec<String> {
        let extras: Vec<String> = self
            .extras
            .iter()
            .map(|(k, _)| crate::flags::normalize_key(k))
            .collect();
        self.to_ini_extras()
            .into_iter()
            .map(|(k, _)| k)
            .filter(|k| !extras.contains(k))
            .collect()
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
            spec: Some(SpecType::NgramMod.into()),
            mmproj: Some(r"C:\modelos\mmproj.gguf".into()),
            vision: Some(VisionMode::Always),
            kv_offload: Some(false),
            load_mode: Some(LoadMode::MmapMlock),
            parallel: Some(4),
            source: ProfileSource::Recommended,
            ..Default::default()
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
        assert_eq!(m["no-kv-offload"], "1");
        assert_eq!(m["load-mode"], "mmap+mlock");
        assert_eq!(m["parallel"], "4");
        // O parser do INI do llama.cpp trata `\` como escape.
        assert_eq!(m["mmproj"], "C:/modelos/mmproj.gguf");
        assert_eq!(m["fit"], "off");
    }

    /// Um perfil gravado antes da b10441 guardava a mesma decisão em dois
    /// campos que o llama.cpp depois aposentou. Ler o perfil antigo não pode
    /// virar flag obsoleta no INI — nem, pior, silêncio.
    #[test]
    fn an_old_profile_becomes_the_load_mode_it_always_meant() {
        let caso = |mmap, mlock| {
            ModelProfile {
                mmap,
                mlock,
                ..Default::default()
            }
            .effective_load_mode()
        };

        // `--no-mmap` sozinho era "carregue tudo para a RAM".
        assert_eq!(caso(Some(false), None), Some(LoadMode::None));
        // `--mlock` sozinho era "mapeie e trave as páginas".
        assert_eq!(caso(None, Some(true)), Some(LoadMode::MmapMlock));
        // Os dois juntos eram "não mapeie e trave".
        assert_eq!(caso(Some(false), Some(true)), Some(LoadMode::Mlock));
        // `mmap = true` era o padrão: nada a dizer ao motor.
        assert_eq!(caso(Some(true), None), None);
        assert_eq!(caso(None, None), None);

        // E nenhum deles emite mais a flag obsoleta.
        let antigo = ModelProfile {
            mmap: Some(false),
            ..Default::default()
        };
        let m: std::collections::HashMap<_, _> = antigo.to_ini_extras().into_iter().collect();
        assert_eq!(m["load-mode"], "none");
        assert!(!m.contains_key("no-mmap"), "flag obsoleta não sai mais");
    }

    /// A escolha nova manda: ler um perfil antigo é conversão, não empate.
    #[test]
    fn the_new_choice_wins_over_the_fields_it_replaced() {
        let p = ModelProfile {
            load_mode: Some(LoadMode::Mmap),
            mmap: Some(false),
            mlock: Some(true),
            ..Default::default()
        };
        assert_eq!(p.effective_load_mode(), Some(LoadMode::Mmap));
        let m: std::collections::HashMap<_, _> = p.to_ini_extras().into_iter().collect();
        assert_eq!(m["load-mode"], "mmap");
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
            spec: Some(SpecType::None.into()),
            ..Default::default()
        };
        assert!(p.to_ini_extras().is_empty());
    }

    /// As sub-flags da especulação só saem quando há especulação — soltas,
    /// seriam ruído que o llama.cpp ignoraria hoje e recusaria amanhã.
    #[test]
    fn draft_knobs_only_leave_with_speculation_on() {
        let sozinhas = ModelProfile {
            spec_draft_n_max: Some(4),
            spec_draft_p_min: Some(0.75),
            ..Default::default()
        };
        assert!(sozinhas.to_ini_extras().is_empty());

        let com_mtp = ModelProfile {
            spec: Some(SpecType::DraftMtp.into()),
            ..sozinhas.clone()
        };
        let m: std::collections::HashMap<_, _> = com_mtp.to_ini_extras().into_iter().collect();
        assert_eq!(m["spec-type"], "draft-mtp");
        assert_eq!(m["spec-draft-n-max"], "4");
        assert_eq!(m["spec-draft-p-min"], "0.75");

        // "Sem especulação" dito com todas as letras desliga até o rascunho.
        let desligada = ModelProfile {
            spec: Some(SpecType::None.into()),
            spec_draft_model: Some("/m/draft.gguf".into()),
            ..sozinhas
        };
        assert!(desligada.to_ini_extras().is_empty());
    }

    /// Um modelo de rascunho sozinho significa o tipo clássico de segundo
    /// modelo — ninguém deveria precisar saber que ele se chama
    /// `draft-simple`.
    #[test]
    fn a_draft_model_alone_means_classic_speculation() {
        let p = ModelProfile {
            spec_draft_model: Some(r"C:\m\draft.gguf".into()),
            spec_draft_n_max: Some(3),
            ..Default::default()
        };
        let m: std::collections::HashMap<_, _> = p.to_ini_extras().into_iter().collect();
        assert_eq!(m["spec-type"], "draft-simple");
        assert_eq!(m["spec-draft-model"], "C:/m/draft.gguf");
        assert_eq!(m["spec-draft-n-max"], "3");
    }

    /// Extras seguem para o INI — mas campo tipado vence a mesma chave e a
    /// denylist não passa nem por aqui.
    #[test]
    fn extras_flow_but_never_overrule_typed_fields() {
        let p = ModelProfile {
            ctx: Some(8192),
            extras: vec![
                ("cache-reuse".into(), "256".into()),
                ("--jinja".into(), "true".into()),
                ("ctx-size".into(), "999".into()),
                ("port".into(), "6666".into()),
            ],
            ..Default::default()
        };
        let pares = p.to_ini_extras();
        let m: std::collections::HashMap<_, _> = pares.clone().into_iter().collect();
        assert_eq!(m["ctx-size"], "8192", "o campo tipado vence o extra");
        assert_eq!(m["cache-reuse"], "256");
        assert_eq!(m["jinja"], "true", "traços do prefixo caem na emissão");
        assert!(!m.contains_key("port"), "chave gerenciada não passa");
        // E a chave de desempenho enxerga os extras: mudou extra, mudou key.
        let sem_extras = ModelProfile {
            ctx: Some(8192),
            ..Default::default()
        };
        assert_ne!(p.key(), sem_extras.key());
    }

    /// `fit = off` é derivado — mas um extra explícito tem a palavra final.
    #[test]
    fn an_explicit_fit_extra_wins_over_the_derived_one() {
        let p = ModelProfile {
            ctx: Some(8192),
            ngl: Some(36),
            extras: vec![("fit".into(), "on".into())],
            ..Default::default()
        };
        let m: std::collections::HashMap<_, _> = p.to_ini_extras().into_iter().collect();
        assert_eq!(m["fit"], "on");
    }

    /// O `spec` gravado como string solta (todas as versões até a lista) tem
    /// de continuar entrando — e produzindo EXATAMENTE o mesmo INI.
    ///
    /// Este é o teste que separa "atualizou" de "perdeu a configuração":
    /// `Store::model_profile` desserializa com `.ok()`, então um formato
    /// recusado não dá erro, apaga o perfil em silêncio.
    #[test]
    fn every_legacy_spec_string_still_loads() {
        for (gravado, esperado) in [("mtp", "draft-mtp"), ("ngram", "ngram-mod"), ("none", "")] {
            let json = format!(r#"{{"ctx":8192,"spec":"{gravado}","source":"tested"}}"#);
            let p: ModelProfile =
                serde_json::from_str(&json).expect("perfil legado tem de desserializar");
            let m: std::collections::HashMap<_, _> = p.to_ini_extras().into_iter().collect();
            match esperado {
                "" => assert!(!m.contains_key("spec-type"), "{gravado} não liga nada"),
                v => assert_eq!(m["spec-type"], v, "{gravado} vira {v}"),
            }
        }
    }

    /// A lista é o motivo de tudo isto: um rascunho adivinha texto novo, o
    /// n-grama adivinha o que já está no prompt, e o llama.cpp aceita os dois
    /// juntos separados por vírgula.
    #[test]
    fn a_combined_set_becomes_one_comma_separated_value() {
        let p = ModelProfile {
            spec: Some(SpecSet::new([SpecType::NgramMod, SpecType::DraftMtp])),
            ..Default::default()
        };
        let m: std::collections::HashMap<_, _> = p.to_ini_extras().into_iter().collect();
        // Ordenado pelo rank do enum, não pela ordem de quem escreveu.
        assert_eq!(m["spec-type"], "draft-mtp,ngram-mod");
    }

    /// Normalização: a mesma escolha dita de duas formas é a MESMA
    /// configuração — e portanto a mesma linha do histórico de desempenho.
    #[test]
    fn the_set_normalizes_so_the_history_key_stays_stable() {
        let a = SpecSet::new([SpecType::DraftMtp, SpecType::NgramMod]);
        let b = SpecSet::new([SpecType::NgramMod, SpecType::DraftMtp, SpecType::DraftMtp]);
        assert_eq!(a, b, "ordem e repetição não fazem configuração nova");

        let perfil = |s: SpecSet| ModelProfile {
            spec: Some(s),
            ..Default::default()
        };
        assert_eq!(perfil(a).key(), perfil(b).key());

        // `None` acompanhado é contradição: some.
        let c = SpecSet::new([SpecType::None, SpecType::DraftMtp]);
        assert_eq!(c.as_ini_value(), "draft-mtp");
        assert!(!c.is_off());

        // Vazio e só-`None` são a mesma coisa: desligado.
        assert!(SpecSet::new([]).is_off());
        assert!(SpecSet::new([SpecType::None]).is_off());
        assert!(
            !perfil(SpecSet::new([]))
                .to_ini_extras()
                .iter()
                .any(|(k, _)| k == "spec-type")
        );
    }

    /// Ida e volta pelo JSON, que é como o perfil vive no SQLite.
    #[test]
    fn the_set_round_trips_through_json() {
        let s = SpecSet::new([SpecType::DraftMtp, SpecType::NgramMod]);
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(
            json, r#"["draftMtp","ngramMod"]"#,
            "grava sempre como lista"
        );
        assert_eq!(serde_json::from_str::<SpecSet>(&json).unwrap(), s);
    }

    /// Perfis gravados antes dos campos novos continuam legíveis.
    #[test]
    fn an_old_profile_json_still_deserializes() {
        let antigo = r#"{"ctx":32768,"ngl":null,"ncmoe":null,"kvK":"q8_0","kvV":"q8_0",
            "flashAttn":true,"batch":null,"ubatch":null,"threads":null,"spec":"mtp",
            "mmproj":null,"vision":null,"kvOffload":null,"mmap":null,"mlock":null,
            "parallel":null,"source":"tested"}"#;
        let p: ModelProfile = serde_json::from_str(antigo).unwrap();
        assert_eq!(p.ctx, Some(32768));
        assert_eq!(p.spec, Some(SpecType::DraftMtp.into()));
        assert!(p.extras.is_empty());
        assert!(p.spec_draft_n_max.is_none());
        // E sem escolhas novas, o INI é o mesmo de antes — byte a byte.
        assert_eq!(
            p.to_ini_extras(),
            vec![
                ("ctx-size".to_string(), "32768".to_string()),
                ("cache-type-k".to_string(), "q8_0".to_string()),
                ("cache-type-v".to_string(), "q8_0".to_string()),
                ("flash-attn".to_string(), "on".to_string()),
                ("spec-type".to_string(), "draft-mtp".to_string()),
            ]
        );
    }

    /// A validação de extras precisa saber o que o perfil tipado já emite.
    #[test]
    fn typed_keys_tell_the_validator_what_is_taken() {
        let p = ModelProfile {
            ctx: Some(8192),
            spec: Some(SpecType::DraftMtp.into()),
            extras: vec![("cache-reuse".into(), "256".into())],
            ..Default::default()
        };
        let typed = p.typed_ini_keys();
        assert!(typed.contains(&"ctx-size".to_string()));
        assert!(typed.contains(&"spec-type".to_string()));
        assert!(
            !typed.contains(&"cache-reuse".to_string()),
            "extra não é campo tipado"
        );
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
            spec: Some(SpecType::DraftMtp.into()),
            source: ProfileSource::Tested,
            ..Default::default()
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"kvK\":\"q4_0\""), "{json}");
        // Grava como lista desde que a especulação passou a ser combinável;
        // a string antiga continua sendo LIDA (ver
        // `every_legacy_spec_string_still_loads`).
        assert!(json.contains("\"spec\":[\"draftMtp\"]"), "{json}");
        assert_eq!(serde_json::from_str::<ModelProfile>(&json).unwrap(), p);
    }
}
