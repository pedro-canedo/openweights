//! Orquestrador: mDNS + HTTP de controle + um par host/worker.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify, Semaphore};

use crate::http::{self, HttpRequest};
use crate::persist::{self, ClusterPersist, LastRole};
use crate::split::{self, SplitPlan};
use crate::types::{
    ClusterRole, ClusterSnapshot, ConnectedView, Hello, NodeIdentity, PairAccept, PairReject,
    PairRequest, PeerView,
};
use crate::worker::{DEFAULT_RPC_PORT, RpcWorker};

const SERVICE_TYPE: &str = "_openweights._tcp.local.";
const CONTROL_PORT: u16 = 17890;
const MAX_INFLIGHT: usize = 16;
const HEARTBEAT_SECS: u64 = 5;
const HEARTBEAT_MISS: u32 = 3;
/// Um pedido de emparelhamento não fica na tela para sempre. Sem prazo, um
/// card órfão (host que sumiu no meio) tranca a máquina: `busy()` recusa
/// qualquer outro pedido e só "Recusar" destrava.
const PENDING_TTL: Duration = Duration::from_secs(60);

pub type OnUpdate = Arc<dyn Fn(ClusterSnapshot) + Send + Sync>;
pub type RpcExeFn = Arc<dyn Fn() -> Option<PathBuf> + Send + Sync>;
pub type SaveFn = Arc<dyn Fn(&ClusterPersist) + Send + Sync>;
pub type NotifyFn = Arc<dyn Fn(&str, &str) + Send + Sync>;
pub type PidFn = Arc<dyn Fn(u32) + Send + Sync>;
/// `true` quando o llama-server local está no ar. Emprestar a GPU nesse
/// momento reserva a mesma VRAM duas vezes.
pub type EngineBusyFn = Arc<dyn Fn() -> bool + Send + Sync>;

struct PendingIn {
    req: PairRequest,
    from_ip: String,
    since: Instant,
}

impl PendingIn {
    fn expired(&self) -> bool {
        self.since.elapsed() >= PENDING_TTL
    }
}

struct PendingOut {
    peer_id: String,
    token: String,
}

struct LiveHost {
    peer: PeerView,
    rpc_addr: String,
    plan: SplitPlan,
    token: String,
}

struct LiveWorker {
    peer_id: String,
    hostname: String,
    control_ip: String,
    control_port: u16,
    token: String,
    rpc_port: u16,
}

struct SplitCache {
    tensor_split: String,
    remote_vram: u64,
    extra_args: Vec<String>,
}

struct State {
    persist: ClusterPersist,
    seen: HashMap<String, PeerView>,
    id_by_fullname: HashMap<String, String>,
    pending_in: Option<PendingIn>,
    pending_out: Option<PendingOut>,
    live_host: Option<LiveHost>,
    live_worker: Option<LiveWorker>,
    warning: Option<String>,
    rpc_ready: bool,
}

pub struct ClusterHost {
    identity: NodeIdentity,
    control_port: Mutex<u16>,
    inner: Mutex<State>,
    save: SaveFn,
    rpc_exe: RpcExeFn,
    on_update: Mutex<Option<OnUpdate>>,
    on_notify: NotifyFn,
    on_pid: PidFn,
    engine_busy: EngineBusyFn,
    cancel: Mutex<Option<Arc<Notify>>>,
    hb_cancel: Mutex<Option<Arc<Notify>>>,
    worker: std::sync::Mutex<Option<RpcWorker>>,
    split_cache: std::sync::RwLock<Option<SplitCache>>,
    conns: Arc<Semaphore>,
}

impl ClusterHost {
    pub fn new(
        identity: NodeIdentity,
        persist: ClusterPersist,
        save: SaveFn,
        rpc_exe: RpcExeFn,
        on_notify: NotifyFn,
        on_pid: PidFn,
        engine_busy: EngineBusyFn,
    ) -> Arc<Self> {
        Arc::new(Self {
            identity,
            control_port: Mutex::new(CONTROL_PORT),
            inner: Mutex::new(State {
                persist,
                seen: HashMap::new(),
                id_by_fullname: HashMap::new(),
                pending_in: None,
                pending_out: None,
                live_host: None,
                live_worker: None,
                warning: None,
                rpc_ready: false,
            }),
            save,
            rpc_exe,
            on_update: Mutex::new(None),
            on_notify,
            on_pid,
            engine_busy,
            cancel: Mutex::new(None),
            hb_cancel: Mutex::new(None),
            worker: std::sync::Mutex::new(None),
            split_cache: std::sync::RwLock::new(None),
            conns: Arc::new(Semaphore::new(MAX_INFLIGHT)),
        })
    }

    pub async fn set_rpc_ready(&self, ready: bool) {
        self.inner.lock().await.rpc_ready = ready;
        self.emit().await;
    }

    pub async fn start(self: &Arc<Self>, on_update: OnUpdate) -> Result<(), String> {
        *self.on_update.lock().await = Some(on_update);
        let enabled = self.inner.lock().await.persist.enabled;
        if enabled {
            self.start_network().await?;
        }
        self.emit().await;
        Ok(())
    }

    pub async fn set_enabled(self: &Arc<Self>, on: bool) -> Result<(), String> {
        {
            let mut st = self.inner.lock().await;
            st.persist.enabled = on;
            (self.save)(&st.persist);
        }
        if on {
            self.start_network().await?;
        } else {
            self.teardown(true).await;
            self.stop_network().await;
        }
        self.emit().await;
        Ok(())
    }

