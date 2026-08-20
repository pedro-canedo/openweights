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
    /// Segredo combinado no aceite. Não é o `id` do mDNS (esse é público).
    #[serde(default)]
    pub token: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterPersist {
    pub instance_id: String,
    pub peers: Vec<PairedPeer>,
    /// Desligado por padrão: anunciar e escutar na LAN é uma escolha.
    #[serde(default)]
    pub enabled: bool,
}

impl ClusterPersist {
    pub fn known(&self, id: &str) -> Option<&PairedPeer> {
        self.peers.iter().find(|p| p.id == id)
    }

    pub fn remember(&mut self, id: String, hostname: String, last_role: LastRole, token: String) {
        if let Some(p) = self.peers.iter_mut().find(|p| p.id == id) {
            p.hostname = hostname;
            p.last_role = last_role;
            if !token.is_empty() {
                p.token = token;
            }
            return;
        }
        self.peers.push(PairedPeer {
            id,
            hostname,
            last_role,
            token,
        });
    }

    pub fn forget(&mut self, id: &str) {
        self.peers.retain(|p| p.id != id);
    }
}

/// Comparação em tempo constante. String vazia nunca casa — par legado
/// (sem token) não reconecta sozinho.
pub fn tokens_match(a: &str, b: &str) -> bool {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return false;
    }
    a.as_bytes()
        .iter()
        .zip(b.as_bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// Worker só auto-aceita se o pedido trouxer o segredo gravado no aceite.
pub fn auto_worker_ok(peer: &PairedPeer, token: &str) -> bool {
    peer.last_role == LastRole::Worker && tokens_match(&peer.token, token)
}

/// Host só dispara o pedido de novo se tiver um segredo (não só o id público).
pub fn auto_host_ok(peer: &PairedPeer) -> bool {
    peer.last_role == LastRole::Host && !peer.token.is_empty()
}

/// `/v1/pair/ok` só vale para o pedido que nós fizemos, com o mesmo token.
pub fn pair_ok_allowed(
    pending_peer_id: Option<&str>,
    pending_token: Option<&str>,
    accept_id: &str,
    accept_token: &str,
) -> bool {
    pending_peer_id == Some(accept_id) && tokens_match(pending_token.unwrap_or(""), accept_token)
}

/// 16 bytes aleatórios em hex — id estável desta instalação, ou token de par.
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
            enabled: false,
        };
        p.remember("x".into(), "Mac".into(), LastRole::Worker, "tok".into());
        p.remember("x".into(), "MacBook".into(), LastRole::Host, "tok2".into());
        assert_eq!(p.peers.len(), 1);
        assert_eq!(p.peers[0].hostname, "MacBook");
        assert_eq!(p.peers[0].last_role, LastRole::Host);
        assert_eq!(p.peers[0].token, "tok2");
    }

    #[test]
    fn empty_token_on_update_keeps_the_secret() {
        let mut p = ClusterPersist::default();
        p.remember("x".into(), "Mac".into(), LastRole::Worker, "secret".into());
        p.remember("x".into(), "Mac".into(), LastRole::Worker, String::new());
        assert_eq!(p.peers[0].token, "secret");
    }

    #[test]
    fn forget_drops_the_peer() {
        let mut p = ClusterPersist {
            instance_id: "a".into(),
            peers: vec![PairedPeer {
                id: "x".into(),
                hostname: "Mac".into(),
                last_role: LastRole::Host,
                token: "t".into(),
            }],
            enabled: false,
        };
        p.forget("x");
        assert!(p.peers.is_empty());
    }

    #[test]
    fn tokens_must_match_exactly_and_not_be_empty() {
        assert!(tokens_match("abcd", "abcd"));
        assert!(!tokens_match("abcd", "abce"));
        assert!(!tokens_match("", ""));
        assert!(!tokens_match("a", ""));
        assert!(!tokens_match("ab", "abc"));
    }

    #[test]
    fn auto_reconnect_requires_the_secret_not_just_the_public_id() {
        let peer = PairedPeer {
            id: "public".into(),
            hostname: "Mac".into(),
            last_role: LastRole::Worker,
            token: "secret".into(),
        };
        assert!(auto_worker_ok(&peer, "secret"));
        assert!(!auto_worker_ok(&peer, "public"));
        assert!(!auto_worker_ok(&peer, ""));
        let legacy = PairedPeer {
            token: String::new(),
            ..peer.clone()
        };
        assert!(!auto_worker_ok(&legacy, "anything"));
        let host = PairedPeer {
            last_role: LastRole::Host,
            token: "secret".into(),
            ..peer
        };
        assert!(auto_host_ok(&host));
        assert!(!auto_host_ok(&legacy));
    }

    #[test]
    fn unsolicited_pair_ok_is_rejected() {
        assert!(!pair_ok_allowed(None, None, "w", "tok"));
        assert!(!pair_ok_allowed(Some("w"), Some("tok"), "other", "tok"));
        assert!(!pair_ok_allowed(Some("w"), Some("tok"), "w", "nope"));
        assert!(pair_ok_allowed(Some("w"), Some("tok"), "w", "tok"));
    }
}
