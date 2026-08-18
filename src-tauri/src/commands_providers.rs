//! Comandos das fontes de LLM além do llama.cpp local.
//!
//! O que passa por aqui é configuração e catálogo — nunca o streaming da
//! conversa. O chat continua falando direto com o endpoint a partir do
//! webview (ver `src/lib/llama.ts`), inclusive com os provedores remotos: um
//! segundo caminho de streaming pelo IPC teria backpressure e cancelamento
//! próprios para não ganhar nada.

use crate::state::AppState;
use serde::Serialize;
use std::time::{Duration, Instant};
use tauri::State;

use lr_providers::{
    KeyInfo, ModelRef, OpenRouterModel, ProviderId, ProvidersConfig, ResolvedEndpoint,
};

type CmdResult<T> = Result<T, String>;

fn err_str<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Chave do setting onde a configuração dos provedores mora.
pub const SETTING: &str = "providers.config";

/// Por quanto tempo o catálogo do OpenRouter é reaproveitado.
///
/// São 400+ modelos e a lista muda de horas em horas — rebuscar a cada
/// abertura da tela seria desperdício sem ganho.
const CACHE_TTL: Duration = Duration::from_secs(10 * 60);

/// Como um provedor aparece na tela.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderView {
    pub id: &'static str,
    /// Pronto para receber uma conversa agora.
    pub ready: bool,
    /// Motivo de não estar pronto, já em texto exibível.
    pub reason: Option<String>,
    pub base_url: Option<String>,
}

// --------------------------------------------------------------- config ---

pub(crate) fn load_config(state: &AppState) -> ProvidersConfig {
    let raw = state.store.get_setting(SETTING).ok().flatten();
    ProvidersConfig::from_json_or_default(raw.as_deref())
}

/// Devolve o JSON cru, como o `web_config_get`: quem interpreta é a tela, e
/// traduzir aqui só criaria um segundo dialeto para manter em dia.
#[tauri::command]
pub fn providers_config_get(state: State<'_, AppState>) -> CmdResult<Option<String>> {
    state.store.get_setting(SETTING).map_err(err_str)
}

/// Valida ANTES de gravar.
///
/// O leitor cai nos padrões em silêncio quando o JSON está estragado, então
/// aceitar qualquer texto aqui gravaria uma configuração que nunca teria
/// efeito — e a pessoa acharia que salvou.
#[tauri::command]
pub fn providers_config_set(state: State<'_, AppState>, json: String) -> CmdResult<()> {
    serde_json::from_str::<ProvidersConfig>(&json)
        .map_err(|e| format!("configuração de provedores inválida: {e}"))?;
    state.store.set_setting(SETTING, &json).map_err(err_str)
}

// ---------------------------------------------------------------- estado ---

/// Estado dos três provedores, para a tela desenhar sem adivinhar.
#[tauri::command]
pub async fn providers_list(state: State<'_, AppState>) -> CmdResult<Vec<ProviderView>> {
    let cfg = load_config(&state);
    let local_base = {
        let guard = state.server.lock().await;
        guard
            .as_ref()
            .filter(|s| s.is_spawned())
            .map(|s| s.config().connect_url())
    };

    Ok([
        ProviderId::Local,
        ProviderId::NineRouter,
        ProviderId::OpenRouter,
    ]
    .into_iter()
    .map(|id| match cfg.resolve(id, local_base.as_deref()) {
        Ok(ep) => ProviderView {
            id: id.as_str(),
            ready: true,
            reason: None,
            base_url: Some(ep.base_url),
        },
        Err(e) => ProviderView {
            id: id.as_str(),
            ready: false,
            reason: Some(e.to_string()),
            base_url: None,
        },
    })
    .collect())
}

