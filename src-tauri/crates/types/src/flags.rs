//! O catálogo de flags do llama-server — a fonte única do que a interface
//! pode oferecer e do que o backend aceita gravar.
//!
//! Duas origens se encontram aqui. As flags **curadas** são as ~55 que valem
//! uma tela boa: têm categoria, tipo de controle, faixa, requisitos e textos
//! em `flags.catalog.<chave>.*` no i18n. As **dinâmicas** vêm do `--help` do
//! binário pinado (parseado em `lr_advisor::help`) e garantem que TODA flag
//! da build atual apareça na busca, mesmo sem curadoria — uma atualização do
//! llama.cpp nunca deixa a interface para trás.
//!
//! O mesmo catálogo valida o que o usuário escolheu antes de virar INI ou
//! argumento de processo. A regra de ouro: o que o app gerencia (porta, chave
//! de API, caminhos de modelo, cluster) aparece como "gerenciada" e não é
//! editável livre — senão a tela de flags viraria uma segunda porta dos
//! fundos para as mesmas escolhas.

use serde::{Deserialize, Serialize};

/// Que controle a interface desenha, e como o valor é validado.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum FlagKind {
    /// Presença = ligado. No INI: `chave = 1`.
    Bool,
    /// `auto`/`on`/`off` — ausente = auto (o llama.cpp decide).
    Tri,
    Int {
        min: i64,
        max: i64,
        step: i64,
    },
    Float {
        min: f64,
        max: f64,
        step: f64,
    },
    Enum {
        options: Vec<String>,
    },
    Text,
    Path,
    /// Valores separados por vírgula (ex.: `override-kv` repetível).
    List,
}

/// Onde a flag pode ser aplicada.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlagScope {
    /// Só argumentos do processo do servidor (CLI).
    Global,
    /// Só seção de modelo no INI do Router.
    PerModel,
    /// Vale nos dois lugares (global vai para a seção `[*]`, nunca CLI — a
    /// CLI atropelaria a escolha por modelo, porque a precedência do Router é
    /// CLI > seção do modelo > `[*]`).
    Both,
    /// Chave que só existe no INI do Router (`load-on-startup`…).
    RouterOnly,
    /// O app é quem decide (porta, chave de API, cluster…). Aparece na busca
    /// com cadeado, apontando para o controle próprio.
    Managed,
}

/// O que precisa ser verdade na máquina/modelo para a flag fazer sentido.
/// A interface mostra como badge; nada aqui bloqueia — metadado ausente não
/// pode virar proibição.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlagRequirement {
    Gpu,
    MultiGpu,
    MoeModel,
    /// GGUF com cabeça de previsão multi-token (`{arch}.nextn_predict_layers`).
    MtpModel,
    MmprojPresent,
    /// Só faz efeito com `spec-type` definido.
    SpecEnabled,
    /// Só faz efeito com `rope-scaling = yarn`.
    RopeYarn,
    /// KV quantizado exige flash attention ligada.
    FlashAttnOn,
}

/// Uma flag como o catálogo a descreve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlagSpec {
    /// Chave canônica: o nome longo sem traços iniciais (`ctx-size`), que é
    /// exatamente a chave aceita pelo INI do Router.
    pub key: String,
    /// Formas curtas (`c`, `ngl`) e nomes de env (`LLAMA_ARG_*`), também sem
    /// traços — tudo que deve resolver para [`Self::key`] na busca/validação.
    pub aliases: Vec<String>,
    /// Id estável de categoria para o i18n (`flags.categories.<id>`).
    pub category: String,
    pub kind: FlagKind,
    /// Default do llama.cpp, quando conhecido — informativo, nunca gravado.
    pub default: Option<String>,
    pub scope: FlagScope,
    /// `true` → a interface busca rótulo/dica em `flags.catalog.<key>.*`.
    pub curated: bool,
    /// Descrição em inglês vinda do `--help` — o fallback das dinâmicas e o
    /// texto secundário das curadas.
    pub help_text: Option<String>,
    pub requires: Vec<FlagRequirement>,
    /// Chaves que não convivem com esta (ex.: `no-mmap` × `mlock`).
    pub conflicts: Vec<String>,
    /// Chave que precisa estar presente para esta valer (ex.:
    /// `spec-draft-n-max` → `spec-type`).
    pub depends_on: Option<String>,
    /// Campo camelCase do `ModelProfile` que já cobre esta flag. Presente →
    /// a interface redireciona para o controle tipado e um extra duplicado é
    /// rejeitado na validação.
    pub typed_field: Option<String>,
}

impl FlagSpec {
    fn new(key: &str, category: &str, kind: FlagKind, scope: FlagScope) -> Self {
        Self {
            key: key.into(),
            aliases: Vec::new(),
            category: category.into(),
            kind,
            default: None,
            scope,
            curated: true,
            help_text: None,
            requires: Vec::new(),
            conflicts: Vec::new(),
            depends_on: None,
            typed_field: None,
        }
    }

