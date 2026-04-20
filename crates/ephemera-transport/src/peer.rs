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

// Tests removed: previously used TcpTransport which has been removed.
// PeerRegistry is tested through the gossip integration layer.
