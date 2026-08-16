//! Cliente HTTP do llama-server para o **harness agêntico**.
//!
//! O `llama.ts` do webview continua dono do chat normal; este cliente existe
//! porque o loop do agente vive em Rust e precisa de coisas que o chat não
//! usa: `tools`/`tool_choice`, agregação de tool-call deltas, `/props`,
//! `/v1/embeddings` e contagem de tokens do prompt.
//!
//! Fatos do protocolo (llama-server, ago/2026) que moldam o código:
//! - Em SSE, `delta.tool_calls` chega como `[{index, id?, type?,
//!   function:{name?, arguments?}}]` e o campo `arguments` vem
//!   **fragmentado** entre chunks — tem que ser concatenado por `index`.
//!   O `id` normalmente só aparece no primeiro fragmento da chamada.
//! - `reasoning_content` **coexiste** com tool calls no mesmo stream.
//! - O último chunk traz `timings` (`predicted_n`, `predicted_ms`,
//!   `predicted_per_second`), e o fim é `finish_reason: "tool_calls"` ou
//!   `"stop"`, seguido de `data: [DONE]`.
//! - O parser de JSON do servidor é chato com `null`: todo campo opcional
//!   sai do corpo via `skip_serializing_if` em vez de virar `null`.

use crate::EngineError;
use crate::props::{ServerProps, parse_props};
use lr_types::agent::ToolSpec;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::time::Duration;

// ------------------------------------------------------------- mensagens ---

/// Parte de conteúdo multimodal (mesmo formato do OpenAI, aceito pelo
/// llama-server quando há `--mmproj`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageUrl {
    /// URL `data:` (base64) ou http — o servidor baixa por conta própria.
    pub url: String,
}

impl ContentPart {
    pub fn text(t: impl Into<String>) -> Self {
        Self::Text { text: t.into() }
    }
    pub fn image(url: impl Into<String>) -> Self {
        Self::ImageUrl {
            image_url: ImageUrl { url: url.into() },
        }
    }
}

/// `content` é string (caso normal) ou lista de partes (multimodal).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl MessageContent {
    /// Texto concatenado — usado para heurísticas de contagem de tokens.
    pub fn as_plain_text(&self) -> String {
        match self {
            Self::Text(t) => t.clone(),
            Self::Parts(parts) => parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text { text } => Some(text.as_str()),
                    // Imagem não entra na heurística de caracteres.
                    ContentPart::ImageUrl { .. } => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

/// Uma tool call como o modelo a emitiu — precisa voltar **idêntica** no
/// histórico do passo seguinte, senão o template quebra o pareamento com a
/// mensagem `role: "tool"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallMsg {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionCallMsg,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionCallMsg {
    pub name: String,
    /// JSON **em string** (é assim que o OpenAI define e o llama-server espera).
    pub arguments: String,
}

/// Mensagem do histórico. Cobre os quatro papéis e os dois formatos de
/// conteúdo; `toolCalls`/`toolCallId` fecham o ciclo pedido→resultado que
/// mantém o loop do agente coerente entre passos.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChatMessage {
    /// `system` | `user` | `assistant` | `tool`.
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<MessageContent>,
    /// Só em `assistant`: ferramentas que o modelo pediu neste turno.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCallMsg>,
    /// Só em `tool`: id da chamada que este resultado responde.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Só em `tool`: nome da ferramenta (alguns templates usam).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl ChatMessage {
    fn with_role(role: &str, content: MessageContent) -> Self {
        Self {
            role: role.to_string(),
            content: Some(content),
            ..Default::default()
        }
    }

    pub fn system(text: impl Into<String>) -> Self {
        Self::with_role("system", MessageContent::Text(text.into()))
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self::with_role("user", MessageContent::Text(text.into()))
    }

    pub fn user_parts(parts: Vec<ContentPart>) -> Self {
        Self::with_role("user", MessageContent::Parts(parts))
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self::with_role("assistant", MessageContent::Text(text.into()))
    }

    /// Turno do assistente que pediu ferramentas. O `content` vai mesmo
    /// vazio: templates que só olham `tool_calls` ignoram, e os que
    /// interpolam `content` recebem string vazia em vez de `undefined`.
    pub fn assistant_with_tool_calls(text: impl Into<String>, calls: Vec<ToolCallMsg>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: Some(MessageContent::Text(text.into())),
            tool_calls: calls,
            ..Default::default()
        }
    }

    /// Resultado de uma ferramenta, pareado por `tool_call_id`.
    pub fn tool_result(
        call_id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            role: "tool".to_string(),
            content: Some(MessageContent::Text(content.into())),
            tool_call_id: Some(call_id.into()),
            name: Some(name.into()),
            ..Default::default()
        }
    }

    pub fn text(&self) -> String {
        self.content
            .as_ref()
            .map(MessageContent::as_plain_text)
            .unwrap_or_default()
    }
}

