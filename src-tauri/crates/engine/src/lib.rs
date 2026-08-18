//! Engines de inferência. O primeiro (e principal) é o llama-server do
//! llama.cpp em **Router mode**: um único processo que carrega/descarrega/
//! troca modelos dinamicamente conforme o campo `model` das requisições.
//!
//! A fronteira do trait é "endpoint OpenAI-compatible + gestão do processo",
//! o que cobre também Ollama e LlamaBarn/"Llama" como adapters futuros.
//!
//! Fatos (ago/2026):
//! - Router mode: subir `llama-server` SEM `-m`; `--models-dir` só vê GGUFs
//!   na raiz (não é recursivo). Modelos em `autor/repo/` entram via
//!   `--models-preset` (INI gerado em [`write_models_preset`]).
//!   `GET /models` (status), `POST /models/load`, `POST /models/unload`,
//!   progresso em `/models/sse`; `--models-max` limita simultâneos;
//!   `--sleep-idle-seconds` descarrega ociosos.
//! - `-ngl` já é `auto` por padrão; `--fit` ajusta `-c`/`-ngl` à VRAM.
//! - `/health`: 503 carregando, 200 pronto — readiness probe.
//! - Sidecars NÃO morrem com o app no Tauri: [`LlamaServer::stop_blocking`]
//!   precisa ser chamado em `ExitRequested`/`Exit`. No Windows o processo
//!   entra num Job Object (`KILL_ON_JOB_CLOSE`) para a árvore morrer mesmo
//!   num crash; `taskkill /T` é o cinto se o job não puder ser criado.

pub mod client;
pub mod props;

pub use client::{
    ChatDelta, ChatMessage, ChatOutcome, ChatRequest, ContentPart, FunctionCallMsg, ImageUrl,
    LlamaClient, MessageContent, NamedFunction, NamedToolChoice, Timings, ToolCallMsg, ToolCallReq,
    ToolChoice, tool_specs_to_api,
};
pub use props::{ChatTemplateCaps, ServerProps, parse_props};

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::{Child, Command};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("falha ao iniciar o processo: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("falha de rede ao falar com o engine: {0}")]
    Network(#[from] reqwest::Error),
    #[error("engine não está rodando")]
    NotRunning,
    /// O servidor respondeu, mas com status de erro (corpo já resumido).
    #[error("HTTP {status} do engine: {body}")]
    Http { status: u16, body: String },
    /// O roteador não conseguiu carregar o modelo pedido. Quase sempre é
    /// memória de vídeo: outro modelo ocupando, ou o modelo grande demais
    /// para a placa. Merece variante própria porque a saída é diferente de
    /// qualquer outro erro HTTP.
    #[error(
        "não consegui carregar o modelo `{model}`. Em geral é memória de vídeo: \
         feche o que estiver usando a GPU, escolha uma quantização menor ou \
         reduza a janela de contexto do modelo"
    )]
    ModelLoad { model: String },
    /// Resposta bem-sucedida porém fora do formato esperado.
    #[error("resposta inesperada do engine: {0}")]
    Protocol(String),
    /// O stream abriu e depois emudeceu: nenhum byte dentro do prazo.
    ///
    /// Sem esta variante um `chat_stream` pendurado (servidor travado, GPU
    /// presa, conexão zumbi) segurava o run PARA SEMPRE — o cliente não tem
    /// timeout global de propósito, porque geração longa é legítima. O que
    /// não é legítimo é silêncio: `fase` diz se foi antes do primeiro token
    /// (prompt processing) ou entre pedaços (geração).
    #[error("o modelo ficou {segundos}s sem emitir nada ({fase})")]
    Stalled { fase: &'static str, segundos: u64 },
}

impl EngineError {
    /// O servidor recusou o passo porque não conseguiu ler os argumentos da
    /// chamada de ferramenta que o modelo emitiu.
    ///
    /// Acontece com arquivo grande escrito de uma vez: o modelo erra o escape
    /// no meio de alguns milhares de caracteres, o JSON fica sem fechar a
    /// aspa e o llama.cpp devolve 500 para a requisição inteira. É erro do
    /// MODELO, não do servidor nem da rede — e tem conserto, então não pode
    /// derrubar a execução.
    pub fn is_bad_tool_arguments(&self) -> bool {
        match self {
            EngineError::Http { body, .. } => {
                let b = body.to_lowercase();
                b.contains("parse tool call") || b.contains("tool call arguments")
            }
            _ => false,
        }
    }

