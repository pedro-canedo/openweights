//! Cliente HTTP da API do Hugging Face Hub.

use crate::card::card_em_markdown;
use crate::{HF_BASE, ModelCaps, ModelSummary, ModelsError, RepoFile};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    Trending,
    Downloads,
    Likes,
    Updated,
}

impl SortBy {
    fn param(self) -> &'static str {
        match self {
            SortBy::Trending => "trendingScore",
            SortBy::Downloads => "downloads",
            SortBy::Likes => "likes",
            SortBy::Updated => "lastModified",
        }
    }
}

#[derive(Clone)]
pub struct HfClient {
    http: reqwest::Client,
    /// Não segue redirects: o probe de acesso quer LER o 302 do `/resolve/`
    /// como "liberado", não segui-lo até a CDN e começar a baixar bytes.
    http_noredir: reqwest::Client,
    token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiModel {
    id: String,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    likes: u64,
    #[serde(default)]
    gated: serde_json::Value,
    #[serde(default)]
    last_modified: Option<String>,
    #[serde(default)]
    gguf: Option<ApiGguf>,
    // O `rename_all = "camelCase"` da struct procuraria `pipelineTag`, e o
    // Hub manda `pipeline_tag` — sem este rename a etiqueta chega sempre
    // vazia e nenhum modelo teria o selo de visão.
    #[serde(default, rename = "pipeline_tag")]
    pipeline_tag: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    card_data: Option<ApiCardData>,
}

/// O cabeçalho do README (`cardData`) — daqui sai a licença.
#[derive(Deserialize)]
struct ApiCardData {
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    license_name: Option<String>,
}

#[derive(Deserialize)]
struct ApiGguf {
    #[serde(default)]
    total: Option<u64>,
    #[serde(default)]
    architecture: Option<String>,
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    chat_template: Option<String>,
}

/// Os campos do `config.json` do transformers que descrevem a forma.
///
/// Nomes seguem o arquivo; ausência é `None`, e o advisor completa o que
/// faltar. `num_key_value_heads` some em modelos sem GQA — ali ele é igual a
/// `num_attention_heads`, e é assim que a leitura o trata.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct BaseConfig {
    #[serde(default)]
    pub num_hidden_layers: Option<u32>,
    #[serde(default)]
    pub num_attention_heads: Option<u32>,
    #[serde(default)]
    pub num_key_value_heads: Option<u32>,
    #[serde(default)]
    pub head_dim: Option<u32>,
    #[serde(default)]
    pub hidden_size: Option<u32>,
    /// `num_experts` no Qwen; `num_local_experts` no Mixtral e derivados.
    #[serde(default, alias = "num_local_experts")]
    pub num_experts: Option<u32>,
    #[serde(default, alias = "num_experts_per_token")]
    pub num_experts_per_tok: Option<u32>,
    /// Dimensão interna de um especialista roteado.
    #[serde(default)]
    pub moe_intermediate_size: Option<u32>,
    /// Dimensão interna do especialista compartilhado, quando existe.
    #[serde(default, alias = "shared_expert_intermediate_size")]
    pub moe_shared_expert_intermediate_size: Option<u32>,
}

/// Metadados GGUF de um repositório (via `expand[]=gguf`), sem baixar nada.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GgufRepoMeta {
    pub params_total: Option<u64>,
    pub architecture: Option<String>,
    pub context_length: Option<u64>,
    pub chat_template: Option<String>,
    /// Tags do repositório — é aqui que `base_model:<autor>/<nome>` aponta
    /// para o repositório original, o único que publica a geometria.
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Deserialize)]
struct ApiTreeEntry {
    #[serde(rename = "type")]
    kind: String,
    path: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    lfs: Option<ApiLfs>,
}

#[derive(Deserialize)]
struct ApiLfs {
    size: u64,
}

