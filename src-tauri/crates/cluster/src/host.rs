//! Orquestrador: mDNS + HTTP de controle + um par host/worker.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify};

use crate::http::{self, HttpRequest};
use crate::persist::{ClusterPersist, LastRole};
use crate::split::{self, SplitPlan};
use crate::types::{
    ClusterRole, ClusterSnapshot, ConnectedView, Hello, NodeIdentity, PairAccept, PairReject,
    PairRequest, PeerView,
};
use crate::worker::{RpcWorker, DEFAULT_RPC_PORT};

const SERVICE_TYPE: &str = "_openweights._tcp.local.";
const CONTROL_PORT: u16 = 17890;

pub type OnUpdate = Arc<dyn Fn(ClusterSnapshot) + Send + Sync>;
pub type RpcExeFn = Arc<dyn Fn() -> Option<PathBuf> + Send + Sync>;
pub type SaveFn = Arc<dyn Fn(&ClusterPersist) + Send + Sync>;
pub type NotifyFn = Arc<dyn Fn(&str, &str) + Send + Sync>;

struct PendingIn {
    req: PairRequest,
    from_ip: String,
}

struct LiveHost {
    peer: PeerView,
    rpc_addr: String,
    plan: SplitPlan,
}

struct State {
    persist: ClusterPersist,
    seen: HashMap<String, PeerView>,
    pending_in: Option<PendingIn>,
    pending_out_id: Option<String>,
    live_host: Option<LiveHost>,
    live_worker_peer: Option<(String, String)>,
    worker: Option<RpcWorker>,
    warning: Option<String>,
    rpc_ready: bool,
}

pub struct ClusterHost {
    identity: NodeIdentity,
    local_ip: Option<Ipv4Addr>,
    control_port: Mutex<u16>,
    inner: Mutex<State>,
    save: SaveFn,
    rpc_exe: RpcExeFn,
    on_update: Mutex<Option<OnUpdate>>,
    on_notify: NotifyFn,
    shutdown: Notify,
}

impl ClusterHost {
    pub fn new(
        identity: NodeIdentity,
        persist: ClusterPersist,
        save: SaveFn,
        rpc_exe: RpcExeFn,
        on_notify: NotifyFn,
    ) -> Arc<Self> {
        Arc::new(Self {
            identity,
            local_ip: local_ipv4(),
            control_port: Mutex::new(CONTROL_PORT),
            inner: Mutex::new(State {
                persist,
                seen: HashMap::new(),
                pending_in: None,
                pending_out_id: None,
                live_host: None,
                live_worker_peer: None,
                worker: None,
                warning: None,
                rpc_ready: false,
            }),
            save,
            rpc_exe,
            on_update: Mutex::new(None),
            on_notify,
            shutdown: Notify::new(),
        })
    }

    pub async fn set_rpc_ready(&self, ready: bool) {
        self.inner.lock().await.rpc_ready = ready;
        self.emit().await;
    }

    pub async fn start(self: &Arc<Self>, on_update: OnUpdate) -> Result<(), String> {
        *self.on_update.lock().await = Some(on_update);

        let listener = bind_control().await?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        *self.control_port.lock().await = port;

        let this = Arc::clone(self);
        tokio::spawn(async move { this.serve_control(listener).await });

        match self.register_mdns(port) {
            Ok(mdns) => {
                let this = Arc::clone(self);
                tokio::spawn(async move { this.browse(mdns).await });
            }
            Err(e) => {
                log::warn!("cluster mDNS: {e}");
                self.inner.lock().await.warning =
                    Some("não consegui anunciar na rede (mDNS). Verifique o firewall.".into());
            }
        }

        self.emit().await;
        Ok(())
    }