    pub async fn snapshot(&self) -> ClusterSnapshot {
        let mut st = self.inner.lock().await;
        prune_pending(&mut st);
        let st = &*st;
        let role = if st.live_host.is_some() {
            ClusterRole::Host
        } else if st.live_worker.is_some() {
            ClusterRole::Worker
        } else if st.pending_in.is_some() || st.pending_out.is_some() {
            ClusterRole::Pending
        } else {
            ClusterRole::Idle
        };
        let mut peers: Vec<PeerView> = st
            .seen
            .values()
            .cloned()
            .map(|mut p| {
                p.paired = st.persist.known(&p.id).is_some();
                p
            })
            .collect();
        peers.sort_by(|a, b| a.hostname.cmp(&b.hostname));
        let pending_from = st.pending_in.as_ref().map(|p| {
            let mut v = peer_from_request(&p.req, &p.from_ip, &self.identity.llama_tag);
            v.paired = st.persist.known(&v.id).is_some();
            v
        });
        let connected = if let Some(h) = st.live_host.as_ref() {
            Some(ConnectedView::from_plan(
                &h.peer,
                h.rpc_addr.clone(),
                &h.plan,
            ))
        } else {
            st.live_worker.as_ref().map(|w| ConnectedView {
                peer_id: w.peer_id.clone(),
                hostname: w.hostname.clone(),
                gpu_name: st
                    .seen
                    .get(&w.peer_id)
                    .map(|p| p.gpu_name.clone())
                    .unwrap_or_default(),
                devices: self.identity.device_id.clone().unwrap_or_default(),
                tensor_split: String::new(),
                rpc_addr: format!("0.0.0.0:{}", w.rpc_port),
            })
        };
        ClusterSnapshot {
            instance_id: self.identity.id.clone(),
            hostname: self.identity.hostname.clone(),
            llama_tag: self.identity.llama_tag.clone(),
            rpc_ready: st.rpc_ready,
            device_id: self.identity.device_id.clone(),
            advertised_bytes: self.identity.advertised_bytes,
            role,
            peers,
            pending_from,
            connected,
            warning: st.warning.clone(),
            enabled: st.persist.enabled,
        }
    }

    pub async fn remote_vram(&self) -> u64 {
        self.split_cache
            .read()
            .ok()
            .and_then(|c| c.as_ref().map(|s| s.remote_vram))
            .unwrap_or(0)
    }

    pub async fn host_extra_args(&self) -> Vec<String> {
        self.split_cache
            .read()
            .ok()
            .and_then(|c| c.as_ref().map(|s| s.extra_args.clone()))
            .unwrap_or_default()
    }

    pub async fn tensor_split(&self) -> Option<String> {
        self.split_cache
            .read()
            .ok()
            .and_then(|c| c.as_ref().map(|s| s.tensor_split.clone()))
    }

    /// Cache atualizado junto com o par — não depende do mutex do estado.
    pub fn tensor_split_now(&self) -> Option<String> {
        self.split_cache
            .read()
            .ok()
            .and_then(|c| c.as_ref().map(|s| s.tensor_split.clone()))
    }

    pub fn remote_vram_now(&self) -> u64 {
        self.split_cache
            .read()
            .ok()
            .and_then(|c| c.as_ref().map(|s| s.remote_vram))
            .unwrap_or(0)
    }

    pub fn host_extra_args_now(&self) -> Vec<String> {
        self.split_cache
            .read()
            .ok()
            .and_then(|c| c.as_ref().map(|s| s.extra_args.clone()))
            .unwrap_or_default()
    }

    pub async fn worker_pid(&self) -> u32 {
        self.worker
            .lock()
            .ok()
            .and_then(|g| g.as_ref().and_then(|w| w.pid()))
            .unwrap_or(0)
    }

    pub fn stop_blocking(&self) {
        if let Ok(mut c) = self.cancel.try_lock()
            && let Some(n) = c.take()
        {
            n.notify_waiters();
        }
        if let Ok(mut h) = self.hb_cancel.try_lock()
            && let Some(n) = h.take()
        {
            n.notify_waiters();
        }
        self.kill_worker();
    }

    pub async fn request_pair(self: &Arc<Self>, peer_id: &str) -> Result<(), String> {
        let (peer, token) = {
            let st = self.inner.lock().await;
            if !st.persist.enabled {
                return Err("o cluster está desligado".into());
            }
            if busy(&st) {
                return Err("já tem um par nesta sessão".into());
            }
            let peer = st
                .seen
                .get(peer_id)
                .cloned()
                .ok_or_else(|| "esse OpenWeights saiu da rede".to_string())?;
            if !peer.tag_ok {
                return Err("os dois apps precisam da mesma versão do motor llama.cpp".into());
            }
            if peer.advertised_bytes == 0 || peer.device_id.is_empty() {
                return Err("esse OpenWeights não tem GPU para emprestar".into());
            }
            if self.identity.device_id.is_none() || self.identity.advertised_bytes == 0 {
                return Err("esta máquina não tem GPU para o cluster".into());
            }
            let token = st
                .persist
                .known(&peer.id)
                .filter(|p| !p.token.is_empty())
                .map(|p| p.token.clone())
                .unwrap_or_else(persist::new_instance_id);
            (peer, token)
        };
        let req = PairRequest {
            id: self.identity.id.clone(),
            hostname: self.identity.hostname.clone(),
            os: self.identity.os.clone(),
            gpu_name: self.identity.gpu_name.clone(),
            device_id: self.identity.device_id.clone(),
            advertised_bytes: self.identity.advertised_bytes,
            llama_tag: self.identity.llama_tag.clone(),
            control_port: *self.control_port.lock().await,
            token: token.clone(),
        };
        {
            let mut st = self.inner.lock().await;
            if busy(&st) {
                return Err("já tem um par nesta sessão".into());
            }
            st.pending_out = Some(PendingOut {
                peer_id: peer.id.clone(),
                token,
            });
        }
        self.emit().await;
        match http::post_json::<serde_json::Value>(&peer.ip, peer.control_port, "/v1/pair", &req)
            .await
        {
            Ok(_) => Ok(()),
            Err(e) => {
                {
                    let mut st = self.inner.lock().await;
                    if st
                        .pending_out
                        .as_ref()
                        .is_some_and(|p| p.peer_id == peer.id)
                    {
                        st.pending_out = None;
                    }
                }
                self.emit().await;
                Err(e)
            }
        }
    }