impl HfClient {
    pub fn new(token: Option<String>) -> Self {
        let ua = concat!("OpenWeights/", env!("CARGO_PKG_VERSION"));
        let http = reqwest::Client::builder()
            .user_agent(ua)
            .build()
            .expect("reqwest client");
        let http_noredir = reqwest::Client::builder()
            .user_agent(ua)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("reqwest client");
        Self {
            http,
            http_noredir,
            token,
        }
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(t) => req.bearer_auth(t),
            None => req,
        }
    }

    /// Busca modelos GGUF. `query` vazio retorna os em alta.
    pub async fn search(
        &self,
        query: &str,
        sort: SortBy,
        limit: u32,
    ) -> Result<Vec<ModelSummary>, ModelsError> {
        Ok(self.search_cursor(query, sort, limit, None).await?.0)
    }

    /// Busca paginada: devolve a página e o cursor da próxima (se houver).
    ///
    /// A API pagina via header `Link` (`rel="next"`); o cursor é a URL
    /// completa da próxima página — quando presente, `query`/`sort`/`limit`
    /// são ignorados e essa URL é usada direto.
    pub async fn search_cursor(
        &self,
        query: &str,
        sort: SortBy,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<(Vec<ModelSummary>, Option<String>), ModelsError> {
        let url = match cursor {
            Some(next) => {
                // O cursor vem do header Link de uma resposta anterior; só
                // aceitamos URLs do próprio Hub (o bearer token vai junto).
                if !next.starts_with(HF_BASE) {
                    return Err(ModelsError::Api(format!("cursor inválido: {next}")));
                }
                next.to_string()
            }
            None => {
                let mut url = format!(
                    "{HF_BASE}/api/models?filter=gguf&sort={}&direction=-1&limit={}&expand[]=gguf&expand[]=gated&expand[]=downloads&expand[]=likes&expand[]=lastModified&expand[]=pipeline_tag&expand[]=tags&expand[]=cardData",
                    sort.param(),
                    limit.clamp(1, 100),
                );
                if !query.trim().is_empty() {
                    url.push_str(&format!("&search={}", urlencode(query.trim())));
                }
                url
            }
        };

        let resp = self.auth(self.http.get(&url)).send().await?;
        if !resp.status().is_success() {
            return Err(ModelsError::Api(format!(
                "busca retornou HTTP {}",
                resp.status()
            )));
        }
        let next = resp
            .headers()
            .get("link")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_link_next);
        let raw: Vec<ApiModel> = resp.json().await?;
        Ok((raw.into_iter().map(to_summary).collect(), next))
    }

    /// Metadados GGUF de um repositório (nº de parâmetros, arquitetura,
    /// context_length, chat_template). `None` quando o repo não expõe o
    /// bloco `gguf`.
    pub async fn gguf_meta(&self, repo_id: &str) -> Result<Option<GgufRepoMeta>, ModelsError> {
        // As tags vêm junto porque é nelas que mora `base_model:` — o
        // ponteiro para o repositório que tem a geometria completa.
        let url = format!("{HF_BASE}/api/models/{repo_id}?expand[]=gguf&expand[]=tags");
        let resp = self.auth(self.http.get(&url)).send().await?;
        match resp.status().as_u16() {
            200 => {}
            401 | 403 => return Err(ModelsError::Gated),
            s => return Err(ModelsError::Api(format!("metadados retornaram HTTP {s}"))),
        }
        #[derive(Deserialize)]
        struct Resp {
            #[serde(default)]
            gguf: Option<ApiGguf>,
            #[serde(default)]
            tags: Vec<String>,
        }
        let raw: Resp = resp.json().await?;
        let tags = raw.tags;
        Ok(raw.gguf.map(|g| GgufRepoMeta {
            params_total: g.total,
            architecture: g.architecture,
            context_length: g.context_length,
            chat_template: g.chat_template,
            tags,
        }))
    }

    /// A geometria do modelo, do `config.json` do repositório BASE.
    ///
    /// O bloco `gguf` da API responde parâmetros, arquitetura e janela de
    /// treino — e para o resto ficava a tabela de chute por faixa de
    /// parâmetros, que descreve modelos densos e não tem o que dizer sobre
    /// mistura de especialistas. O `config.json` tem tudo: quantas camadas,
    /// quantas cabeças de KV, quantos especialistas existem e quantos
    /// disparam por token.
    ///
    /// Ele mora no repositório ORIGINAL, não no de GGUF — e o repositório de
    /// GGUF diz qual é, na tag `base_model:<autor>/<nome>`. Sem a tag, sem
    /// resposta: seguir para um palpite de nome seria trocar "não sei" por
    /// "talvez", que é pior.
    pub async fn base_config(&self, tags: &[String]) -> Option<BaseConfig> {
        let base = tags.iter().find_map(|t| {
            t.strip_prefix("base_model:")
                .filter(|r| !r.contains(':') && r.contains('/'))
        })?;
        let url = format!("{HF_BASE}/{base}/raw/main/config.json");
        let resp = self.auth(self.http.get(&url)).send().await.ok()?;
        if resp.status() != 200 {
            return None;
        }
        resp.json::<BaseConfig>().await.ok()
    }

    /// O README do repositório, sem o cabeçalho YAML e já em Markdown puro.
    ///
    /// É o texto que o autor escreveu sobre o modelo — na tela de descoberta,
    /// a única fonte de "o que é isto" que não seja o nome do arquivo. Vem do
    /// `raw`, não da API: o cartão inteiro renderizado seria HTML, e aqui o
    /// que se quer é o Markdown. O que vem do `raw` ainda é Markdown MISTURADO
    /// com HTML — `card_em_markdown` resolve essa parte (ver `card.rs`).
    pub async fn readme(&self, repo_id: &str) -> Result<String, ModelsError> {
        let url = format!("{HF_BASE}/{repo_id}/raw/main/README.md");
        let resp = self.auth(self.http.get(&url)).send().await?;
        match resp.status().as_u16() {
            200 => {}
            401 | 403 => return Err(ModelsError::Gated),
            404 => return Ok(String::new()),
            s => return Err(ModelsError::Api(format!("README retornou HTTP {s}"))),
        }
        let texto = resp.text().await?;
        // Corta antes de traduzir: o custo da tradução é o do que vai à tela,
        // não o do cartão de 400 KB que alguns autores publicam.
        let cortado: String = sem_frontmatter(&texto)
            .chars()
            .take(MAX_README_CHARS)
            .collect();
        Ok(card_em_markdown(&cortado))
    }

    /// Lista os arquivos (com tamanhos) de um repositório.
    pub async fn repo_files(&self, repo_id: &str) -> Result<Vec<RepoFile>, ModelsError> {
        let url = format!("{HF_BASE}/api/models/{repo_id}/tree/main?recursive=true");
        let resp = self.auth(self.http.get(&url)).send().await?;
        match resp.status().as_u16() {
            200 => {}
            401 | 403 => return Err(ModelsError::Gated),
            s => return Err(ModelsError::Api(format!("tree retornou HTTP {s}"))),
        }
        let raw: Vec<ApiTreeEntry> = resp.json().await?;
        Ok(raw
            .into_iter()
            .filter(|e| e.kind == "file")
            .map(|e| RepoFile {
                size_bytes: e.lfs.as_ref().map(|l| l.size).unwrap_or(e.size),
                path: e.path,
            })
            .collect())
    }

    /// A foto de perfil do autor no Hub — `None` quando ele não tem uma.
    ///
    /// Não há caminho previsível para o avatar: `huggingface.co/{autor}.png`
    /// é convenção do GitHub e aqui responde 404 para todo mundo. Quem sabe a
    /// URL é o `overview` do perfil, e o perfil mora em uma de duas rotas
    /// conforme o autor seja uma pessoa ou uma organização — daí as duas
    /// tentativas, e daí o cache de quem chama: cada nome custa uma ida à
    /// rede que não muda de resposta durante a sessão.
    ///
    /// Quem nunca subiu foto recebe do Hub um identicon gerado
    /// (`/avatars/<hash>.svg`), o mesmo losango colorido para milhares de
    /// perfis. Esse caso volta `None` de propósito: as iniciais do nome
    /// distinguem mais do que ele.
    pub async fn author_avatar(&self, author: &str) -> Option<String> {
        if !nome_de_perfil(author) {
            return None;
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Overview {
            #[serde(default)]
            avatar_url: Option<String>,
        }
        for rota in ["users", "organizations"] {
            let url = format!("{HF_BASE}/api/{rota}/{author}/overview");
            let Ok(resp) = self.auth(self.http.get(&url)).send().await else {
                continue;
            };
            if resp.status() != 200 {
                continue;
            }
            // O perfil existe: qualquer que seja a resposta daqui, procurar na
            // outra rota é gastar uma requisição para receber 404.
            let Ok(ov) = resp.json::<Overview>().await else {
                return None;
            };
            return ov.avatar_url.as_deref().and_then(avatar_absoluto);
        }
        None
    }
}

