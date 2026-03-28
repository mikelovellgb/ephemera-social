//! Peer discovery mechanisms.
//!
//! Provides the [`PeerDiscovery`] trait and a [`BootstrapDiscovery`]
//! implementation that connects to hardcoded bootstrap nodes.

use crate::error::TransportError;
use crate::PeerAddr;
use async_trait::async_trait;
use ephemera_types::NodeId;

/// Trait for peer discovery strategies.
///
/// Implementations provide different ways to find peers on the network:
/// bootstrap nodes, DHT-based discovery, mDNS, peer exchange, etc.
#[async_trait]
pub trait PeerDiscovery: Send + Sync {
    /// Discover peers and return their addresses.
    async fn discover(&self) -> Result<Vec<PeerAddr>, TransportError>;

    /// A human-readable name for this discovery strategy.
    fn name(&self) -> &str;
}

/// Bootstrap discovery -- connects to well-known bootstrap nodes.
///
/// These are hardcoded, geographically diverse VPS instances operated
/// by the project team. They are full DHT participants and relay nodes
/// but have no special privileges beyond being initial entry points.
pub struct BootstrapDiscovery {
    /// Addresses of bootstrap nodes.
    bootstrap_addrs: Vec<PeerAddr>,
}

impl BootstrapDiscovery {
    /// Create a new bootstrap discovery from a list of addresses.
    pub fn new(addrs: Vec<PeerAddr>) -> Self {
        Self {
            bootstrap_addrs: addrs,
        }
    }

    /// Create bootstrap discovery from the default hardcoded nodes.
    ///
    /// In a real deployment these would resolve DNS names to actual
    /// node IDs and addresses. For now, they are placeholders.
    #[must_use]
    pub fn default_nodes() -> Self {
        Self {
            bootstrap_addrs: DEFAULT_BOOTSTRAP_NODES
                .iter()
                .enumerate()
                .map(|(i, addr)| PeerAddr {
                    node_id: NodeId::from_bytes({
                        let mut bytes = [0u8; 32];
                        bytes[0] = (i + 1) as u8;
                        bytes
                    }),
                    addresses: vec![(*addr).to_string()],
                })
                .collect(),
        }
    }
}

/// Default bootstrap node addresses.
pub const DEFAULT_BOOTSTRAP_NODES: &[&str] = &[
    "bootstrap-eu-west.ephemera.social:4433",
    "bootstrap-us-east.ephemera.social:4433",
    "bootstrap-us-west.ephemera.social:4433",
    "bootstrap-ap-southeast.ephemera.social:4433",
    "bootstrap-eu-east.ephemera.social:4433",
];

#[async_trait]
impl PeerDiscovery for BootstrapDiscovery {
    async fn discover(&self) -> Result<Vec<PeerAddr>, TransportError> {
        tracing::info!(
            count = self.bootstrap_addrs.len(),
            "returning bootstrap nodes"
        );
        Ok(self.bootstrap_addrs.clone())
    }

    fn name(&self) -> &str {
        "bootstrap"
    }
}

/// Static peer list discovery -- useful for testing or private networks.
pub struct StaticDiscovery {
    peers: Vec<PeerAddr>,
}

impl StaticDiscovery {
    /// Create a static discovery from an explicit peer list.
    pub fn new(peers: Vec<PeerAddr>) -> Self {
        Self { peers }
    }
}

#[async_trait]
impl PeerDiscovery for StaticDiscovery {
    async fn discover(&self) -> Result<Vec<PeerAddr>, TransportError> {
        Ok(self.peers.clone())
    }

    fn name(&self) -> &str {
        "static"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bootstrap_returns_default_nodes() {
        let disc = BootstrapDiscovery::default_nodes();
        let peers = disc.discover().await.unwrap();
        assert_eq!(peers.len(), 5);
    }

    #[tokio::test]
    async fn static_discovery() {
        let peers = vec![PeerAddr {
            node_id: NodeId::from_bytes([42; 32]),
            addresses: vec!["127.0.0.1:9000".into()],
        }];
        let disc = StaticDiscovery::new(peers);
        let result = disc.discover().await.unwrap();
        assert_eq!(result.len(), 1);
    }
}
