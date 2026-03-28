//! Anti-abuse subsystem for the Ephemera platform.
//!
//! Provides proof-of-work generation and verification, per-identity rate
//! limiting with token buckets, reputation scoring with time-based decay,
//! and SimHash-based near-duplicate content detection for spam filtering.

mod error;
mod fingerprint;
mod pow;
mod rate_limit;
mod reputation;

pub use error::AbuseError;
pub use fingerprint::{ContentFingerprint, FingerprintStore};
pub use pow::{PowChallenge, PowDifficulty, ProofOfWork};
pub use rate_limit::{ActionType, RateLimitConfig, RateLimiter};
pub use reputation::{Capability, ReputationEvent, ReputationScore};