    fn aka(mut self, aliases: &[&str]) -> Self {
        self.aliases = aliases.iter().map(|s| s.to_string()).collect();
        self
    }
    fn preset(mut self, default: &str) -> Self {
        self.default = Some(default.into());
        self
    }
    fn req(mut self, r: &[FlagRequirement]) -> Self {
        self.requires = r.to_vec();
        self
    }
    fn conflita(mut self, keys: &[&str]) -> Self {
        self.conflicts = keys.iter().map(|s| s.to_string()).collect();
        self
    }
    fn depende(mut self, key: &str) -> Self {
        self.depends_on = Some(key.into());
        self
    }
    fn tipado(mut self, field: &str) -> Self {
        self.typed_field = Some(field.into());
        self
    }
}

/// Chaves que o app controla e a tela de flags nunca grava. Não é a lista de
/// "flags que existem no ServerConfig" — é a lista do que abriria uma segunda
/// porta para a mesma decisão: rede e segredo (host/port/api-key), caminhos e
/// identidade de modelo (o Router sobrescreve ao carregar), e o cluster, que
/// tem painel próprio e uma ordem de argumentos que importa.
pub fn managed_keys() -> &'static [&'static str] {
    &[
        "host",
        "port",
        "api-key",
        "api-key-file",
        "model",
        "model-url",
        "hf-repo",
        "hf-file",
        "hf-token",
        "alias",
        "models-dir",
        "models-max",
        "models-preset",
        "sleep-idle-seconds",
        "rpc",
        "device",
        "tensor-split",
        "mmproj",
    ]
}

fn int(min: i64, max: i64) -> FlagKind {
    FlagKind::Int { min, max, step: 1 }
}
fn float(min: f64, max: f64, step: f64) -> FlagKind {
    FlagKind::Float { min, max, step }
}
fn opts(o: &[&str]) -> FlagKind {
    FlagKind::Enum {
        options: o.iter().map(|s| s.to_string()).collect(),
    }
}