// ------------------------------------------------------------- acesso ---

/// O que o Hub responde quando ESTA conta pede um arquivo do repositório.
///
/// Não confundir com `ModelSummary::gated`: aquele campo é propriedade do
/// REPOSITÓRIO ("exige aceite de licença") e continua verdadeiro para sempre,
/// inclusive depois que a pessoa aceitou. Ele nunca ia responder à pergunta
/// que a tela precisa fazer — *eu já posso baixar?* —, e por isso o aviso
/// amarelo ficava na frente de quem já tinha acesso.
///
/// Isto aqui é sobre a CONTA, e é o que muda quando o aceite acontece no
/// navegador: o Hub grava a permissão no perfil de quem aceitou, e o app só
/// consegue se apresentar como esse perfil pelo token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HfAccess {
    /// O arquivo desce. Nada a avisar.
    Granted,
    /// Restrito e sem token: o app não tem como dizer quem é.
    NoToken,
    /// Há token, mas esta conta não aceitou a licença — ou o token é
    /// fine-grained sem alcance sobre repositórios com licença.
    NeedsLicense,
    /// O Hub não reconhece o token gravado (expirado ou revogado).
    BadToken,
}

/// Quem é o dono do token, do ponto de vista do Hub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HfIdentity {
    pub name: String,
    /// `read`, `write` ou `fineGrained`, quando o Hub informa.
    pub role: Option<String>,
    /// `Some(false)` quando o token é fine-grained e NÃO alcança o conteúdo
    /// de repositórios públicos com licença — a armadilha que faz o download
    /// responder 403 mesmo com a licença já aceita, e que nenhuma mensagem
    /// de erro do Hub explica. `None` quando a resposta não deixa concluir:
    /// a interface só avisa no `Some(false)`, porque um alarme falso aqui
    /// manda a pessoa refazer um token que estava certo.
    pub can_read_gated: Option<bool>,
}