    pub async fn snapshot(&self) -> ClusterSnapshot {
        let st = self.inner.lock().await;
        let role = if st.live_host.is_some() {
            ClusterRole::Host
        } else if st.live_worker_peer.is_some() {
            ClusterRole::Worker
        } else if st.pending_in.is_some() || st.pending_out_id.is_some() {
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
            let mut v = peer_from_request(&p.req, &p.from_ip);
            v.paired = st.persist.known(&v.id).is_some();
            v
        });
        let connected = if let Some(h) = st.live_host.as_ref() {
            Some(ConnectedView::from_plan(&h.peer, h.rpc_addr.clone(), &h.plan))
        } else if let Some((id, hostname)) = st.live_worker_peer.as_ref() {
            Some(ConnectedView {
                peer_id: id.clone(),
                hostname: hostname.clone(),
                gpu_name: st
                    .seen
                    .get(id)
                    .map(|p| p.gpu_name.clone())
                    .unwrap_or_default(),
                devices: self.identity.device_id.clone().unwrap_or_default(),
                tensor_split: String::new(),
                rpc_addr: st
                    .worker
                    .as_ref()
                    .map(|w| format!("0.0.0.0:{}", w.port))
                    .unwrap_or_default(),
            })
        } else {
            None
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
        }
    }

    /// VRAM extra quando somos o host com o worker ligado.
    pub async fn remote_vram(&self) -> u64 {
        self.inner
            .lock()
            .await
            .live_host
            .as_ref()
            .map(|h| h.peer.advertised_bytes)
            .unwrap_or(0)
    }

    pub async fn host_extra_args(&self) -> Vec<String> {
        let st = self.inner.lock().await;
        match &st.live_host {
            Some(h) => split::llama_rpc_args(&h.rpc_addr, &h.plan),
            None => Vec::new(),
        }
    }

    pub async fn tensor_split(&self) -> Option<String> {
        self.inner
            .lock()
            .await
            .live_host
            .as_ref()
            .map(|h| h.plan.tensor_split.clone())
    }

    /// Versão síncrona para o preset INI (escrito no boot, sem await).
    pub fn host_extra_args_now(&self) -> Vec<String> {
        self.inner
            .try_lock()
            .ok()
            .and_then(|st| {
                st.live_host
                    .as_ref()
                    .map(|h| split::llama_rpc_args(&h.rpc_addr, &h.plan))
            })
            .unwrap_or_default()
    }

    pub fn remote_vram_now(&self) -> u64 {
        self.inner
            .try_lock()
            .ok()
            .and_then(|st| st.live_host.as_ref().map(|h| h.peer.advertised_bytes))
            .unwrap_or(0)
    }

    pub fn tensor_split_now(&self) -> Option<String> {
        self.inner
            .try_lock()
            .ok()
            .and_then(|st| st.live_host.as_ref().map(|h| h.plan.tensor_split.clone()))
    }

    pub async fn worker_pid(&self) -> u32 {
        self.inner
            .lock()
            .await
            .worker
            .as_ref()
            .and_then(|w| w.pid())
            .unwrap_or(0)
    }

    pub fn stop_blocking(&self) {
        self.shutdown.notify_waiters();
        if let Ok(mut st) = self.inner.try_lock()
            && let Some(mut w) = st.worker.take()
        {
            w.stop_blocking();
        }
    }

    pub async fn request_pair(&self, peer_id: &str) -> Result<(), String> {
        let peer = {
            let st = self.inner.lock().await;
            st.seen
                .get(peer_id)
                .cloned()
                .ok_or_else(|| "esse OpenWeights saiu da rede".to_string())?
        };
        if !peer.tag_ok {
            return Err(
                "os dois apps precisam da mesma versão do motor llama.cpp".into(),
            );
        }
        if self.identity.device_id.is_none() || self.identity.advertised_bytes == 0 {
            return Err("esta máquina não tem GPU para o cluster".into());
        }
        let token = crate::persist::new_instance_id();
        let ip = self
            .local_ip
            .map(|i| i.to_string())
            .ok_or("sem endereço IPv4 na rede")?;
        let req = PairRequest {
            id: self.identity.id.clone(),
            hostname: self.identity.hostname.clone(),
            os: self.identity.os.clone(),
            gpu_name: self.identity.gpu_name.clone(),
            device_id: self.identity.device_id.clone(),
            advertised_bytes: self.identity.advertised_bytes,
            llama_tag: self.identity.llama_tag.clone(),
            ip,
            control_port: *self.control_port.lock().await,
            token,
        };
        self.inner.lock().await.pending_out_id = Some(peer.id.clone());
        self.emit().await;
        let _: serde_json::Value =
            http::post_json(&peer.ip, peer.control_port, "/v1/pair", &req).await?;
        Ok(())
    }

    pub async fn accept_incoming(&self) -> Result<(), String> {
        let (req, from_ip) = {
            let mut st = self.inner.lock().await;
            let p = st
                .pending_in
                .take()
                .ok_or("não há pedido para aceitar")?;
            (p.req, p.from_ip)
        };
        self.start_as_worker(&req, &from_ip, true).await
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

    pub async fn forget(&self, peer_id: &str) {
        let should_disconnect = {
            let mut st = self.inner.lock().await;
            st.persist.forget(peer_id);
            (self.save)(&st.persist);
            st.live_host.as_ref().is_some_and(|h| h.peer.id == peer_id)
                || st
                    .live_worker_peer
                    .as_ref()
                    .is_some_and(|(id, _)| id == peer_id)
        };
        if should_disconnect {
            self.disconnect().await;
        } else {
            self.emit().await;
        }
    }

    pub async fn disconnect(&self) {
        let mut st = self.inner.lock().await;
        if let Some(mut w) = st.worker.take() {
            w.stop_blocking();
        }
        st.live_host = None;
        st.live_worker_peer = None;
        st.pending_in = None;
        st.pending_out_id = None;
        drop(st);
        self.emit().await;
    }

    async fn start_as_worker(
        &self,
        req: &PairRequest,
        _from_ip: &str,
        remember: bool,
    ) -> Result<(), String> {
        let device = self
            .identity
            .device_id
            .clone()
            .ok_or("esta máquina não tem GPU para emprestar")?;
        let exe = (self.rpc_exe)().ok_or(
            "o motor instalado não traz o ggml-rpc-server — instale o pacote RPC da mesma tag",
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
        http::post_json::<serde_json::Value>(&req.ip, req.control_port, "/v1/pair/ok", &accept)
            .await?;

        let mut st = self.inner.lock().await;
        if remember {
            st.persist
                .remember(req.id.clone(), req.hostname.clone(), LastRole::Worker);
            (self.save)(&st.persist);
        }
        st.worker = Some(worker);
        st.live_worker_peer = Some((req.id.clone(), req.hostname.clone()));
        st.pending_in = None;
        drop(st);
        self.emit().await;
        Ok(())
    }

    async fn become_host(&self, accept: PairAccept, peer: PeerView) -> Result<(), String> {
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
        let mut st = self.inner.lock().await;
        st.persist
            .remember(peer.id.clone(), peer.hostname.clone(), LastRole::Host);
        (self.save)(&st.persist);
        st.live_host = Some(LiveHost {
            peer,
            rpc_addr,
            plan,
        });
        st.pending_out_id = None;
        drop(st);
        self.emit().await;
        Ok(())
    }

    async fn serve_control(self: Arc<Self>, listener: TcpListener) {
        loop {
            tokio::select! {
                _ = self.shutdown.notified() => break,
                acc = listener.accept() => {
                    let Ok((mut stream, addr)) = acc else { continue };
                    let this = Arc::clone(&self);
                    tokio::spawn(async move {
                        let req = match http::read_request(&mut stream, addr).await {
                            Ok(r) => r,
                            Err(e) => {
                                log::debug!("cluster HTTP: {e}");
                                return;
                            }
                        };
                        this.handle_http(&mut stream, req).await;
                    });
                }
            }
        }
    }

    async fn handle_http(&self, stream: &mut tokio::net::TcpStream, req: HttpRequest) {
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
                        let auto = {
                            let st = self.inner.lock().await;
                            st.persist
                                .known(&pair.id)
                                .is_some_and(|p| p.last_role == LastRole::Worker)
                        };
                        if auto {
                            let _ = self.start_as_worker(&pair, &from_ip, false).await;
                            http::write_json(stream, 202, "Accepted", "{\"auto\":true}").await;
                        } else {
                            {
                                let mut st = self.inner.lock().await;
                                st.pending_in = Some(PendingIn {
                                    req: pair.clone(),
                                    from_ip,
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
                            http::write_json(stream, 202, "Accepted", "{\"pending\":true}").await;
                        }
                    }
                    Err(_) => http::write_empty(stream, 400, "Bad Request").await,
                }
            }
            ("POST", "/v1/pair/ok") => {
                let parsed: Result<PairAccept, _> = serde_json::from_slice(&req.body);
                match parsed {
                    Ok(acc) => {
                        let peer = self.inner.lock().await.seen.get(&acc.id).cloned();
                        if let Some(peer) = peer {
                            let _ = self.become_host(acc, peer).await;
                            http::write_empty(stream, 200, "OK").await;
                        } else {
                            http::write_empty(stream, 404, "Not Found").await;
                        }
                    }
                    Err(_) => http::write_empty(stream, 400, "Bad Request").await,
                }
            }
            ("POST", "/v1/pair/no") => {
                self.inner.lock().await.pending_out_id = None;
                self.emit().await;
                http::write_empty(stream, 200, "OK").await;
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
        props.insert(
            "vram".into(),
            self.identity.advertised_bytes.to_string(),
        );
        if let Some(d) = &self.identity.device_id {
            props.insert("dev".into(), d.clone());
        }
        let mut info = ServiceInfo::new(SERVICE_TYPE, &instance, &host, "", port, Some(props))
            .map_err(|e| e.to_string())?;
        info = info.enable_addr_auto();
        mdns.register(info).map_err(|e| e.to_string())?;
        Ok(mdns)
    }

    async fn browse(self: Arc<Self>, mdns: ServiceDaemon) {
        let rx = match mdns.browse(SERVICE_TYPE) {
            Ok(r) => r,
            Err(e) => {
                log::warn!("cluster browse: {e}");
                return;
            }
        };
        loop {
            tokio::select! {
                _ = self.shutdown.notified() => break,
                ev = rx.recv_async() => {
                    let Ok(ev) = ev else { break };
                    match ev {
                        ServiceEvent::ServiceResolved(info) => {
                            if let Some(peer) = peer_from_mdns(&info, &self.identity) {
                                let auto_host = {
                                    let mut st = self.inner.lock().await;
                                    st.seen.insert(peer.id.clone(), peer.clone());
                                    st.persist.known(&peer.id).is_some_and(|p| p.last_role == LastRole::Host)
                                        && st.live_host.is_none()
                                        && st.live_worker_peer.is_none()
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
                            st.seen.retain(|_, p| !fullname.contains(&p.hostname));
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

    async fn emit(&self) {
        let snap = self.snapshot().await;
        if let Some(cb) = self.on_update.lock().await.as_ref() {
            cb(snap);
        }
    }
}

fn peer_from_request(req: &PairRequest, ip: &str) -> PeerView {
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
        tag_ok: true,
        paired: false,
    }
}

fn peer_from_mdns(info: &ServiceInfo, me: &NodeIdentity) -> Option<PeerView> {
    let id = info.get_property_val_str("id")?.to_string();
    if id == me.id {
        return None;
    }
    let ip = info
        .get_addresses_v4()
        .iter()
        .next()
        .copied()
        .map(|a| a.to_string())?;
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
