//! Persistência dos pares já aceitos. JSON na tabela `settings`.

use serde::{Deserialize, Serialize};

pub const SETTING_KEY: &str = "cluster.peers";

/// Quem fomos na última sessão com este par.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LastRole {
    Host,
    Worker,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairedPeer {
    pub id: String,
    pub hostname: String,
    pub last_role: LastRole,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterPersist {
    pub instance_id: String,
    pub peers: Vec<PairedPeer>,
}

impl ClusterPersist {
    pub fn known(&self, id: &str) -> Option<&PairedPeer> {
        self.peers.iter().find(|p| p.id == id)
    }

    pub fn remember(&mut self, id: String, hostname: String, last_role: LastRole) {
        if let Some(p) = self.peers.iter_mut().find(|p| p.id == id) {
            p.hostname = hostname;
            p.last_role = last_role;
            return;
        }
        self.peers.push(PairedPeer {
            id,
            hostname,
            last_role,
        });
    }

    pub fn forget(&mut self, id: &str) {
        self.peers.retain(|p| p.id != id);
    }
}

/// 16 bytes aleatórios em hex — id estável desta instalação.
pub fn new_instance_id() -> String {
    let mut buf = [0u8; 16];
    getrandom::fill(&mut buf).expect("fonte de aleatoriedade do sistema indisponível");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remember_updates_role_without_duplicating() {
        let mut p = ClusterPersist {
            instance_id: "a".into(),
            peers: vec![],
        };
        p.remember("x".into(), "Mac".into(), LastRole::Worker);
        p.remember("x".into(), "MacBook".into(), LastRole::Host);
        assert_eq!(p.peers.len(), 1);
        assert_eq!(p.peers[0].hostname, "MacBook");
        assert_eq!(p.peers[0].last_role, LastRole::Host);
    }

    #[test]
    fn forget_drops_the_peer() {
        let mut p = ClusterPersist {
            instance_id: "a".into(),
            peers: vec![PairedPeer {
                id: "x".into(),
                hostname: "Mac".into(),
                last_role: LastRole::Host,
            }],
        };
        p.forget("x");
        assert!(p.peers.is_empty());
    }
}