    pub async fn accept_incoming(self: &Arc<Self>) -> Result<(), String> {
        let (req, from_ip) = {
            let st = self.inner.lock().await;
            let p = st.pending_in.as_ref().ok_or("não há pedido para aceitar")?;
            (p.req.clone(), p.from_ip.clone())
        };
        match self.start_as_worker(&req, &from_ip, true).await {
            Ok(()) => Ok(()),
            Err(e) => {
                self.inner.lock().await.warning = Some(e.clone());
                self.emit().await;
                Err(e)
            }
        }
    }

    pub async fn reject_incoming(&self) -> Result<(), String> {
        let pending = self.inner.lock().await.pending_in.take();
        self.emit().await;
        let Some(p) = pending else {
            return Ok(());
        };
        let body = PairReject {
            id: self.identity.id.clone(),
            token: p.req.token,
        };
        let _ = http::post_json::<serde_json::Value>(
            &p.from_ip,
            p.req.control_port,
            "/v1/pair/no",
            &body,
        )
        .await;
        Ok(())
    }

    pub async fn forget(self: &Arc<Self>, peer_id: &str) {
        let should_disconnect = {
            let mut st = self.inner.lock().await;
            st.persist.forget(peer_id);
            (self.save)(&st.persist);
            st.live_host.as_ref().is_some_and(|h| h.peer.id == peer_id)
                || st
                    .live_worker
                    .as_ref()
                    .is_some_and(|w| w.peer_id == peer_id)
        };
        if should_disconnect {
            self.teardown(true).await;
        } else {
            self.emit().await;
        }
    }

    pub async fn disconnect(self: &Arc<Self>) {
        self.teardown(true).await;
    }

    async fn start_as_worker(
        self: &Arc<Self>,
        req: &PairRequest,
        from_ip: &str,
        remember: bool,
    ) -> Result<(), String> {
        {
            let st = self.inner.lock().await;
            if st.live_host.is_some() || st.live_worker.is_some() {
                return Err("já tem um par nesta sessão".into());
            }
        }
        let device = self
            .identity
            .device_id
            .clone()
            .ok_or("esta máquina não tem GPU para emprestar")?;
        let exe = (self.rpc_exe)().ok_or(
            "o motor instalado não traz o ggml-rpc-server — atualize o motor de IA",
        )?;
        let port = {
            let prefer = DEFAULT_RPC_PORT;
            if std::net::TcpListener::bind(("0.0.0.0", prefer)).is_ok() {
                prefer
            } else {
                lr_proc::free_port(prefer)
            }
        };
        let worker = RpcWorker::spawn(&exe, port, &device).map_err(|e| e.to_string())?;
        let accept = PairAccept {
            id: self.identity.id.clone(),
            token: req.token.clone(),
            rpc_port: port,
            device_id: device,
            advertised_bytes: self.identity.advertised_bytes,
            gpu_name: self.identity.gpu_name.clone(),
            hostname: self.identity.hostname.clone(),
        };
        if let Err(e) =
            http::post_json::<serde_json::Value>(from_ip, req.control_port, "/v1/pair/ok", &accept)
                .await
        {
            drop(worker);
            return Err(e);
        }

        let pid = worker.pid().unwrap_or(0);
        {
            let mut guard = self.worker.lock().map_err(|e| e.to_string())?;
            *guard = Some(worker);
        }
        (self.on_pid)(pid);

        {
            let mut st = self.inner.lock().await;
            if remember {
                st.persist.remember(
                    req.id.clone(),
                    req.hostname.clone(),
                    LastRole::Worker,
                    req.token.clone(),
                );
                (self.save)(&st.persist);
            }
            st.live_worker = Some(LiveWorker {
                peer_id: req.id.clone(),
                hostname: req.hostname.clone(),
                control_ip: from_ip.to_string(),
                control_port: req.control_port,
                token: req.token.clone(),
                rpc_port: port,
            });
            st.pending_in = None;
            st.warning = None;
        }
        self.start_heartbeat(from_ip.to_string(), req.control_port, req.id.clone())
            .await;
        self.emit().await;
        Ok(())
    }

    async fn become_host(
        self: &Arc<Self>,
        accept: PairAccept,
        peer: PeerView,
    ) -> Result<(), String> {
        let local_dev = self
            .identity
            .device_id
            .clone()
            .ok_or("esta máquina não tem GPU")?;
        let plan = split::plan_split(
            &local_dev,
            self.identity.advertised_bytes,
            accept.advertised_bytes,
        )
        .ok_or("não deu para calcular o split das GPUs")?;
        let rpc_addr = format!("{}:{}", peer.ip, accept.rpc_port);
        let live = LiveHost {
            peer: peer.clone(),
            rpc_addr,
            plan,
            token: accept.token.clone(),
        };
        self.publish_split(Some(&live));
        {
            let mut st = self.inner.lock().await;
            st.persist.remember(
                peer.id.clone(),
                peer.hostname.clone(),
                LastRole::Host,
                accept.token.clone(),
            );
            (self.save)(&st.persist);
            st.live_host = Some(live);
            st.pending_out = None;
            st.warning = None;
        }
        self.start_heartbeat(peer.ip, peer.control_port, peer.id)
            .await;
        self.emit().await;
        Ok(())
    }

