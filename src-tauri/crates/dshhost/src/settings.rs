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

/// Nível de raciocínio oferecido além de `off`.
///
/// Com `qwen-chat-template` o dispatch só distingue LIGADO de DESLIGADO
/// (`enable_thinking: !!reasoningEffort`), então oferecer "low/medium/high"
/// seria três botões fazendo a mesma coisa. Um nível só, e honesto.
const NIVEL_LIGADO: &str = "high";

/// Formato de raciocínio dos modelos locais cujo template lê
/// `enable_thinking`.
///
/// O pi-ai traduz isto em `chat_template_kwargs: { enable_thinking,
/// preserve_thinking }` — exatamente os dois nomes que o template do Qwen3
/// lê, e que o llama.cpp repassa ao aplicar o template. Verificado contra o
/// servidor real: com `enable_thinking: false` a resposta vem sem
/// `reasoning_content`.
const FORMATO_THINKING: &str = "qwen-chat-template";

/// Teto de saída para uma janela de contexto — metade dela, entre 2 048 e
/// 65 536 tokens.
///
/// Metade porque a saída divide a janela com o prompt, e um agente de código
/// entra com prompt de sistema, ferramentas e arquivos: prometer a janela
/// inteira de saída é prometer o que não cabe. O piso serve ao modelo
/// pequeno (uma janela de 4k não pode render um teto de 32k, que é o que o
/// dsh assume sozinho); o teto existe porque, passado certo ponto, quem
/// bate no limite está num laço, não escrevendo um arquivo grande — e
/// esperar por isso é pior que cortar.
pub fn teto_de_saida(context_window: u32) -> u32 {
    (context_window / 2).clamp(2_048, 65_536)
}

/// Os níveis que o harness sabe nomear, em ordem de esforço.
///
/// O schema dele aceita só este vocabulário; um modelo que chame o seu nível
/// de outro jeito ainda pode ser oferecido, desde que caiba num destes
/// nomes — o VALOR enviado ao motor continua sendo o do template.
const NIVEIS_DO_HARNESS: [&str; 6] = ["minimal", "low", "medium", "high", "xhigh", "max"];

/// O nome do harness para um nível do template.
///
/// Igual quando o vocabulário coincide (`low`, `medium`, `xhigh`); `high`
/// para qualquer coisa que o harness não saiba nomear, porque um nível
/// oferecido com nome errado é pior que um nível a menos.
fn apelido_do_nivel(nivel: &str) -> &'static str {
    NIVEIS_DO_HARNESS
        .iter()
        .find(|n| **n == nivel)
        .copied()
        .unwrap_or("high")
}