// ------------------------------------------------------------ requisição ---

/// `tool_choice`: `"auto"`/`"none"`/`"required"` ou uma função específica.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
    Mode(String),
    Named(NamedToolChoice),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedToolChoice {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: NamedFunction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedFunction {
    pub name: String,
}

impl ToolChoice {
    pub fn auto() -> Self {
        Self::Mode("auto".into())
    }
    pub fn none() -> Self {
        Self::Mode("none".into())
    }
    pub fn required() -> Self {
        Self::Mode("required".into())
    }
    pub fn function(name: impl Into<String>) -> Self {
        Self::Named(NamedToolChoice {
            kind: "function".into(),
            function: NamedFunction { name: name.into() },
        })
    }
}

/// Corpo de `/v1/chat/completions`.
///
/// Todo campo opcional some do JSON quando é `None` (o llama-server rejeita
/// ou ignora mal vários deles quando chegam como `null`).
#[derive(Debug, Clone, Default, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    /// Só é honrado se `chat_template_caps.supports_parallel_tool_calls`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    pub stream: bool,
    /// Campos extras do llama-server (`reasoning_effort`,
    /// `chat_template_kwargs`, `cache_prompt`...) sem precisar mexer aqui.
    #[serde(flatten, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

impl ChatRequest {
    pub fn new(model: impl Into<String>, messages: Vec<ChatMessage>) -> Self {
        Self {
            model: model.into(),
            messages,
            ..Default::default()
        }
    }

    pub fn with_tools(mut self, tools: Vec<Value>) -> Self {
        if !tools.is_empty() {
            self.tool_choice.get_or_insert_with(ToolChoice::auto);
        }
        self.tools = tools;
        self
    }

    pub fn with_extra(mut self, key: impl Into<String>, value: Value) -> Self {
        self.extra.insert(key.into(), value);
        self
    }

    fn with_stream(&self, stream: bool) -> Self {
        let mut copy = self.clone();
        copy.stream = stream;
        copy
    }
}

// -------------------------------------------------------------- resposta ---

/// Pedaço de resposta entregue ao chamador enquanto o stream corre.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatDelta {
    Text(String),
    Reasoning(String),
    /// Fragmento de tool call. `id`/`name` costumam vir só no primeiro
    /// fragmento de cada `index`; `args_fragment` pode ser vazio.
    ToolCall {
        index: u32,
        id: Option<String>,
        name: Option<String>,
        args_fragment: String,
    },
}

/// Tool call já reconstruída (fragmentos concatenados).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallReq {
    pub id: String,
    pub name: String,
    /// Argumentos como JSON em string, exatamente como o modelo emitiu.
    pub arguments_json: String,
}

impl ToolCallReq {
    /// Argumentos parseados; `Err` quando o modelo emitiu JSON inválido
    /// (acontece com modelos pequenos — quem chama transforma em erro de
    /// ferramenta e devolve ao modelo).
    pub fn arguments(&self) -> Result<Value, serde_json::Error> {
        if self.arguments_json.trim().is_empty() {
            return Ok(json!({}));
        }
        serde_json::from_str(&self.arguments_json)
    }
}

/// `timings` do llama-server (o chat usa para tokens/s; o agente, para o
/// orçamento de contexto e as estatísticas do run).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Timings {
    pub prompt_n: Option<u32>,
    pub prompt_ms: Option<f64>,
    pub predicted_n: Option<u32>,
    pub predicted_ms: Option<f64>,
    pub predicted_per_second: Option<f64>,
}

/// Resultado completo de um passo do loop.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatOutcome {
    pub content: String,
    pub reasoning: String,
    pub tool_calls: Vec<ToolCallReq>,
    /// `"stop"`, `"tool_calls"`, `"length"`... (ausente em alguns builds).
    pub finish_reason: Option<String>,
    pub timings: Option<Timings>,
}

impl ChatOutcome {
    pub fn wants_tools(&self) -> bool {
        !self.tool_calls.is_empty()
    }

    /// Turno do assistente para reinjetar no histórico do próximo passo.
    pub fn to_assistant_message(&self) -> ChatMessage {
        if self.tool_calls.is_empty() {
            return ChatMessage::assistant(self.content.clone());
        }
        let calls = self
            .tool_calls
            .iter()
            .map(|c| ToolCallMsg {
                id: c.id.clone(),
                kind: "function".into(),
                function: FunctionCallMsg {
                    name: c.name.clone(),
                    arguments: c.arguments_json.clone(),
                },
            })
            .collect();
        ChatMessage::assistant_with_tool_calls(self.content.clone(), calls)
    }
}

// ------------------------------------------------------- <think> inline ---

/// Separa `<think>...</think>` embutido no texto (modelos sem `--jinja` ou
/// sem parser de reasoning emitem assim). Espelha o splitter do `llama.ts`:
/// como a tag pode chegar partida entre dois deltas, seguramos o maior
/// sufixo do buffer que ainda pode ser o começo dela.
#[derive(Default)]
struct ThinkSplitter {
    pending: String,
    in_think: bool,
}