    async fn start_network(self: &Arc<Self>) -> Result<(), String> {
        self.stop_network().await;
        let cancel = Arc::new(Notify::new());
        *self.cancel.lock().await = Some(Arc::clone(&cancel));

        let listener = bind_control().await?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        *self.control_port.lock().await = port;

        let this = Arc::clone(self);
        let c = Arc::clone(&cancel);
        tokio::spawn(async move { this.serve_control(listener, c).await });

        match self.register_mdns(port) {
            Ok(mdns) => {
                let this = Arc::clone(self);
                tokio::spawn(async move { this.browse(mdns, cancel).await });
            }
            Err(e) => {
                log::warn!("cluster mDNS: {e}");
                self.inner.lock().await.warning =
                    Some("não consegui anunciar na rede (mDNS). Verifique o firewall.".into());
            }
        }
        Ok(())
    }

    async fn stop_network(&self) {
        if let Some(c) = self.cancel.lock().await.take() {
            c.notify_waiters();
        }
        if let Some(h) = self.hb_cancel.lock().await.take() {
            h.notify_waiters();
        }
    }

    async fn serve_control(self: Arc<Self>, listener: TcpListener, cancel: Arc<Notify>) {
        loop {
            tokio::select! {
                _ = cancel.notified() => break,
                acc = listener.accept() => {
                    let Ok((mut stream, addr)) = acc else { continue };
                    let Ok(permit) = self.conns.clone().try_acquire_owned() else {
                        http::write_empty(&mut stream, 429, "Too Many Requests").await;
                        continue;
                    };
                    let this = Arc::clone(&self);
                    tokio::spawn(async move {
                        let req = match http::read_request(&mut stream, addr).await {
                            Ok(r) => r,
                            Err(e) => {
                                log::debug!("cluster HTTP: {e}");
                                drop(permit);
                                return;
                            }
                        };
                        this.handle_http(&mut stream, req).await;
                        drop(permit);
                    });
                }
            }
        }
    }

