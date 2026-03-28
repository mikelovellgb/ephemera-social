//! Peer registry: tracks connected peers and supports broadcast.
//!
//! [`PeerRegistry`] wraps a [`Transport`] and provides higher-level
//! operations like broadcasting a message to all connected peers.

use crate::error::TransportError;
use crate::Transport;
use ephemera_types::NodeId;

use std::sync::Arc;

/// A registry of connected peers with broadcast capabilities.
///
/// Wraps any [`Transport`] implementation and adds convenience methods
/// for working with the peer set.
pub struct PeerRegistry<T: Transport> {
    transport: Arc<T>,
}

impl<T: Transport> PeerRegistry<T> {
    /// Create a new peer registry wrapping the given transport.
    pub fn new(transport: Arc<T>) -> Self {
        Self { transport }
    }

    /// Get a reference to the underlying transport.
    pub fn transport(&self) -> &Arc<T> {
        &self.transport
    }

    /// List all currently connected peers.
    pub fn connected_peers(&self) -> Vec<NodeId> {
        self.transport.connected_peers()
    }

    /// Check if a specific peer is connected.
    pub fn is_connected(&self, peer: &NodeId) -> bool {
        self.transport.is_connected(peer)
    }

    /// Send data to a specific peer.
    pub async fn send(&self, peer: &NodeId, data: &[u8]) -> Result<(), TransportError> {
        self.transport.send(peer, data).await
    }

    /// Broadcast data to all connected peers.
    ///
    /// Returns the number of peers that were successfully sent to.
    /// Errors on individual sends are logged but do not stop the broadcast.
    pub async fn broadcast(&self, data: &[u8]) -> usize {
        let peers = self.connected_peers();
        let mut success_count = 0;

        for peer in &peers {
            match self.transport.send(peer, data).await {
                Ok(()) => success_count += 1,
                Err(e) => {
                    tracing::warn!(?peer, error = %e, "broadcast send failed");
                }
            }
        }

        success_count
    }

    /// Broadcast data to all connected peers except the specified one.
    ///
    /// This is used to forward messages received from a peer to all
    /// other peers without sending it back to the source.
    pub async fn broadcast_except(&self, data: &[u8], exclude: &NodeId) -> usize {
        let peers = self.connected_peers();
        let mut success_count = 0;

        for peer in &peers {
            if peer == exclude {
                continue;
            }
            match self.transport.send(peer, data).await {
                Ok(()) => success_count += 1,
                Err(e) => {
                    tracing::warn!(?peer, error = %e, "broadcast send failed");
                }
            }
        }

        success_count
    }

    /// Number of connected peers.
    pub fn peer_count(&self) -> usize {
        self.connected_peers().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tcp::TcpTransport;
    use crate::PeerAddr;

    #[tokio::test]
    async fn broadcast_to_multiple_peers() {
        let id_server = NodeId::from_bytes([1; 32]);
        let id_a = NodeId::from_bytes([2; 32]);
        let id_b = NodeId::from_bytes([3; 32]);

        // Start a server.
        let server = Arc::new(TcpTransport::new(id_server));
        let addr = server.listen("127.0.0.1:0").await.unwrap();

        // Connect two clients.
        let client_a = Arc::new(TcpTransport::new(id_a));
        client_a
            .connect(&PeerAddr {
                node_id: id_server,
                addresses: vec![addr.to_string()],
            })
            .await
            .unwrap();

        let client_b = Arc::new(TcpTransport::new(id_b));
        client_b
            .connect(&PeerAddr {
                node_id: id_server,
                addresses: vec![addr.to_string()],
            })
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        // Server broadcasts.
        let registry = PeerRegistry::new(Arc::clone(&server));
        assert_eq!(registry.peer_count(), 2);

        let sent = registry.broadcast(b"hello all").await;
        assert_eq!(sent, 2);

        // Both clients receive.
        let (_, data_a) = client_a.recv().await.unwrap();
        assert_eq!(data_a, b"hello all");

        let (_, data_b) = client_b.recv().await.unwrap();
        assert_eq!(data_b, b"hello all");

        server.shutdown();
    }

    #[tokio::test]
    async fn broadcast_except() {
        let id_server = NodeId::from_bytes([10; 32]);
        let id_a = NodeId::from_bytes([20; 32]);
        let id_b = NodeId::from_bytes([30; 32]);

        let server = Arc::new(TcpTransport::new(id_server));
        let addr = server.listen("127.0.0.1:0").await.unwrap();

        let client_a = Arc::new(TcpTransport::new(id_a));
        client_a
            .connect(&PeerAddr {
                node_id: id_server,
                addresses: vec![addr.to_string()],
            })
            .await
            .unwrap();

        let client_b = Arc::new(TcpTransport::new(id_b));
        client_b
            .connect(&PeerAddr {
                node_id: id_server,
                addresses: vec![addr.to_string()],
            })
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        let registry = PeerRegistry::new(Arc::clone(&server));

        // Broadcast except client_a.
        let sent = registry.broadcast_except(b"not for A", &id_a).await;
        assert_eq!(sent, 1);

        // Only B should receive.
        let (sender, data) = client_b.recv().await.unwrap();
        assert_eq!(sender, id_server);
        assert_eq!(data, b"not for A");

        server.shutdown();
    }
}
