//! TTL-aware Kademlia DHT for the Ephemera P2P network.
//!
//! Provides point lookups for prekey bundles, user profiles, hashtag indexes,
//! and content providers. Records carry a TTL and are garbage-collected after
//! expiration. Built on top of the [`ephemera_transport`] QUIC layer.

pub mod query;
pub mod routing;
pub mod storage;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Errors from the DHT subsystem.
#[derive(Debug, thiserror::Error)]
pub enum DhtError {
    /// The record was not found.
    #[error("record not found for key {key}")]
    NotFound {
        /// Hex-encoded DHT key.
        key: String,
    },

    /// The record has expired.
    #[error("record expired")]
    Expired,

    /// The record exceeds the maximum allowed size.
    #[error("record too large: {size} bytes (max {max})")]
    RecordTooLarge {
        /// Actual size.
        size: usize,
        /// Maximum allowed.
        max: usize,
    },

    /// The record's TTL exceeds the maximum (30 days).
    #[error("TTL too large: {ttl_secs}s (max {max_secs}s)")]
    TtlTooLarge {
        /// Provided TTL.
        ttl_secs: u32,
        /// Maximum allowed.
        max_secs: u32,
    },

    /// The record's signature is invalid.
    #[error("invalid record signature")]
    InvalidSignature,

    /// The local storage is full.
    #[error("storage full: {count}/{max} records")]
    StorageFull {
        /// Current record count.
        count: usize,
        /// Maximum allowed.
        max: usize,
    },

    /// Transport-level error.
    #[error("transport error: {0}")]
    Transport(#[from] ephemera_transport::TransportError),

    /// Query timed out.
    #[error("query timed out")]
    Timeout,
}

/// DHT record types from the architecture spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DhtRecordType {
    /// Prekey bundle for X3DH key exchange.
    PrekeyBundle,
    /// User profile (display name, bio, avatar CID).
    Profile,
    /// Hashtag index entry (maps tag -> content hashes).
    HashtagIndex,
    /// Content provider record (maps content hash -> nodes holding it).
    ContentProvider,
    /// Relay node advertisement.
    RelayAdvertisement,
    /// Dead drop mailbox record for offline message delivery.
    DeadDrop,
}

/// A record stored in the DHT.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DhtRecord {
    /// 32-byte DHT key (BLAKE3 hash).
    pub key: [u8; 32],
    /// Record type.
    pub record_type: DhtRecordType,
    /// Serialized value (max 8 KiB).
    pub value: Vec<u8>,
    /// Publisher's public key.
    pub publisher: [u8; 32],
    /// HLC timestamp (Unix seconds).
    pub timestamp: u64,
    /// TTL in seconds (max 30 days = 2,592,000).
    pub ttl_seconds: u32,
    /// Ed25519 signature over all fields above (64 bytes).
    pub signature: Vec<u8>,
}

/// Maximum record value size: 8 KiB.
pub const MAX_RECORD_SIZE: usize = 8 * 1024;

/// Maximum TTL: 30 days in seconds.
pub const MAX_TTL_SECONDS: u32 = 30 * 24 * 60 * 60;

/// The DHT service trait.
#[async_trait]
pub trait DhtService: Send + Sync {
    /// Store a record in the DHT.
    async fn put(&self, record: DhtRecord) -> Result<(), DhtError>;

    /// Retrieve a record by key.
    async fn get(&self, key: &[u8; 32]) -> Result<Option<DhtRecord>, DhtError>;

    /// Remove a record by key (local only -- DHT records expire via TTL).
    async fn remove(&self, key: &[u8; 32]) -> Result<(), DhtError>;
}

/// DHT configuration parameters from the architecture spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DhtConfig {
    /// Kademlia k parameter (bucket size).
    pub k: usize,
    /// Kademlia alpha parameter (parallel lookups).
    pub alpha: usize,
    /// Replication factor for stored records.
    pub replication: usize,
    /// Routing table refresh interval.
    pub refresh_interval_secs: u64,
    /// Record republish interval.
    pub republish_interval_secs: u64,
    /// Maximum records stored per node.
    pub max_records: usize,
    /// Maximum record value size.
    pub max_record_size: usize,
    /// Stale contact timeout in seconds.
    pub stale_timeout_secs: u64,
}

impl Default for DhtConfig {
    fn default() -> Self {
        Self {
            k: 20,
            alpha: 3,
            replication: 5,
            refresh_interval_secs: 60,
            republish_interval_secs: 3600,
            max_records: 100_000,
            max_record_size: MAX_RECORD_SIZE,
            stale_timeout_secs: 300,
        }
    }
}