/// A tabela curada. Rótulos e dicas moram no i18n (`flags.catalog.<key>`);
/// aqui ficam só os fatos que não dependem de idioma.
pub fn curated_flags() -> Vec<FlagSpec> {
    use FlagKind::{Bool, List, Path, Text, Tri};
    use FlagRequirement as R;
    use FlagScope::{Both, Global, PerModel, RouterOnly};

    vec![
        // ── memória & GPU ───────────────────────────────────────────────
        FlagSpec::new("n-gpu-layers", "memory", int(0, 999), PerModel)
            .aka(&["ngl", "gpu-layers"])
            .req(&[R::Gpu])
            .tipado("ngl"),
        FlagSpec::new("n-cpu-moe", "memory", int(0, 999), PerModel)
            .aka(&["ncmoe"])
            .req(&[R::MoeModel])
            .tipado("ncmoe"),
        FlagSpec::new(
            "split-mode",
            "memory",
            opts(&["none", "layer", "row", "tensor"]),
            Both,
        )
        .aka(&["sm"])
        .preset("layer")
        .req(&[R::MultiGpu]),
        FlagSpec::new("main-gpu", "memory", int(0, 15), Both)
            .aka(&["mg"])
            .preset("0")
            .req(&[R::MultiGpu]),
        FlagSpec::new("no-kv-offload", "memory", Bool, PerModel)
            .aka(&["nkvo"])
            .tipado("kvOffload"),
        FlagSpec::new("no-mmap", "memory", Bool, PerModel)
            .conflita(&["mlock"])
            .tipado("mmap"),
        FlagSpec::new("mlock", "memory", Bool, PerModel)
            .conflita(&["no-mmap"])
            .tipado("mlock"),
        FlagSpec::new("fit", "memory", opts(&["on", "off"]), PerModel).preset("on"),
        FlagSpec::new(
            "load-mode",
            "memory",
            opts(&["auto", "none", "mmap", "mlock", "mmap+mlock", "dio"]),
            PerModel,
        )
        .aka(&["lm"])
        .preset("auto")
        .conflita(&["no-mmap", "mlock"]),
        // ── contexto & lote ─────────────────────────────────────────────
        FlagSpec::new("ctx-size", "context", int(512, 262_144), PerModel)
            .aka(&["c"])
            .tipado("ctx"),
        FlagSpec::new("batch-size", "context", int(1, 8192), PerModel)
            .aka(&["b"])
            .preset("2048")
            .tipado("batch"),
        FlagSpec::new("ubatch-size", "context", int(1, 8192), PerModel)
            .aka(&["ub"])
            .preset("512")
            .tipado("ubatch"),
        FlagSpec::new("keep", "context", int(-1, 262_144), PerModel).preset("0"),
        FlagSpec::new("context-shift", "context", Bool, PerModel),
        FlagSpec::new("cache-reuse", "context", int(0, 262_144), PerModel).preset("0"),
        // ── KV cache ────────────────────────────────────────────────────
        FlagSpec::new(
            "cache-type-k",
            "kv",
            opts(&["f16", "q8_0", "q4_0"]),
            PerModel,
        )
        .aka(&["ctk"])
        .preset("f16")
        .tipado("kvK"),
        FlagSpec::new(
            "cache-type-v",
            "kv",
            opts(&["f16", "q8_0", "q4_0"]),
            PerModel,
        )
        .aka(&["ctv"])
        .preset("f16")
        .req(&[R::FlashAttnOn])
        .tipado("kvV"),
        FlagSpec::new("flash-attn", "kv", Tri, PerModel)
            .aka(&["fa"])
            .preset("auto")
            .req(&[R::Gpu])
            .tipado("flashAttn"),
        // ── RoPE / YaRN ─────────────────────────────────────────────────
        FlagSpec::new(
            "rope-scaling",
            "rope",
            opts(&["none", "linear", "yarn"]),
            PerModel,
        ),
        FlagSpec::new(
            "rope-freq-base",
            "rope",
            float(0.0, 10_000_000.0, 1.0),
            PerModel,
        ),
        FlagSpec::new("rope-freq-scale", "rope", float(0.0, 8.0, 0.01), PerModel),
        FlagSpec::new("yarn-orig-ctx", "rope", int(0, 262_144), PerModel)
            .depende("rope-scaling")
            .req(&[R::RopeYarn]),
        FlagSpec::new("yarn-ext-factor", "rope", float(-1.0, 8.0, 0.05), PerModel)
            .depende("rope-scaling")
            .req(&[R::RopeYarn]),
        FlagSpec::new("yarn-attn-factor", "rope", float(0.0, 8.0, 0.05), PerModel)
            .depende("rope-scaling")
            .req(&[R::RopeYarn]),
        FlagSpec::new("yarn-beta-fast", "rope", float(0.0, 128.0, 0.5), PerModel)
            .depende("rope-scaling")
            .req(&[R::RopeYarn]),
        FlagSpec::new("yarn-beta-slow", "rope", float(0.0, 8.0, 0.05), PerModel)
            .depende("rope-scaling")
            .req(&[R::RopeYarn]),
        // ── especulação ─────────────────────────────────────────────────
        FlagSpec::new(
            "spec-type",
            "spec",
            opts(&[
                "none",
                "draft-simple",
                "draft-eagle3",
                "draft-mtp",
                "draft-dflash",
                "draft-dspark",
                "ngram-simple",
                "ngram-map-k",
                "ngram-map-k4v",
                "ngram-mod",
                "ngram-cache",
            ]),
            PerModel,
        )
        .preset("none")
        .tipado("spec"),
        FlagSpec::new("spec-draft-n-max", "spec", int(1, 16), PerModel)
            .preset("3")
            .depende("spec-type")
            .req(&[R::SpecEnabled])
            .tipado("specDraftNMax"),
        FlagSpec::new("spec-draft-n-min", "spec", int(0, 16), PerModel)
            .preset("0")
            .depende("spec-type")
            .req(&[R::SpecEnabled])
            .tipado("specDraftNMin"),
        FlagSpec::new("spec-draft-p-min", "spec", float(0.0, 1.0, 0.05), PerModel)
            .depende("spec-type")
            .req(&[R::SpecEnabled])
            .tipado("specDraftPMin"),
        FlagSpec::new("spec-draft-model", "spec", Path, PerModel)
            .aka(&["model-draft", "md"])
            .tipado("specDraftModel"),
        FlagSpec::new(
            "spec-draft-p-split",
            "spec",
            float(0.0, 1.0, 0.05),
            PerModel,
        )
        .aka(&["draft-p-split"])
        .depende("spec-type")
        .req(&[R::SpecEnabled]),
        FlagSpec::new("spec-ngram-mod-n-max", "spec", int(1, 256), PerModel)
            .depende("spec-type")
            .req(&[R::SpecEnabled]),
        FlagSpec::new("spec-ngram-mod-n-min", "spec", int(1, 256), PerModel)
            .depende("spec-type")
            .req(&[R::SpecEnabled]),
        // ── multimodal ──────────────────────────────────────────────────
        FlagSpec::new("no-mmproj-offload", "multimodal", Bool, PerModel).req(&[R::MmprojPresent]),
        FlagSpec::new("image-min-tokens", "multimodal", int(1, 16_384), PerModel)
            .req(&[R::MmprojPresent]),
        FlagSpec::new("image-max-tokens", "multimodal", int(1, 16_384), PerModel)
            .req(&[R::MmprojPresent]),
        // ── adaptadores ─────────────────────────────────────────────────
        FlagSpec::new("lora", "adapters", Path, PerModel),
        FlagSpec::new("lora-scaled", "adapters", List, PerModel),
        FlagSpec::new("control-vector", "adapters", Path, PerModel),
        FlagSpec::new("control-vector-scaled", "adapters", List, PerModel),
        // ── CPU ─────────────────────────────────────────────────────────
        FlagSpec::new("threads", "cpu", int(1, 512), PerModel)
            .aka(&["t"])
            .tipado("threads"),
        FlagSpec::new("threads-batch", "cpu", int(1, 512), PerModel).aka(&["tb"]),
        FlagSpec::new(
            "numa",
            "cpu",
            opts(&["distribute", "isolate", "numactl"]),
            Global,
        ),
        FlagSpec::new("prio", "cpu", int(-1, 3), Global).preset("0"),
        // ── servidor/rede (processo) ────────────────────────────────────
        FlagSpec::new("parallel", "server", int(1, 64), PerModel)
            .aka(&["np"])
            .tipado("parallel"),
        FlagSpec::new("no-cont-batching", "server", Bool, Global).aka(&["nocb"]),
        FlagSpec::new("metrics", "server", Bool, Global),
        FlagSpec::new("slots", "server", Bool, Global),
        FlagSpec::new("timeout", "server", int(0, 86_400), Global)
            .aka(&["to"])
            .preset("3600"),
        FlagSpec::new("ssl-cert-file", "server", Path, Global),
        FlagSpec::new("ssl-key-file", "server", Path, Global),
        FlagSpec::new("no-webui", "server", Bool, Global),
        // ── Router (só INI) ─────────────────────────────────────────────
        FlagSpec::new("load-on-startup", "router", Bool, RouterOnly),
        FlagSpec::new("stop-timeout", "router", int(0, 3600), RouterOnly),
        // `Both` e não `RouterOnly`: na seção `[*]` é onde ela faz sentido
        // (esconder duplicatas do cache na listagem inteira).
        FlagSpec::new("dedup-cache-models", "router", Bool, Both),
        // ── uso do modelo ───────────────────────────────────────────────
        FlagSpec::new("jinja", "usage", Bool, PerModel),
        FlagSpec::new("chat-template", "usage", Text, PerModel),
        FlagSpec::new("chat-template-file", "usage", Path, PerModel),
        FlagSpec::new(
            "reasoning-format",
            "usage",
            opts(&["auto", "none", "deepseek", "deepseek-legacy"]),
            PerModel,
        )
        .preset("auto"),
        FlagSpec::new("reasoning", "usage", Tri, PerModel)
            .aka(&["rea"])
            .preset("auto"),
        FlagSpec::new(
            "reasoning-effort",
            "usage",
            opts(&[
                "default", "minimal", "low", "medium", "high", "xhigh", "max",
            ]),
            PerModel,
        )
        .preset("default"),
        FlagSpec::new("reasoning-budget", "usage", int(-1, 262_144), PerModel).preset("-1"),
        FlagSpec::new(
            "pooling",
            "usage",
            opts(&["none", "mean", "cls", "last", "rank"]),
            PerModel,
        ),
        FlagSpec::new("embeddings", "usage", Bool, PerModel).aka(&["embedding"]),
        FlagSpec::new("reranking", "usage", Bool, PerModel).aka(&["rerank"]),
        FlagSpec::new("seed", "usage", int(-1, i64::MAX >> 1), Both)
            .aka(&["s"])
            .preset("-1"),
        // ── avançado ────────────────────────────────────────────────────
        FlagSpec::new("override-kv", "advanced", List, PerModel),
        FlagSpec::new("override-tensor", "advanced", Text, PerModel).aka(&["ot"]),
        FlagSpec::new("no-op-offload", "advanced", Bool, PerModel),
        FlagSpec::new("check-tensors", "advanced", Bool, Both),
    ]
}