    async fn handle_http(self: &Arc<Self>, stream: &mut tokio::net::TcpStream, req: HttpRequest) {
        if !self.inner.lock().await.persist.enabled
            && req.path != "/v1/unpair"
            && req.path != "/v1/hello"
        {
            http::write_empty(stream, 403, "Forbidden").await;
            return;
        }
        match (req.method.as_str(), req.path.as_str()) {
            ("GET", "/v1/hello") => {
                let hello = Hello {
                    id: self.identity.id.clone(),
                    hostname: self.identity.hostname.clone(),
                    os: self.identity.os.clone(),
                    llama_tag: self.identity.llama_tag.clone(),
                    gpu_name: self.identity.gpu_name.clone(),
                    device_id: self.identity.device_id.clone(),
                    advertised_bytes: self.identity.advertised_bytes,
                };
                let body = serde_json::to_string(&hello).unwrap_or_else(|_| "{}".into());
                http::write_json(stream, 200, "OK", &body).await;
            }
            ("POST", "/v1/pair") => {
                let parsed: Result<PairRequest, _> = serde_json::from_slice(&req.body);
                match parsed {
                    Ok(pair) => {
                        let from_ip = req.peer.ip().to_string();
                        let engine_running = (self.engine_busy)();
                        let (verdict, repareia) = {
                            let mut st = self.inner.lock().await;
                            prune_pending(&mut st);
                            let v = admit(&Admission {
                                known: st.persist.known(&pair.id),
                                requester: &pair.id,
                                token: &pair.token,
                                live_peer: live_pair_id(&st),
                                live_worker: st.live_worker.as_ref().map(|w| w.peer_id.as_str()),
                                pending_from: st.pending_in.as_ref().map(|p| p.req.id.as_str()),
                                engine_running,
                            });
                            let repareia = st
                                .live_worker
                                .as_ref()
                                .is_some_and(|w| w.peer_id == pair.id);
                            (v, repareia)
                        };
                        match verdict {
                            Verdict::Busy => http::write_empty(stream, 409, "Conflict").await,
                            Verdict::EngineBusy => {
                                let msg = "o servidor local está rodando nesta máquina; \
                                           pare-o para emprestar a GPU";
                                self.inner.lock().await.warning = Some(msg.into());
                                self.emit().await;
                                erro_json(stream, 503, "Unavailable", msg).await;
                            }
                            Verdict::Auto => {
                                // Mesmo par pedindo de novo (o app dele reiniciou dentro
                                // da janela do heartbeat): derruba o worker velho em vez
                                // de responder "ocupado" para quem já é dono do lugar.
                                if repareia {
                                    self.teardown(false).await;
                                }
                                match self.start_as_worker(&pair, &from_ip, false).await {
                                    Ok(()) => {
                                        http::write_json(
                                            stream,
                                            202,
                                            "Accepted",
                                            "{\"auto\":true}",
                                        )
                                        .await;
                                    }
                                    Err(e) => {
                                        self.inner.lock().await.warning = Some(e.clone());
                                        self.emit().await;
                                        erro_json(stream, 503, "Unavailable", &e).await;
                                    }
                                }
                            }
                            Verdict::Ask => {
                                {
                                    // O veredito foi dado com o lock solto; entre
                                    // um e outro alguém pode ter chegado antes.
                                    let mut st = self.inner.lock().await;
                                    prune_pending(&mut st);
                                    if st.pending_in.as_ref().is_some_and(|p| p.req.id != pair.id)
                                        || live_pair_id(&st).is_some()
                                    {
                                        http::write_empty(stream, 409, "Conflict").await;
                                        return;
                                    }
                                    st.pending_in = Some(PendingIn {
                                        req: pair.clone(),
                                        from_ip,
                                        since: Instant::now(),
                                    });
                                }
                                (self.on_notify)(
                                    "OpenWeights",
                                    &format!(
                                        "{} quer usar esta GPU como extra. Aceite em Servidor local.",
                                        pair.hostname
                                    ),
                                );
                                self.emit().await;
                                self.expire_pending(pair.id.clone());
                                http::write_json(stream, 202, "Accepted", "{\"pending\":true}")
                                    .await;
                            }
                        }
                    }
                    Err(_) => http::write_empty(stream, 400, "Bad Request").await,
                }
            }
            ("POST", "/v1/pair/ok") => {
                let parsed: Result<PairAccept, _> = serde_json::from_slice(&req.body);
                match parsed {
                    Ok(acc) => {
                        let allowed = {
                            let st = self.inner.lock().await;
                            persist::pair_ok_allowed(
                                st.pending_out.as_ref().map(|p| p.peer_id.as_str()),
                                st.pending_out.as_ref().map(|p| p.token.as_str()),
                                &acc.id,
                                &acc.token,
                            )
                        };
                        if !allowed {
                            http::write_empty(stream, 403, "Forbidden").await;
                            return;
                        }
                        let peer = self.inner.lock().await.seen.get(&acc.id).cloned();
                        if let Some(peer) = peer {
                            match self.become_host(acc, peer).await {
                                Ok(()) => http::write_empty(stream, 200, "OK").await,
                                Err(e) => {
                                    self.inner.lock().await.warning = Some(e);
                                    self.emit().await;
                                    http::write_empty(stream, 500, "Error").await;
                                }
                            }
                        } else {
                            http::write_empty(stream, 404, "Not Found").await;
                        }
                    }
                    Err(_) => http::write_empty(stream, 400, "Bad Request").await,
                }
            }
            ("POST", "/v1/pair/no") => {
                let parsed: Result<PairReject, _> = serde_json::from_slice(&req.body);
                match parsed {
                    Ok(rej) => {
                        let allowed = {
                            let st = self.inner.lock().await;
                            persist::pair_ok_allowed(
                                st.pending_out.as_ref().map(|p| p.peer_id.as_str()),
                                st.pending_out.as_ref().map(|p| p.token.as_str()),
                                &rej.id,
                                &rej.token,
                            )
                        };
                        if !allowed {
                            http::write_empty(stream, 403, "Forbidden").await;
                            return;
                        }
                        self.inner.lock().await.pending_out = None;
                        self.emit().await;
                        http::write_empty(stream, 200, "OK").await;
                    }
                    Err(_) => http::write_empty(stream, 400, "Bad Request").await,
                }
            }
            ("POST", "/v1/unpair") => {
                let parsed: Result<PairReject, _> = serde_json::from_slice(&req.body);
                match parsed {
                    Ok(rej) => {
                        let ours = {
                            let st = self.inner.lock().await;
                            st.live_host.as_ref().is_some_and(|h| {
                                h.peer.id == rej.id && persist::tokens_match(&h.token, &rej.token)
                            }) || st.live_worker.as_ref().is_some_and(|w| {
                                w.peer_id == rej.id && persist::tokens_match(&w.token, &rej.token)
                            })
                        };
                        if ours {
                            self.teardown(false).await;
                            http::write_empty(stream, 200, "OK").await;
                        } else {
                            http::write_empty(stream, 403, "Forbidden").await;
                        }
                    }
                    Err(_) => http::write_empty(stream, 400, "Bad Request").await,
                }
            }
            _ => http::write_empty(stream, 404, "Not Found").await,
        }
    }

    fn register_mdns(&self, port: u16) -> Result<ServiceDaemon, String> {
        let mdns = ServiceDaemon::new().map_err(|e| e.to_string())?;
        let instance = sanitize_instance(&format!(
            "{}-{}",
            self.identity.hostname,
            &self.identity.id[..8.min(self.identity.id.len())]
        ));
        let host = format!("{instance}.local.");
        let mut props = HashMap::new();
        props.insert("id".into(), self.identity.id.clone());
        props.insert("name".into(), self.identity.hostname.clone());
        props.insert("tag".into(), self.identity.llama_tag.clone());
        props.insert("os".into(), self.identity.os.clone());
        props.insert("gpu".into(), self.identity.gpu_name.clone());
        props.insert("vram".into(), self.identity.advertised_bytes.to_string());
        if let Some(d) = &self.identity.device_id {
            props.insert("dev".into(), d.clone());
        }
        let mut info = ServiceInfo::new(SERVICE_TYPE, &instance, &host, "", port, Some(props))
            .map_err(|e| e.to_string())?;
        info = info.enable_addr_auto();
        mdns.register(info).map_err(|e| e.to_string())?;
        Ok(mdns)
    }

