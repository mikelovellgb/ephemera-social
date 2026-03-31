//! Network integration: wires transport and gossip service into
//! the node, handling peer connections and post propagation.
//!
//! Supports two transport backends:
//! - **TCP** (always available): Simple length-prefixed TCP framing.
//! - **Iroh** (feature `iroh-transport`): QUIC with NAT traversal.

use ephemera_gossip::service::EagerGossipService;
use ephemera_gossip::topic::{GossipTopic, TopicSubscription};
use ephemera_transport::tcp::TcpTransport;
use ephemera_transport::{PeerAddr, Transport};
use ephemera_types::NodeId;

use std::sync::Arc;
use tracing;

/// Which transport backend the network subsystem is using.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    /// TCP with length-prefixed framing.
    Tcp,
    /// Iroh QUIC with NAT traversal.
    #[cfg(feature = "iroh-transport")]
    Iroh,
}

/// Relay connection status for the Iroh transport.
///
/// When the relay is unavailable (e.g. on mobile networks without IPv6),
/// the node falls back to direct IP connections only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayState {
    /// Relay is connected -- NAT traversal and discovery work.
    Connected,
    /// Relay timed out -- only direct IP:port connections work.
    Unavailable,
    /// Not applicable (TCP transport, no relay involved).
    NotApplicable,
}

/// Network subsystem for the Ephemera node.
///
/// Owns the transport and gossip service. Provides methods for:
/// - Starting the listener
/// - Connecting to peers
/// - Publishing posts to the gossip network
/// - Receiving posts from peers
///
/// The transport is type-erased behind `Arc<dyn Transport>` so the gossip
/// layer and all callers are backend-agnostic.
pub struct NetworkSubsystem {
    /// Our node identity.
    local_id: NodeId,
    /// The transport (type-erased).
    transport: Arc<dyn Transport>,
    /// The gossip service.
    gossip: EagerGossipService,
    /// Which backend is in use.
    kind: TransportKind,
    /// TCP transport reference (kept for TCP-specific operations like `listen`).
    /// `None` when using Iroh backend.
    tcp_transport: Option<Arc<TcpTransport>>,
    /// Relay connection state (Iroh only).
    relay_state: RelayState,
}

impl NetworkSubsystem {
    /// Create a new network subsystem using the TCP transport backend.
    ///
    /// The transport is created but not yet listening. Call [`start`](Self::start)
    /// to bind to a port and begin accepting connections.
    pub fn new(local_id: NodeId) -> Self {
        let transport = Arc::new(TcpTransport::new(local_id));
        let gossip = EagerGossipService::new(local_id, Arc::clone(&transport));
        Self {
            local_id,
            transport: Arc::clone(&transport) as Arc<dyn Transport>,
            gossip,
            kind: TransportKind::Tcp,
            tcp_transport: Some(transport),
            relay_state: RelayState::NotApplicable,
        }
    }

    /// Create a new network subsystem using the Iroh QUIC transport backend.
    ///
    /// The Iroh endpoint is created and begins accepting connections immediately.
    /// No separate `start` call is needed (Iroh binds on construction).
    #[cfg(feature = "iroh-transport")]
    pub async fn new_iroh() -> Result<Self, ephemera_transport::TransportError> {
        let iroh = ephemera_transport::IrohTransport::new().await?;
        let local_id = NodeId::from_bytes(*iroh.endpoint().id().as_bytes());
        let relay_state = match iroh.relay_status() {
            ephemera_transport::RelayStatus::Connected => RelayState::Connected,
            ephemera_transport::RelayStatus::TimedOut => RelayState::Unavailable,
        };
        let transport = Arc::new(iroh);
        let gossip = EagerGossipService::new(local_id, Arc::clone(&transport));
        Ok(Self {
            local_id,
            transport: Arc::clone(&transport) as Arc<dyn Transport>,
            gossip,
            kind: TransportKind::Iroh,
            tcp_transport: None,
            relay_state,
        })
    }

    /// Create a new network subsystem using the Iroh transport with a
    /// deterministic secret key.
    #[cfg(feature = "iroh-transport")]
    pub async fn new_iroh_with_key(
        secret_key: [u8; 32],
    ) -> Result<Self, ephemera_transport::TransportError> {
        let iroh = ephemera_transport::IrohTransport::with_secret_key(secret_key).await?;
        let local_id = NodeId::from_bytes(*iroh.endpoint().id().as_bytes());
        let relay_state = match iroh.relay_status() {
            ephemera_transport::RelayStatus::Connected => RelayState::Connected,
            ephemera_transport::RelayStatus::TimedOut => RelayState::Unavailable,
        };
        let transport = Arc::new(iroh);
        let gossip = EagerGossipService::new(local_id, Arc::clone(&transport));
        Ok(Self {
            local_id,
            transport: Arc::clone(&transport) as Arc<dyn Transport>,
            gossip,
            kind: TransportKind::Iroh,
            tcp_transport: None,
            relay_state,
        })
    }

    /// Start listening for incoming TCP connections on the given address.
    ///
    /// Use `"127.0.0.1:0"` for a random port (useful for tests).
    ///
    /// Only applicable for the TCP backend. For Iroh, the endpoint begins
    /// accepting connections on construction; this method returns `Ok` with
    /// an unspecified address.
    pub async fn start(
        &self,
        listen_addr: &str,
    ) -> Result<std::net::SocketAddr, ephemera_transport::TransportError> {
        match &self.tcp_transport {
            Some(tcp) => {
                let addr = tcp.listen(listen_addr).await?;
                tracing::info!(%addr, node_id = ?self.local_id, "network subsystem started (TCP)");
                Ok(addr)
            }
            None => {
                // Iroh transport -- already listening. Return a placeholder address.
                tracing::info!(node_id = ?self.local_id, "network subsystem started (Iroh)");
                Ok("0.0.0.0:0".parse().unwrap())
            }
        }
    }

