//! QUIC transport layer for the Ephemera P2P network.
//!
//! Provides the [`Transport`] trait abstraction, a [`ConnectionManager`] for
//! managing multiple peer connections, peer discovery, NAT traversal helpers,
//! and transport configuration.
//!
//! Two concrete transport implementations are available:
//!
//! - **TCP** (`TcpTransport`): Simple length-prefixed TCP framing. Always
//!   available. Useful for local testing and environments where QUIC is not
//!   feasible.
//!
//! - **Iroh** (`IrohTransport`): QUIC-based transport with built-in NAT
//!   traversal via relay servers and hole-punching. Enabled by the
//!   `iroh-transport` feature flag (on by default).

pub mod config;
pub mod connection;
pub mod discovery;
pub mod error;
pub mod manager;
pub mod nat;
pub mod peer;
pub mod tcp;

#[cfg(feature = "iroh-transport")]
pub mod iroh_transport;

pub use config::TransportConfig;
pub use connection::PeerConnection;
pub use error::TransportError;
pub use manager::ConnectionManager;
pub use peer::PeerRegistry;
pub use tcp::TcpTransport;

#[cfg(feature = "iroh-transport")]
pub use iroh_transport::IrohTransport;

use async_trait::async_trait;
use ephemera_types::NodeId;

/// Address information for reaching a peer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PeerAddr {
    /// The peer's 32-byte node ID.
    pub node_id: NodeId,
    /// Known socket addresses for this peer.
    pub addresses: Vec<String>,
}

/// The core transport trait. All networking goes through this abstraction.
///
/// Implementations may use Iroh QUIC, TCP, or mock transports for testing.
#[async_trait]
pub trait Transport: Send + Sync + 'static {
    /// Send raw bytes to a specific peer.
    async fn send(&self, peer: &NodeId, data: &[u8]) -> Result<(), TransportError>;

    /// Receive the next inbound message. Returns the sender and payload.
    async fn recv(&self) -> Result<(NodeId, Vec<u8>), TransportError>;

    /// Connect to a peer by address.
    async fn connect(&self, addr: &PeerAddr) -> Result<(), TransportError>;

    /// Disconnect from a peer.
    async fn disconnect(&self, peer: &NodeId) -> Result<(), TransportError>;

    /// List currently connected peers.
    fn connected_peers(&self) -> Vec<NodeId>;

    /// Check if we are connected to a specific peer.
    fn is_connected(&self, peer: &NodeId) -> bool;
}