/// Uma flag como o `--help` do binário a descreve. O parser mora em
/// `lr_advisor::help`; o tipo mora aqui para o merge e a inferência de
/// controle serem parte do catálogo, não do parser.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelpFlag {
    /// Nome longo sem traços (`ctx-size`).
    pub key: String,
    /// Demais formas sem traços (`c`) e env (`LLAMA_ARG_CTX_SIZE`).
    pub aliases: Vec<String>,
    /// O que o `--help` mostra depois do nome (`N`, `FNAME`, `[on|off|auto]`).
    pub value_hint: Option<String>,
    pub description: String,
    pub env: Option<String>,
    pub default: Option<String>,
    /// Título da seção do `--help` de onde a flag veio.
    pub section: String,
}

impl HelpFlag {
    /// Que controle desenhar para uma flag sem curadoria. Na dúvida, `Text`:
    /// o valor segue verbatim para o INI e quem valida é o llama.cpp — errar
    /// para o lado permissivo nunca esconde uma flag.
    pub fn infer_kind(&self) -> FlagKind {
        let hint = self.value_hint.as_deref().unwrap_or("");
        if hint.is_empty() {
            return FlagKind::Bool;
        }
        // `[a|b|c]` ou `{a,b,c}` → enum com essas opções.
        let inner = hint
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .or_else(|| hint.strip_prefix('{').and_then(|h| h.strip_suffix('}')));
        if let Some(inner) = inner {
            let sep = if inner.contains('|') { '|' } else { ',' };
            let options: Vec<String> = inner
                .split(sep)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if options.len() > 1 {
                return FlagKind::Enum { options };
            }
        }
        let up = hint.to_ascii_uppercase();
        if up.contains("FNAME") || up.contains("PATH") || up.contains("FILE") {
            return FlagKind::Path;
        }
        match self.default.as_deref() {
            Some(d) if d.parse::<i64>().is_ok() => int(i64::MIN >> 1, i64::MAX >> 1),
            Some(d) if d.parse::<f64>().is_ok() => float(f64::MIN, f64::MAX, 0.01),
            _ if up == "N" => int(i64::MIN >> 1, i64::MAX >> 1),
            _ => FlagKind::Text,
        }
    }
}

