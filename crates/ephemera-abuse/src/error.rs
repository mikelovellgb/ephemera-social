//! Error types for the anti-abuse subsystem.

/// Errors arising from anti-abuse operations.
#[derive(Debug, thiserror::Error)]
pub enum AbuseError {
    /// Proof-of-work verification failed.
    #[error("PoW verification failed: {reason}")]
    PowInvalid {
        /// Human-readable reason.
        reason: String,
    },

    /// The action was rate-limited.
    #[error("rate limited: {action} (retry after {retry_after_secs}s)")]
    RateLimited {
        /// The action that was denied.
        action: String,
        /// Seconds until the next attempt is allowed.
        retry_after_secs: u64,
    },

    /// Reputation is too low for the requested capability.
    #[error("insufficient reputation: need {required}, have {current}")]
    InsufficientReputation {
        /// Required reputation score.
        required: f64,
        /// Current reputation score.
        current: f64,
    },
}
