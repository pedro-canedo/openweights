//! Escrita CIRÚRGICA do `settings.yaml` do dsh.
//!
//! O arquivo é um documento multi-namespace (`ui-theme`, `locale`,
//! `permission`, `shell`, `agent-loop`, `llm-deepseek`, …) que a própria UI
//! do dsh grava — reescrevê-lo por inteiro apagaria tema, idioma, preset de
//! permissão e tudo o mais que a pessoa ajustou por lá. Aqui o documento é
//! lido, APENAS a chave `llm-pi-ai` é substituída (e o `agent-default-model`
//! ajustado só quando está quebrado), e o resto volta como estava.
//!
//! Limites conhecidos e aceitos:
//! - **Comentários manuais do YAML são perdidos** na reescrita: o serde_yaml
//!   não os representa. O próprio dsh preserva comentários porque usa um
//!   parser de documento (yaml/CST); reproduzir isso aqui não paga o custo.
//! - A **chave de API nunca entra no arquivo** — só o NOME da variável de
//!   ambiente (`apiKeyEnv`), resolvida por requisição pelo dsh.
//!
//! Concorrência: o dsh escreve o arquivo sob um lock `settings.yaml.lock`
//! (com o PID dentro) + rename atômico, e observa mudanças com hot-reload.
//! Este módulo respeita o mesmo protocolo: espera o lock alheio com backoff
//! curto, cria o próprio com o PID durante a escrita, grava via arquivo
//! temporário + rename e mantém modo 0600 no Unix.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_yaml_ng::{Mapping, Value};

use crate::DshError;

/// Envs de chave referenciadas pelo arquivo (`apiKeyEnv`). Os VALORES vão só
/// no ambiente do processo `dsh`.
pub const OPENWEIGHTS_KEY_ENV: &str = "OPENWEIGHTS_API_KEY";
pub const OPENROUTER_KEY_ENV: &str = "OPENROUTER_API_KEY";
pub const NINEROUTER_KEY_ENV: &str = "NINEROUTER_API_KEY";

/// As rotas que ESTE app gerencia dentro do `llm-pi-ai`. Qualquer outra chave
/// do dict é da pessoa — e o merge não a inventa nem a apaga... na prática o
/// dict inteiro é nosso (`llm-pi-ai` é substituído), mas a lista existe para
/// o ajuste do `agent-default-model` saber o que é "nosso".
pub const IDS_GERENCIADOS: [&str; 3] = ["openweights", "openrouter", "ninerouter"];

/// Provider nativo do dsh (composição do `dsh-base`): um default apontando
/// para ele é escolha da pessoa e não é tocado.
const PROVEDOR_NATIVO: &str = "deepseek-official";

/// Cabeçalho prependado ao documento gerado. Comentário YAML puro: o dsh o
/// ignora (e o perde na próxima escrita dele — sem prejuízo).
const CABECALHO: &str = "\
# Seção `llm-pi-ai` gerada pelo OpenWeights a cada início do DeepSeek Harness.\n\
# As demais seções deste arquivo são preservadas; comentários manuais são\n\
# perdidos nesta reescrita. A chave de API nunca entra aqui — apenas o nome\n\
# da variável de ambiente (apiKeyEnv), exportada no processo do dsh.\n";

/// Um modelo de uma rota, com os únicos campos que o schema do dsh aceita e
/// que o app conhece (`id`, `name`, `contextWindow`; os demais — `maxTokens`,
/// `input`, `reasoningEfforts`, `compat` — ficam nos defaults do dsh).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeloDsh {
    pub id: String,
    pub name: String,
    /// `None` = omitir e deixar o `defaultContextWindow` do dsh valer.
    pub context_window: Option<u32>,
}

/// Uma rota do adapter `llm-pi-ai`. O `api` é sempre `openai-completions`:
/// dos três valores que o dsh aceita, é o único que fala com um gateway
/// OpenAI-compatible declarado à mão.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvedorDsh {
    pub display_name: String,
    /// Base COM `/v1` (ex.: `http://127.0.0.1:11711/v1`) — é o contrato do
    /// adapter, verificado com a rota local desde a primeira integração.
    pub base_url: String,
    /// NOME da variável de ambiente com a credencial. Sempre declarado: o
    /// adapter exige credencial, e a rota sem chave real recebe um dummy
    /// (`local`) exportado no ambiente do processo.
    pub api_key_env: String,
    pub models: Vec<ModeloDsh>,
}