/// Junta curadas e dinâmicas num catálogo só.
///
/// A curada vence por chave, mas absorve do `--help` o que não depende de
/// opinião: a descrição original, o default e aliases que não conhecíamos.
/// Flags do `--help` sem curadoria entram como dinâmicas (`curated: false`);
/// as da denylist entram como `Managed`, para a busca explicar em vez de
/// esconder.
pub fn merge_catalog(curated: Vec<FlagSpec>, help: &[HelpFlag]) -> Vec<FlagSpec> {
    let managed = managed_keys();
    let mut out = curated;

    // Índice chave/alias → posição no catálogo curado.
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, f) in out.iter().enumerate() {
        index.insert(f.key.clone(), i);
        for a in &f.aliases {
            index.insert(a.clone(), i);
        }
    }

    for h in help {
        if let Some(&i) = index
            .get(&h.key)
            .or_else(|| h.aliases.iter().find_map(|a| index.get(a)))
        {
            let f = &mut out[i];
            f.help_text = Some(h.description.clone());
            if f.default.is_none() {
                f.default = h.default.clone();
            }
            for a in h.aliases.iter().chain(h.env.iter()) {
                if *a != f.key && !f.aliases.contains(a) {
                    f.aliases.push(a.clone());
                }
            }
            continue;
        }
        let scope = if managed.contains(&h.key.as_str()) {
            FlagScope::Managed
        } else if h.section.to_ascii_lowercase().contains("server") {
            FlagScope::Global
        } else {
            FlagScope::Both
        };
        let mut aliases = h.aliases.clone();
        if let Some(env) = &h.env
            && !aliases.contains(env)
        {
            aliases.push(env.clone());
        }
        out.push(FlagSpec {
            key: h.key.clone(),
            aliases,
            category: "dynamic".into(),
            kind: h.infer_kind(),
            default: h.default.clone(),
            scope,
            curated: false,
            help_text: Some(h.description.clone()),
            requires: Vec::new(),
            conflicts: Vec::new(),
            depends_on: None,
            typed_field: None,
        });
    }

    // Curadas que o `--help` conhece como gerenciadas continuam como estão;
    // mas chaves da denylist que entraram curadas por engano viram Managed.
    for f in &mut out {
        if managed.contains(&f.key.as_str()) {
            f.scope = FlagScope::Managed;
        }
    }
    out
}

/// Para onde uma flag global vai no boot.
///
/// A decisão é tomada NO MOMENTO DE SALVAR, com o catálogo completo em mãos,
/// e gravada junto do valor — no boot o app só reproduz, sem depender de o
/// cache do `--help` existir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlagPlacement {
    /// Argumento de linha de comando do processo.
    Args,
    /// Chave da seção `[*]` do INI do Router (herdável, vencida pela seção
    /// do modelo — que é o que "padrão global" deve significar).
    Ini,
}

/// Onde uma flag deste escopo entra quando aplicada globalmente.
pub fn global_placement(spec: &FlagSpec) -> FlagPlacement {
    match spec.scope {
        FlagScope::Global => FlagPlacement::Args,
        _ => FlagPlacement::Ini,
    }
}

