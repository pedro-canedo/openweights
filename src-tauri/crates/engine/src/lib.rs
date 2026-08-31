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
pub mod metrics;
pub mod props;

pub use client::{
    ChatDelta, ChatMessage, ChatOutcome, ChatRequest, ContentPart, Dialect, FunctionCallMsg,
    ImageUrl, LlamaClient, MessageContent, NamedFunction, NamedToolChoice, Timings, ToolCallMsg,
    ToolCallReq, ToolChoice,
};
pub use props::{ChatTemplateCaps, ServerProps, parse_props};

/// Onde o modelo atende.
///
/// Nem sempre é o llama-server local: uma conversa apontada para o OpenRouter
/// ou para o 9router resolve para o endereço deles. Por isso os dois campos
/// extras — `headers` carrega a atribuição que o provedor pede, e `dialect`
/// diz se as extensões do llama.cpp podem viajar no corpo da requisição.
#[derive(Debug, Clone, Default)]
pub struct Endpoint {
    pub base_url: String,
    pub api_key: Option<String>,
    /// Cabeçalhos extras exigidos pelo provedor. Vazio no servidor local.
    pub headers: Vec<(String, String)>,
    pub dialect: Dialect,
}

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
    /// Flags globais escolhidas pelo usuário que valem para TODO modelo — vão
    /// para a seção `[*]` do INI, nunca para a linha de comando: a precedência
    /// do Router é CLI > seção do modelo > `[*]`, e na CLI elas atropelariam a
    /// configuração por modelo em vez de servir de padrão herdável.
    #[serde(default)]
    pub global_ini_extras: Vec<(String, String)>,
    /// Variáveis de ambiente escolhidas pela pessoa (setting `server_env_vars`).
    ///
    /// Nem tudo que muda o comportamento do motor é flag: o backend CUDA lê o
    /// ambiente, e é lá que mora o limiar que decide se um especialista viaja
    /// para a VRAM ou é multiplicado na CPU.
    #[serde(default)]
    pub env_extra: Vec<(String, String)>,
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
            global_ini_extras: Vec::new(),
            env_extra: Vec::new(),
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
            // O padrão do servidor responde `Access-Control-Allow-Headers: *`,
            // e pela spec Fetch o wildcard NÃO cobre `Authorization` — sem
            // isto o preflight do webview quebra ao mandar Bearer. Inócuo
            // quando não há chave, então vai sempre.
            "--cors-headers".into(),
            "Content-Type, Authorization".into(),
            // `GET /metrics` (Prometheus): é de onde saem as estatísticas de
            // serviço do app. Não é endpoint público — respeita a chave de
            // API como o resto; sem chave fica aberto como os demais.
            "--metrics".into(),
        ];
        if let Some(preset) = &self.models_preset {
            args.push("--models-preset".into());
            args.push(preset.to_string_lossy().into_owned());
        }
        if self.sleep_idle_seconds > 0 {
            args.push("--sleep-idle-seconds".into());
            args.push(self.sleep_idle_seconds.to_string());
        }
        args.extend(self.extra_args.iter().cloned());
        args
    }

    /// Variáveis de ambiente do processo. A chave de API vai por aqui —
    /// binding `LLAMA_API_KEY` do `--api-key` no b10441 (nome EXATO; não é o
    /// padrão `LLAMA_ARG_*` dos outros args) — e nunca pelo argv: argumento
    /// aparece no process list e no log do spawn.
    pub fn env_vars(&self) -> Vec<(String, String)> {
        let mut envs = Vec::new();
        if let Some(key) = &self.api_key {
            envs.push(("LLAMA_API_KEY".to_string(), key.clone()));
        }
        // As da pessoa entram depois, mas nunca por cima do que o app
        // gerencia: setting é arquivo editável por fora, e uma
        // `LLAMA_API_KEY` vinda dali trocaria o segredo em silêncio.
        for (k, v) in &self.env_extra {
            let chave = k.trim().to_string();
            if chave.is_empty() || lr_types::flags::env_is_managed(&chave) {
                continue;
            }
            if envs.iter().any(|(existente, _)| *existente == chave) {
                continue;
            }
            envs.push((chave, v.clone()));
        }
        envs
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
/// tratar `\` do Windows como escape. `global` vira a seção `[*]`, herdada
/// por todos os modelos e vencida por qualquer chave igual na seção deles —
/// exatamente o comportamento de "padrão do usuário".
pub fn write_models_preset(
    path: &Path,
    global: &[(String, String)],
    entries: &[PresetEntry],
) -> std::io::Result<()> {
    let body = render_models_preset(global, entries);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, body)
}