// -------------------------------------------------------------- merge ---

fn valor_provedor(p: &ProvedorDsh) -> Value {
    let mut m = Mapping::new();
    m.insert("displayName".into(), p.display_name.clone().into());
    m.insert("api".into(), "openai-completions".into());
    m.insert("baseURL".into(), p.base_url.clone().into());
    m.insert("apiKeyEnv".into(), p.api_key_env.clone().into());
    let modelos: Vec<Value> = p
        .models
        .iter()
        .map(|mo| {
            let mut mm = Mapping::new();
            mm.insert("id".into(), mo.id.clone().into());
            mm.insert("name".into(), mo.name.clone().into());
            if let Some(cw) = mo.context_window {
                mm.insert("contextWindow".into(), Value::from(u64::from(cw)));
            }
            Value::Mapping(mm)
        })
        .collect();
    m.insert("models".into(), Value::Sequence(modelos));
    Value::Mapping(m)
}

/// A seção `llm-pi-ai` inteira: `providers` é um DICT keyed pela rota (não
/// um array) — fato do schema real do pacote.
fn valor_llm_pi_ai(provedores: &[(String, ProvedorDsh)]) -> Value {
    let mut dict = Mapping::new();
    for (id, p) in provedores {
        dict.insert(id.clone().into(), valor_provedor(p));
    }
    let mut secao = Mapping::new();
    secao.insert("providers".into(), Value::Mapping(dict));
    Value::Mapping(secao)
}

/// O `agent-default-model` atual, se existir e estiver bem formado.
fn default_atual(doc: &Mapping) -> Option<(String, String)> {
    let m = doc.get("agent-default-model")?.as_mapping()?;
    let provider = m.get("provider")?.as_str()?;
    let model = m.get("model")?.as_str()?;
    Some((provider.to_string(), model.to_string()))
}

/// O default precisa ser trocado?
///
/// Regra: mexer SÓ quando está ausente ou aponta para uma rota quebrada —
/// nunca por cima de uma escolha válida da pessoa.
/// - ausente → sim;
/// - rota gerenciada por nós → válida só se ainda existe E o modelo consta
///   nela (um `openweights` apontando um modelo apagado está quebrado);
/// - `deepseek-official` (nativo do dsh) ou rota presente no dict novo →
///   não tocar;
/// - qualquer outro nome → rota inexistente (ex.: o `openai` herdado de
///   versões antigas), trocar.
fn default_precisa_trocar(
    atual: Option<&(String, String)>,
    provedores: &[(String, ProvedorDsh)],
) -> bool {
    let Some((prov, modelo)) = atual else {
        return true;
    };
    if IDS_GERENCIADOS.contains(&prov.as_str()) {
        return !provedores
            .iter()
            .any(|(id, p)| id == prov && p.models.iter().any(|m| &m.id == modelo));
    }
    if prov == PROVEDOR_NATIVO || provedores.iter().any(|(id, _)| id == prov) {
        return false;
    }
    true
}

