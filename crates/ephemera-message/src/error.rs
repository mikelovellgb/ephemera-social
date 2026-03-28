//! Error types for the messaging subsystem.

/// Errors arising from messaging operations.
#[derive(Debug, thiserror::Error)]
pub enum MessageError {
    /// The message body exceeds the protocol limit.
    #[error("message body exceeds {max} bytes (got {got})")]
    BodyTooLarge {
        /// Actual size in bytes.
        got: usize,
        /// Maximum allowed size in bytes.
        max: usize,
    },

    /// Encryption or decryption failed.
    #[error("encryption error: {0}")]
    Encryption(#[from] ephemera_crypto::CryptoError),

    /// A message request was in an invalid state for the attempted operation.
    #[error("invalid request state: expected {expected}, got {got}")]
    InvalidRequestState {
        /// Expected state.
        expected: String,
        /// Actual state.
        got: String,
    },

    /// The TTL value was invalid.
    #[error("invalid TTL: {0}")]
    InvalidTtl(#[from] ephemera_types::EphemeraError),

    /// The conversation was not found.
    #[error("conversation not found for peer {peer_id}")]
    ConversationNotFound {
        /// The peer identity.
        peer_id: String,
    },

    /// The sender is not allowed to message this recipient (request rejected or not accepted).
    #[error("messaging not allowed: {reason}")]
    NotAllowed {
        /// Reason messaging is not permitted.
        reason: String,
    },

    /// Storage layer error.
    #[error("storage error: {0}")]
    Store(#[from] ephemera_store::StoreError),

    /// SQLite error (direct rusqlite access in service layer).
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// Deserialization error for inner envelope payloads.
    #[error("deserialization error: {0}")]
    Deserialization(String),

    /// The message has expired.
    #[error("message expired")]
    Expired,

    /// Prekey bundle validation failed.
    #[error("invalid prekey bundle: {reason}")]
    InvalidPrekeyBundle {
        /// Human-readable reason.
        reason: String,
    },

    /// X3DH key exchange failed.
    #[error("X3DH key exchange failed: {reason}")]
    X3dhFailed {
        /// Human-readable reason.
        reason: String,
    },

    /// Ratchet protocol error.
    #[error("ratchet error: {reason}")]
    RatchetError {
        /// Human-readable reason.
        reason: String,
    },
}