const THINK_OPEN: &str = "<think>";
const THINK_CLOSE: &str = "</think>";

/// Tamanho (em bytes) do maior sufixo de `s` que é prefixo próprio de `tag`.
/// `tag` é ASCII, então fatiar por bytes é seguro.
fn partial_suffix(s: &str, tag: &str) -> usize {
    let max = s.len().min(tag.len() - 1);
    (1..=max)
        .rev()
        .find(|k| s.ends_with(&tag[..*k]))
        .unwrap_or(0)
}

impl ThinkSplitter {
    /// Devolve segmentos `(é_raciocínio, texto)` já resolvidos.
    fn feed(&mut self, delta: &str) -> Vec<(bool, String)> {
        self.pending.push_str(delta);
        let mut out = Vec::new();
        loop {
            let tag = if self.in_think {
                THINK_CLOSE
            } else {
                THINK_OPEN
            };
            if let Some(idx) = self.pending.find(tag) {
                let before = self.pending[..idx].to_string();
                if !before.is_empty() {
                    out.push((self.in_think, before));
                }
                self.pending = self.pending[idx + tag.len()..].to_string();
                self.in_think = !self.in_think;
                continue;
            }
            let hold = partial_suffix(&self.pending, tag);
            let split = self.pending.len() - hold;
            if split > 0 {
                out.push((self.in_think, self.pending[..split].to_string()));
                self.pending = self.pending[split..].to_string();
            }
            break;
        }
        out
    }

    fn flush(&mut self) -> Option<(bool, String)> {
        if self.pending.is_empty() {
            return None;
        }
        let rest = std::mem::take(&mut self.pending);
        Some((self.in_think, rest))
    }
}

// --------------------------------------------------------------- cliente ---

/// Cliente do llama-server local. Barato de clonar? Não: crie um por uso —
/// o `reqwest::Client` interno já tem pool de conexões próprio.
pub struct LlamaClient {
    base_url: String,
    http: reqwest::Client,
    api_key: Option<String>,
}

/// Requisições curtas (props/tokenize/embeddings) não podem pendurar o loop.
const SHORT_TIMEOUT: Duration = Duration::from_secs(30);

/// Pedir as props de um modelo específico faz o roteador CARREGAR o modelo.
/// O relógio aqui é o do disco e da placa, não o de uma consulta.
const LOAD_TIMEOUT: Duration = Duration::from_secs(300);