/// O resultado de perguntar ao Hub "de quem é este token?".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum HfWhoami {
    /// Não há token gravado.
    NoToken,
    /// Há token e o Hub o recusou.
    Invalid,
    Ok(HfIdentity),
}

impl HfClient {
    /// Se esta conta consegue baixar deste repositório — perguntando ao
    /// endpoint que de fato aplica o portão.
    ///
    /// O portão do Hub não está nos metadados: `/api/models/{id}` e a árvore
    /// de arquivos respondem 200 para repositório restrito também (a página é
    /// pública; o que é restrito são os BYTES). Quem responde a verdade é o
    /// `/resolve/`, e um HEAD nele custa um cabeçalho: o 302 para a CDN já é
    /// o "pode baixar", sem começar download nenhum.
    ///
    /// `file` é o caminho a testar dentro do repositório. O padrão é
    /// `.gitattributes` porque o Hub cria esse arquivo em todo repositório —
    /// dá para sondar sem antes listar a árvore. Quem já tem o nome do
    /// arquivo da quantização deve passá-lo: é o teste do arquivo que a
    /// pessoa vai baixar de verdade.
    pub async fn access(&self, repo_id: &str, file: Option<&str>) -> Result<HfAccess, ModelsError> {
        let file = file.unwrap_or(".gitattributes");
        let url = format!("{HF_BASE}/{repo_id}/resolve/main/{file}");
        let resp = self.auth(self.http_noredir.head(&url)).send().await?;
        let status = resp.status().as_u16();
        classificar_acesso(status, self.token.is_some())
            .ok_or_else(|| ModelsError::Api(format!("resolve retornou HTTP {status}")))
    }

    /// De quem é o token gravado, e se ele alcança repositórios com licença.
    pub async fn whoami(&self) -> Result<HfWhoami, ModelsError> {
        if self.token.is_none() {
            return Ok(HfWhoami::NoToken);
        }
        let url = format!("{HF_BASE}/api/whoami-v2");
        let resp = self.auth(self.http.get(&url)).send().await?;
        match resp.status().as_u16() {
            200 => {}
            401 | 403 => return Ok(HfWhoami::Invalid),
            s => return Err(ModelsError::Api(format!("whoami retornou HTTP {s}"))),
        }
        let raw: serde_json::Value = resp.json().await?;
        Ok(HfWhoami::Ok(identidade(&raw)))
    }
}

/// Traduz a resposta do `/resolve/` em veredito. `None` = resposta que não
/// diz nada sobre acesso (e vira erro de API para quem chamou).
///
/// Qual código vem para quem não pode baixar depende de haver credencial: o
/// Hub responde 401 a quem não se identificou e 403 a quem se identificou e
/// não está na lista. É essa diferença que separa "falta o token" de "a
/// licença não foi aceita por esta conta" — as duas coisas que o aviso antigo
/// dizia com a mesma frase.
fn classificar_acesso(status: u16, tem_token: bool) -> Option<HfAccess> {
    Some(match status {
        // 302 é a resposta normal do arquivo LFS liberado (o redirect para a
        // CDN); 200 vem dos arquivos pequenos, servidos direto. 404 é o
        // portão aberto para um caminho que não existe — quem não passa pelo
        // portão recebe 401/403 antes de o Hub olhar o caminho.
        200..=399 | 404 => HfAccess::Granted,
        401 | 403 if !tem_token => HfAccess::NoToken,
        401 => HfAccess::BadToken,
        403 => HfAccess::NeedsLicense,
        _ => return None,
    })
}