/// Para onde o webview deve mandar esta conversa.
///
/// Devolve a chave de API junto: o `fetch` do chat acontece no renderer, que
/// é o mesmo processo que já exibe a chave na tela de provedores. Roteá-la
/// pelo IPC não a esconderia de ninguém.
#[tauri::command]
pub async fn provider_endpoint(
    state: State<'_, AppState>,
    model_ref: String,
) -> CmdResult<ResolvedEndpoint> {
    let referencia = ModelRef::parse(&model_ref);
    let mut cfg = load_config(&state);

    // Rede de segurança para o 9router: a chave normalmente vem do
    // `ninerouter_start`, mas uma configuração gravada antes desta
    // funcionalidade existir não tem nenhuma — e a conversa morreria com 401
    // se a pessoa tivesse ligado "Require API key". Só corre quando falta.
    if referencia.provider == ProviderId::NineRouter
        && cfg.nine_router.api_key.trim().is_empty()
        && state.ninerouter.lock().await.is_some()
    {
        let l = layout(&state);
        let porta = cfg.nine_router.port;
        match lr_ninerouter::garantir_api_key(&l, porta).await {
            Ok(chave) => {
                cfg.nine_router.api_key = chave;
                let _ = state.store.set_setting(SETTING, &cfg.to_json());
            }
            Err(e) => log::warn!("9router sem chave de API utilizável: {e}"),
        }
    }

    let local_base = {
        let guard = state.server.lock().await;
        guard
            .as_ref()
            .filter(|s| s.is_spawned())
            .map(|s| s.config().connect_url())
    };
    cfg.resolve(referencia.provider, local_base.as_deref())
        .map_err(err_str)
}

// ------------------------------------------------------------ openrouter ---

/// Catálogo do OpenRouter, com cache de 10 minutos.
///
/// Não exige chave: dá para ver os modelos e os preços antes de decidir se
/// vale criar conta.
#[tauri::command]
pub async fn openrouter_models(state: State<'_, AppState>) -> CmdResult<Vec<OpenRouterModel>> {
    {
        let cache = state.openrouter_cache.lock().await;
        if let Some((gravado_em, modelos)) = cache.as_ref()
            && gravado_em.elapsed() < CACHE_TTL
        {
            return Ok(modelos.clone());
        }
    }

    let modelos = lr_providers::openrouter::list_models()
        .await
        .map_err(err_str)?;
    *state.openrouter_cache.lock().await = Some((Instant::now(), modelos.clone()));
    Ok(modelos)
}

/// Saldo e limite da chave gravada. É também o "testar conexão" da tela.
#[tauri::command]
pub async fn openrouter_key_info(state: State<'_, AppState>) -> CmdResult<KeyInfo> {
    let cfg = load_config(&state);
    let chave = cfg.open_router.api_key.trim().to_string();
    if chave.is_empty() {
        return Err("falta a chave de API do OpenRouter".to_string());
    }
    lr_providers::openrouter::key_info(&chave)
        .await
        .map_err(err_str)
}

// -------------------------------------------------------------- 9router ---

use lr_ninerouter::{Layout, NineRouterEvent, RunConfig};
use tauri::{AppHandle, Emitter, Manager};

/// Nome do evento por onde saem progresso e log dos provedores gerenciados.
const EVENTO: &str = "provider";

/// Rótulo da janela do painel do 9router.
///
/// Ela NÃO está na capability `default` (presa à `main`), e isso é
/// intencional: carrega conteúdo remoto e não tem por que enxergar IPC.
const JANELA_PAINEL: &str = "ninerouter-panel";

/// Situação do 9router para a tela desenhar.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NineRouterStatus {
    pub node_installed: bool,
    pub installed: bool,
    pub running: bool,
    pub port: u16,
    /// URL do painel, quando está no ar — é o que o iframe carrega.
    pub dashboard_url: Option<String>,
    /// Senha do primeiro acesso. Fica visível de propósito: o painel roda em
    /// outra origem e o app não tem como preencher o formulário por ela.
    pub password: String,
    pub version: String,
}

fn layout(state: &AppState) -> Layout {
    Layout::new(&state.data_dir.join("providers"))
}

#[tauri::command]
pub async fn ninerouter_status(state: State<'_, AppState>) -> CmdResult<NineRouterStatus> {
    let cfg = load_config(&state);
    let l = layout(&state);
    let running = state.ninerouter.lock().await.is_some();
    let port = cfg.nine_router.port;
    Ok(NineRouterStatus {
        node_installed: state.node.state().installed,
        installed: l.instalado(),
        running,
        port,
        dashboard_url: running.then(|| format!("http://127.0.0.1:{port}/dashboard")),
        password: cfg.nine_router.password.clone(),
        version: cfg.nine_router.version.clone(),
    })
}