impl LlamaClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let base = base_url.into().trim_end_matches('/').to_string();
        Self {
            base_url: base,
            // Sem timeout global: geração pode legitimamente demorar minutos.
            // O connect_timeout evita pendurar quando o servidor caiu.
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            api_key: None,
        }
    }

    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        let key = key.into();
        self.api_key = (!key.is_empty()).then_some(key);
        self
    }

    /// Versão para `ServerConfig::api_key`, que já é `Option`.
    pub fn with_optional_api_key(mut self, key: Option<String>) -> Self {
        self.api_key = key.filter(|k| !k.is_empty());
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.api_key {
            Some(key) => rb.bearer_auth(key),
            None => rb,
        }
    }

    /// `GET /props` sem dizer o modelo.
    ///
    /// **No modo Router isto NÃO descreve modelo nenhum** — devolve as props
    /// do roteador (veja [`ServerProps::describes_model`]). Serve para saber
    /// se o servidor está de pé; para decidir qualquer coisa sobre um modelo,
    /// use [`LlamaClient::props_for`].
    ///
    /// Erro de status vira `Err`, nunca um `ServerProps` vazio: um default
    /// silencioso seria lido como "modelo não suporta ferramentas" e a UI
    /// mostraria o badge errado em vez de avisar que o servidor caiu.
    pub async fn props(&self) -> Result<ServerProps, EngineError> {
        self.props_for(None).await
    }

    /// `GET /props?model=…` → capacidades daquele modelo.
    ///
    /// Duas coisas que o call site precisa saber:
    ///
    /// - **Sem o `model`, o roteador responde por si mesmo.** Era o que
    ///   acontecia no laço do agente, e o efeito era o modo agente rodar sem
    ///   ferramenta nenhuma, calado, com qualquer modelo.
    /// - **Perguntar CARREGA o modelo** (autoload do roteador). Por isso o
    ///   timeout aqui é de carga, não o curto: numa placa ocupada, subir um
    ///   modelo de 9B passa folgado dos 30 s.
    pub async fn props_for(&self, model: Option<&str>) -> Result<ServerProps, EngineError> {
        let modelo = model.map(str::trim).filter(|m| !m.is_empty());
        let mut rb = self.auth(self.http.get(self.url("/props")));
        if let Some(m) = modelo {
            rb = rb.query(&[("model", m)]);
        }
        let mut res = rb
            .timeout(if modelo.is_some() {
                LOAD_TIMEOUT
            } else {
                SHORT_TIMEOUT
            })
            .send()
            .await?;
        ensure_ok(&mut res).await?;
        let v: Value = read_json(res).await?;
        Ok(parse_props(&v))
    }

    /// `GET /v1/models` → nomes dos modelos **já carregados**.
    ///
    /// Existe para quem quer as capacidades de um modelo sem pagar por elas:
    /// [`LlamaClient::props_for`] carrega o modelo que não está no ar, e com
    /// `models-max = 1` isso despeja o que estava carregado. Uma tela que só
    /// quer desenhar um selo não pode causar isso.
    pub async fn loaded_models(&self) -> Result<Vec<String>, EngineError> {
        let mut res = self
            .auth(self.http.get(self.url("/v1/models")))
            .timeout(SHORT_TIMEOUT)
            .send()
            .await?;
        ensure_ok(&mut res).await?;
        let v: Value = read_json(res).await?;
        Ok(v["data"]
            .as_array()
            .map(|itens| {
                itens
                    .iter()
                    .filter(|m| m["status"]["value"].as_str() == Some("loaded"))
                    .filter_map(|m| m["id"].as_str())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default())
    }

    /// `POST /v1/chat/completions` com `stream: true`.
    ///
    /// Chama `on_delta` a cada pedaço (texto, raciocínio ou fragmento de tool
    /// call) e devolve o resultado agregado no fim.
    pub async fn chat_stream<F>(
        &self,
        req: &ChatRequest,
        on_delta: &mut F,
    ) -> Result<ChatOutcome, EngineError>
    where
        F: FnMut(ChatDelta),
    {
        let body = req.with_stream(true);
        let mut res = self
            .auth(self.http.post(self.url("/v1/chat/completions")))
            .json(&body)
            .send()
            .await?;
        ensure_ok(&mut res).await?;

        let mut acc = StreamAcc::default();
        // Buffer de BYTES: um caractere UTF-8 pode ser partido entre dois
        // chunks TCP, e '\n' nunca aparece dentro de uma sequência
        // multibyte — logo, cortar por '\n' sempre cai em fronteira válida.
        let mut buf: Vec<u8> = Vec::new();
        let mut done = false;

        while !done {
            let Some(chunk) = res.chunk().await? else {
                break;
            };
            buf.extend_from_slice(&chunk);
            while let Some(nl) = buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buf.drain(..=nl).collect();
                let line = String::from_utf8_lossy(&line[..line.len() - 1]);
                if acc.handle_line(line.trim_end_matches('\r'), on_delta)? {
                    done = true;
                    break;
                }
            }
        }
        // Sobra sem '\n' no fim (servidor fechou a conexão sem newline).
        if !done && !buf.is_empty() {
            let line = String::from_utf8_lossy(&buf).to_string();
            acc.handle_line(line.trim_end_matches('\r'), on_delta)?;
        }
        Ok(acc.finish(on_delta))
    }

    /// `POST /v1/chat/completions` com `stream: false`. Para tarefas
    /// auxiliares (título, sumarização, verificação) em que streaming só
    /// atrapalha.
    pub async fn complete_once(&self, req: &ChatRequest) -> Result<ChatOutcome, EngineError> {
        let body = req.with_stream(false);
        let mut res = self
            .auth(self.http.post(self.url("/v1/chat/completions")))
            .json(&body)
            .send()
            .await?;
        ensure_ok(&mut res).await?;
        let v: Value = read_json(res).await?;
        Ok(parse_completion(&v))
    }

    /// `POST /v1/embeddings`. No Router mode o `model` escolhe o modelo de
    /// embedding (subido com `pooling = mean` via preset).
    pub async fn embeddings(
        &self,
        model: &str,
        inputs: &[String],
    ) -> Result<Vec<Vec<f32>>, EngineError> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        let mut res = self
            .auth(self.http.post(self.url("/v1/embeddings")))
            .json(&json!({ "model": model, "input": inputs }))
            .send()
            .await?;
        ensure_ok(&mut res).await?;
        let v: Value = read_json(res).await?;
        let data = v["data"]
            .as_array()
            .ok_or_else(|| EngineError::Protocol("resposta de /v1/embeddings sem `data`".into()))?;
        let mut out = Vec::with_capacity(data.len());
        for item in data {
            out.push(parse_embedding(&item["embedding"]).ok_or_else(|| {
                EngineError::Protocol("item de /v1/embeddings sem `embedding` numérico".into())
            })?);
        }
        Ok(out)
    }

    /// Quantos tokens o prompt desta requisição ocupa — base do orçamento de
    /// contexto (auto-compact em ~80% de `n_ctx`).
    ///
    /// Três níveis, do exato ao aproximado:
    /// 1. `POST /v1/chat/completions/input_tokens` (builds recentes);
    /// 2. `POST /apply-template` + `POST /tokenize` (renderiza o prompt pelo
    ///    template real e conta os tokens de verdade);
    /// 3. heurística `caracteres / 4 + 4 por mensagem` — grosseira, mas só
    ///    entra quando o servidor não oferece nenhum dos dois endpoints.
    pub async fn input_tokens(&self, req: &ChatRequest) -> Result<u32, EngineError> {
        let body = req.with_stream(false);
        if let Ok(res) = self
            .auth(
                self.http
                    .post(self.url("/v1/chat/completions/input_tokens")),
            )
            .json(&body)
            .timeout(SHORT_TIMEOUT)
            .send()
            .await
            && res.status().is_success()
            && let Ok(v) = res.json::<Value>().await
            && let Some(n) = extract_token_count(&v)
        {
            return Ok(n);
        }

        if let Some(n) = self.tokens_via_template(&body).await {
            return Ok(n);
        }

        Ok(estimate_tokens(&body.messages))
    }

    /// Passo 2 do `input_tokens`: renderiza o prompt e tokeniza.
    async fn tokens_via_template(&self, req: &ChatRequest) -> Option<u32> {
        let res = self
            .auth(self.http.post(self.url("/apply-template")))
            .json(&json!({ "messages": req.messages }))
            .timeout(SHORT_TIMEOUT)
            .send()
            .await
            .ok()?;
        if !res.status().is_success() {
            return None;
        }
        let v: Value = res.json().await.ok()?;
        let prompt = v["prompt"].as_str().or_else(|| v.as_str())?;

        let res = self
            .auth(self.http.post(self.url("/tokenize")))
            .json(&json!({ "content": prompt }))
            .timeout(SHORT_TIMEOUT)
            .send()
            .await
            .ok()?;
        if !res.status().is_success() {
            return None;
        }
        let v: Value = res.json().await.ok()?;
        extract_token_count(&v)
    }
}

