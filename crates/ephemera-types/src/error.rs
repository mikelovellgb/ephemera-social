//! Error types for the Ephemera platform.
//!
//! Provides a unified error enum used across all crates. Each crate
//! maps its internal failures into variants of [`EphemeraError`].

/// Unified error type for the Ephemera platform.
///
/// Domain-specific errors are grouped by subsystem. All crate-level
/// functions that can fail return `Result<T, EphemeraError>`.
#[derive(Debug, thiserror::Error)]
pub enum EphemeraError {
    // ── Identity & Crypto ───────────────────────────────────────────
    /// An Ed25519 signature failed verification.
    #[error("signature verification failed: {reason}")]
    SignatureInvalid { reason: String },

    /// A cryptographic key had an invalid format or length.
    #[error("invalid key: {reason}")]
    InvalidKey { reason: String },

    /// Symmetric encryption or decryption failed.
    #[error("encryption error: {reason}")]
    EncryptionError { reason: String },

    /// Key derivation failed.
    #[error("key derivation failed: {reason}")]
    KeyDerivationError { reason: String },

    /// Keystore could not be opened (wrong password, corrupt file, etc.).
    #[error("keystore error: {reason}")]
    KeystoreError { reason: String },

    // ── TTL & Timestamps ────────────────────────────────────────────
    /// A TTL value was outside the valid range.
    #[error("invalid TTL: {value_secs}s (must be {min_secs}s..={max_secs}s)")]
    InvalidTtl {
        value_secs: u64,
        min_secs: u64,
        max_secs: u64,
    },

    /// A timestamp was too far in the future (clock skew).
    #[error("timestamp too far in future: {remote_secs}s vs local {local_secs}s")]
    TimestampSkew { remote_secs: u64, local_secs: u64 },

    /// Content has expired and should not be accepted.
    #[error("content expired")]
    ContentExpired,

    // ── Content ─────────────────────────────────────────────────────
    /// Content failed validation (bad format, missing fields, etc.).
    #[error("invalid content: {reason}")]
    InvalidContent { reason: String },

    /// The requested content was not found in storage.
    #[error("content not found: {id}")]
    ContentNotFound { id: String },

    /// A proof-of-work stamp was invalid or insufficient.
    #[error("invalid proof of work: {reason}")]
    InvalidPow { reason: String },

    // ── Network ─────────────────────────────────────────────────────
    /// A network operation timed out.
    #[error("network timeout after {duration_ms}ms")]
    NetworkTimeout { duration_ms: u64 },

    /// A peer connection failed.
    #[error("peer connection failed: {reason}")]
    PeerConnectionFailed { reason: String },

    /// The gossip subsystem encountered an error.
    #[error("gossip error: {reason}")]
    GossipError { reason: String },

    /// The DHT subsystem encountered an error.
    #[error("DHT error: {reason}")]
    DhtError { reason: String },

    // ── Storage ─────────────────────────────────────────────────────
    /// A storage read or write failed.
    #[error("storage error: {reason}")]
    StorageError { reason: String },

    /// The local storage quota has been exceeded.
    #[error("storage quota exceeded: used {used_bytes} of {max_bytes} bytes")]
    StorageQuotaExceeded { used_bytes: u64, max_bytes: u64 },

    // ── Configuration ───────────────────────────────────────────────
    /// A configuration value was invalid.
    #[error("configuration error: {reason}")]
    ConfigError { reason: String },

    /// An I/O error occurred (file read, network socket, etc.).
    #[error("I/O error: {source}")]
    Io {
        #[from]
        source: std::io::Error,
    },

    // ── Moderation ──────────────────────────────────────────────────
    /// Content was rejected by the CSAM filter.
    #[error("content rejected by safety filter")]
    ContentRejectedBySafetyFilter,

    /// A moderation quorum vote was invalid.
    #[error("invalid moderation vote: {reason}")]
    InvalidModerationVote { reason: String },

    // ── Rate Limiting ───────────────────────────────────────────────
    /// The caller has been rate-limited.
    #[error("rate limited: retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    // ── Serialization ───────────────────────────────────────────────
    /// Serialization or deserialization failed.
    #[error("serialization error: {reason}")]
    SerializationError { reason: String },
}

/// Convenience alias used throughout the Ephemera codebase.
pub type EphemeraResult<T> = Result<T, EphemeraError>;

/// Lightweight error for type-level validation failures (e.g., TTL out of range).
///
/// Used by crates that need a simple, `From`-compatible type error without
/// pulling in the full `EphemeraError` enum.
#[derive(Debug, thiserror::Error)]
pub enum TypeError {
    /// A value was outside its valid range.
    #[error("{message}")]
    OutOfRange {
        /// Human-readable description.
        message: String,
    },
}

impl EphemeraError {
    /// Whether this error is transient and the operation may succeed on retry.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::NetworkTimeout { .. }
                | Self::PeerConnectionFailed { .. }
                | Self::RateLimited { .. }
        )
    }

    /// Whether this error indicates a security violation.
    #[must_use]
    pub fn is_security_violation(&self) -> bool {
        matches!(
            self,
            Self::SignatureInvalid { .. }
                | Self::InvalidPow { .. }
                | Self::ContentRejectedBySafetyFilter
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display() {
        let err = EphemeraError::InvalidTtl {
            value_secs: 60,
            min_secs: 3600,
            max_secs: 2_592_000,
        };
        let msg = format!("{err}");
        assert!(msg.contains("60s"));
        assert!(msg.contains("3600s"));
    }

    #[test]
    fn transient_errors() {
        assert!(EphemeraError::NetworkTimeout { duration_ms: 5000 }.is_transient());
        assert!(EphemeraError::RateLimited {
            retry_after_secs: 30
        }
        .is_transient());
        assert!(!EphemeraError::ContentExpired.is_transient());
    }

    #[test]
    fn security_violations() {
        assert!(EphemeraError::SignatureInvalid {
            reason: "bad sig".into()
        }
        .is_security_violation());
        assert!(EphemeraError::ContentRejectedBySafetyFilter.is_security_violation());
        assert!(!EphemeraError::ContentExpired.is_security_violation());
    }

    #[test]
    fn io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: EphemeraError = io_err.into();
        assert!(matches!(err, EphemeraError::Io { .. }));
    }
}