/// Lê o `whoami-v2` sem modelar a árvore inteira.
///
/// O bloco `fineGrained` é o que mais muda de forma entre versões do Hub, e
/// aqui só interessa uma resposta de três valores. Modelar o resto seria
/// trocar um `None` honesto por um erro de desserialização na primeira vez
/// que o Hub acrescentar um campo.
fn identidade(raw: &serde_json::Value) -> HfIdentity {
    let name = raw
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let tok = raw.get("auth").and_then(|a| a.get("accessToken"));
    let role = tok
        .and_then(|t| t.get("role"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let can_read_gated = match role.as_deref() {
        // Tokens clássicos leem tudo o que a conta pode ler.
        Some("read") | Some("write") => Some(true),
        Some("fineGrained") => {
            let fg = tok.and_then(|t| t.get("fineGrained"));
            fg.and_then(|f| f.get("canReadGatedRepos"))
                .and_then(|v| v.as_bool())
                .or_else(|| fg.map(|f| permissoes(f).any(|p| p == "repo.content.read")))
        }
        _ => None,
    };
    HfIdentity {
        name,
        role,
        can_read_gated,
    }
}

/// Toda permissão citada no bloco `fineGrained`, global ou por entidade.
fn permissoes(fg: &serde_json::Value) -> impl Iterator<Item = &str> {
    let global = fg.get("global").and_then(|v| v.as_array());
    let scoped = fg.get("scoped").and_then(|v| v.as_array());
    let diretas = global.into_iter().flatten();
    let por_entidade = scoped
        .into_iter()
        .flatten()
        .filter_map(|e| e.get("permissions"))
        .filter_map(|p| p.as_array())
        .flatten();
    diretas.chain(por_entidade).filter_map(|v| v.as_str())
}

/// Nome de perfil do Hub: letras, dígitos, `-`, `_` e `.`.
///
/// O nome entra em uma URL sem escape, então o que não couber aqui não vira
/// requisição — nem para descobrir que não existe.
fn nome_de_perfil(nome: &str) -> bool {
    !nome.is_empty()
        && nome.len() <= 96
        && nome
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// A URL do avatar, absoluta — e `None` para o identicon gerado.
fn avatar_absoluto(url: &str) -> Option<String> {
    if url.is_empty() || url.starts_with("/avatars/") {
        return None;
    }
    if url.starts_with("https://") || url.starts_with("http://") {
        return Some(url.to_string());
    }
    url.starts_with('/').then(|| format!("{HF_BASE}{url}"))
}

fn to_summary(m: ApiModel) -> ModelSummary {
    let (author, name) =
        m.id.split_once('/')
            .map(|(a, n)| (a.to_string(), n.to_string()))
            .unwrap_or_else(|| (String::new(), m.id.clone()));
    ModelSummary {
        gated: match &m.gated {
            serde_json::Value::Bool(b) => *b,
            serde_json::Value::String(_) => true,
            _ => false,
        },
        id: m.id,
        author,
        name,
        downloads: m.downloads,
        likes: m.likes,
        params_total: m.gguf.as_ref().and_then(|g| g.total),
        architecture: m.gguf.as_ref().and_then(|g| g.architecture.clone()),
        context_length: m.gguf.as_ref().and_then(|g| g.context_length),
        updated_at: m.last_modified,
        license: m
            .card_data
            .as_ref()
            .and_then(|c| c.license_name.clone().or_else(|| c.license.clone())),
        caps: capacidades(
            m.pipeline_tag.as_deref(),
            &m.tags,
            m.gguf.as_ref().and_then(|g| g.chat_template.as_deref()),
        ),
    }
}

/// Deriva as capacidades do que o Hub entrega.
///
/// Nenhuma delas é adivinhada pelo NOME do modelo, que erra nos dois
/// sentidos. Visão é o `pipeline_tag` (a etiqueta que o próprio autor
/// escolheu); ferramentas e raciocínio saem do chat template, que é o
/// arquivo que o llama.cpp de fato executa — se ele tem o ramo, a
/// capacidade existe.
fn capacidades(pipeline: Option<&str>, tags: &[String], chat_template: Option<&str>) -> ModelCaps {
    let visao = |s: &str| s == "image-text-to-text" || s == "visual-question-answering";
    let tpl = chat_template.unwrap_or_default();
    ModelCaps {
        vision: pipeline.is_some_and(visao) || tags.iter().any(|t| visao(t)),
        tools: tpl.contains("tools"),
        reasoning: tpl.contains("enable_thinking"),
    }
}

/// Extrai a URL `rel="next"` de um header `Link` (subset da RFC 8288:
/// entradas separadas por vírgula, parâmetros por ponto e vírgula).
fn parse_link_next(link: &str) -> Option<String> {
    for entry in link.split(',') {
        let mut parts = entry.split(';');
        let url_part = parts.next()?.trim();
        if !(url_part.starts_with('<') && url_part.ends_with('>')) {
            continue;
        }
        let is_next = parts.any(|p| {
            let p = p.trim();
            p.eq_ignore_ascii_case("rel=\"next\"") || p.eq_ignore_ascii_case("rel=next")
        });
        if is_next {
            return Some(url_part[1..url_part.len() - 1].to_string());
        }
    }
    None
}

/// Teto do README trazido para a tela. Cartões de modelo chegam a centenas
/// de KB de tabelas de benchmark; o que interessa está no começo, e o resto
/// só custa memória e tempo de renderização.
const MAX_README_CHARS: usize = 60_000;

/// Remove o bloco YAML de metadados do topo do README.
///
/// O cartão do Hub começa com `---` … `---` (licença, tags, modelo base) —
/// informação que a interface já mostra em campos próprios, e que como texto
/// solto abriria a descrição com uma parede de chaves e traços.
fn sem_frontmatter(texto: &str) -> &str {
    let t = texto.trim_start_matches('\u{feff}');
    let Some(resto) = t.strip_prefix("---") else {
        return t;
    };
    // A abertura precisa ser a linha inteira `---`; um `---abc` não é
    // frontmatter, é texto.
    let resto = match resto
        .strip_prefix('\n')
        .or_else(|| resto.strip_prefix("\r\n"))
    {
        Some(r) => r,
        None => return t,
    };
    for (i, linha) in resto.match_indices('\n') {
        let anterior = &resto[..i];
        let fim = anterior.rsplit('\n').next().unwrap_or(anterior).trim_end();
        if fim == "---" || fim == "..." {
            return resto[i + 1..].trim_start();
        }
        let _ = linha;
    }
    // Abriu e não fechou: o documento inteiro seria comido pelo corte, então
    // devolvê-lo como está é o comportamento menos destrutivo.
    t
}

fn urlencode(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => vec![c],
            ' ' => vec!['+'],
            other => format!("%{:02X}", other as u32).chars().collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uma resposta REAL da busca, recortada: se o Hub mudar o nome de um
    /// campo, é aqui que se descobre — e não numa tela que passou a mostrar
    /// "sem licença" e nenhum selo para todo mundo.
    #[test]
    fn a_real_search_row_fills_every_field_the_screen_shows() {
        let json = r#"[{
            "id": "unsloth/Qwen3.8-27B-GGUF",
            "author": "unsloth",
            "downloads": 8839153,
            "likes": 3237,
            "gated": false,
            "lastModified": "2026-08-20T12:04:25.000Z",
            "pipeline_tag": "image-text-to-text",
            "tags": ["gguf", "conversational"],
            "cardData": { "license": "apache-2.0", "base_model": ["Qwen/Qwen3.8-27B"] },
            "gguf": {
                "total": 27320697856,
                "architecture": "qwen35",
                "context_length": 262144,
                "chat_template": "{%- if tools %}...{%- endif %}{%- if enable_thinking %}<think>{%- endif %}"
            }
        }]"#;
        let raw: Vec<ApiModel> = serde_json::from_str(json).expect("desserializa");
        let m = to_summary(raw.into_iter().next().unwrap());

        assert_eq!(m.author, "unsloth");
        assert_eq!(m.name, "Qwen3.8-27B-GGUF");
        assert_eq!(m.downloads, 8_839_153);
        assert_eq!(m.params_total, Some(27_320_697_856));
        assert_eq!(m.architecture.as_deref(), Some("qwen35"));
        assert_eq!(m.context_length, Some(262_144));
        assert_eq!(m.license.as_deref(), Some("apache-2.0"));
        assert!(!m.gated);
        assert_eq!(
            m.caps,
            ModelCaps {
                vision: true,
                tools: true,
                reasoning: true
            }
        );
    }

    /// `license_name` vence `license` quando existe: é o nome específico da
    /// licença própria de um autor ("qwen-community-1.0"), e "other" não diz
    /// nada a ninguém.
    #[test]
    fn a_named_license_wins_over_the_generic_one() {
        let json = r#"[{
            "id": "a/b",
            "cardData": { "license": "other", "license_name": "qwen-community-1.0" }
        }]"#;
        let raw: Vec<ApiModel> = serde_json::from_str(json).expect("desserializa");
        let m = to_summary(raw.into_iter().next().unwrap());
        assert_eq!(m.license.as_deref(), Some("qwen-community-1.0"));
    }

    /// O cartão do Hub começa com um bloco YAML que a tela já mostra em
    /// campos próprios — como texto, ele só empurraria a descrição para
    /// baixo atrás de uma parede de chaves.
    #[test]
    fn the_card_header_is_stripped_from_the_readme() {
        let com = "---\nlicense: apache-2.0\ntags:\n- unsloth\n---\n\n# Qwen3.8\n\nUm modelo.";
        assert_eq!(sem_frontmatter(com), "# Qwen3.8\n\nUm modelo.");

        // Sem cabeçalho, o texto passa inteiro.
        let sem = "# Qwen3.8\n\nUm modelo.";
        assert_eq!(sem_frontmatter(sem), sem);

        // `---` no meio de uma linha não abre bloco nenhum: é texto.
        let falso = "---abc\n# Título";
        assert_eq!(sem_frontmatter(falso), falso);

        // Abriu e nunca fechou: devolver tudo é menos destrutivo que comer o
        // documento inteiro.
        let aberto = "---\nlicense: mit\n\n# Título sem fim";
        assert_eq!(sem_frontmatter(aberto), aberto);

        // Um fechamento com `...` também é YAML válido.
        let pontos = "---\nlicense: mit\n...\n# Título";
        assert_eq!(sem_frontmatter(pontos), "# Título");
    }

    /// Visão vem da etiqueta que o autor escolheu; ferramentas e raciocínio,
    /// do template que o llama.cpp executa. Nada sai do nome do modelo.
    #[test]
    fn capabilities_come_from_the_hub_not_from_the_name() {
        let tpl_pensante = "{%- if enable_thinking %}<think>{%- endif %}{%- if tools %}...";
        let c = capacidades(
            Some("image-text-to-text"),
            &["gguf".to_string()],
            Some(tpl_pensante),
        );
        assert!(c.vision && c.tools && c.reasoning);

        // Um nome cheio de promessas não vale nada sem as fontes.
        let c = capacidades(
            Some("text-generation"),
            &["gguf".to_string()],
            Some("{{ bos_token }}"),
        );
        assert_eq!(c, ModelCaps::default());

        // Sem template, o que sobra é a etiqueta.
        let c = capacidades(None, &["visual-question-answering".to_string()], None);
        assert!(c.vision);
        assert!(!c.tools && !c.reasoning);
    }

    #[test]
    fn avatar_do_hub_vira_url_absoluta() {
        assert_eq!(
            avatar_absoluto("https://cdn-avatars.huggingface.co/v1/x.png").as_deref(),
            Some("https://cdn-avatars.huggingface.co/v1/x.png")
        );
        assert_eq!(
            avatar_absoluto("/foto.png").as_deref(),
            Some("https://huggingface.co/foto.png")
        );
        // identicon gerado: melhor as iniciais do nome
        assert_eq!(avatar_absoluto("/avatars/69e3.svg"), None);
        assert_eq!(avatar_absoluto(""), None);
    }

    #[test]
    fn nome_de_perfil_recusa_o_que_nao_e_nome() {
        assert!(nome_de_perfil("unsloth"));
        assert!(nome_de_perfil("huihui-ai"));
        assert!(nome_de_perfil("TheBloke_2.0"));
        assert!(!nome_de_perfil(""));
        assert!(!nome_de_perfil("um/dois"));
        assert!(!nome_de_perfil("../etc"));
        assert!(!nome_de_perfil("nome com espaço"));
    }

    #[test]
    fn urlencode_basics() {
        assert_eq!(urlencode("qwen 8b"), "qwen+8b");
        assert_eq!(urlencode("c++"), "c%2B%2B");
    }

    #[test]
    fn parses_link_next_header() {
        // formato real da API do Hub: uma entrada só, rel="next"
        let l = "<https://huggingface.co/api/models?filter=gguf&cursor=eyJfaWQiOnt9fQ%3D%3D>; rel=\"next\"";
        assert_eq!(
            parse_link_next(l).as_deref(),
            Some("https://huggingface.co/api/models?filter=gguf&cursor=eyJfaWQiOnt9fQ%3D%3D")
        );
        // múltiplas entradas: escolhe a rel="next"
        let multi = "<https://x/prev>; rel=\"prev\", <https://x/next>; rel=\"next\"";
        assert_eq!(parse_link_next(multi).as_deref(), Some("https://x/next"));
        // rel sem aspas também é aceito
        assert_eq!(
            parse_link_next("<https://x/n>; rel=next").as_deref(),
            Some("https://x/n")
        );
        // sem rel="next" → None
        assert_eq!(parse_link_next("<https://x/prev>; rel=\"prev\""), None);
        assert_eq!(parse_link_next(""), None);
    }

    #[test]
    fn gated_value_variants() {
        for (v, expected) in [
            (serde_json::json!(false), false),
            (serde_json::json!(true), true),
            (serde_json::json!("manual"), true),
            (serde_json::json!("auto"), true),
        ] {
            let m = ApiModel {
                id: "a/b".into(),
                downloads: 0,
                likes: 0,
                gated: v,
                last_modified: None,
                gguf: None,
                pipeline_tag: None,
                tags: Vec::new(),
                card_data: None,
            };
            assert_eq!(to_summary(m).gated, expected);
        }
    }

    /// Teste live (rede): metadados GGUF + paginação por cursor.
    /// O que o `/resolve/` responde vira qual veredito — inclusive a
    /// diferença que o aviso antigo não fazia: sem token e com token são
    /// problemas diferentes com o mesmo código HTTP.
    #[test]
    fn resolve_status_becomes_the_right_verdict() {
        // Liberado: 302 (LFS), 200 (arquivo pequeno) e 404 (passou pelo
        // portão, o caminho é que não existe).
        for s in [200, 302, 404] {
            assert_eq!(classificar_acesso(s, false), Some(HfAccess::Granted));
            assert_eq!(classificar_acesso(s, true), Some(HfAccess::Granted));
        }
        // Sem token, negar é sempre a mesma história: falta dizer quem é.
        assert_eq!(classificar_acesso(401, false), Some(HfAccess::NoToken));
        assert_eq!(classificar_acesso(403, false), Some(HfAccess::NoToken));
        // Com token, o Hub separa "não te reconheço" de "você não está na
        // lista" — e é a segunda que o aceite no navegador resolve.
        assert_eq!(classificar_acesso(401, true), Some(HfAccess::BadToken));
        assert_eq!(classificar_acesso(403, true), Some(HfAccess::NeedsLicense));
        // Qualquer outra coisa não é veredito de acesso: vira erro.
        assert_eq!(classificar_acesso(500, true), None);
    }

    /// O token clássico lê tudo o que a conta lê; o fine-grained só lê o que
    /// foi marcado — e é ele que dá 403 depois da licença aceita.
    #[test]
    fn whoami_says_whether_the_token_reaches_gated_repos() {
        let classico = serde_json::json!({
            "name": "alguem",
            "auth": { "accessToken": { "role": "read" } }
        });
        let id = identidade(&classico);
        assert_eq!(id.name, "alguem");
        assert_eq!(id.can_read_gated, Some(true));

        // Fine-grained que o Hub responde direto.
        let direto = serde_json::json!({
            "name": "alguem",
            "auth": { "accessToken": { "role": "fineGrained",
                "fineGrained": { "canReadGatedRepos": false } } }
        });
        assert_eq!(identidade(&direto).can_read_gated, Some(false));

        // Sem o campo direto, a permissão aparece na lista — global ou por
        // entidade, as duas formas que o Hub usa.
        let global = serde_json::json!({
            "name": "alguem",
            "auth": { "accessToken": { "role": "fineGrained",
                "fineGrained": { "global": ["repo.content.read"], "scoped": [] } } }
        });
        assert_eq!(identidade(&global).can_read_gated, Some(true));

        let escopado = serde_json::json!({
            "name": "alguem",
            "auth": { "accessToken": { "role": "fineGrained", "fineGrained": {
                "global": [],
                "scoped": [{ "entity": { "type": "user", "name": "alguem" },
                             "permissions": ["repo.write"] }] } } }
        });
        assert_eq!(identidade(&escopado).can_read_gated, Some(false));

        // Sem saber o tipo do token, a resposta honesta é "não sei" — a tela
        // só avisa quando a permissão comprovadamente falta.
        let mudo = serde_json::json!({ "name": "alguem" });
        assert_eq!(identidade(&mudo).can_read_gated, None);
        assert_eq!(identidade(&mudo).role, None);
    }

    /// O portão não está nos metadados — está no `/resolve/`.
    ///
    /// Verificado ao vivo: `/api/models/{id}` responde 200 para repositório
    /// restrito também (a PÁGINA é pública; restritos são os BYTES). Quem
    /// tentasse ler acesso dali concluiria "liberado" para todo mundo.
    #[tokio::test]
    #[ignore = "acessa a rede (Hugging Face)"]
    async fn the_gate_answers_at_resolve_not_at_the_metadata() {
        let c = HfClient::new(None);
        assert_eq!(
            c.access("unsloth/Qwen3-0.6B-GGUF", None).await.unwrap(),
            HfAccess::Granted
        );
        assert_eq!(
            c.access("meta-llama/Llama-3.2-1B", None).await.unwrap(),
            HfAccess::NoToken
        );
    }

    #[tokio::test]
    #[ignore = "acessa a rede (Hugging Face)"]
    async fn live_gguf_meta_and_search_cursor() {
        let c = HfClient::new(None);

        let meta = c.gguf_meta("Qwen/Qwen3-0.6B-GGUF").await.unwrap();
        let meta = meta.expect("repo GGUF deveria expor o bloco gguf");
        assert!(meta.params_total.unwrap_or(0) > 100_000_000);
        assert!(meta.architecture.is_some());

        let (page1, next) = c
            .search_cursor("qwen", SortBy::Downloads, 5, None)
            .await
            .unwrap();
        assert_eq!(page1.len(), 5);
        let next = next.expect("deveria haver próxima página");
        let (page2, _) = c
            .search_cursor("", SortBy::Downloads, 5, Some(&next))
            .await
            .unwrap();
        assert!(!page2.is_empty());
        assert_ne!(page1[0].id, page2[0].id);
    }
}