    /// O servidor recusou o pedido por causa da LISTA de ferramentas.
    ///
    /// É o único erro em que refazer o passo sem `tools` faz sentido: o
    /// template do modelo não sabe renderizá-las, e vai recusar de novo em
    /// toda tentativa. Qualquer outro erro (rede, 503, silêncio) é transitório
    /// e merece retry DO MESMO pedido — tratá-lo como recusa de template era o
    /// que fazia um blip de rede rebaixar o agente a chatbot pelo resto do
    /// run. Cheque `is_bad_tool_arguments` antes: aquele corpo também fala em
    /// "tool", mas o conserto é outro.
    pub fn is_tools_rejection(&self) -> bool {
        match self {
            EngineError::Http { body, .. } => {
                let b = body.to_lowercase();
                b.contains("jinja")
                    || b.contains("template")
                    || b.contains("grammar")
                    || b.contains("tools param")
                    || b.contains("does not support tools")
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod erro_tests {
    use super::EngineError;

    /// Corpo real devolvido pelo llama.cpp quando o modelo escreveu um HTML
    /// de 7 KB dentro dos argumentos e errou o escape no meio.
    #[test]
    fn a_broken_tool_call_is_told_apart_from_a_server_failure() {
        let quebrado = EngineError::Http {
            status: 500,
            body: r#"{"error":{"code":500,"message":"Failed to parse tool call arguments as JSON: [json.exception.parse_error.101] parse error at line 1, column 7018: syntax error while parsing value - invalid string: missing closing quote"}}"#.into(),
        };
        assert!(quebrado.is_bad_tool_arguments());

        // Um 500 de verdade (o servidor caiu) não pode virar "tente de novo
        // em pedaços" — ali não há o que o modelo conserte.
        let outro = EngineError::Http {
            status: 500,
            body: "internal server error".into(),
        };
        assert!(!outro.is_bad_tool_arguments());
        assert!(!EngineError::NotRunning.is_bad_tool_arguments());
    }
}

/// Configuração de inicialização do llama-server em Router mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    pub exe_path: PathBuf,
    pub models_dir: PathBuf,
    pub host: String,
    pub port: u16,
    /// Conversas atendidas ao mesmo tempo (`--parallel`).
    ///
    /// Cada uma leva uma fatia da janela de contexto: com 4, quem pedia 32k
    /// recebia 8k por conversa — e o app anunciava os 32k. Uma por vez é o
    /// uso real de um app de desktop, e é o que faz o número da tela ser
    /// verdade.
    pub parallel: u32,
    /// Quantos modelos o roteador mantém carregados ao mesmo tempo.
    ///
    /// Um. Trocar de modelo no meio da conversa é o caso comum, e o segundo
    /// não cabe na placa quando o primeiro ainda está lá — o roteador
    /// descarrega o menos usado ao bater no teto, então o teto ser 1 é
    /// exatamente "descarregue o anterior para carregar o novo".
    pub models_max: u32,
    pub api_key: Option<String>,
    /// Segundos de ociosidade até descarregar um modelo (0 = nunca).
    pub sleep_idle_seconds: u32,
    pub extra_args: Vec<String>,
    /// INI do `--models-preset` (modelos em subpastas; `--models-dir` não recorre).
    #[serde(default)]
    pub models_preset: Option<PathBuf>,
}

impl ServerConfig {
    pub fn new(exe_path: PathBuf, models_dir: PathBuf, port: u16) -> Self {
        Self {
            exe_path,
            models_dir,
            host: "127.0.0.1".to_string(),
            port,
            models_max: 1,
            parallel: 1,
            api_key: None,
            sleep_idle_seconds: 0,
            extra_args: Vec::new(),
            models_preset: None,
        }
    }

    /// Argumentos de linha de comando (sem o executável).
    pub fn to_args(&self) -> Vec<String> {
        let mut args = vec![
            "--models-dir".into(),
            self.models_dir.to_string_lossy().into_owned(),
            "--host".into(),
            self.host.clone(),
            "--port".into(),
            self.port.to_string(),
            "--models-max".into(),
            self.models_max.to_string(),
            // Slots de KV no mesmo processo (mesmo load de pesos). A janela
            // pedida é dividida entre eles.
            "--parallel".into(),
            self.parallel.max(1).to_string(),
        ];
        if let Some(preset) = &self.models_preset {
            args.push("--models-preset".into());
            args.push(preset.to_string_lossy().into_owned());
        }
        if self.sleep_idle_seconds > 0 {
            args.push("--sleep-idle-seconds".into());
            args.push(self.sleep_idle_seconds.to_string());
        }
        if let Some(key) = &self.api_key {
            args.push("--api-key".into());
            args.push(key.clone());
        }
        args.extend(self.extra_args.iter().cloned());
        args
    }

    /// URL de BIND (pode ser 0.0.0.0 em modo LAN) — para logs/exibição.
    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }

    /// URL CONECTÁVEL a partir desta máquina: bind em wildcard (0.0.0.0/::)
    /// não é roteável como destino no Windows e é bloqueado pelo CSP — a UI e
    /// o health-check devem sempre usar esta.
    pub fn connect_url(&self) -> String {
        let host = if self.host == "0.0.0.0" || self.host == "::" {
            "127.0.0.1"
        } else {
            self.host.as_str()
        };
        format!("http://{host}:{}", self.port)
    }
}

/// Uma seção do INI do `--models-preset`.
///
/// `extras` são pares `chave = valor` gravados junto do `model` na mesma
/// seção — é assim que um modelo de embedding recebe `pooling = mean` e um
/// reranker recebe `reranking = true` sem afetar os modelos de chat, que
/// dividem o mesmo processo em Router mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetEntry {
    /// Valor que a UI/API manda no campo `model`.
    pub id: String,
    pub path: PathBuf,
    pub extras: Vec<(String, String)>,
}

impl PresetEntry {
    pub fn new(id: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            id: id.into(),
            path: path.into(),
            extras: Vec::new(),
        }
    }