/// Um modelo de uma rota, com os campos do schema do dsh que o app sabe
/// preencher (`input` e o resto do `compat` ficam nos defaults do dsh).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeloDsh {
    pub id: String,
    pub name: String,
    /// `None` = omitir e deixar o `defaultContextWindow` do dsh valer.
    pub context_window: Option<u32>,
    /// Teto de saída por resposta. `None` deixa valer o `defaultMaxTokens` do
    /// dsh — 32768, um número que ele aplica sem saber o tamanho da janela e
    /// que num modelo de 8k de contexto é uma promessa impossível.
    pub max_tokens: Option<u32>,
    /// Níveis de esforço que o chat template ACEITA, na ordem dele.
    ///
    /// Vazio = o modelo só sabe ligar e desligar. Cada nome aqui foi lido da
    /// linha em que o próprio template recusa o que não conhece, então o
    /// seletor do harness nunca oferece um valor que devolveria erro 500.
    pub efforts: Vec<String>,
    /// O raciocínio deste modelo pode ser ligado e desligado por quem chama.
    ///
    /// Só quando isto é verdade a rota declara `reasoningEfforts`, e é a
    /// declaração que faz o seletor de esforço aparecer no harness. Um modelo
    /// sem o interruptor não ganha botão nenhum — um botão que não muda nada
    /// é pior que a ausência dele.
    pub thinking: bool,
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
            if let Some(mt) = mo.max_tokens {
                mm.insert("maxTokens".into(), Value::from(u64::from(mt)));
            }
            if mo.thinking {
                // `off` sem valor é o que o schema pede: só ele pode vir
                // vazio, e é assim que o dispatch sabe "não mandar esforço"
                // — que no fim vira `enable_thinking: false`.
                let mut niveis = Mapping::new();
                niveis.insert("off".into(), Value::Null);
                let mut compat = Mapping::new();

                if mo.efforts.is_empty() {
                    // Template que só sabe ligar e desligar: um nível ligado,
                    // e o formato que manda apenas o booleano.
                    niveis.insert(NIVEL_LIGADO.into(), NIVEL_LIGADO.into());
                    compat.insert("thinkingFormat".into(), FORMATO_THINKING.into());
                } else {
                    // O template aceita níveis: oferecer os DELE, com o nome
                    // que ele valida. O formato genérico é o único que manda
                    // o valor escolhido (`$var: thinking.effort`) além do
                    // liga/desliga — o `qwen-chat-template` manda só o
                    // booleano, e o esforço se perderia no caminho.
                    for nivel in &mo.efforts {
                        let nome = apelido_do_nivel(nivel);
                        niveis.insert(nome.into(), nivel.clone().into());
                    }
                    compat.insert("thinkingFormat".into(), "chat-template".into());
                    let mut kwargs = Mapping::new();
                    let mut enabled = Mapping::new();
                    enabled.insert("$var".into(), "thinking.enabled".into());
                    kwargs.insert("enable_thinking".into(), Value::Mapping(enabled));
                    let mut effort = Mapping::new();
                    effort.insert("$var".into(), "thinking.effort".into());
                    // Desligado não manda esforço: o template levanta exceção
                    // se receber um valor que não conhece, e "nenhum" não é
                    // um dos que ele conhece.
                    effort.insert("omitWhenOff".into(), true.into());
                    kwargs.insert("reasoning_effort".into(), Value::Mapping(effort));
                    compat.insert("chatTemplateKwargs".into(), Value::Mapping(kwargs));
                }

                mm.insert("reasoningEfforts".into(), Value::Mapping(niveis));
                mm.insert("compat".into(), Value::Mapping(compat));
            }
            Value::Mapping(mm)
        })
        .collect();
    m.insert("models".into(), Value::Sequence(modelos));
    // Nível padrão da rota. Sem ele o dsh não manda esforço nenhum, e um
    // modelo que hoje raciocina passaria a não raciocinar só porque ganhou o
    // botão — mudança silenciosa de comportamento na cara de quem atualiza.
    // Com ele, o padrão continua sendo o de sempre e o botão é ganho puro.
    if p.models.iter().any(|mo| mo.thinking) {
        m.insert("reasoning".into(), NIVEL_LIGADO.into());
    }
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
            max_tokens: ctx.map(teto_de_saida),
            efforts: Vec::new(),
            thinking: false,
        }
    }

    /// Como o app declara um modelo de rota remota: janela sim, teto não —
    /// o teto de saída de lá é do provedor.
    fn modelo_remoto(id: &str, ctx: u32) -> ModeloDsh {
        ModeloDsh {
            max_tokens: None,
            ..modelo(id, Some(ctx))
        }
    }

    fn modelo_pensante(id: &str, ctx: u32) -> ModeloDsh {
        ModeloDsh {
            thinking: true,
            ..modelo(id, Some(ctx))
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

    /// O teto de saída acompanha a janela, mas não sem limites: um modelo
    /// minúsculo não pode herdar os 32k que o dsh assume sozinho, e um de
    /// janela enorme não ganha um teto que só serve para esperar mais por um
    /// laço.
    #[test]
    fn the_output_cap_follows_the_window_between_a_floor_and_a_ceiling() {
        assert_eq!(teto_de_saida(131_072), 65_536, "metade da janela");
        assert_eq!(teto_de_saida(32_768), 16_384);
        assert_eq!(teto_de_saida(4_096), 2_048, "no piso");
        assert_eq!(teto_de_saida(1_024), 2_048, "abaixo do piso, sobe ao piso");
        assert_eq!(teto_de_saida(1_000_000), 65_536, "no teto");
    }

    /// O interruptor de raciocínio é uma declaração de DUAS partes: os níveis
    /// (que fazem o seletor aparecer) e o formato (que diz ao dispatch como
    /// mandar). Uma sem a outra não liga botão nenhum.
    ///
    /// `off:` sai sem valor de propósito — é o que o schema do dsh aceita
    /// só para esse nível, e o parser dele (YAML 1.2) lê a chave como a
    /// string "off", não como o booleano falso do YAML 1.1.
    #[test]
    fn a_thinking_model_declares_levels_and_format() {
        let rota = (
            "openweights".to_string(),
            ProvedorDsh {
                display_name: "OpenWeights (local)".to_string(),
                base_url: "http://127.0.0.1:11711/v1".to_string(),
                api_key_env: OPENWEIGHTS_KEY_ENV.to_string(),
                models: vec![
                    modelo_pensante("Qwen3.8-27B.gguf", 131_072),
                    modelo("Phi-5.gguf", Some(32_768)),
                ],
            },
        );
        let saida = merge_settings("", std::slice::from_ref(&rota));

        assert!(saida.contains("reasoningEfforts:"));
        assert!(saida.contains("off: null"));
        assert!(saida.contains("thinkingFormat: qwen-chat-template"));
        assert!(saida.contains("maxTokens: 65536"));
        // O padrão da rota mantém o comportamento de antes do botão.
        assert!(saida.contains("reasoning: high"));

        // O modelo SEM interruptor não ganha seletor — só o teto.
        let doc: Value = serde_yaml_ng::from_str(&saida).unwrap();
        let modelos = doc["llm-pi-ai"]["providers"]["openweights"]["models"]
            .as_sequence()
            .unwrap();
        assert!(modelos[1].get("reasoningEfforts").is_none());
        assert!(modelos[1].get("compat").is_none());
        assert_eq!(modelos[1]["maxTokens"], Value::from(16_384));
    }

    /// Um modelo que aceita NÍVEIS oferece os níveis dele — com os nomes que
    /// o próprio template valida, e pelo formato que de fato envia o valor
    /// escolhido. O outro formato manda só o booleano, e o esforço se
    /// perderia no caminho sem ninguém notar.
    #[test]
    fn a_model_with_effort_levels_offers_exactly_what_its_template_accepts() {
        let rota = (
            "openweights".to_string(),
            ProvedorDsh {
                display_name: "OpenWeights (local)".to_string(),
                base_url: "http://127.0.0.1:11711/v1".to_string(),
                api_key_env: OPENWEIGHTS_KEY_ENV.to_string(),
                models: vec![ModeloDsh {
                    efforts: vec!["xhigh".into(), "medium".into(), "low".into()],
                    ..modelo_pensante("Qwen3.8-27B.gguf", 131_072)
                }],
            },
        );
        let saida = merge_settings("", &[rota]);
        let doc: Value = serde_yaml_ng::from_str(&saida).unwrap();
        let m = &doc["llm-pi-ai"]["providers"]["openweights"]["models"][0];

        // Os três níveis do template, mais o desligado.
        let niveis = m["reasoningEfforts"].as_mapping().unwrap();
        assert_eq!(niveis["off"], Value::Null);
        assert_eq!(niveis["xhigh"], Value::from("xhigh"));
        assert_eq!(niveis["medium"], Value::from("medium"));
        assert_eq!(niveis["low"], Value::from("low"));

        // O formato que manda o VALOR, não só o liga/desliga.
        assert_eq!(m["compat"]["thinkingFormat"], Value::from("chat-template"));
        let kw = &m["compat"]["chatTemplateKwargs"];
        assert_eq!(
            kw["enable_thinking"]["$var"],
            Value::from("thinking.enabled")
        );
        assert_eq!(
            kw["reasoning_effort"]["$var"],
            Value::from("thinking.effort")
        );
        // Desligado não manda esforço: o template levanta exceção com um
        // valor que ele não conhece, e "nenhum" não é um deles.
        assert_eq!(kw["reasoning_effort"]["omitWhenOff"], Value::from(true));
    }

    /// Template que só sabe ligar e desligar continua com um nível só — e
    /// pelo formato enxuto, que é o que ele entende.
    #[test]
    fn a_model_without_levels_keeps_the_simple_switch() {
        let rota = (
            "openweights".to_string(),
            ProvedorDsh {
                display_name: "OpenWeights (local)".to_string(),
                base_url: "http://127.0.0.1:11711/v1".to_string(),
                api_key_env: OPENWEIGHTS_KEY_ENV.to_string(),
                models: vec![modelo_pensante("Qwen3-8B.gguf", 32_768)],
            },
        );
        let saida = merge_settings("", &[rota]);
        let doc: Value = serde_yaml_ng::from_str(&saida).unwrap();
        let m = &doc["llm-pi-ai"]["providers"]["openweights"]["models"][0];
        assert_eq!(
            m["compat"]["thinkingFormat"],
            Value::from("qwen-chat-template")
        );
        assert!(m["compat"].get("chatTemplateKwargs").is_none());
        assert_eq!(m["reasoningEfforts"]["high"], Value::from("high"));
    }

    /// Rota remota não recebe teto nosso: o do provedor é dele, e pedir mais
    /// do que ele aceita é um 400 no meio da conversa.
    #[test]
    fn a_remote_route_keeps_the_harness_defaults() {
        let saida = merge_settings("", &[openrouter()]);
        assert!(!saida.contains("maxTokens"));
        assert!(!saida.contains("reasoningEfforts"));
    }

    fn openrouter() -> (String, ProvedorDsh) {
        (
            "openrouter".to_string(),
            ProvedorDsh {
                display_name: "OpenRouter".to_string(),
                base_url: "https://openrouter.ai/api/v1".to_string(),
                api_key_env: OPENROUTER_KEY_ENV.to_string(),
                models: vec![modelo_remoto("meta-llama/llama-4-maverick", 1_048_576)],
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
                models: vec![modelo_remoto("gcli/grok-4.6", 256_000)],
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