    /// Connect to a remote peer.
    pub async fn connect_to_peer(
        &self,
        peer_addr: &PeerAddr,
    ) -> Result<(), ephemera_transport::TransportError> {
        self.transport.connect(peer_addr).await
    }

    /// Subscribe to the public feed gossip topic.
    pub async fn subscribe_public_feed(
        &self,
    ) -> Result<TopicSubscription, ephemera_gossip::GossipError> {
        let topic = GossipTopic::public_feed();
        self.gossip.subscribe(&topic).await
    }

    /// Subscribe to a specific gossip topic.
    pub async fn subscribe(
        &self,
        topic: &GossipTopic,
    ) -> Result<TopicSubscription, ephemera_gossip::GossipError> {
        self.gossip.subscribe(topic).await
    }

    /// Publish a post to the public feed.
    ///
    /// The post payload (serialized post bytes) will be sent to all
    /// connected peers and delivered to local subscribers.
    pub async fn publish_post(&self, payload: Vec<u8>) -> Result<(), ephemera_gossip::GossipError> {
        let topic = GossipTopic::public_feed();
        self.gossip.publish(&topic, payload).await
    }

    /// Publish a message to a specific gossip topic.
    pub async fn publish(
        &self,
        topic: &GossipTopic,
        payload: Vec<u8>,
    ) -> Result<(), ephemera_gossip::GossipError> {
        self.gossip.publish(topic, payload).await
    }

    /// Get the list of currently connected peers.
    pub fn connected_peers(&self) -> Vec<NodeId> {
        self.transport.connected_peers()
    }

    /// Check if connected to a specific peer.
    pub fn is_connected(&self, peer: &NodeId) -> bool {
        self.transport.is_connected(peer)
    }

    /// Number of connected peers.
    pub fn peer_count(&self) -> usize {
        self.connected_peers().len()
    }

    /// Our local node ID.
    pub fn local_id(&self) -> &NodeId {
        &self.local_id
    }

    /// Which transport backend is active.
    pub fn transport_kind(&self) -> TransportKind {
        self.kind
    }

    /// Relay connection state.
    ///
    /// - [`RelayState::Connected`]: Iroh relay is working, NAT traversal available.
    /// - [`RelayState::Unavailable`]: Iroh relay timed out (e.g. no IPv6), only
    ///   direct IP connections work.
    /// - [`RelayState::NotApplicable`]: TCP transport, no relay involved.
    pub fn relay_state(&self) -> RelayState {
        self.relay_state
    }

    /// Disconnect from a specific peer by node ID.
    pub async fn disconnect_peer(
        &self,
        peer: &NodeId,
    ) -> Result<(), ephemera_transport::TransportError> {
        self.transport.disconnect(peer).await
    }

    /// Shut down the network subsystem.
    ///
    /// For TCP, this shuts down the listener and drops connections.
    /// For Iroh, this closes the QUIC endpoint gracefully.
    pub async fn shutdown(&self) {
        match self.kind {
            TransportKind::Tcp => {
                if let Some(tcp) = &self.tcp_transport {
                    tcp.shutdown();
                }
            }
            #[cfg(feature = "iroh-transport")]
            TransportKind::Iroh => {
                // The Iroh endpoint is inside the Arc<dyn Transport>.
                // Dropping the subsystem will drop the endpoint, but we
                // log here for symmetry with the TCP path.
                tracing::info!("Iroh transport shutting down (endpoint will close on drop)");
            }
        }
        tracing::info!("network subsystem shut down");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn network_subsystem_basic() {
        let id = NodeId::from_bytes([42; 32]);
        let net = NetworkSubsystem::new(id);
        let addr = net.start("127.0.0.1:0").await.unwrap();
        assert_ne!(addr.port(), 0);
        assert_eq!(net.peer_count(), 0);
        assert_eq!(net.transport_kind(), TransportKind::Tcp);
        net.shutdown().await;
    }

    #[tokio::test]
    async fn two_nodes_via_subsystem() {
        let id_a = NodeId::from_bytes([1; 32]);
        let id_b = NodeId::from_bytes([2; 32]);

        let net_a = NetworkSubsystem::new(id_a);
        let addr_a = net_a.start("127.0.0.1:0").await.unwrap();

        let net_b = NetworkSubsystem::new(id_b);
        let _addr_b = net_b.start("127.0.0.1:0").await.unwrap();

        net_b
            .connect_to_peer(&PeerAddr {
                node_id: id_a,
                addresses: vec![addr_a.to_string()],
            })
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        assert_eq!(net_a.peer_count(), 1);
        assert_eq!(net_b.peer_count(), 1);

        // Subscribe and publish.
        let mut sub_b = net_b.subscribe_public_feed().await.unwrap();
        let _sub_a = net_a.subscribe_public_feed().await.unwrap();

        net_a.publish_post(b"hello network".to_vec()).await.unwrap();

        let msg = tokio::time::timeout(std::time::Duration::from_secs(3), sub_b.recv())
            .await
            .expect("timeout")
            .expect("channel closed");

        assert_eq!(msg.payload, b"hello network");

        net_a.shutdown().await;
        net_b.shutdown().await;
    }
}