    pub fn with_extra(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extras.push((key.into(), value.into()));
        self
    }
}

impl From<(String, PathBuf)> for PresetEntry {
    fn from((id, path): (String, PathBuf)) -> Self {
        Self::new(id, path)
    }
}

/// Gera o INI do `--models-preset` para o llama-server.
///
/// Cada entrada vira uma seção: o `id` é o valor de `"model"` nas
/// requisições OpenAI. Caminhos usam `/` para o parser do llama.cpp não
/// tratar `\` do Windows como escape.
pub fn write_models_preset(path: &Path, entries: &[PresetEntry]) -> std::io::Result<()> {
    let mut used = HashSet::new();
    let mut body = String::from("; gerado automaticamente — não edite\nversion = 1\n\n");
    for entry in entries {
        let id = unique_preset_id(&entry.id, &mut used);
        let abs = path_for_ini(&entry.path);
        body.push_str(&format!("[{id}]\nmodel = {abs}\n"));
        for (key, value) in &entry.extras {
            body.push_str(&format!("{key} = {value}\n"));
        }
        body.push('\n');
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, body)
}

fn unique_preset_id(name: &str, used: &mut HashSet<String>) -> String {
    let base = name.replace([']', '\n', '\r'], "_");
    let base = if base.is_empty() {
        "model".to_string()
    } else {
        base
    };
    if used.insert(base.clone()) {
        return base;
    }
    for i in 2.. {
        let cand = format!("{base}-{i}");
        if used.insert(cand.clone()) {
            return cand;
        }
    }
    base
}