/// Uma flag global escolhida pelo usuário, como fica gravada no setting
/// `server_extra_flags` (JSON de uma lista destas).
///
/// `switch` também é decidido no salvamento: um boolean vira `--chave` seco
/// na CLI, enquanto `--prio 1` precisa do valor — e no boot ninguém tem o
/// catálogo em mãos para distinguir "1 de ligado" de "1 de prioridade".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalFlag {
    pub key: String,
    pub value: String,
    pub place: FlagPlacement,
    /// `true` = flag de presença (kind [`FlagKind::Bool`]).
    #[serde(default)]
    pub switch: bool,
}

/// Problemas que a validação devolve — cada código tem texto no i18n
/// (`flags.issues.<código>`), então aqui vai só o fato.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlagIssue {
    pub key: String,
    pub code: IssueCode,
    /// Detalhe interpolável (o valor rejeitado, a chave conflitante…).
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IssueCode {
    Unknown,
    Managed,
    WrongScope,
    BadValue,
    DuplicateOfTyped,
    Conflict,
    MissingDependency,
    Duplicate,
}

/// `--ctx-size`, `-c`, `LLAMA_ARG_CTX_SIZE` → forma sem traços, minúscula
/// quando não é env. O prefixo `no-` NÃO é removido: `no-mmap` é uma chave
/// legítima do catálogo, distinta de `mmap`.
pub fn normalize_key(raw: &str) -> String {
    let s = raw.trim().trim_start_matches('-');
    if s.starts_with("LLAMA_ARG_") {
        s.to_string()
    } else {
        s.to_ascii_lowercase()
    }
}

/// Acha a flag no catálogo por chave canônica ou alias.
pub fn resolve<'a>(catalog: &'a [FlagSpec], raw: &str) -> Option<&'a FlagSpec> {
    let k = normalize_key(raw);
    catalog
        .iter()
        .find(|f| f.key == k)
        .or_else(|| catalog.iter().find(|f| f.aliases.contains(&k)))
}

fn value_ok(kind: &FlagKind, v: &str) -> bool {
    match kind {
        FlagKind::Bool => matches!(
            v.to_ascii_lowercase().as_str(),
            "1" | "0" | "true" | "false" | "on" | "off"
        ),
        FlagKind::Tri => matches!(v.to_ascii_lowercase().as_str(), "on" | "off" | "auto"),
        FlagKind::Int { min, max, .. } => v
            .parse::<i64>()
            .map(|n| n >= *min && n <= *max)
            .unwrap_or(false),
        FlagKind::Float { min, max, .. } => v
            .parse::<f64>()
            .map(|n| n.is_finite() && n >= *min && n <= *max)
            .unwrap_or(false),
        FlagKind::Enum { options } => options.iter().any(|o| o == v),
        FlagKind::Text | FlagKind::Path | FlagKind::List => !v.trim().is_empty(),
    }
}

