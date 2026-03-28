//! Typed error types for node services.
//!
//! Replaces the pervasive `format!("{e}")` pattern with structured errors
//! that preserve type information and enable programmatic error handling.

/// Errors that can occur during service operations.
#[derive(Debug, thiserror::Error)]
pub enum NodeServiceError {
    /// A mutex lock was poisoned (indicates a prior panic in a lock holder).
    #[error("mutex poisoned: {context}")]
    MutexPoisoned {
        /// Which mutex was poisoned.
        context: &'static str,
    },

    /// The keystore is locked (no identity is loaded).
    #[error("identity locked -- create or unlock first")]
    IdentityLocked,

    /// A cryptographic operation failed.
    #[error("crypto error: {0}")]
    Crypto(#[from] ephemera_types::EphemeraError),

    /// A storage operation failed.
    #[error("storage error: {0}")]
    Storage(#[from] ephemera_store::StoreError),

    /// A database query failed.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// Post construction or validation failed.
    #[error("post error: {0}")]
    Post(#[from] ephemera_post::PostError),

    /// Invalid input provided by the caller.
    #[error("{0}")]
    InvalidInput(String),

    /// A keystore operation failed.
    #[error("keystore error: {0}")]
    Keystore(String),

    /// Hex decoding failed.
    #[error("hex decode error: {0}")]
    HexDecode(#[from] hex::FromHexError),

    /// Serialization/deserialization failed.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl From<NodeServiceError> for String {
    fn from(e: NodeServiceError) -> Self {
        e.to_string()
    }
}

/// Extension trait to convert a poisoned mutex error into a `NodeServiceError`.
pub(crate) trait MutexResultExt<T> {
    /// Convert a `PoisonError` into a `NodeServiceError::MutexPoisoned`.
    fn map_mutex_err(self, context: &'static str) -> Result<T, NodeServiceError>;
}

impl<T> MutexResultExt<T> for Result<T, std::sync::PoisonError<T>> {
    fn map_mutex_err(self, context: &'static str) -> Result<T, NodeServiceError> {
        self.map_err(|_| NodeServiceError::MutexPoisoned { context })
    }
}
