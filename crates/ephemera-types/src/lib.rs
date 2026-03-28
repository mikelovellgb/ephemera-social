//! Shared primitive types for the Ephemera decentralized social platform.
//!
//! This is the leaf crate in the dependency graph -- every other Ephemera
//! crate depends on it. It contains zero heavy dependencies and defines
//! the foundational type system: identifiers, timestamps, TTL, content
//! classification, and the unified error type.

pub mod content;
pub mod error;
pub mod id;
pub mod identity;
pub mod network;
pub mod timestamp;
pub mod ttl;

// Re-export the most commonly used types at the crate root for ergonomic imports.
pub use content::{Audience, ContentKind, ContentMetadata, MediaType, Quality, SensitivityLabel};
pub use error::{EphemeraError, EphemeraResult, TypeError};
pub use id::ContentId;
pub use identity::{IdentityKey, NodeId, Signature};
pub use network::NetworkStatus;
pub use timestamp::{HlcTimestamp, Timestamp};
pub use ttl::{Expiry, Ttl};

/// Maximum TTL in seconds (30 days).
pub const MAX_TTL_SECONDS: u64 = 30 * 24 * 60 * 60;

/// Minimum TTL in seconds (1 hour).
pub const MIN_TTL_SECONDS: u64 = 60 * 60;

/// Clock skew tolerance in seconds.
pub const CLOCK_SKEW_TOLERANCE: u64 = 300;