/// Valida um conjunto de extras contra o catálogo.
///
/// `scope` é onde os extras serão aplicados (`PerModel` para a seção do
/// modelo, `Global` para o processo/`[*]`). `typed_keys` são as chaves de INI
/// que o perfil tipado já emite — dependências podem ser satisfeitas por
/// elas, e um extra que duplica uma delas é rejeitado.
pub fn validate_extras(
    catalog: &[FlagSpec],
    scope: FlagScope,
    extras: &[(String, String)],
    typed_keys: &[String],
) -> Vec<FlagIssue> {
    let mut issues = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let keys_present: Vec<String> = extras
        .iter()
        .map(|(k, _)| normalize_key(k))
        .chain(typed_keys.iter().cloned())
        .collect();

    for (raw_key, value) in extras {
        let norm = normalize_key(raw_key);
        let issue = |code, detail: String| FlagIssue {
            key: norm.clone(),
            code,
            detail,
        };

        if seen.contains(&norm) {
            issues.push(issue(IssueCode::Duplicate, String::new()));
            continue;
        }
        seen.push(norm.clone());

        let Some(spec) = resolve(catalog, raw_key) else {
            issues.push(issue(IssueCode::Unknown, raw_key.clone()));
            continue;
        };
        // A partir daqui, tudo em termos da chave canônica.
        let canon = spec.key.clone();
        let issue = |code, detail: String| FlagIssue {
            key: canon.clone(),
            code,
            detail,
        };

        if spec.scope == FlagScope::Managed {
            issues.push(issue(IssueCode::Managed, String::new()));
            continue;
        }
        let scope_ok = match scope {
            FlagScope::PerModel => matches!(
                spec.scope,
                FlagScope::PerModel | FlagScope::Both | FlagScope::RouterOnly
            ),
            FlagScope::Global => matches!(spec.scope, FlagScope::Global | FlagScope::Both),
            _ => false,
        };
        if !scope_ok {
            issues.push(issue(IssueCode::WrongScope, format!("{:?}", spec.scope)));
            continue;
        }
        if let Some(field) = &spec.typed_field {
            issues.push(issue(IssueCode::DuplicateOfTyped, field.clone()));
            continue;
        }
        if !value_ok(&spec.kind, value) {
            issues.push(issue(IssueCode::BadValue, value.clone()));
            continue;
        }
        if let Some(dep) = &spec.depends_on
            && !keys_present.contains(dep)
        {
            issues.push(issue(IssueCode::MissingDependency, dep.clone()));
        }
        for c in &spec.conflicts {
            if keys_present.iter().filter(|k| *k == c).count() > 0 {
                issues.push(issue(IssueCode::Conflict, c.clone()));
            }
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalogo() -> Vec<FlagSpec> {
        curated_flags()
    }

    #[test]
    fn curated_keys_are_unique_and_never_managed() {
        let cat = catalogo();
        let mut keys: Vec<_> = cat.iter().map(|f| f.key.clone()).collect();
        let n = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), n, "chave curada repetida");
        for f in &cat {
            assert!(
                !managed_keys().contains(&f.key.as_str()),
                "{} é gerenciada e não pode ser curada editável",
                f.key
            );
        }
    }

    #[test]
    fn aliases_resolve_to_the_canonical_key() {
        let cat = catalogo();
        assert_eq!(resolve(&cat, "-c").unwrap().key, "ctx-size");
        assert_eq!(resolve(&cat, "--ngl").unwrap().key, "n-gpu-layers");
        assert_eq!(resolve(&cat, "ctk").unwrap().key, "cache-type-k");
        assert!(resolve(&cat, "porta-magica").is_none());
    }

    #[test]
    fn no_prefix_is_a_key_of_its_own() {
        // `no-mmap` e `mlock` são chaves distintas e conflitantes — remover o
        // prefixo faria as duas caírem no mesmo lugar.
        assert_eq!(normalize_key("--no-mmap"), "no-mmap");
        let cat = catalogo();
        assert!(resolve(&cat, "no-mmap").is_some());
    }

    #[test]
    fn a_managed_key_is_refused_with_its_own_code() {
        let cat = merge_catalog(
            catalogo(),
            &[HelpFlag {
                key: "port".into(),
                aliases: vec![],
                value_hint: Some("PORT".into()),
                description: "port to listen on".into(),
                env: None,
                default: Some("8080".into()),
                section: "server params".into(),
            }],
        );
        let issues = validate_extras(
            &cat,
            FlagScope::Global,
            &[("port".into(), "9999".into())],
            &[],
        );
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, IssueCode::Managed);
    }

    #[test]
    fn scopes_protect_each_side() {
        let cat = catalogo();
        // `no-webui` é do processo; na seção do modelo não faz nada.
        let per_modelo = validate_extras(
            &cat,
            FlagScope::PerModel,
            &[("no-webui".into(), "1".into())],
            &[],
        );
        assert_eq!(per_modelo[0].code, IssueCode::WrongScope);
        // `load-on-startup` só existe no INI; na CLI o servidor recusaria.
        let global = validate_extras(
            &cat,
            FlagScope::Global,
            &[("load-on-startup".into(), "1".into())],
            &[],
        );
        assert_eq!(global[0].code, IssueCode::WrongScope);
    }

    #[test]
    fn a_typed_field_wins_over_a_loose_extra() {
        let cat = catalogo();
        let issues = validate_extras(
            &cat,
            FlagScope::PerModel,
            &[("ctx-size".into(), "8192".into())],
            &[],
        );
        assert_eq!(issues[0].code, IssueCode::DuplicateOfTyped);
        assert_eq!(issues[0].detail, "ctx");
        // …e o alias curto cai na mesma regra.
        let via_alias = validate_extras(
            &cat,
            FlagScope::PerModel,
            &[("-c".into(), "8192".into())],
            &[],
        );
        assert_eq!(via_alias[0].code, IssueCode::DuplicateOfTyped);
    }

    #[test]
    fn values_are_checked_by_kind() {
        let cat = catalogo();
        let ok = validate_extras(
            &cat,
            FlagScope::PerModel,
            &[
                ("cache-reuse".into(), "256".into()),
                ("rope-scaling".into(), "yarn".into()),
                ("jinja".into(), "true".into()),
            ],
            &[],
        );
        assert!(ok.is_empty(), "{ok:?}");

        let ruim = validate_extras(
            &cat,
            FlagScope::PerModel,
            &[
                ("cache-reuse".into(), "muitos".into()),
                ("rope-scaling".into(), "espiral".into()),
            ],
            &[],
        );
        assert_eq!(ruim.len(), 2);
        assert!(ruim.iter().all(|i| i.code == IssueCode::BadValue));
    }

    #[test]
    fn dependencies_can_be_satisfied_by_the_typed_profile() {
        let cat = catalogo();
        // Sozinha, `spec-ngram-mod-n-max` não tem de quem depender…
        let sem = validate_extras(
            &cat,
            FlagScope::PerModel,
            &[("spec-ngram-mod-n-max".into(), "64".into())],
            &[],
        );
        assert_eq!(sem[0].code, IssueCode::MissingDependency);
        // …mas o perfil tipado emitindo `spec-type` resolve.
        let com = validate_extras(
            &cat,
            FlagScope::PerModel,
            &[("spec-ngram-mod-n-max".into(), "64".into())],
            &["spec-type".into()],
        );
        assert!(com.is_empty(), "{com:?}");
    }

    #[test]
    fn conflicting_keys_accuse_each_other() {
        // Conflito declarado numa flag sem campo tipado: o extra é aceito
        // sozinho e recusado quando a chave conflitante já veio do perfil.
        let cat = vec![
            FlagSpec::new("aaa", "advanced", FlagKind::Bool, FlagScope::PerModel)
                .conflita(&["bbb"]),
            FlagSpec::new("bbb", "advanced", FlagKind::Bool, FlagScope::PerModel),
        ];
        let sozinho = validate_extras(
            &cat,
            FlagScope::PerModel,
            &[("aaa".into(), "1".into())],
            &[],
        );
        assert!(sozinho.is_empty(), "{sozinho:?}");
        let contra_tipado = validate_extras(
            &cat,
            FlagScope::PerModel,
            &[("aaa".into(), "1".into())],
            &["bbb".into()],
        );
        assert_eq!(contra_tipado[0].code, IssueCode::Conflict);
        assert_eq!(contra_tipado[0].detail, "bbb");
        let entre_extras = validate_extras(
            &cat,
            FlagScope::PerModel,
            &[("aaa".into(), "1".into()), ("bbb".into(), "1".into())],
            &[],
        );
        assert!(entre_extras.iter().any(|i| i.code == IssueCode::Conflict));
    }

    #[test]
    fn a_repeated_extra_is_flagged_once() {
        let cat = catalogo();
        let issues = validate_extras(
            &cat,
            FlagScope::PerModel,
            &[
                ("cache-reuse".into(), "256".into()),
                ("cache-reuse".into(), "512".into()),
            ],
            &[],
        );
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, IssueCode::Duplicate);
    }

    #[test]
    fn the_help_merge_enriches_but_does_not_overrule() {
        let curadas = catalogo();
        let n_curadas = curadas.len();
        let help = vec![
            HelpFlag {
                key: "ctx-size".into(),
                aliases: vec!["c".into()],
                value_hint: Some("N".into()),
                description: "size of the prompt context".into(),
                env: Some("LLAMA_ARG_CTX_SIZE".into()),
                default: Some("4096".into()),
                section: "common params".into(),
            },
            HelpFlag {
                key: "swa-full".into(),
                aliases: vec![],
                value_hint: None,
                description: "use full-size SWA cache".into(),
                env: None,
                default: None,
                section: "common params".into(),
            },
        ];
        let cat = merge_catalog(curadas, &help);
        // A curada segue curada, com texto e env absorvidos.
        let ctx = cat.iter().find(|f| f.key == "ctx-size").unwrap();
        assert!(ctx.curated);
        assert_eq!(ctx.help_text.as_deref(), Some("size of the prompt context"));
        assert!(ctx.aliases.contains(&"LLAMA_ARG_CTX_SIZE".to_string()));
        assert_eq!(ctx.kind, int(512, 262_144), "faixa curada não é atropelada");
        // A desconhecida entra como dinâmica booleana.
        let swa = cat.iter().find(|f| f.key == "swa-full").unwrap();
        assert!(!swa.curated);
        assert_eq!(swa.kind, FlagKind::Bool);
        assert_eq!(cat.len(), n_curadas + 1);
    }

    #[test]
    fn kind_inference_reads_the_help_hints() {
        let base = HelpFlag {
            key: "x".into(),
            aliases: vec![],
            value_hint: None,
            description: String::new(),
            env: None,
            default: None,
            section: String::new(),
        };
        assert_eq!(base.infer_kind(), FlagKind::Bool);
        let enumerada = HelpFlag {
            value_hint: Some("[on|off|auto]".into()),
            ..base.clone()
        };
        assert_eq!(enumerada.infer_kind(), opts(&["on", "off", "auto"]),);
        let caminho = HelpFlag {
            value_hint: Some("FNAME".into()),
            ..base.clone()
        };
        assert_eq!(caminho.infer_kind(), FlagKind::Path);
        let numero = HelpFlag {
            value_hint: Some("N".into()),
            default: Some("512".into()),
            ..base.clone()
        };
        assert!(matches!(numero.infer_kind(), FlagKind::Int { .. }));
        let ambigua = HelpFlag {
            value_hint: Some("SPEC".into()),
            ..base
        };
        assert_eq!(ambigua.infer_kind(), FlagKind::Text, "na dúvida, texto");
    }
}