// -------------------------------------------------------- SSE / agregação ---

#[derive(Default)]
struct ToolCallAcc {
    id: Option<String>,
    name: Option<String>,
    args: String,
}

/// Estado da agregação de um stream.
#[derive(Default)]
struct StreamAcc {
    content: String,
    reasoning: String,
    calls: BTreeMap<u32, ToolCallAcc>,
    last_index: Option<u32>,
    finish_reason: Option<String>,
    timings: Option<Timings>,
    splitter: ThinkSplitter,
}

impl StreamAcc {
    /// Processa uma linha do SSE. Devolve `true` quando veio `[DONE]`.
    fn handle_line<F: FnMut(ChatDelta)>(
        &mut self,
        line: &str,
        on_delta: &mut F,
    ) -> Result<bool, EngineError> {
        let line = line.trim();
        // Linhas vazias (separador de evento) e comentários (`: ping`) são
        // parte normal do protocolo SSE.
        let Some(payload) = line.strip_prefix("data:") else {
            return Ok(false);
        };
        let payload = payload.trim();
        if payload == "[DONE]" {
            return Ok(true);
        }
        let Ok(chunk) = serde_json::from_str::<Value>(payload) else {
            // Ruído ou linha que não é JSON — o chat do webview também
            // ignora em silêncio.
            return Ok(false);
        };
        if let Some(msg) = error_message(&chunk) {
            return Err(EngineError::Protocol(msg));
        }
        self.absorb_timings(&chunk);

        let choice = &chunk["choices"][0];
        if let Some(reason) = choice["finish_reason"].as_str() {
            self.finish_reason = Some(reason.to_string());
        }
        let delta = &choice["delta"];
        if let Some(r) = non_empty_str(&delta["reasoning_content"]) {
            self.reasoning.push_str(r);
            on_delta(ChatDelta::Reasoning(r.to_string()));
        }
        if let Some(t) = non_empty_str(&delta["content"]) {
            for (is_reasoning, seg) in self.splitter.feed(t) {
                if is_reasoning {
                    self.reasoning.push_str(&seg);
                    on_delta(ChatDelta::Reasoning(seg));
                } else {
                    self.content.push_str(&seg);
                    on_delta(ChatDelta::Text(seg));
                }
            }
        }
        if let Some(items) = delta["tool_calls"].as_array() {
            for item in items {
                self.absorb_tool_call(item, on_delta);
            }
        }
        Ok(false)
    }

    fn absorb_timings(&mut self, chunk: &Value) {
        let t = &chunk["timings"];
        if !t.is_object() {
            return;
        }
        self.timings = Some(Timings {
            prompt_n: t["prompt_n"].as_u64().map(|n| n as u32),
            prompt_ms: t["prompt_ms"].as_f64(),
            predicted_n: t["predicted_n"].as_u64().map(|n| n as u32),
            predicted_ms: t["predicted_ms"].as_f64(),
            predicted_per_second: t["predicted_per_second"].as_f64(),
        });
    }