/// O conteúdo do INI, sem tocar o disco — é o que o preview da interface
/// mostra, e por construção é EXATAMENTE o que [`write_models_preset`] grava.
pub fn render_models_preset(global: &[(String, String)], entries: &[PresetEntry]) -> String {
    let mut used = HashSet::new();
    let mut body = String::from("; gerado automaticamente — não edite\nversion = 1\n\n");
    if !global.is_empty() {
        body.push_str("[*]\n");
        for (key, value) in global {
            body.push_str(&format!("{key} = {value}\n"));
        }
        body.push('\n');
    }
    for entry in entries {
        let id = unique_preset_id(&entry.id, &mut used);
        let abs = path_for_ini(&entry.path);
        body.push_str(&format!("[{id}]\nmodel = {abs}\n"));
        for (key, value) in &entry.extras {
            body.push_str(&format!("{key} = {value}\n"));
        }
        body.push('\n');
    }
    body
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

/// Um modelo como o Router o lista em `GET /models`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterModel {
    pub id: String,
    /// `unloaded`, `loading` ou `loaded` — o `status.value` do Router.
    pub state: String,
}

/// Lê o corpo do `GET /models` do Router (formato verificado no b10441):
/// `{"data":[{"id":"…","status":{"value":"unloaded",…},…}],"object":"list"}`.
pub fn parse_models_status(v: &serde_json::Value) -> Option<Vec<RouterModel>> {
    let data = v.get("data")?.as_array()?;
    Some(
        data.iter()
            .filter_map(|m| {
                Some(RouterModel {
                    id: m.get("id")?.as_str()?.to_string(),
                    state: m
                        .get("status")
                        .and_then(|s| s.get("value"))
                        .and_then(|s| s.as_str())
                        .unwrap_or("unknown")
                        .to_string(),
                })
            })
            .collect(),
    )
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
            // A chave de API entra por ambiente, nunca por argv (vazaria no
            // process list e no `log::info!` logo abaixo).
            .envs(self.config.env_vars())
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

    /// Pede ao Router para carregar um modelo agora (`POST /models/load`).
    ///
    /// A API não aceita argumentos por requisição — a configuração vem toda
    /// do INI, lido no boot. Quem mudou config precisa reiniciar ANTES de
    /// carregar, e é a interface que garante essa ordem.
    pub async fn load_model(&self, id: &str) -> Result<(), EngineError> {
        self.model_op("load", id).await
    }

    /// Descarrega um modelo (`POST /models/unload`) — VRAM de volta na hora.
    pub async fn unload_model(&self, id: &str) -> Result<(), EngineError> {
        self.model_op("unload", id).await
    }

    async fn model_op(&self, op: &str, id: &str) -> Result<(), EngineError> {
        let url = format!("{}/models/{op}", self.config.connect_url());
        let mut req = self
            .http
            .post(&url)
            .json(&serde_json::json!({ "model": id }));
        if let Some(key) = &self.config.api_key {
            req = req.bearer_auth(key);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let body = resp.text().await.unwrap_or_default();
        if op == "load" {
            // Carga recusada é quase sempre memória — a variante certa já
            // existe e a UI sabe explicá-la.
            return Err(EngineError::ModelLoad { model: id.into() });
        }
        Err(EngineError::Http {
            status: status.as_u16(),
            body: body.chars().take(300).collect(),
        })
    }

    /// Estado de cada modelo registrado no Router (`GET /models`).
    pub async fn models_status(&self) -> Result<Vec<RouterModel>, EngineError> {
        let url = format!("{}/models", self.config.connect_url());
        let mut req = self.http.get(&url);
        if let Some(key) = &self.config.api_key {
            req = req.bearer_auth(key);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(EngineError::Http {
                status: status.as_u16(),
                body: body.chars().take(300).collect(),
            });
        }
        let bruto: serde_json::Value = resp.json().await?;
        parse_models_status(&bruto)
            .ok_or_else(|| EngineError::Protocol("GET /models sem a lista `data`".into()))
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

    /// O setting é um arquivo editável por fora do app. Uma variável vinda
    /// dali não pode trocar o segredo nem abrir uma segunda porta para as
    /// mesmas flags do INI — esta é a última conferência antes do spawn.
    #[test]
    fn the_users_environment_never_touches_what_the_app_owns() {
        let mut cfg = ServerConfig::new(
            PathBuf::from("/tmp/llama-server"),
            PathBuf::from("/tmp/models"),
            8080,
        );
        cfg.api_key = Some("segredo".into());
        cfg.env_extra = vec![
            ("GGML_OP_OFFLOAD_MIN_BATCH".into(), "256".into()),
            // O segredo tem campo próprio.
            ("LLAMA_API_KEY".into(), "roubada".into()),
            // Atropelaria em silêncio a configuração por modelo.
            ("LLAMA_ARG_CTX_SIZE".into(), "512".into()),
            // Nome vazio não é variável.
            ("   ".into(), "x".into()),
        ];
        let envs = cfg.env_vars();
        let m: std::collections::HashMap<_, _> = envs.into_iter().collect();
        assert_eq!(m["LLAMA_API_KEY"], "segredo", "a chave do app é a que vale");
        assert_eq!(m["GGML_OP_OFFLOAD_MIN_BATCH"], "256");
        assert!(!m.contains_key("LLAMA_ARG_CTX_SIZE"));
        assert_eq!(m.len(), 2);
    }

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
        // CORS liberando `Authorization` vai SEMPRE: sem ele o webview não
        // consegue mandar Bearer quando a chave existir.
        assert!(joined.contains("--cors-headers Content-Type, Authorization"));
        // `/metrics` sempre ligado: as estatísticas de serviço dependem dele.
        assert!(joined.contains("--metrics"));
        // Router mode = SEM -m.
        assert!(!joined.contains(" -m "));
        assert!(!args.contains(&"-m".to_string()));
    }

    #[test]
    fn optional_args_appear_when_set() {
        let mut cfg = ServerConfig::new(PathBuf::from("srv"), PathBuf::from("models"), 9000);
        cfg.sleep_idle_seconds = 300;
        let joined = cfg.to_args().join(" ");
        assert!(joined.contains("--sleep-idle-seconds 300"));
    }

    /// A chave de API nunca pode aparecer no argv: ele vaza no process list e
    /// no log do spawn. Ela viaja por ambiente (`LLAMA_API_KEY`, o binding do
    /// `--api-key` no b10441).
    #[test]
    fn the_api_key_never_reaches_the_argv() {
        let mut cfg = ServerConfig::new(PathBuf::from("srv"), PathBuf::from("models"), 9000);
        cfg.api_key = Some("secreta".into());
        let joined = cfg.to_args().join(" ");
        assert!(!joined.contains("--api-key"));
        assert!(!joined.contains("secreta"));
        assert_eq!(
            cfg.env_vars(),
            vec![("LLAMA_API_KEY".to_string(), "secreta".to_string())]
        );
    }

    /// Sem chave configurada, o ambiente do processo fica limpo — exportar
    /// `LLAMA_API_KEY` vazio ligaria autenticação com chave "".
    #[test]
    fn no_api_key_means_no_env_var() {
        let cfg = ServerConfig::new(PathBuf::from("srv"), PathBuf::from("models"), 9000);
        assert!(cfg.env_vars().is_empty());
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

    #[test]
    fn extra_args_append_after_the_router_flags() {
        let mut cfg = ServerConfig::new(PathBuf::from("srv"), PathBuf::from("m"), 9000);
        cfg.extra_args = vec![
            "--rpc".into(),
            "192.168.1.8:50052".into(),
            "--device".into(),
            "RPC0,CUDA0".into(),
            "--tensor-split".into(),
            "3,2".into(),
        ];
        let joined = cfg.to_args().join(" ");
        assert!(joined.contains("--rpc 192.168.1.8:50052"));
        assert!(joined.contains("--device RPC0,CUDA0"));
        assert!(joined.contains("--tensor-split 3,2"));
        let rpc_at = joined.find("--rpc").unwrap();
        let port_at = joined.find("--port").unwrap();
        assert!(rpc_at > port_at, "RPC entra por último, depois do roteador");
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
            &[],
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
            &[],
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
            &[],
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

    /// Corpo real do `GET /models` do b10441 (capturado do binário oficial,
    /// resumido aos campos que importam).
    #[test]
    fn the_router_model_list_is_read_from_status_value() {
        let corpo: serde_json::Value = serde_json::from_str(
            r#"{"data":[
                {"id":"dummy","object":"model","owned_by":"llamacpp",
                 "status":{"value":"unloaded","args":["…"],"preset":"[dummy]\n"}},
                {"id":"qwen","object":"model","status":{"value":"loaded"}}
            ],"object":"list"}"#,
        )
        .unwrap();
        let modelos = parse_models_status(&corpo).unwrap();
        assert_eq!(
            modelos,
            vec![
                RouterModel {
                    id: "dummy".into(),
                    state: "unloaded".into()
                },
                RouterModel {
                    id: "qwen".into(),
                    state: "loaded".into()
                },
            ]
        );
        assert!(parse_models_status(&serde_json::json!({"objeto": "nada"})).is_none());
    }

    /// Extras globais vão para a seção `[*]`, antes das seções de modelo —
    /// e a precedência do Router faz o resto (seção do modelo vence `[*]`).
    #[test]
    fn global_extras_become_the_star_section() {
        let dir = temp_dir("star");
        let ini = dir.join("router-models.ini");
        write_models_preset(
            &ini,
            &[
                ("jinja".to_string(), "true".to_string()),
                ("cache-reuse".to_string(), "256".to_string()),
            ],
            &[PresetEntry::new("chat.gguf", "/m/chat.gguf").with_extra("cache-reuse", "0")],
        )
        .unwrap();
        let text = std::fs::read_to_string(&ini).unwrap();
        assert_eq!(
            text,
            "; gerado automaticamente — não edite\nversion = 1\n\n\
             [*]\njinja = true\ncache-reuse = 256\n\n\
             [chat.gguf]\nmodel = /m/chat.gguf\ncache-reuse = 0\n\n"
        );
        let star = text.find("[*]").unwrap();
        let modelo = text.find("[chat.gguf]").unwrap();
        assert!(star < modelo);
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
