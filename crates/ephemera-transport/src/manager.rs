//! Connection manager -- maintains the pool of active peer connections.
//!
//! [`ConnectionManager`] is responsible for:
//! - Establishing and tearing down connections
//! - Enforcing connection limits
//! - Routing messages to the correct peer connection
//! - Periodic health checks and stale connection cleanup

use crate::config::TransportConfig;
use crate::connection::{ConnectionState, PeerConnection};
use crate::error::TransportError;
use crate::PeerAddr;
use ephemera_types::NodeId;
use std::collections::HashMap;
use tracing;

/// Manages the pool of active peer connections.
pub struct ConnectionManager {
    /// Configuration for the transport layer.
    config: TransportConfig,
    /// Active connections indexed by peer node ID.
    connections: HashMap<NodeId, PeerConnection>,
    /// Our own node ID.
    local_id: NodeId,
}

impl ConnectionManager {
    /// Create a new connection manager.
    pub fn new(local_id: NodeId, config: TransportConfig) -> Self {
        Self {
            config,
            connections: HashMap::new(),
            local_id,
        }
    }

    /// Our local node ID.
    #[must_use]
    pub fn local_id(&self) -> &NodeId {
        &self.local_id
    }

    /// Number of active connections.
    #[must_use]
    pub fn connection_count(&self) -> usize {
        self.connections.values().filter(|c| c.is_active()).count()
    }

    /// Whether we have capacity for more connections.
    #[must_use]
    pub fn has_capacity(&self) -> bool {
        self.connection_count() < self.config.max_connections
    }

    /// Attempt to add a new peer connection.
    ///
    /// Returns an error if we've hit the connection limit or are already
    /// connected to this peer.
    pub fn add_connection(&mut self, addr: PeerAddr) -> Result<(), TransportError> {
        let peer_id = addr.node_id;

        if peer_id == self.local_id {
            return Err(TransportError::ConnectionFailed {
                peer: format!("{peer_id:?}"),
                reason: "cannot connect to self".into(),
            });
        }

        if self.connections.contains_key(&peer_id) && self.connections[&peer_id].is_active() {
            tracing::debug!(peer = %peer_id, "already connected");
            return Ok(());
        }

        if !self.has_capacity() {
            return Err(TransportError::ConnectionLimitReached {
                current: self.connection_count(),
                max: self.config.max_connections,
            });
        }

        let conn = PeerConnection::new(peer_id, addr, 64);
        self.connections.insert(peer_id, conn);
        tracing::info!(peer = %peer_id, "peer connection added");
        Ok(())
    }

    /// Remove a peer connection.
    pub fn remove_connection(&mut self, peer: &NodeId) {
        if let Some(mut conn) = self.connections.remove(peer) {
            conn.set_state(ConnectionState::Disconnected);
            tracing::info!(peer = %peer, "peer connection removed");
        }
    }

    /// Get a mutable reference to a peer's connection.
    pub fn get_connection_mut(&mut self, peer: &NodeId) -> Option<&mut PeerConnection> {
        self.connections.get_mut(peer).filter(|c| c.is_active())
    }

    /// Get a reference to a peer's connection.
    pub fn get_connection(&self, peer: &NodeId) -> Option<&PeerConnection> {
        self.connections.get(peer).filter(|c| c.is_active())
    }

    /// Check if we are connected to a specific peer.
    #[must_use]
    pub fn is_connected(&self, peer: &NodeId) -> bool {
        self.connections
            .get(peer)
            .is_some_and(PeerConnection::is_active)
    }

    /// List all connected peer IDs.
    #[must_use]
    pub fn connected_peers(&self) -> Vec<NodeId> {
        self.connections
            .iter()
            .filter(|(_, c)| c.is_active())
            .map(|(id, _)| *id)
            .collect()
    }