    /// Junta um fragmento de tool call ao acumulador do seu `index`.
    fn absorb_tool_call<F: FnMut(ChatDelta)>(&mut self, item: &Value, on_delta: &mut F) {
        let id = non_empty_str(&item["id"]).map(str::to_string);
        let name = non_empty_str(&item["function"]["name"]).map(str::to_string);
        let args = item["function"]["arguments"].as_str().unwrap_or_default();

        // `index` é obrigatório na spec, mas builds antigos omitem: com `id`
        // novo abrimos a próxima posição livre, senão continuamos a última.
        let index = match item["index"].as_u64() {
            Some(i) => i as u32,
            None if id.is_some() => self.calls.keys().next_back().map_or(0, |m| m + 1),
            None => self.last_index.unwrap_or(0),
        };
        self.last_index = Some(index);

        let entry = self.calls.entry(index).or_default();
        if let Some(id) = &id {
            entry.id = Some(id.clone());
        }
        if let Some(name) = &name {
            // Alguns servidores repetem o nome em cada fragmento; outros o
            // mandam partido junto com os argumentos. Concatenar quebraria o
            // primeiro caso, então o primeiro nome não-vazio vence.
            entry.name.get_or_insert_with(|| name.clone());
        }
        entry.args.push_str(args);

        on_delta(ChatDelta::ToolCall {
            index,
            id,
            name,
            args_fragment: args.to_string(),
        });
    }

    fn finish<F: FnMut(ChatDelta)>(mut self, on_delta: &mut F) -> ChatOutcome {
        if let Some((is_reasoning, seg)) = self.splitter.flush() {
            if is_reasoning {
                self.reasoning.push_str(&seg);
                on_delta(ChatDelta::Reasoning(seg));
            } else {
                self.content.push_str(&seg);
                on_delta(ChatDelta::Text(seg));
            }
        }
        let tool_calls = self
            .calls
            .into_values()
            .map(|acc| ToolCallReq {
                // Sem `id` (raro, mas acontece) sintetizamos um. Ele
                // pareia `assistant.tool_calls` com a mensagem `role:
                // "tool"` do passo seguinte, mas não pode se repetir entre
                // passos: o id é chave primária na trilha, e um `call_0` por
                // passo faria a chamada nova sobrescrever a antiga.
                id: acc.id.unwrap_or_else(synthetic_id),
                name: acc.name.unwrap_or_default(),
                arguments_json: acc.args,
            })
            // Fragmento sem nome nenhum é lixo: não vira chamada.
            .filter(|c| !c.name.is_empty())
            .collect();
        ChatOutcome {
            content: self.content,
            reasoning: self.reasoning,
            tool_calls,
            finish_reason: self.finish_reason,
            timings: self.timings,
        }
    }
}

