//! Connection management for the mutual social graph.
//!
//! Connections are bidirectional, consent-required relationships between
//! pseudonyms. This module provides the data structures and service trait
//! for managing the connection lifecycle.

use ephemera_types::{IdentityKey, Timestamp};
use serde::{Deserialize, Serialize};

/// Status of a connection between two pseudonyms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConnectionStatus {
    /// We sent a request, waiting for a response.
    PendingOutgoing,
    /// We received a request, haven't responded yet.
    PendingIncoming,
    /// Mutual connection is active.
    Active,
    /// The connection has been explicitly blocked.
    Blocked,
}

/// A connection record between two pseudonyms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    /// The pseudonym that initiated the connection request.
    pub initiator: IdentityKey,
    /// The pseudonym that received the connection request.
    pub responder: IdentityKey,
    /// Current status of the connection.
    pub status: ConnectionStatus,
    /// When the connection was first created (request sent).
    pub created_at: Timestamp,
    /// When the status last changed.
    pub updated_at: Timestamp,
    /// Optional message attached to the connection request (max 280 chars).
    pub message: Option<String>,
}

impl Connection {
    /// Whether the connection is currently active (mutual).
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.status == ConnectionStatus::Active
    }

    /// Whether the connection is blocked.
    #[must_use]
    pub fn is_blocked(&self) -> bool {
        self.status == ConnectionStatus::Blocked
    }

    /// Return the "other" identity relative to the given local identity.
    #[must_use]
    pub fn remote_party(&self, local: &IdentityKey) -> &IdentityKey {
        if &self.initiator == local {
            &self.responder
        } else {
            &self.initiator
        }
    }
}

/// Errors from connection operations.
#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    /// A connection already exists in an incompatible state.
    #[error("connection already exists with status: {status:?}")]
    AlreadyExists {
        /// The current status of the existing connection.
        status: ConnectionStatus,
    },

    /// The connection was not found.
    #[error("connection not found")]
    NotFound,

    /// The connection message exceeds the 280-character limit.
    #[error("connection message too long: {len} chars, max 280")]
    MessageTooLong {
        /// Actual message length.
        len: usize,
    },

    /// Storage layer error.
    #[error("storage error: {0}")]
    Storage(String),
}

/// Maximum length for a connection request message.
pub const MAX_CONNECTION_MESSAGE_LEN: usize = 280;

/// Service trait for managing connections.
///
/// Implementations handle persistence and network propagation.
#[async_trait::async_trait]
pub trait ConnectionService: Send + Sync {
    /// Send a connection request from `from` to `to`.
    async fn request(
        &self,
        from: &IdentityKey,
        to: &IdentityKey,
        message: Option<&str>,
    ) -> Result<Connection, ConnectionError>;

    /// Accept a pending incoming connection request.
    async fn accept(
        &self,
        local: &IdentityKey,
        remote: &IdentityKey,
    ) -> Result<Connection, ConnectionError>;

    /// Reject a pending incoming connection request.
    async fn reject(
        &self,
        local: &IdentityKey,
        remote: &IdentityKey,
    ) -> Result<(), ConnectionError>;

    /// Remove (disconnect from) an existing active connection.
    async fn remove(
        &self,
        local: &IdentityKey,
        remote: &IdentityKey,
    ) -> Result<(), ConnectionError>;

    /// List all connections for a given pseudonym, optionally filtered by status.
    async fn list(
        &self,
        identity: &IdentityKey,
        status_filter: Option<ConnectionStatus>,
    ) -> Result<Vec<Connection>, ConnectionError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alice() -> IdentityKey {
        IdentityKey::from_bytes([0x01; 32])
    }

    fn bob() -> IdentityKey {
        IdentityKey::from_bytes([0x02; 32])
    }

    #[test]
    fn remote_party() {
        let conn = Connection {
            initiator: alice(),
            responder: bob(),
            status: ConnectionStatus::Active,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
            message: None,
        };
        assert_eq!(conn.remote_party(&alice()), &bob());
        assert_eq!(conn.remote_party(&bob()), &alice());
    }

    #[test]
    fn status_checks() {
        let mut conn = Connection {
            initiator: alice(),
            responder: bob(),
            status: ConnectionStatus::Active,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
            message: None,
        };
        assert!(conn.is_active());
        assert!(!conn.is_blocked());

        conn.status = ConnectionStatus::Blocked;
        assert!(!conn.is_active());
        assert!(conn.is_blocked());
    }

    #[test]
    fn message_length_limit() {
        let long_msg = "x".repeat(MAX_CONNECTION_MESSAGE_LEN + 1);
        let err = ConnectionError::MessageTooLong {
            len: long_msg.len(),
        };
        assert!(format!("{err}").contains("281"));
    }
}
