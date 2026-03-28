//! Individual peer connection management.
//!
//! [`PeerConnection`] represents an active connection to a single remote peer.
//! It tracks connection state, message queues, and liveness metrics.

use crate::error::TransportError;
use crate::PeerAddr;
use ephemera_types::NodeId;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing;

/// State of a peer connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Attempting to establish a connection.
    Connecting,
    /// Connected through a relay (higher latency, functional).
    Relayed,
    /// Direct connection established (hole-punch succeeded).
    Direct,
    /// Connection has been closed or failed.
    Disconnected,
}

/// An active connection to a single remote peer.
pub struct PeerConnection {
    /// The remote peer's node ID.
    peer_id: NodeId,
    /// Address information used to establish this connection.
    addr: PeerAddr,
    /// Current connection state.
    state: ConnectionState,
    /// When this connection was established.
    connected_at: Instant,
    /// Last time we received data from this peer.
    last_recv: Instant,
    /// Last time we sent data to this peer.
    last_send: Instant,
    /// Outbound message channel.
    outbound_tx: mpsc::Sender<Vec<u8>>,
    /// Inbound message channel.
    inbound_rx: mpsc::Receiver<Vec<u8>>,
    /// Count of messages sent.
    messages_sent: u64,
    /// Count of messages received.
    messages_received: u64,
    /// Last measured round-trip time.
    last_rtt: Option<std::time::Duration>,
}

impl PeerConnection {
    /// Create a new peer connection.
    ///
    /// This sets up the internal message channels. The actual QUIC connection
    /// is established by the [`ConnectionManager`](crate::ConnectionManager).
    pub fn new(peer_id: NodeId, addr: PeerAddr, buffer_size: usize) -> Self {
        let (outbound_tx, _outbound_rx) = mpsc::channel(buffer_size);
        let (_inbound_tx, inbound_rx) = mpsc::channel(buffer_size);
        let now = Instant::now();
        Self {
            peer_id,
            addr,
            state: ConnectionState::Connecting,
            connected_at: now,
            last_recv: now,
            last_send: now,
            outbound_tx,
            inbound_rx,
            messages_sent: 0,
            messages_received: 0,
            last_rtt: None,
        }
    }

    /// The remote peer's node ID.
    #[must_use]
    pub fn peer_id(&self) -> &NodeId {
        &self.peer_id
    }

    /// Current connection state.
    #[must_use]
    pub fn state(&self) -> ConnectionState {
        self.state
    }

    /// Update the connection state.
    pub fn set_state(&mut self, state: ConnectionState) {
        tracing::debug!(peer = %self.peer_id, ?state, "connection state changed");
        self.state = state;
    }

    /// Whether this connection is active (not disconnected).
    #[must_use]
    pub fn is_active(&self) -> bool {
        !matches!(self.state, ConnectionState::Disconnected)
    }

    /// Queue a message to be sent to this peer.
    pub async fn send(&mut self, data: Vec<u8>) -> Result<(), TransportError> {
        self.outbound_tx
            .send(data)
            .await
            .map_err(|_| TransportError::ConnectionClosed {
                reason: "outbound channel closed".into(),
            })?;
        self.last_send = Instant::now();
        self.messages_sent += 1;
        Ok(())
    }

    /// Receive the next message from this peer.
    pub async fn recv(&mut self) -> Result<Vec<u8>, TransportError> {
        self.inbound_rx
            .recv()
            .await
            .ok_or(TransportError::ConnectionClosed {
                reason: "inbound channel closed".into(),
            })
            .inspect(|_data| {
                self.last_recv = Instant::now();
                self.messages_received += 1;
            })
    }

    /// Duration since we last received data from this peer.
    #[must_use]
    pub fn idle_duration(&self) -> std::time::Duration {
        self.last_recv.elapsed()
    }

    /// Duration since this connection was established.
    #[must_use]
    pub fn uptime(&self) -> std::time::Duration {
        self.connected_at.elapsed()
    }

    /// Total messages sent on this connection.
    #[must_use]
    pub fn messages_sent(&self) -> u64 {
        self.messages_sent
    }

    /// Total messages received on this connection.
    #[must_use]
    pub fn messages_received(&self) -> u64 {
        self.messages_received
    }

    /// Set the last measured round-trip time.
    pub fn set_rtt(&mut self, rtt: std::time::Duration) {
        self.last_rtt = Some(rtt);
    }

    /// Get the last measured round-trip time.
    #[must_use]
    pub fn rtt(&self) -> Option<std::time::Duration> {
        self.last_rtt
    }

    /// The address info for this connection.
    #[must_use]
    pub fn addr(&self) -> &PeerAddr {
        &self.addr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_addr() -> PeerAddr {
        PeerAddr {
            node_id: NodeId::from_bytes([1; 32]),
            addresses: vec!["127.0.0.1:4433".into()],
        }
    }

    #[test]
    fn new_connection_is_connecting() {
        let conn = PeerConnection::new(NodeId::from_bytes([1; 32]), test_addr(), 32);
        assert_eq!(conn.state(), ConnectionState::Connecting);
        assert!(conn.is_active());
        assert_eq!(conn.messages_sent(), 0);
        assert_eq!(conn.messages_received(), 0);
    }

    #[test]
    fn state_transitions() {
        let mut conn = PeerConnection::new(NodeId::from_bytes([1; 32]), test_addr(), 32);
        conn.set_state(ConnectionState::Relayed);
        assert_eq!(conn.state(), ConnectionState::Relayed);
        assert!(conn.is_active());

        conn.set_state(ConnectionState::Direct);
        assert_eq!(conn.state(), ConnectionState::Direct);
        assert!(conn.is_active());

        conn.set_state(ConnectionState::Disconnected);
        assert!(!conn.is_active());
    }
}
