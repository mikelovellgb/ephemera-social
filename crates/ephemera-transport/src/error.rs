//! Transport-layer error types.

/// Errors that can occur in the transport layer.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// Failed to connect to a peer.
    #[error("connection failed to {peer}: {reason}")]
    ConnectionFailed {
        /// Hex-encoded peer node ID (truncated).
        peer: String,
        /// Human-readable failure reason.
        reason: String,
    },

    /// The connection was unexpectedly closed.
    #[error("connection closed: {reason}")]
    ConnectionClosed {
        /// Why the connection was closed.
        reason: String,
    },

    /// A send operation timed out.
    #[error("send timed out after {duration_ms}ms")]
    SendTimeout {
        /// Timeout duration in milliseconds.
        duration_ms: u64,
    },

    /// A receive operation timed out.
    #[error("receive timed out after {duration_ms}ms")]
    RecvTimeout {
        /// Timeout duration in milliseconds.
        duration_ms: u64,
    },

    /// The message exceeds the maximum allowed size.
    #[error("message too large: {size} bytes (max {max})")]
    MessageTooLarge {
        /// Actual size.
        size: usize,
        /// Maximum allowed.
        max: usize,
    },

    /// Peer is not connected.
    #[error("peer not connected: {peer}")]
    PeerNotConnected {
        /// Hex-encoded peer node ID.
        peer: String,
    },

    /// Maximum number of connections reached.
    #[error("connection limit reached: {current}/{max}")]
    ConnectionLimitReached {
        /// Current number of connections.
        current: usize,
        /// Maximum allowed.
        max: usize,
    },

    /// An I/O error from the underlying transport.
    #[error("transport I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The transport has been shut down.
    #[error("transport is shut down")]
    Shutdown,
}