/// Mescla o documento existente com as rotas novas e devolve o YAML final.
///
/// Documento ilegível não derruba nada: vira base vazia com aviso — o
/// arquivo já estava quebrado, e o dsh mantém "o último valor bom" em
/// memória de qualquer forma.
pub fn merge_settings(existente: &str, provedores: &[(String, ProvedorDsh)]) -> String {
    let mut doc: Mapping = match serde_yaml_ng::from_str::<Value>(existente) {
        Ok(Value::Mapping(m)) => m,
        Ok(Value::Null) => Mapping::new(),
        Ok(_) => {
            log::warn!("settings.yaml não é um mapa; começando de um documento vazio");
            Mapping::new()
        }
        Err(e) => {
            log::warn!("settings.yaml ilegível ({e}); começando de um documento vazio");
            Mapping::new()
        }
    };

    // Substitui APENAS a nossa chave; todo o resto do documento fica.
    doc.insert("llm-pi-ai".into(), valor_llm_pi_ai(provedores));

    let atual = default_atual(&doc);
    if default_precisa_trocar(atual.as_ref(), provedores) {
        // O reset aponta para o primeiro modelo local. Sem modelo local não
        // há o que apontar — aí é melhor deixar como está do que inventar.
        let primeiro = provedores
            .iter()
            .find(|(id, _)| id == "openweights")
            .and_then(|(_, p)| p.models.first());
        if let Some(modelo) = primeiro {
            let mut m = Mapping::new();
            m.insert("provider".into(), "openweights".into());
            m.insert("model".into(), modelo.id.clone().into());
            doc.insert("agent-default-model".into(), Value::Mapping(m));
        }
    }

    let corpo = serde_yaml_ng::to_string(&Value::Mapping(doc)).unwrap_or_else(|e| {
        // Inalcançável na prática (o Value veio de parse ou de literais);
        // se acontecer, um arquivo vazio é recuperável, um pânico não.
        log::error!("falha ao serializar settings.yaml: {e}");
        String::new()
    });
    format!("{CABECALHO}{corpo}")
}

// ----------------------------------------------------------- lock + IO ---

/// Idade a partir da qual um lock alheio é considerado lixo de processo
/// morto. O dsh segura o lock por milissegundos (diff + rename).
const LOCK_OBSOLETO: Duration = Duration::from_secs(10);

/// Lock cooperativo no protocolo do dsh: arquivo `settings.yaml.lock` com o
/// PID dentro, removido ao soltar.
struct LockSettings {
    caminho: PathBuf,
}

impl LockSettings {
    fn adquirir(arquivo: &Path, prazo: Duration, obsoleto: Duration) -> Result<Self, DshError> {
        let caminho = arquivo.with_extension("yaml.lock");
        let limite = Instant::now() + prazo;
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&caminho)
            {
                Ok(mut f) => {
                    use std::io::Write as _;
                    let _ = write!(f, "{}", std::process::id());
                    return Ok(Self { caminho });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Lock alheio (o dsh grava o PID dele). Se está parado há
                    // tempo demais, é sobra de um processo que morreu no meio
                    // da escrita — remover destrava para sempre.
                    let idade = std::fs::metadata(&caminho)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|m| m.elapsed().ok());
                    if idade.is_some_and(|i| i > obsoleto) {
                        let _ = std::fs::remove_file(&caminho);
                        continue;
                    }
                    if Instant::now() >= limite {
                        return Err(DshError::SettingsLock);
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
}

impl Drop for LockSettings {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.caminho);
    }
}