/// Resposta não-streaming → mesmo `ChatOutcome`.
fn parse_completion(v: &Value) -> ChatOutcome {
    let choice = &v["choices"][0];
    let msg = &choice["message"];
    let mut content = msg["content"].as_str().unwrap_or_default().to_string();
    let mut reasoning = msg["reasoning_content"]
        .as_str()
        .unwrap_or_default()
        .to_string();

    // `<think>` inline também pode aparecer sem streaming.
    if content.contains(THINK_OPEN) {
        let mut splitter = ThinkSplitter::default();
        let mut text = String::new();
        let mut segments = splitter.feed(&content);
        segments.extend(splitter.flush());
        for (is_reasoning, seg) in segments {
            if is_reasoning {
                reasoning.push_str(&seg);
            } else {
                text.push_str(&seg);
            }
        }
        content = text;
    }

    let tool_calls = msg["tool_calls"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| ToolCallReq {
                    id: non_empty_str(&item["id"])
                        .map(str::to_string)
                        .unwrap_or_else(synthetic_id),
                    name: item["function"]["name"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    arguments_json: item["function"]["arguments"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                })
                .filter(|c| !c.name.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let timings = v["timings"].is_object().then(|| Timings {
        prompt_n: v["timings"]["prompt_n"].as_u64().map(|n| n as u32),
        prompt_ms: v["timings"]["prompt_ms"].as_f64(),
        predicted_n: v["timings"]["predicted_n"].as_u64().map(|n| n as u32),
        predicted_ms: v["timings"]["predicted_ms"].as_f64(),
        predicted_per_second: v["timings"]["predicted_per_second"].as_f64(),
    });

    ChatOutcome {
        content,
        reasoning,
        tool_calls,
        finish_reason: choice["finish_reason"].as_str().map(str::to_string),
        timings,
    }
}

// ------------------------------------------------------------- utilidades ---

/// Id de chamada quando o servidor não manda um.
///
/// Único no processo: dois passos do mesmo run — ou dois runs — não podem
/// gerar o mesmo id, senão a linha da trilha de um sobrescreve a do outro.
fn synthetic_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(1);
    format!("call_{}", SEQ.fetch_add(1, Ordering::Relaxed))
}

fn non_empty_str(v: &Value) -> Option<&str> {
    v.as_str().filter(|s| !s.is_empty())
}

/// Mensagem de erro do servidor, em qualquer um dos formatos que ele usa.
fn error_message(v: &Value) -> Option<String> {
    let e = &v["error"];
    if e.is_null() {
        return None;
    }
    Some(
        e["message"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| e.to_string()),
    )
}

/// Falha cedo com corpo legível (o llama-server devolve JSON com `error`).
async fn ensure_ok(res: &mut reqwest::Response) -> Result<(), EngineError> {
    if res.status().is_success() {
        return Ok(());
    }
    let status = res.status().as_u16();
    let mut body = String::new();
    while let Ok(Some(chunk)) = res.chunk().await {
        body.push_str(&String::from_utf8_lossy(&chunk));
        if body.len() > 2000 {
            break;
        }
    }
    let parsed = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| error_message(&v));
    let body = parsed.unwrap_or(body);

    // "model name=X failed to load" é o jeito do roteador dizer que não coube
    // na placa. Repassar isso cru manda a pessoa procurar um erro de HTTP
    // quando o problema é memória — e a saída é outra.
    if let Some(model) = failed_to_load(&body) {
        return Err(EngineError::ModelLoad { model });
    }

    Err(EngineError::Http {
        status,
        body: body.chars().take(500).collect(),
    })
}

/// Extrai o nome do modelo de um "model name=X failed to load".
fn failed_to_load(body: &str) -> Option<String> {
    if !body.contains("failed to load") {
        return None;
    }
    let nome = body
        .split("name=")
        .nth(1)
        .map(|rest| {
            rest.split_whitespace()
                .next()
                .unwrap_or("")
                .trim_end_matches(&[',', '"', '\''][..])
                .to_string()
        })
        .filter(|s| !s.is_empty());
    Some(nome.unwrap_or_else(|| "escolhido".to_string()))
}

async fn read_json(res: reqwest::Response) -> Result<Value, EngineError> {
    let text = res.text().await?;
    serde_json::from_str(&text).map_err(|e| {
        EngineError::Protocol(format!(
            "JSON inválido do engine: {e} — corpo: {}",
            text.chars().take(200).collect::<String>()
        ))
    })
}

/// `[0.1, 0.2]` ou `[[0.1, 0.2]]` (pooling `none` devolve por token —
/// nesse caso ficamos com a primeira linha).
fn parse_embedding(v: &Value) -> Option<Vec<f32>> {
    let arr = v.as_array()?;
    if arr.first().is_some_and(Value::is_array) {
        return parse_embedding(arr.first()?);
    }
    arr.iter().map(|n| n.as_f64().map(|f| f as f32)).collect()
}

/// Extrai uma contagem de tokens de qualquer um dos formatos conhecidos:
/// número puro, array de ids, ou objeto com `tokens`/`prompt_tokens`/...
fn extract_token_count(v: &Value) -> Option<u32> {
    match v {
        Value::Number(n) => n.as_u64().and_then(|u| u32::try_from(u).ok()),
        Value::Array(items) => u32::try_from(items.len()).ok(),
        Value::Object(_) => {
            for key in [
                "prompt_tokens",
                "input_tokens",
                "n_tokens",
                "count",
                "size",
                "tokens",
            ] {
                match &v[key] {
                    Value::Number(n) => {
                        if let Some(u) = n.as_u64().and_then(|u| u32::try_from(u).ok()) {
                            return Some(u);
                        }
                    }
                    Value::Array(items) => return u32::try_from(items.len()).ok(),
                    _ => {}
                }
            }
            // `{"usage": {"prompt_tokens": N}}`
            extract_token_count(&v["usage"])
        }
        _ => None,
    }
}

/// Último recurso: ~4 caracteres por token + 4 tokens por mensagem (as
/// marcas de turno do template). É uma aproximação grosseira — erra para
/// menos em código e em CJK —, então quem usa isto para orçar contexto deve
/// manter margem. Só entra quando o servidor não expõe nem
/// `input_tokens` nem `/tokenize`.
fn estimate_tokens(messages: &[ChatMessage]) -> u32 {
    let mut chars = 0usize;
    for m in messages {
        chars += m.text().chars().count();
        chars += m.role.len();
        for c in &m.tool_calls {
            chars += c.function.name.chars().count() + c.function.arguments.chars().count();
        }
    }
    let est = chars / 4 + messages.len() * 4;
    u32::try_from(est).unwrap_or(u32::MAX)
}

// ----------------------------------------------------- schema → API tools ---

/// Palavras-chave de JSON Schema que o conversor GBNF do llama.cpp não
/// suporta. Deixá-las passar produz gramática inválida ou — pior — o bug
/// *fail-open* em que a restrição some sem aviso. Removidas na entrada.
const UNSUPPORTED_KEYWORDS: &[&str] = &[
    "$schema",
    "$id",
    "$ref",
    "$defs",
    "$anchor",
    "$comment",
    "$dynamicRef",
    "$dynamicAnchor",
    "$vocabulary",
    "definitions",
    "not",
    "if",
    "then",
    "else",
    "dependencies",
    "dependentSchemas",
    "dependentRequired",
    "propertyNames",
    "patternProperties",
    "contains",
    "minContains",
    "maxContains",
    "unevaluatedProperties",
    "unevaluatedItems",
    "additionalItems",
];

/// Chaves cujo valor é um sub-schema.
const SCHEMA_VALUE_KEYS: &[&str] = &["items", "additionalProperties"];
/// Chaves cujo valor é uma lista de sub-schemas.
const SCHEMA_LIST_KEYS: &[&str] = &["anyOf", "oneOf", "allOf", "prefixItems"];
/// Chaves cujo valor é um mapa `nome → sub-schema` (os nomes são dados, não
/// palavras-chave: uma propriedade chamada `if` tem que sobreviver).
const SCHEMA_MAP_KEYS: &[&str] = &["properties"];

/// `true` quando a regex usa construções que o conversor GBNF rejeita.
///
/// Ele entende literais, classes `[...]`, `.`, `*`, `+`, `?`, `{n,m}`,
/// alternância e grupos simples — mas **não** os atalhos PCRE (`\d`, `\w`,
/// `\s`, `\b`, `\p{...}`), backreferences nem grupos `(?...)`.
fn regex_is_gbnf_safe(pattern: &str) -> bool {
    if pattern.contains("(?") {
        return false;
    }
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                // `\.` `\-` `\/` etc. são escapes de literal: ok.
                Some(next) if next.is_ascii_alphanumeric() => return false,
                Some(_) => {}
                None => return false,
            }
        }
    }
    true
}

