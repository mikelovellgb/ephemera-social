//! Network integration: wires the Iroh QUIC transport and gossip service
//! into the node, handling peer connections and post propagation.

use ephemera_gossip::service::EagerGossipService;
use ephemera_gossip::topic::{GossipTopic, TopicSubscription};
use ephemera_transport::{PeerAddr, Transport};
use ephemera_types::NodeId;

use std::sync::Arc;
use tracing;

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
}

/// Network subsystem for the Ephemera node.
///
/// Owns the Iroh QUIC transport and gossip service. Provides methods for:
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
    /// Relay connection state.
    relay_state: RelayState,
}

impl NetworkSubsystem {
    /// Create a new network subsystem using the Iroh QUIC transport with a
    /// deterministic secret key derived from the user's identity.
    ///
    /// The Iroh endpoint is created and begins accepting connections immediately.
    pub async fn new(
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
            relay_state,
        })
    }

    /// Create a new network subsystem using the Iroh QUIC transport with a
    /// random key. Useful for testing or ephemeral sessions.
    pub async fn new_random() -> Result<Self, ephemera_transport::TransportError> {
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
            relay_state,
        })
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

    /// Relay connection state.
    ///
    /// - [`RelayState::Connected`]: Iroh relay is working, NAT traversal available.
    /// - [`RelayState::Unavailable`]: Iroh relay timed out, only direct connections.
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
    pub async fn shutdown(&self) {
        tracing::info!("Iroh transport shutting down (endpoint will close on drop)");
        tracing::info!("network subsystem shut down");
    }
}