/// Escrita atômica no protocolo do dsh: temporário no MESMO diretório (o
/// rename só é atômico dentro do mesmo volume) + rename por cima. Um write
/// parcial dispararia o hot-reload (chokidar) do dsh com o arquivo pela
/// metade — é exatamente o que o rename evita.
fn escrever_atomico(destino: &Path, conteudo: &str) -> std::io::Result<()> {
    let tmp = destino.with_file_name(format!("settings.yaml.{}.tmp", std::process::id()));
    std::fs::write(&tmp, conteudo)?;
    #[cfg(unix)]
    {
        // Mesmo modo que o dsh usa: o arquivo referencia nomes de envs de
        // credencial e o perfil inteiro é da pessoa.
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    match std::fs::rename(&tmp, destino) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Lê-modifica-grava o `settings.yaml` do `DSH_HOME` dado.
///
/// Bloqueante (backoff do lock + E/S): chamar via `spawn_blocking` de dentro
/// de um comando.
pub fn escrever_settings(
    dsh_home: &Path,
    provedores: &[(String, ProvedorDsh)],
) -> Result<(), DshError> {
    std::fs::create_dir_all(dsh_home)?;
    let arquivo = dsh_home.join("settings.yaml");
    let _lock = LockSettings::adquirir(&arquivo, Duration::from_secs(5), LOCK_OBSOLETO)?;
    let existente = std::fs::read_to_string(&arquivo).unwrap_or_default();
    let novo = merge_settings(&existente, provedores);
    escrever_atomico(&arquivo, &novo)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modelo(id: &str, ctx: Option<u32>) -> ModeloDsh {
        ModeloDsh {
            id: id.to_string(),
            name: id.to_string(),
            context_window: ctx,
        }
    }

    fn local() -> (String, ProvedorDsh) {
        (
            "openweights".to_string(),
            ProvedorDsh {
                display_name: "OpenWeights (local)".to_string(),
                base_url: "http://127.0.0.1:11711/v1".to_string(),
                api_key_env: OPENWEIGHTS_KEY_ENV.to_string(),
                models: vec![
                    modelo("Qwen3.6-27B.gguf", Some(131_072)),
                    modelo("Phi-5.gguf", None),
                ],
            },
        )
    }

    fn openrouter() -> (String, ProvedorDsh) {
        (
            "openrouter".to_string(),
            ProvedorDsh {
                display_name: "OpenRouter".to_string(),
                base_url: "https://openrouter.ai/api/v1".to_string(),
                api_key_env: OPENROUTER_KEY_ENV.to_string(),
                models: vec![modelo("meta-llama/llama-4-maverick", Some(1_048_576))],
            },
        )
    }

    fn ninerouter() -> (String, ProvedorDsh) {
        (
            "ninerouter".to_string(),
            ProvedorDsh {
                display_name: "9Router".to_string(),
                base_url: "http://127.0.0.1:20128/v1".to_string(),
                api_key_env: NINEROUTER_KEY_ENV.to_string(),
                models: vec![modelo("gcli/grok-4.6", Some(256_000))],
            },
        )
    }

    fn parse(yaml: &str) -> Mapping {
        match serde_yaml_ng::from_str::<Value>(yaml).unwrap() {
            Value::Mapping(m) => m,
            outro => panic!("documento não é um mapa: {outro:?}"),
        }
    }

    /// Documento real com as seções que a UI do dsh grava: TODAS precisam
    /// sobreviver, e a `llm-pi-ai` antiga precisa sumir por inteiro.
    #[test]
    fn the_merge_preserves_every_foreign_namespace() {
        let antes = r#"
ui-theme:
  mode: dark
locale:
  language: pt-BR
ui-onboarding:
  welcomeNoticeVersion: 2026-08-13.1
permission:
  defaultPreset: cautious
shell:
  prefer: bash
agent-loop:
  maxTurns: 40
llm-deepseek:
  apiKeyEnv: DEEPSEEK_API_KEY
llm-pi-ai:
  providers:
    velho:
      api: openai-completions
      baseURL: http://127.0.0.1:9/v1
      models:
        - id: sumiu
"#;
        let depois = parse(&merge_settings(antes, &[local()]));

        for chave in [
            "ui-theme",
            "locale",
            "ui-onboarding",
            "permission",
            "shell",
            "agent-loop",
            "llm-deepseek",
        ] {
            assert!(
                depois.contains_key(chave),
                "a seção {chave} foi perdida no merge"
            );
        }
        assert_eq!(
            depois["ui-theme"]["mode"].as_str(),
            Some("dark"),
            "o conteúdo alheio tem de voltar intacto"
        );

        let providers = depois["llm-pi-ai"]["providers"].as_mapping().unwrap();
        assert!(providers.contains_key("openweights"));
        assert!(
            !providers.contains_key("velho"),
            "a chave llm-pi-ai é substituída por inteiro, não mesclada"
        );
    }

    /// `providers` é um DICT keyed pela rota (schema real do dsh), nunca um
    /// array — e cada rota carrega o formato do adapter.
    #[test]
    fn the_providers_section_is_a_dict_with_the_adapter_shape() {
        let depois = parse(&merge_settings("", &[local()]));
        let ow = &depois["llm-pi-ai"]["providers"]["openweights"];
        assert_eq!(ow["api"].as_str(), Some("openai-completions"));
        assert_eq!(ow["baseURL"].as_str(), Some("http://127.0.0.1:11711/v1"));
        assert_eq!(ow["apiKeyEnv"].as_str(), Some("OPENWEIGHTS_API_KEY"));
        let modelos = ow["models"].as_sequence().unwrap();
        assert_eq!(modelos.len(), 2);
        assert_eq!(modelos[0]["id"].as_str(), Some("Qwen3.6-27B.gguf"));
        assert_eq!(modelos[0]["contextWindow"].as_u64(), Some(131_072));
        // Sem contexto conhecido o campo é omitido: vale o default do dsh.
        assert!(modelos[1].get("contextWindow").is_none());
    }

    /// Os três cenários do dict: só local; +openrouter; +ninerouter.
    #[test]
    fn the_dict_carries_exactly_the_active_providers() {
        let so_local = parse(&merge_settings("", &[local()]));
        let p = so_local["llm-pi-ai"]["providers"].as_mapping().unwrap();
        assert_eq!(p.len(), 1);

        let com_or = parse(&merge_settings("", &[local(), openrouter()]));
        let p = com_or["llm-pi-ai"]["providers"].as_mapping().unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(
            com_or["llm-pi-ai"]["providers"]["openrouter"]["baseURL"].as_str(),
            Some("https://openrouter.ai/api/v1")
        );

        let completo = parse(&merge_settings("", &[local(), openrouter(), ninerouter()]));
        let p = completo["llm-pi-ai"]["providers"].as_mapping().unwrap();
        assert_eq!(p.len(), 3);
        assert_eq!(
            completo["llm-pi-ai"]["providers"]["ninerouter"]["apiKeyEnv"].as_str(),
            Some("NINEROUTER_API_KEY")
        );
    }

    /// A chave NUNCA entra no arquivo — só o nome da env.
    #[test]
    fn no_secret_value_ever_reaches_the_file() {
        let yaml = merge_settings("", &[local(), openrouter()]);
        assert!(yaml.contains("apiKeyEnv"));
        assert!(
            !yaml.contains("apiKey:"),
            "campo de valor direto não existe"
        );
        assert!(
            yaml.starts_with('#'),
            "o cabeçalho explica quem escreve o quê"
        );
    }

    // ------------------------------------------- agent-default-model ---

    #[test]
    fn an_absent_default_model_points_at_the_first_local_model() {
        let depois = parse(&merge_settings("", &[local()]));
        assert_eq!(
            depois["agent-default-model"]["provider"].as_str(),
            Some("openweights")
        );
        assert_eq!(
            depois["agent-default-model"]["model"].as_str(),
            Some("Qwen3.6-27B.gguf")
        );
    }

    /// Escolha da pessoa pelo provider nativo do dsh: intocável (inclusive o
    /// `reasoningEffort` que só existe lá).
    #[test]
    fn a_native_default_model_is_left_alone() {
        let antes = "agent-default-model:\n  provider: deepseek-official\n  model: deepseek-v4-flash\n  reasoningEffort: high\n";
        let depois = parse(&merge_settings(antes, &[local()]));
        assert_eq!(
            depois["agent-default-model"]["provider"].as_str(),
            Some("deepseek-official")
        );
        assert_eq!(
            depois["agent-default-model"]["reasoningEffort"].as_str(),
            Some("high")
        );
    }

    /// O caso real que motivou a regra: um default apontando para uma rota
    /// que não existe (`openai`, herdado de escrita antiga) não resolve nada
    /// no dsh — é lixo, e lixo é trocado.
    #[test]
    fn a_default_pointing_at_a_missing_route_is_repaired() {
        let antes = "agent-default-model:\n  provider: openai\n  model: Qwen3.8-27B.gguf\n";
        let depois = parse(&merge_settings(antes, &[local()]));
        assert_eq!(
            depois["agent-default-model"]["provider"].as_str(),
            Some("openweights")
        );
    }

    /// Rota nossa E modelo ainda presente: escolha válida, fica.
    #[test]
    fn a_valid_managed_default_is_kept() {
        let antes = "agent-default-model:\n  provider: openweights\n  model: Phi-5.gguf\n";
        let depois = parse(&merge_settings(antes, &[local()]));
        assert_eq!(
            depois["agent-default-model"]["model"].as_str(),
            Some("Phi-5.gguf")
        );
    }

    /// Rota nossa mas modelo que sumiu do catálogo: quebrado, troca.
    #[test]
    fn a_managed_default_with_a_vanished_model_is_repaired() {
        let antes = "agent-default-model:\n  provider: openweights\n  model: Apagado.gguf\n";
        let depois = parse(&merge_settings(antes, &[local()]));
        assert_eq!(
            depois["agent-default-model"]["model"].as_str(),
            Some("Qwen3.6-27B.gguf")
        );
    }

    /// Sem nenhum modelo local não há para onde apontar: melhor deixar o que
    /// está (ou nada) do que inventar uma referência quebrada nova.
    #[test]
    fn without_local_models_the_default_is_not_invented() {
        let depois = parse(&merge_settings("", &[openrouter()]));
        assert!(!depois.contains_key("agent-default-model"));
    }

    /// Documento ilegível não derruba a escrita: vira base vazia (o arquivo
    /// já estava quebrado) e a nossa seção entra mesmo assim.
    #[test]
    fn an_unreadable_document_still_gets_our_section() {
        let depois = merge_settings(": isto: não: é: yaml: [", &[local()]);
        let doc = parse(&depois);
        assert!(doc.contains_key("llm-pi-ai"));
    }

    // ------------------------------------------------------ lock + IO ---

    #[test]
    fn the_settings_write_creates_the_file_and_removes_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        escrever_settings(dir.path(), &[local()]).unwrap();
        let arquivo = dir.path().join("settings.yaml");
        assert!(arquivo.is_file());
        assert!(
            !dir.path().join("settings.yaml.lock").exists(),
            "o lock tem de sumir junto com a escrita"
        );
        let texto = std::fs::read_to_string(&arquivo).unwrap();
        assert!(texto.contains("llm-pi-ai"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let modo = std::fs::metadata(&arquivo).unwrap().permissions().mode();
            assert_eq!(modo & 0o777, 0o600, "mesmo modo que o dsh usa");
        }
    }

    /// Um lock alheio recente segura a escrita até o prazo — e o prazo
    /// esgotado vira erro claro, não corrupção.
    #[test]
    fn a_fresh_foreign_lock_makes_the_acquire_time_out() {
        let dir = tempfile::tempdir().unwrap();
        let arquivo = dir.path().join("settings.yaml");
        std::fs::write(dir.path().join("settings.yaml.lock"), b"12345").unwrap();
        let r = LockSettings::adquirir(&arquivo, Duration::from_millis(250), LOCK_OBSOLETO);
        assert!(matches!(r, Err(DshError::SettingsLock)));
        assert!(
            dir.path().join("settings.yaml.lock").exists(),
            "o lock alheio não é destruído por desistência"
        );
    }

    /// Lock parado além da idade máxima é sobra de processo morto: removido
    /// e a escrita segue.
    #[test]
    fn a_stale_lock_is_swept_and_the_write_proceeds() {
        let dir = tempfile::tempdir().unwrap();
        let arquivo = dir.path().join("settings.yaml");
        std::fs::write(dir.path().join("settings.yaml.lock"), b"12345").unwrap();
        // Idade máxima zero: qualquer lock existente conta como obsoleto.
        let lock = LockSettings::adquirir(&arquivo, Duration::from_millis(250), Duration::ZERO)
            .expect("o lock obsoleto deveria ser varrido");
        drop(lock);
        assert!(!dir.path().join("settings.yaml.lock").exists());
    }

    /// Reescrever por cima de um arquivo existente preserva o resto — o
    /// round-trip completo, com E/S de verdade.
    #[test]
    fn a_second_write_still_preserves_foreign_sections() {
        let dir = tempfile::tempdir().unwrap();
        let arquivo = dir.path().join("settings.yaml");
        std::fs::write(&arquivo, "ui-theme:\n  mode: dark\n").unwrap();

        escrever_settings(dir.path(), &[local(), ninerouter()]).unwrap();
        let doc = parse(&std::fs::read_to_string(&arquivo).unwrap());
        assert_eq!(doc["ui-theme"]["mode"].as_str(), Some("dark"));
        assert_eq!(doc["llm-pi-ai"]["providers"].as_mapping().unwrap().len(), 2);
    }
}