    async fn browse(self: Arc<Self>, mdns: ServiceDaemon, cancel: Arc<Notify>) {
        let rx = match mdns.browse(SERVICE_TYPE) {
            Ok(r) => r,
            Err(e) => {
                log::warn!("cluster browse: {e}");
                return;
            }
        };
        loop {
            tokio::select! {
                _ = cancel.notified() => break,
                ev = rx.recv_async() => {
                    let Ok(ev) = ev else { break };
                    match ev {
                        ServiceEvent::ServiceResolved(info) => {
                            if let Some(peer) = peer_from_mdns(&info, &self.identity, local_ipv4()) {
                                let fullname = info.get_fullname().to_string();
                                let auto_host = {
                                    let mut st = self.inner.lock().await;
                                    st.id_by_fullname.insert(fullname, peer.id.clone());
                                    st.seen.insert(peer.id.clone(), peer.clone());
                                    st.persist.enabled
                                        && st.persist.known(&peer.id).is_some_and(persist::auto_host_ok)
                                        && st.live_host.is_none()
                                        && st.live_worker.is_none()
                                        && st.pending_out.is_none()
                                };
                                self.emit().await;
                                if auto_host {
                                    let id = peer.id.clone();
                                    let this = Arc::clone(&self);
                                    tokio::spawn(async move {
                                        let _ = this.request_pair(&id).await;
                                    });
                                }
                            }
                        }
                        ServiceEvent::ServiceRemoved(_, fullname) => {
                            let mut st = self.inner.lock().await;
                            if let Some(id) = st.id_by_fullname.remove(&fullname) {
                                st.seen.remove(&id);
                            }
                            drop(st);
                            self.emit().await;
                        }
                        _ => {}
                    }
                }
            }
        }
        let _ = mdns.shutdown();
    }

