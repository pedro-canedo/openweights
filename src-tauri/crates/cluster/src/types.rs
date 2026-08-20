//! Tipos que a UI vê (camelCase) e o anúncio na rede.

use serde::{Deserialize, Serialize};

use crate::split::SplitPlan;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClusterRole {
    Idle,
    Host,
    Worker,
    Pending,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeIdentity {
    pub id: String,
    pub hostname: String,
    pub os: String,
    pub llama_tag: String,
    pub gpu_name: String,
    pub device_id: Option<String>,
    pub advertised_bytes: u64,
}

impl NodeIdentity {
    pub fn from_profile(id: String, profile: &lr_types::HardwareProfile, llama_tag: &str) -> Self {
        let hostname = hostname::get()
            .ok()
            .map(|h| h.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "OpenWeights".into());
        Self {
            id,
            hostname,
            os: profile.os.clone(),
            llama_tag: llama_tag.to_string(),
            gpu_name: crate::budget::gpu_label(profile),
            device_id: crate::budget::device_pin(profile),
            advertised_bytes: crate::budget::advertised_bytes(profile),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerView {
    pub id: String,
    pub hostname: String,
    pub os: String,
    pub gpu_name: String,
    pub device_id: String,
    pub advertised_bytes: u64,
    pub llama_tag: String,
    pub ip: String,
    pub control_port: u16,
    pub tag_ok: bool,
    pub paired: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectedView {
    pub peer_id: String,
    pub hostname: String,
    pub gpu_name: String,
    pub devices: String,
    pub tensor_split: String,
    pub rpc_addr: String,
}

impl ConnectedView {
    pub fn from_plan(peer: &PeerView, rpc_addr: String, plan: &SplitPlan) -> Self {
        Self {
            peer_id: peer.id.clone(),
            hostname: peer.hostname.clone(),
            gpu_name: peer.gpu_name.clone(),
            devices: plan.devices.clone(),
            tensor_split: plan.tensor_split.clone(),
            rpc_addr,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSnapshot {
    pub instance_id: String,
    pub hostname: String,
    pub llama_tag: String,
    pub rpc_ready: bool,
    pub device_id: Option<String>,
    pub advertised_bytes: u64,
    pub role: ClusterRole,
    pub peers: Vec<PeerView>,
    pub pending_from: Option<PeerView>,
    pub connected: Option<ConnectedView>,
    pub warning: Option<String>,
    /// mDNS + porta de controle. Desligado por padrão.
    pub enabled: bool,
}

/// Pedido de emparelhamento (host → worker).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairRequest {
    pub id: String,
    pub hostname: String,
    pub os: String,
    pub gpu_name: String,
    pub device_id: Option<String>,
    pub advertised_bytes: u64,
    pub llama_tag: String,
    /// Porta do plano de controle. O ENDEREÇO não vem no corpo de propósito:
    /// quem recebe usa o IP observado da conexão, que ninguém pode mentir.
    pub control_port: u16,
    pub token: String,
}

/// Resposta do worker depois do aceite.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairAccept {
    pub id: String,
    pub token: String,
    pub rpc_port: u16,
    pub device_id: String,
    pub advertised_bytes: u64,
    pub gpu_name: String,
    pub hostname: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairReject {
    pub id: String,
    pub token: String,
}

/// Hello do plano de controle — o host confirma que o peer ainda é quem diz ser.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub id: String,
    pub hostname: String,
    pub os: String,
    pub llama_tag: String,
    pub gpu_name: String,
    pub device_id: Option<String>,
    pub advertised_bytes: u64,
}