/// Poda um JSON Schema para o que o conversor GBNF do llama.cpp aguenta.
fn sanitize_schema(v: &Value) -> Value {
    let Value::Object(map) = v else {
        return v.clone();
    };
    let mut out = Map::new();
    for (key, val) in map {
        if UNSUPPORTED_KEYWORDS.contains(&key.as_str()) {
            continue;
        }
        if key == "pattern" {
            if val.as_str().is_some_and(regex_is_gbnf_safe) {
                out.insert(key.clone(), val.clone());
            }
            continue;
        }
        if SCHEMA_MAP_KEYS.contains(&key.as_str()) {
            if let Value::Object(props) = val {
                let inner: Map<String, Value> = props
                    .iter()
                    .map(|(name, schema)| (name.clone(), sanitize_schema(schema)))
                    .collect();
                out.insert(key.clone(), Value::Object(inner));
            }
            continue;
        }
        if SCHEMA_LIST_KEYS.contains(&key.as_str()) {
            if let Value::Array(items) = val {
                out.insert(
                    key.clone(),
                    Value::Array(items.iter().map(sanitize_schema).collect()),
                );
            }
            continue;
        }
        if SCHEMA_VALUE_KEYS.contains(&key.as_str()) {
            out.insert(
                key.clone(),
                match val {
                    // `items` pode ser lista de schemas (tuplas).
                    Value::Array(items) => {
                        Value::Array(items.iter().map(sanitize_schema).collect())
                    }
                    other => sanitize_schema(other),
                },
            );
            continue;
        }
        // `enum`, `const`, `default`, `examples` carregam DADOS, não
        // schemas: copiar literalmente (um valor de enum pode ter uma chave
        // chamada `$ref` sem ser uma referência).
        out.insert(key.clone(), val.clone());
    }
    Value::Object(out)
}

/// Garante um schema de objeto no topo: modelos pequenos lidam mal com
/// `parameters` ausente ou de outro tipo, e o conversor GBNF exige objeto.
fn normalize_parameters(v: &Value) -> Value {
    let sanitized = sanitize_schema(v);
    let Value::Object(mut map) = sanitized else {
        return json!({ "type": "object", "properties": {} });
    };
    map.entry("type".to_string())
        .or_insert_with(|| Value::String("object".into()));
    if map.get("type").and_then(Value::as_str) == Some("object") {
        map.entry("properties".to_string())
            .or_insert_with(|| Value::Object(Map::new()));
    }
    Value::Object(map)
}

/// Converte o catálogo interno no `tools` da API OpenAI, já saneado.
pub fn tool_specs_to_api(specs: &[ToolSpec]) -> Vec<Value> {
    specs
        .iter()
        .map(|s| {
            json!({
                "type": "function",
                "function": {
                    "name": s.name,
                    "description": s.description,
                    "parameters": normalize_parameters(&s.parameters),
                }
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;