    /// Apaga o card sozinho quando ninguém responde. Sem isto o prazo só
    /// valeria no próximo evento — e a tela ficaria mentindo até lá.
    fn expire_pending(self: &Arc<Self>, peer_id: String) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(PENDING_TTL).await;
            let venceu = {
                let mut st = this.inner.lock().await;
                let alvo = st
                    .pending_in
                    .as_ref()
                    .is_some_and(|p| p.req.id == peer_id && p.expired());
                if alvo {
                    st.pending_in = None;
                }
                alvo
            };
            if venceu {
                this.emit().await;
            }
        });
    }

    async fn start_heartbeat(self: &Arc<Self>, ip: String, port: u16, expected_id: String) {
        if let Some(n) = self.hb_cancel.lock().await.take() {
            n.notify_waiters();
        }
        let cancel = Arc::new(Notify::new());
        *self.hb_cancel.lock().await = Some(Arc::clone(&cancel));
        let this = Arc::clone(self);
        tokio::spawn(async move {
            let mut misses = 0u32;
            loop {
                tokio::select! {
                    _ = cancel.notified() => break,
                    _ = tokio::time::sleep(Duration::from_secs(HEARTBEAT_SECS)) => {
                        match http::get_json::<Hello>(&ip, port, "/v1/hello").await {
                            Ok(h) if h.id == expected_id => misses = 0,
                            _ => {
                                misses += 1;
                                if misses >= HEARTBEAT_MISS {
                                    this.teardown(false).await;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    async fn teardown(self: &Arc<Self>, notify_peer: bool) {
        if let Some(n) = self.hb_cancel.lock().await.take() {
            n.notify_waiters();
        }
        let peer = {
            let mut st = self.inner.lock().await;
            let host = st.live_host.take();
            let worker = st.live_worker.take();
            st.pending_in = None;
            st.pending_out = None;
            host.map(|h| (h.peer.ip, h.peer.control_port, h.peer.id, h.token))
                .or_else(|| worker.map(|w| (w.control_ip, w.control_port, w.peer_id, w.token)))
        };
        self.publish_split(None);
        self.kill_worker();
        if notify_peer && let Some((ip, port, id, token)) = peer {
            let body = PairReject {
                id: self.identity.id.clone(),
                token,
            };
            let _ = http::post_json::<serde_json::Value>(&ip, port, "/v1/unpair", &body).await;
            let _ = id;
        }
        self.emit().await;
    }

    fn kill_worker(&self) {
        if let Ok(mut g) = self.worker.lock()
            && let Some(mut w) = g.take()
        {
            w.stop_blocking();
        }
        (self.on_pid)(0);
    }

    fn publish_split(&self, live: Option<&LiveHost>) {
        let cache = live.map(|h| SplitCache {
            tensor_split: h.plan.tensor_split.clone(),
            remote_vram: h.peer.advertised_bytes,
            extra_args: split::llama_rpc_args(&h.rpc_addr, &h.plan),
        });
        if let Ok(mut g) = self.split_cache.write() {
            *g = cache;
        }
    }

    async fn emit(&self) {
        let snap = self.snapshot().await;
        if let Some(cb) = self.on_update.lock().await.as_ref() {
            cb(snap);
        }
    }
}

fn busy(st: &State) -> bool {
    st.live_host.is_some()
        || st.live_worker.is_some()
        || st.pending_in.as_ref().is_some_and(|p| !p.expired())
        || st.pending_out.is_some()
}

/// Descarta um pedido recebido que passou do prazo.
fn prune_pending(st: &mut State) {
    if st.pending_in.as_ref().is_some_and(PendingIn::expired) {
        st.pending_in = None;
    }
}

/// Id do par vivo desta sessão, seja qual for o papel.
fn live_pair_id(st: &State) -> Option<&str> {
    st.live_host
        .as_ref()
        .map(|h| h.peer.id.as_str())
        .or_else(|| st.live_worker.as_ref().map(|w| w.peer_id.as_str()))
}

/// O que fazer com um `/v1/pair` que chegou.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// Par conhecido com o segredo certo: sobe o worker sem perguntar.
    Auto,
    /// Mostra o card e espera o toque da pessoa.
    Ask,
    /// Já tem outro par (ou outro pedido) nesta sessão.
    Busy,
    /// Emprestar agora tiraria a VRAM do modelo que roda aqui.
    EngineBusy,
}

struct Admission<'a> {
    known: Option<&'a persist::PairedPeer>,
    requester: &'a str,
    token: &'a str,
    live_peer: Option<&'a str>,
    live_worker: Option<&'a str>,
    pending_from: Option<&'a str>,
    engine_running: bool,
}

/// Quem entra, quem espera e quem leva 409 — sem tocar em rede nem em estado.
///
/// A regra que não é óbvia: um pedido do MESMO par que já está vivo não é
/// conflito, é reconexão. Sem isso, o app do host reiniciando dentro dos 15 s
/// do heartbeat leva 409 e o par só volta no próximo anúncio do mDNS.
fn admit(i: &Admission) -> Verdict {
    let conhecido = i.known.is_some_and(|p| persist::auto_worker_ok(p, i.token));

    if let Some(live) = i.live_peer {
        // Reconexão só vale para quem já é o worker daquele par E prova o
        // segredo; sem isso qualquer um derruba o par alheio com um id.
        if live == i.requester && i.live_worker == Some(i.requester) && conhecido {
            return if i.engine_running {
                Verdict::EngineBusy
            } else {
                Verdict::Auto
            };
        }
        return Verdict::Busy;
    }
    if let Some(pend) = i.pending_from
        && pend != i.requester
    {
        return Verdict::Busy;
    }
    if conhecido {
        return if i.engine_running {
            Verdict::EngineBusy
        } else {
            Verdict::Auto
        };
    }
    // Pedido novo (ou repetido) do mesmo peer: rearma o card em vez de
    // devolver conflito para quem está justamente tentando de novo.
    Verdict::Ask
}

async fn erro_json(stream: &mut tokio::net::TcpStream, status: u16, reason: &str, msg: &str) {
    let body = format!(
        "{{\"error\":{}}}",
        serde_json::to_string(msg).unwrap_or_else(|_| "\"\"".into())
    );
    http::write_json(stream, status, reason, &body).await;
}

fn peer_from_request(req: &PairRequest, ip: &str, my_tag: &str) -> PeerView {
    PeerView {
        id: req.id.clone(),
        hostname: req.hostname.clone(),
        os: req.os.clone(),
        gpu_name: req.gpu_name.clone(),
        device_id: req.device_id.clone().unwrap_or_default(),
        advertised_bytes: req.advertised_bytes,
        llama_tag: req.llama_tag.clone(),
        ip: ip.to_string(),
        control_port: req.control_port,
        tag_ok: split::tags_compatible(my_tag, &req.llama_tag),
        paired: false,
    }
}

fn peer_from_mdns(
    info: &ServiceInfo,
    me: &NodeIdentity,
    local: Option<Ipv4Addr>,
) -> Option<PeerView> {
    let id = info.get_property_val_str("id")?.to_string();
    if id == me.id {
        return None;
    }
    let anunciados: Vec<Ipv4Addr> = info.get_addresses_v4().iter().map(|a| **a).collect();
    let ip = pick_peer_ip(&anunciados, local)?.to_string();
    let tag = info.get_property_val_str("tag").unwrap_or("").to_string();
    Some(PeerView {
        id,
        hostname: info
            .get_property_val_str("name")
            .unwrap_or(info.get_hostname())
            .to_string(),
        os: info.get_property_val_str("os").unwrap_or("").to_string(),
        gpu_name: info.get_property_val_str("gpu").unwrap_or("").to_string(),
        device_id: info.get_property_val_str("dev").unwrap_or("").to_string(),
        advertised_bytes: info
            .get_property_val_str("vram")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        llama_tag: tag.clone(),
        ip,
        control_port: info.get_port(),
        tag_ok: split::tags_compatible(&me.llama_tag, &tag),
        paired: false,
    })
}

/// Qual dos endereços anunciados usar para falar com o peer.
///
/// O `mdns-sd` devolve um `HashSet`: pegar o primeiro do iterador é sorteio,
/// e numa máquina com WSL/Docker/VPN o sorteio sai num `172.x` inalcançável —
/// que vira o `--rpc` e o alvo do heartbeat. Escolhemos o endereço com o
/// maior prefixo em comum com o nosso (mesma sub-rede ganha), e o empate
/// desempata pelo menor número, para a escolha não mudar entre eventos.
fn pick_peer_ip(addrs: &[Ipv4Addr], local: Option<Ipv4Addr>) -> Option<Ipv4Addr> {
    let mut melhor: Option<(u32, Ipv4Addr)> = None;
    for a in addrs {
        let pontos = match local {
            Some(l) => (u32::from(l) ^ u32::from(*a)).leading_zeros(),
            None => 0,
        };
        let candidato = (pontos, *a);
        melhor = match melhor {
            Some((p, atual))
                if p > pontos || (p == pontos && u32::from(atual) <= u32::from(*a)) =>
            {
                Some((p, atual))
            }
            _ => Some(candidato),
        };
    }
    melhor.map(|(_, a)| a)
}

fn sanitize_instance(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if out.len() > 60 {
        out.truncate(60);
    }
    if out.is_empty() {
        out = "openweights".into();
    }
    out
}

fn local_ipv4() -> Option<Ipv4Addr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    match s.local_addr().ok()? {
        std::net::SocketAddr::V4(v) => Some(*v.ip()),
        _ => None,
    }
}

async fn bind_control() -> Result<TcpListener, String> {
    match TcpListener::bind(("0.0.0.0", CONTROL_PORT)).await {
        Ok(l) => Ok(l),
        Err(_) => TcpListener::bind(("0.0.0.0", 0))
            .await
            .map_err(|e| e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persist::{LastRole, PairedPeer};

    fn par(role: LastRole, token: &str) -> PairedPeer {
        PairedPeer {
            id: "host-1".into(),
            hostname: "Dev-PC".into(),
            last_role: role,
            token: token.into(),
        }
    }

    fn pedido<'a>(known: Option<&'a PairedPeer>, token: &'a str) -> Admission<'a> {
        Admission {
            known,
            requester: "host-1",
            token,
            live_peer: None,
            live_worker: None,
            pending_from: None,
            engine_running: false,
        }
    }

    #[test]
    fn par_conhecido_com_segredo_entra_sozinho() {
        let p = par(LastRole::Worker, "s3gr3d0");
        assert_eq!(admit(&pedido(Some(&p), "s3gr3d0")), Verdict::Auto);
    }

    #[test]
    fn id_publico_sem_segredo_ainda_pede_aceite() {
        let p = par(LastRole::Worker, "s3gr3d0");
        assert_eq!(admit(&pedido(Some(&p), "chutado")), Verdict::Ask);
        assert_eq!(admit(&pedido(None, "chutado")), Verdict::Ask);
    }

    #[test]
    fn emprestar_com_o_motor_no_ar_e_recusado_no_caminho_automatico() {
        let p = par(LastRole::Worker, "s3gr3d0");
        let mut i = pedido(Some(&p), "s3gr3d0");
        i.engine_running = true;
        assert_eq!(admit(&i), Verdict::EngineBusy);
    }

    /// O caminho manual segue mostrando o card: quem recusa com mensagem é o
    /// comando de aceitar, que sabe explicar "pare o servidor local".
    #[test]
    fn motor_no_ar_nao_esconde_o_pedido_de_um_desconhecido() {
        let mut i = pedido(None, "novo");
        i.engine_running = true;
        assert_eq!(admit(&i), Verdict::Ask);
    }

    #[test]
    fn outro_par_vivo_leva_conflito() {
        let p = par(LastRole::Worker, "s3gr3d0");
        let mut i = pedido(Some(&p), "s3gr3d0");
        i.live_peer = Some("outro");
        i.live_worker = Some("outro");
        assert_eq!(admit(&i), Verdict::Busy);
    }

    /// O app do host reiniciou dentro dos 15 s do heartbeat: para nós ele
    /// ainda está vivo. Responder 409 deixaria o par preso até o próximo
    /// anúncio do mDNS — que pode não vir.
    #[test]
    fn o_mesmo_par_reconectando_nao_e_conflito() {
        let p = par(LastRole::Worker, "s3gr3d0");
        let mut i = pedido(Some(&p), "s3gr3d0");
        i.live_peer = Some("host-1");
        i.live_worker = Some("host-1");
        assert_eq!(admit(&i), Verdict::Auto);
    }

    #[test]
    fn reconexao_sem_o_segredo_certo_nao_derruba_o_par() {
        let p = par(LastRole::Worker, "s3gr3d0");
        let mut i = pedido(Some(&p), "chutado");
        i.live_peer = Some("host-1");
        i.live_worker = Some("host-1");
        assert_eq!(admit(&i), Verdict::Busy);
    }

    /// Somos HOST de alguém: um pedido para virarmos worker é conflito, mesmo
    /// vindo do mesmo peer.
    #[test]
    fn quem_ja_e_host_nao_vira_worker_do_proprio_par() {
        let p = par(LastRole::Worker, "s3gr3d0");
        let mut i = pedido(Some(&p), "s3gr3d0");
        i.live_peer = Some("host-1");
        i.live_worker = None;
        assert_eq!(admit(&i), Verdict::Busy);
    }

    /// Card na tela e o MESMO peer insiste (a resposta anterior se perdeu):
    /// rearma em vez de travar. Era o caminho que só "Recusar" destravava.
    #[test]
    fn pedido_repetido_do_mesmo_peer_rearma_o_card() {
        let mut i = pedido(None, "novo");
        i.pending_from = Some("host-1");
        assert_eq!(admit(&i), Verdict::Ask);
    }

    #[test]
    fn card_de_outro_peer_segura_o_lugar() {
        let mut i = pedido(None, "novo");
        i.pending_from = Some("alguem-mais");
        assert_eq!(admit(&i), Verdict::Busy);
    }

    #[test]
    fn endereco_do_peer_prefere_a_mesma_sub_rede() {
        let local = Some("192.168.1.20".parse().unwrap());
        let anunciados = [
            "172.17.0.1".parse().unwrap(),
            "192.168.1.8".parse().unwrap(),
        ];
        assert_eq!(
            pick_peer_ip(&anunciados, local),
            Some("192.168.1.8".parse().unwrap())
        );
        // A ordem do HashSet não pode mudar a escolha.
        let invertido = [anunciados[1], anunciados[0]];
        assert_eq!(
            pick_peer_ip(&invertido, local),
            pick_peer_ip(&anunciados, local)
        );
    }

    #[test]
    fn sem_ip_local_a_escolha_ainda_e_estavel() {
        let a: Ipv4Addr = "10.0.0.5".parse().unwrap();
        let b: Ipv4Addr = "172.17.0.1".parse().unwrap();
        assert_eq!(pick_peer_ip(&[a, b], None), Some(a));
        assert_eq!(pick_peer_ip(&[b, a], None), Some(a));
        assert_eq!(pick_peer_ip(&[], None), None);
    }
}