    /// Send data to a specific peer.
    pub async fn send_to(&mut self, peer: &NodeId, data: Vec<u8>) -> Result<(), TransportError> {
        if data.len() > self.config.max_message_size {
            return Err(TransportError::MessageTooLarge {
                size: data.len(),
                max: self.config.max_message_size,
            });
        }

        let conn = self
            .connections
            .get_mut(peer)
            .filter(|c| c.is_active())
            .ok_or_else(|| TransportError::PeerNotConnected {
                peer: format!("{peer:?}"),
            })?;

        conn.send(data).await
    }

    /// Remove stale connections that have been idle longer than the configured timeout.
    pub fn cleanup_stale(&mut self) {
        let idle_timeout = self.config.idle_timeout;
        let stale: Vec<NodeId> = self
            .connections
            .iter()
            .filter(|(_, c)| c.is_active() && c.idle_duration() > idle_timeout)
            .map(|(id, _)| *id)
            .collect();

        for peer in &stale {
            tracing::info!(peer = %peer, "removing stale connection");
            self.remove_connection(peer);
        }
    }

    /// Return a snapshot of connection statistics.
    #[must_use]
    pub fn stats(&self) -> ConnectionStats {
        let active = self.connection_count();
        let relayed = self
            .connections
            .values()
            .filter(|c| c.state() == ConnectionState::Relayed)
            .count();
        let direct = self
            .connections
            .values()
            .filter(|c| c.state() == ConnectionState::Direct)
            .count();

        ConnectionStats {
            active,
            relayed,
            direct,
            max: self.config.max_connections,
        }
    }
}

/// Snapshot of connection pool statistics.
#[derive(Debug, Clone)]
pub struct ConnectionStats {
    /// Number of active connections.
    pub active: usize,
    /// Number of relayed connections.
    pub relayed: usize,
    /// Number of direct (hole-punched) connections.
    pub direct: usize,
    /// Maximum allowed connections.
    pub max: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TransportConfig;

    fn test_manager() -> ConnectionManager {
        let config = TransportConfig {
            max_connections: 3,
            ..TransportConfig::default()
        };
        ConnectionManager::new(NodeId::from_bytes([0; 32]), config)
    }

    fn peer_addr(id: u8) -> PeerAddr {
        PeerAddr {
            node_id: NodeId::from_bytes([id; 32]),
            addresses: vec!["127.0.0.1:4433".into()],
        }
    }

    #[test]
    fn add_and_count() {
        let mut mgr = test_manager();
        assert_eq!(mgr.connection_count(), 0);
        mgr.add_connection(peer_addr(1)).unwrap();
        assert_eq!(mgr.connection_count(), 1);
        assert!(mgr.is_connected(&NodeId::from_bytes([1; 32])));
    }

    #[test]
    fn reject_self_connection() {
        let mut mgr = test_manager();
        let result = mgr.add_connection(peer_addr(0));
        assert!(result.is_err());
    }

    #[test]
    fn enforce_connection_limit() {
        let mut mgr = test_manager();
        mgr.add_connection(peer_addr(1)).unwrap();
        mgr.add_connection(peer_addr(2)).unwrap();
        mgr.add_connection(peer_addr(3)).unwrap();
        let result = mgr.add_connection(peer_addr(4));
        assert!(matches!(
            result,
            Err(TransportError::ConnectionLimitReached { .. })
        ));
    }

    #[test]
    fn remove_frees_slot() {
        let mut mgr = test_manager();
        mgr.add_connection(peer_addr(1)).unwrap();
        mgr.add_connection(peer_addr(2)).unwrap();
        mgr.add_connection(peer_addr(3)).unwrap();
        mgr.remove_connection(&NodeId::from_bytes([2; 32]));
        assert_eq!(mgr.connection_count(), 2);
        mgr.add_connection(peer_addr(4)).unwrap();
        assert_eq!(mgr.connection_count(), 3);
    }

    #[test]
    fn stats() {
        let mut mgr = test_manager();
        mgr.add_connection(peer_addr(1)).unwrap();
        let stats = mgr.stats();
        assert_eq!(stats.active, 1);
        assert_eq!(stats.max, 3);
    }
}
