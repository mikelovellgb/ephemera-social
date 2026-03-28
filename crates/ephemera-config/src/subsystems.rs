//! Subsystem configuration structs: transport, storage, gossip, DHT.

use super::*;

/// Transport layer configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportConfig {
    /// Default privacy tier for outbound connections.
    #[serde(default)]
    pub default_tier: PrivacyTier,
    /// Maximum number of concurrent Tor circuits.
    #[serde(default = "default_max_circuits")]
    pub max_circuits: usize,
    /// Relay discovery interval in seconds.
    #[serde(default = "default_relay_discovery_interval")]
    pub relay_discovery_interval_secs: u64,
}

fn default_max_circuits() -> usize { 4 }
fn default_relay_discovery_interval() -> u64 { 300 }

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            default_tier: PrivacyTier::default(),
            max_circuits: default_max_circuits(),
            relay_discovery_interval_secs: default_relay_discovery_interval(),
        }
    }
}

/// Storage engine configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Maximum local storage in bytes.
    #[serde(default = "default_max_storage")]
    pub max_storage_bytes: u64,
    /// Garbage collection interval in seconds.
    #[serde(default = "default_gc_interval")]
    pub gc_interval_secs: u64,
    /// Path to the fjall content store (relative to data_dir).
    #[serde(default = "default_content_dir")]
    pub content_dir: String,
    /// Path to the SQLite metadata database (relative to data_dir).
    #[serde(default = "default_metadata_db")]
    pub metadata_db: String,
}

fn default_max_storage() -> u64 { DEFAULT_MAX_STORAGE_BYTES }
fn default_gc_interval() -> u64 { DEFAULT_GC_INTERVAL_SECS }
fn default_content_dir() -> String { "content".into() }
fn default_metadata_db() -> String { "metadata.db".into() }

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            max_storage_bytes: default_max_storage(),
            gc_interval_secs: default_gc_interval(),
            content_dir: default_content_dir(),
            metadata_db: default_metadata_db(),
        }
    }
}

/// Gossip subsystem configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GossipConfig {
    /// Maximum number of gossip peers.
    #[serde(default = "default_max_gossip_peers")]
    pub max_peers: usize,
    /// Heartbeat interval in seconds.
    #[serde(default = "default_gossip_heartbeat")]
    pub heartbeat_interval_secs: u64,
}

fn default_max_gossip_peers() -> usize { 8 }
fn default_gossip_heartbeat() -> u64 { 30 }

impl Default for GossipConfig {
    fn default() -> Self {
        Self {
            max_peers: default_max_gossip_peers(),
            heartbeat_interval_secs: default_gossip_heartbeat(),
        }
    }
}

/// DHT subsystem configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DhtConfig {
    /// Kademlia k parameter (bucket size).
    #[serde(default = "default_k")]
    pub k: usize,
    /// Kademlia alpha parameter (concurrency).
    #[serde(default = "default_alpha")]
    pub alpha: usize,
    /// Replication factor for stored records.
    #[serde(default = "default_replication")]
    pub replication: u8,
}

fn default_k() -> usize { 20 }
fn default_alpha() -> usize { 3 }
fn default_replication() -> u8 { DEFAULT_DHT_REPLICATION }

impl Default for DhtConfig {
    fn default() -> Self {
        Self {
            k: default_k(),
            alpha: default_alpha(),
            replication: default_replication(),
        }
    }
}