/// Baixa o Node portátil e instala o 9router na pasta isolada.
///
/// As duas etapas emitem no mesmo evento: para quem olha a tela é uma
/// instalação só, e separá-las em dois canais só exigiria que a UI as
/// costurasse de novo.
#[tauri::command]
pub async fn ninerouter_install(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<NineRouterStatus> {
    let l = layout(&state);

    // 1. Node portátil (progresso real: o download tem content-length).
    let app_node = app.clone();
    state
        .node
        .ensure(move |ev| {
            let _ = app_node.emit(EVENTO, &ev);
        })
        .await
        .map_err(err_str)?;

    // 2. Pacote npm. Aqui o progresso é honesto: fase nomeada e log ao vivo,
    // porque o npm não publica porcentagem e inventar uma seria mentir.
    let app_npm = app.clone();
    let emitir = move |ev: NineRouterEvent| {
        let _ = app_npm.emit(EVENTO, &ev);
    };
    lr_ninerouter::instalar(&state.node, &l, &emitir)
        .await
        .map_err(err_str)?;

    // 3. Segredos e porta, gravados na configuração.
    let mut cfg = load_config(&state);
    if cfg.nine_router.jwt_secret.is_empty() {
        cfg.nine_router.jwt_secret = lr_ninerouter::gerar_jwt_secret();
    }
    if cfg.nine_router.password.is_empty() {
        cfg.nine_router.password = lr_ninerouter::gerar_senha();
    }
    cfg.nine_router.installed = true;
    cfg.nine_router.version = lr_ninerouter::PINNED_9ROUTER.to_string();
    state
        .store
        .set_setting(SETTING, &cfg.to_json())
        .map_err(err_str)?;

    let _ = app.emit(EVENTO, &NineRouterEvent::Ready);
    ninerouter_status(state).await
}

#[tauri::command]
pub async fn ninerouter_start(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<NineRouterStatus> {
    let l = layout(&state);
    let mut cfg = load_config(&state);

    // Spawn sob o lock; a espera de readiness fica FORA dele, para não travar
    // o status e o stop por até um minuto — mesmo desenho do `start_engine`.
    {
        let mut guard = state.ninerouter.lock().await;
        if guard.is_some() {
            drop(guard);
            return ninerouter_status(state).await;
        }

        // A porta padrão é preferida; caímos numa efêmera só se alguém já
        // estiver escutando nela (o caso real: instância própria da pessoa).
        let porta = lr_proc::free_port(cfg.nine_router.port);
        if porta != cfg.nine_router.port {
            log::warn!(
                "porta {} ocupada; 9router vai subir em {porta}",
                cfg.nine_router.port
            );
            cfg.nine_router.port = porta;
            state
                .store
                .set_setting(SETTING, &cfg.to_json())
                .map_err(err_str)?;
        }

        let run = RunConfig {
            port: porta,
            password: cfg.nine_router.password.clone(),
            jwt_secret: cfg.nine_router.jwt_secret.clone(),
        };
        let mut nr = lr_ninerouter::NineRouter::spawn(&state.node, &l, &run).map_err(err_str)?;
        if let Some(pid) = nr.pid() {
            state
                .ninerouter_pid
                .store(pid, std::sync::atomic::Ordering::SeqCst);
        }

        // Log do processo → mesmo evento da instalação, para a tela mostrar
        // tudo numa janela só.
        let (stdout, stderr) = nr.take_output();
        if let Some(saida) = stdout {
            let app2 = app.clone();
            tauri::async_runtime::spawn(async move {
                use tokio::io::AsyncBufReadExt as _;
                let mut linhas = tokio::io::BufReader::new(saida).lines();
                while let Ok(Some(line)) = linhas.next_line().await {
                    let _ = app2.emit(EVENTO, &NineRouterEvent::Log { line });
                }
            });
        }
        if let Some(saida) = stderr {
            let app2 = app.clone();
            tauri::async_runtime::spawn(async move {
                use tokio::io::AsyncBufReadExt as _;
                let mut linhas = tokio::io::BufReader::new(saida).lines();
                while let Ok(Some(line)) = linhas.next_line().await {
                    let _ = app2.emit(EVENTO, &NineRouterEvent::Log { line });
                }
            });
        }
        *guard = Some(nr);
    }

    // Espera FORA do lock: o cold start do Next.js passa de 20 s e segurar o
    // mutex aqui travaria `status` e `stop` durante todo esse tempo.
    let porta = load_config(&state).nine_router.port;
    if let Err(e) = lr_ninerouter::aguardar_pronto(porta, lr_ninerouter::READY_TIMEOUT).await {
        // Não subiu: derruba para não deixar processo pendurado segurando a
        // porta, e devolve o erro com a mensagem do próprio 9router.
        let _ = ninerouter_stop_inner(&state).await;
        return Err(err_str(e));
    }

    // Chave de API: pegar agora, não na primeira conversa. O interruptor
    // "Require API key" do painel pode ser ligado a qualquer momento, e sem
    // isto o chat só descobriria o problema como um 401 seco no meio da
    // resposta ("Missing API key"). Falhar aqui não impede o 9router de
    // servir — significa seguir sem chave, que é o que sempre foi.
    {
        let mut cfg = load_config(&state);
        match lr_ninerouter::garantir_api_key(&l, porta).await {
            Ok(chave) if chave != cfg.nine_router.api_key => {
                cfg.nine_router.api_key = chave;
                state
                    .store
                    .set_setting(SETTING, &cfg.to_json())
                    .map_err(err_str)?;
            }
            Ok(_) => {}
            Err(e) => log::warn!("9router sem chave de API utilizável: {e}"),
        }
    }

    let _ = app.emit(EVENTO, &NineRouterEvent::Ready);
    ninerouter_status(state).await
}

/// Modelos que o 9router atende agora.
///
/// A lista vem dele em tempo real, não de configuração nossa: quem decide o
/// que existe ali é a pessoa, no painel — contas conectadas, combos criados,
/// tudo publicado no `/v1/models` dele. Espelhar isso num setting daqui só
/// criaria uma segunda verdade para ficar desatualizada.
///
/// Devolve lista vazia (nunca erro) quando o 9router não está no ar: o
/// seletor do chat chama isto a cada abertura e um erro ali viraria ruído
/// para quem nem usa provedor externo.
#[tauri::command]
pub async fn ninerouter_models(state: State<'_, AppState>) -> CmdResult<Vec<lr_ninerouter::ModeloNine>> {
    if state.ninerouter.lock().await.is_none() {
        return Ok(Vec::new());
    }
    let porta = load_config(&state).nine_router.port;
    Ok(lr_ninerouter::listar_modelos(porta)
        .await
        .unwrap_or_else(|e| {
            log::warn!("9router no ar mas sem catálogo: {e}");
            Vec::new()
        }))
}

/// Abre o painel do 9router numa janela própria do app.
///
/// Não é escolha de layout: o 9router grava a sessão num cookie
/// `auth_token` com `SameSite=Lax` e sem `Secure`. Num `<iframe>` o painel
/// (`http://127.0.0.1:<porta>`) é cross-site em relação à origem do webview
/// do app, e o Chromium recusa o cookie — o login responde sucesso e a tela
/// volta ao formulário, como se a senha estivesse errada. Em janela de
/// primeiro nível o cookie é first-party e o painel se comporta como no
/// navegador, sem sair do app.
#[tauri::command]
pub async fn ninerouter_open_panel(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    if state.ninerouter.lock().await.is_none() {
        return Err("9router não está no ar".to_string());
    }

    // Já aberta: trazer à frente. Recriar custaria a sessão de quem já logou.
    if let Some(janela) = app.get_webview_window(JANELA_PAINEL) {
        let _ = janela.unminimize();
        let _ = janela.show();
        let _ = janela.set_focus();
        return Ok(());
    }

    let porta = load_config(&state).nine_router.port;
    let url = format!("http://127.0.0.1:{porta}/dashboard")
        .parse::<tauri::Url>()
        .map_err(err_str)?;
    tauri::WebviewWindowBuilder::new(&app, JANELA_PAINEL, tauri::WebviewUrl::External(url))
        .title("9Router")
        .inner_size(1180.0, 820.0)
        .min_inner_size(900.0, 600.0)
        .build()
        .map_err(err_str)?;
    Ok(())
}

/// Fecha o painel quando o processo por trás dele deixa de existir — senão
/// sobra uma janela mostrando erro de conexão.
///
/// Também é chamado quando a janela principal morre: uma janela de conteúdo
/// remoto não pode ser o que mantém o app vivo, ou o processo (e o 9router
/// com ele) sobrevive ao fechamento aos olhos de quem já saiu.
pub fn fechar_painel(app: &AppHandle) {
    if let Some(janela) = app.get_webview_window(JANELA_PAINEL) {
        let _ = janela.close();
    }
}

async fn ninerouter_stop_inner(state: &AppState) -> CmdResult<()> {
    let mut guard = state.ninerouter.lock().await;
    if let Some(nr) = guard.as_mut() {
        nr.stop_blocking();
    }
    *guard = None;
    state
        .ninerouter_pid
        .store(0, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub async fn ninerouter_stop(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<NineRouterStatus> {
    ninerouter_stop_inner(&state).await?;
    fechar_painel(&app);
    sincronizar_rotas(&state).await;
    ninerouter_status(state).await
}

/// Para o processo e apaga a instalação.
///
/// `remove_data` leva junto o SQLite do 9router — e com ele as contas e
/// provedores configurados lá dentro. A tela avisa antes.
#[tauri::command]
pub async fn ninerouter_uninstall(
    app: AppHandle,
    state: State<'_, AppState>,
    remove_data: bool,
) -> CmdResult<NineRouterStatus> {
    ninerouter_stop_inner(&state).await?;
    fechar_painel(&app);
    let l = layout(&state);
    lr_ninerouter::desinstalar(&l, remove_data).map_err(err_str)?;

    let mut cfg = load_config(&state);
    cfg.nine_router.installed = false;
    cfg.nine_router.version = String::new();
    if remove_data {
        // Sem o banco, a senha antiga não abre mais nada: guardá-la só daria
        // a impressão de que ainda vale.
        cfg.nine_router.password = String::new();
        cfg.nine_router.jwt_secret = String::new();
        // A chave vivia no banco que acabou de ser apagado.
        cfg.nine_router.api_key = String::new();
    }
    state
        .store
        .set_setting(SETTING, &cfg.to_json())
        .map_err(err_str)?;
    sincronizar_rotas(&state).await;
    ninerouter_status(state).await
}

// -------------------------------------------------------------- gateway ---

use lr_gateway::GatewayEvent;

/// Chave do setting do gateway. Separada de `providers.config` porque o
/// gateway é um recurso opcional que nada no chat consome — misturá-los faria
/// a configuração dos provedores parecer depender dele.
pub const GATEWAY_SETTING: &str = "gateway.config";

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GatewayConfig {
    pub port: u16,
    /// Escutar na rede local. Opt-in explícito: expor os provedores para
    /// outros aparelhos é decisão da pessoa, nunca padrão.
    pub expose_lan: bool,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            port: lr_gateway::DEFAULT_PORT,
            expose_lan: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayStatus {
    pub installed: bool,
    pub running: bool,
    pub port: u16,
    pub expose_lan: bool,
    pub base_url: Option<String>,
    /// Prefixos ativos agora, para a tela mostrar o que já dá para chamar.
    pub routes: Vec<String>,
}

fn gateway_layout(state: &AppState) -> lr_gateway::Layout {
    lr_gateway::Layout::new(&state.data_dir.join("providers"))
}

fn gateway_config(state: &AppState) -> GatewayConfig {
    state
        .store
        .get_setting(GATEWAY_SETTING)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Portas dos provedores que estão de pé agora.
async fn portas_ativas(state: &AppState) -> (Option<u16>, Option<u16>) {
    let local = {
        let guard = state.server.lock().await;
        guard
            .as_ref()
            .filter(|s| s.is_spawned())
            .map(|s| s.config().port)
    };
    let nove = {
        let guard = state.ninerouter.lock().await;
        guard.as_ref().map(|_| load_config(state).nine_router.port)
    };
    (local, nove)
}

/// Reescreve as rotas do gateway, se ele estiver no ar.
///
/// Chamado quando um provedor sobe ou desce: o provider `file` do Traefik
/// observa o arquivo e recarrega sozinho, então a rota nova passa a valer sem
/// reiniciar nada. Fazer isso no backend, e não na tela, garante que vale
/// também quando quem liga o provedor é o agendador.
async fn sincronizar_rotas(state: &AppState) {
    if state.gateway.lock().await.is_none() {
        return;
    }
    let l = gateway_layout(state);
    let cfg = gateway_config(state);
    let (local, nove) = portas_ativas(state).await;
    if let Err(e) = lr_gateway::escrever_config(
        &l,
        cfg.port,
        cfg.expose_lan,
        &lr_gateway::rotas_ativas(local, nove),
    ) {
        log::warn!("não foi possível atualizar as rotas do gateway: {e}");
    }
}

#[tauri::command]
pub async fn gateway_status(state: State<'_, AppState>) -> CmdResult<GatewayStatus> {
    let cfg = gateway_config(&state);
    let l = gateway_layout(&state);
    let running = state.gateway.lock().await.is_some();
    let (local, nove) = portas_ativas(&state).await;
    Ok(GatewayStatus {
        installed: l.instalado(),
        running,
        port: cfg.port,
        expose_lan: cfg.expose_lan,
        base_url: running.then(|| format!("http://127.0.0.1:{}", cfg.port)),
        routes: lr_gateway::rotas_ativas(local, nove)
            .into_iter()
            .map(|r| r.path_prefix)
            .collect(),
    })
}

#[tauri::command]
pub fn gateway_config_set(
    state: State<'_, AppState>,
    port: u16,
    expose_lan: bool,
) -> CmdResult<()> {
    let cfg = GatewayConfig { port, expose_lan };
    let json = serde_json::to_string(&cfg).map_err(err_str)?;
    state
        .store
        .set_setting(GATEWAY_SETTING, &json)
        .map_err(err_str)
}

#[tauri::command]
pub async fn gateway_install(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<GatewayStatus> {
    let l = gateway_layout(&state);
    let app2 = app.clone();
    let emitir = move |ev: GatewayEvent| {
        let _ = app2.emit(EVENTO, &ev);
    };
    lr_gateway::instalar(&l, &state.profile.os, &state.profile.arch, &emitir)
        .await
        .map_err(err_str)?;
    let _ = app.emit(EVENTO, &GatewayEvent::Ready);
    gateway_status(state).await
}

#[tauri::command]
pub async fn gateway_start(state: State<'_, AppState>) -> CmdResult<GatewayStatus> {
    let l = gateway_layout(&state);
    let cfg = gateway_config(&state);
    let (local, nove) = portas_ativas(&state).await;
    let rotas = lr_gateway::rotas_ativas(local, nove);
    lr_gateway::escrever_config(&l, cfg.port, cfg.expose_lan, &rotas).map_err(err_str)?;

    {
        let mut guard = state.gateway.lock().await;
        if guard.is_some() {
            drop(guard);
            return gateway_status(state).await;
        }
        let gw = lr_gateway::Gateway::spawn(&l, cfg.port).map_err(err_str)?;
        if let Some(pid) = gw.pid() {
            state
                .gateway_pid
                .store(pid, std::sync::atomic::Ordering::SeqCst);
        }
        *guard = Some(gw);
    }

    // Fora do lock, pelo mesmo motivo do 9router.
    if let Err(e) = lr_gateway::aguardar_pronto(cfg.port, std::time::Duration::from_secs(20)).await
    {
        let _ = gateway_stop_inner(&state).await;
        return Err(err_str(e));
    }
    gateway_status(state).await
}

async fn gateway_stop_inner(state: &AppState) -> CmdResult<()> {
    let mut guard = state.gateway.lock().await;
    if let Some(gw) = guard.as_mut() {
        gw.stop_blocking();
    }
    *guard = None;
    state
        .gateway_pid
        .store(0, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub async fn gateway_stop(state: State<'_, AppState>) -> CmdResult<GatewayStatus> {
    gateway_stop_inner(&state).await?;
    gateway_status(state).await
}

/// Reescreve só as rotas.
///
/// O provider `file` do Traefik observa o arquivo, então ligar ou desligar um
/// provedor entra em vigor sem reiniciar o gateway — é a única vantagem real
/// que ele traz de graça neste desenho.
#[tauri::command]
pub async fn gateway_refresh_routes(state: State<'_, AppState>) -> CmdResult<GatewayStatus> {
    let l = gateway_layout(&state);
    let cfg = gateway_config(&state);
    let (local, nove) = portas_ativas(&state).await;
    lr_gateway::escrever_config(
        &l,
        cfg.port,
        cfg.expose_lan,
        &lr_gateway::rotas_ativas(local, nove),
    )
    .map_err(err_str)?;
    gateway_status(state).await
}

#[tauri::command]
pub async fn gateway_uninstall(state: State<'_, AppState>) -> CmdResult<GatewayStatus> {
    gateway_stop_inner(&state).await?;
    lr_gateway::desinstalar(&gateway_layout(&state)).map_err(err_str)?;
    gateway_status(state).await
}