fn path_for_ini(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Processo llama-server gerenciado.
pub struct LlamaServer {
    config: ServerConfig,
    child: Option<Child>,
    http: reqwest::Client,
    /// Job Object no Windows: fecha o handle → o kernel mata a árvore
    /// inteira. No Unix é sempre `None` — lá quem cumpre esse papel é o
    /// grupo de processos, e o `kill_process_tree` abaixo dá conta.
    job: Option<lr_proc::JobGuard>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Health {
    Ready,
    Loading,
    Down,
}

impl LlamaServer {
    pub fn new(config: ServerConfig) -> Self {
        Self {
            config,
            child: None,
            http: reqwest::Client::new(),
            job: None,
        }
    }

    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    pub fn is_spawned(&self) -> bool {
        self.child.is_some()
    }

    /// PID do llama-server, para matar a árvore no exit se o mutex estiver ocupado.
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().and_then(Child::id)
    }

    /// Inicia o processo. stdout/stderr ficam disponíveis para streaming de
    /// logs à UI (ligação feita pelo chamador).
    pub fn spawn(&mut self) -> Result<(), EngineError> {
        if self.child.is_some() {
            return Ok(());
        }
        let mut cmd = Command::new(&self.config.exe_path);
        cmd.args(self.config.to_args())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        lr_proc::prepare(&mut cmd);
        if let Some(dir) = self.config.exe_path.parent() {
            cmd.current_dir(dir);
        }
        log::info!(
            "iniciando llama-server: {} {}",
            self.config.exe_path.display(),
            self.config.to_args().join(" ")
        );
        let child = lr_proc::spawn_supervised(&mut cmd)?;
        self.job = lr_proc::attach_job(&child);
        self.child = Some(child);
        Ok(())
    }

    /// Toma os pipes de stdout/stderr do processo (uma única vez) para
    /// streaming de logs.
    pub fn take_output(
        &mut self,
    ) -> (
        Option<tokio::process::ChildStdout>,
        Option<tokio::process::ChildStderr>,
    ) {
        match &mut self.child {
            Some(c) => (c.stdout.take(), c.stderr.take()),
            None => (None, None),
        }
    }

    /// Readiness probe: `GET /health` (200 pronto, 503 carregando).
    pub async fn health(&self) -> Health {
        let url = format!("{}/health", self.config.connect_url());
        match self.http.get(&url).send().await {
            Ok(r) if r.status().is_success() => Health::Ready,
            Ok(r) if r.status().as_u16() == 503 => Health::Loading,
            _ => Health::Down,
        }
    }

    /// Espera o servidor ficar pronto, com timeout.
    pub async fn wait_ready(&self, timeout: std::time::Duration) -> Result<(), EngineError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.health().await == Health::Ready {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(EngineError::NotRunning);
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
    }

    /// Encerra o processo. SEMPRE chamar no exit do app (sidecars não morrem
    /// sozinhos no Tauri). Versão async para os comandos; o exit usa
    /// [`Self::stop_blocking`] para não depender do runtime do Tokio.
    pub async fn stop(&mut self) {
        self.stop_blocking();
    }

    /// Mata o llama-server e os netos de forma síncrona (VRAM libera aqui).
    pub fn stop_blocking(&mut self) {
        let pid = self.pid();
        if pid.is_some() {
            log::info!("encerrando llama-server pid={pid:?}");
        }
        if let Some(job) = self.job.take() {
            lr_proc::terminate_job(&job);
        } else if let Some(pid) = pid {
            kill_process_tree(pid);
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            lr_proc::reap_child(&mut child);
        }
    }
}

impl Drop for LlamaServer {
    fn drop(&mut self) {
        self.stop_blocking();
    }
}

/// Mata o PID e toda a árvore de processos.
///
/// Reexportado de `lr_proc` para não quebrar quem já chamava
/// `lr_engine::kill_process_tree` (o shutdown do app, em `state.rs`).
pub use lr_proc::kill_process_tree;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_include_router_mode_essentials() {
        let cfg = ServerConfig::new(
            PathBuf::from("/tmp/llama-server"),
            PathBuf::from("/tmp/models"),
            8080,
        );
        let args = cfg.to_args();
        let joined = args.join(" ");
        assert!(joined.contains("--models-dir"));
        assert!(joined.contains("--host 127.0.0.1"));
        assert!(joined.contains("--port 8080"));
        // Um modelo por vez: trocar de modelo descarrega o anterior em vez
        // de tentar caber os dois na placa.
        assert!(joined.contains("--models-max 1"));
        // Uma conversa por vez: a janela pedida chega inteira a ela.
        assert!(joined.contains("--parallel 1"));
        // Router mode = SEM -m.
        assert!(!joined.contains(" -m "));
        assert!(!args.contains(&"-m".to_string()));
    }

    #[test]
    fn optional_args_appear_when_set() {
        let mut cfg = ServerConfig::new(PathBuf::from("srv"), PathBuf::from("models"), 9000);
        cfg.api_key = Some("secreta".into());
        cfg.sleep_idle_seconds = 300;
        let joined = cfg.to_args().join(" ");
        assert!(joined.contains("--api-key secreta"));
        assert!(joined.contains("--sleep-idle-seconds 300"));
    }

    /// A janela pedida é dividida entre as conversas simultâneas: é por isso
    /// que o padrão é uma só, e é por isso que o número precisa chegar ao
    /// processo em vez de ficar cravado.
    #[test]
    fn parallel_slots_come_from_the_config() {
        let mut cfg = ServerConfig::new(PathBuf::from("srv"), PathBuf::from("m"), 9000);
        assert!(cfg.to_args().join(" ").contains("--parallel 1"));

        cfg.parallel = 3;
        assert!(cfg.to_args().join(" ").contains("--parallel 3"));

        // Zero seria um servidor sem slot nenhum.
        cfg.parallel = 0;
        assert!(cfg.to_args().join(" ").contains("--parallel 1"));
    }

    #[test]
    fn base_url_formats() {
        let cfg = ServerConfig::new(PathBuf::from("srv"), PathBuf::from("m"), 8081);
        assert_eq!(cfg.base_url(), "http://127.0.0.1:8081");
    }

    #[test]
    fn args_include_models_preset() {
        let mut cfg = ServerConfig::new(PathBuf::from("srv"), PathBuf::from("models"), 9000);
        cfg.models_preset = Some(PathBuf::from("/tmp/router-models.ini"));
        let joined = cfg.to_args().join(" ");
        assert!(joined.contains("--models-preset /tmp/router-models.ini"));
    }

    /// Pasta temporária isolada por teste (o id do processo é o mesmo para
    /// todos os testes do binário).
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lr-preset-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn writes_preset_ini_with_unique_ids() {
        let dir = temp_dir("ids");
        let ini = dir.join("router-models.ini");
        write_models_preset(
            &ini,
            &[
                PresetEntry::new(
                    "Qwen3.8-27B-UD-Q2_K_XL.gguf",
                    r"C:\models\author\repo\Qwen3.8-27B-UD-Q2_K_XL.gguf",
                ),
                PresetEntry::new(
                    "Qwen3.8-27B-UD-Q2_K_XL.gguf",
                    "/other/Qwen3.8-27B-UD-Q2_K_XL.gguf",
                ),
            ],
        )
        .unwrap();
        let text = std::fs::read_to_string(&ini).unwrap();
        assert!(text.contains("[Qwen3.8-27B-UD-Q2_K_XL.gguf]"));
        assert!(text.contains("[Qwen3.8-27B-UD-Q2_K_XL.gguf-2]"));
        assert!(text.contains("model = C:/models/author/repo/Qwen3.8-27B-UD-Q2_K_XL.gguf"));
        assert!(!text.contains('\\'));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Extras viram linhas `chave = valor` DENTRO da seção do modelo —
    /// é o que permite subir chat + embedding no mesmo processo.
    #[test]
    fn writes_preset_extras_inside_the_section() {
        let dir = temp_dir("extras");
        let ini = dir.join("router-models.ini");
        write_models_preset(
            &ini,
            &[
                PresetEntry::new("chat.gguf", "/m/chat.gguf"),
                PresetEntry::new("bge-m3.gguf", "/m/bge-m3.gguf")
                    .with_extra("embedding", "true")
                    .with_extra("pooling", "mean"),
            ],
        )
        .unwrap();
        let text = std::fs::read_to_string(&ini).unwrap();
        assert_eq!(
            text,
            "; gerado automaticamente — não edite\nversion = 1\n\n\
             [chat.gguf]\nmodel = /m/chat.gguf\n\n\
             [bge-m3.gguf]\nmodel = /m/bge-m3.gguf\nembedding = true\npooling = mean\n\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Janela de contexto fixa: `ctx-size` + `fit = off` na seção do modelo.
    #[test]
    fn writes_ctx_size_and_disables_fit() {
        let dir = temp_dir("ctx");
        let ini = dir.join("router-models.ini");
        write_models_preset(
            &ini,
            &[PresetEntry::new("qwen.gguf", "/m/qwen.gguf")
                .with_extra("ctx-size", "32768")
                .with_extra("fit", "off")],
        )
        .unwrap();
        let text = std::fs::read_to_string(&ini).unwrap();
        assert!(text.contains("ctx-size = 32768"));
        assert!(text.contains("fit = off"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// O formato antigo `(id, caminho)` ainda converte em uma linha.
    #[test]
    fn preset_entry_from_tuple() {
        let e: PresetEntry = ("m.gguf".to_string(), PathBuf::from("/m/m.gguf")).into();
        assert_eq!(e, PresetEntry::new("m.gguf", "/m/m.gguf"));
        assert!(e.extras.is_empty());
    }
}
