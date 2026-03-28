//! Configuration management for the Ephemera node.
//!
//! Supports layered configuration: defaults -> TOML config file -> overrides.
//! All configurable parameters are gathered in the top-level `NodeConfig` struct.

use ephemera_types::EphemeraError;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

/// Default maximum local storage in bytes (10 GiB).
pub const DEFAULT_MAX_STORAGE_BYTES: u64 = 10 * 1024 * 1024 * 1024;
/// Default bandwidth limit in bytes per second (0 = unlimited).
pub const DEFAULT_BANDWIDTH_LIMIT_BPS: u64 = 0;
/// Default garbage collection interval in seconds.
pub const DEFAULT_GC_INTERVAL_SECS: u64 = 60;
/// Default DHT replication factor.
pub const DEFAULT_DHT_REPLICATION: u8 = 5;
/// Default maximum number of concurrent peer connections.
pub const DEFAULT_MAX_CONNECTIONS: usize = 256;
/// Default maximum message size in bytes (64 KiB).
pub const DEFAULT_MAX_MESSAGE_SIZE: usize = 64 * 1024;
/// Default connection timeout in seconds.
pub const DEFAULT_CONNECTION_TIMEOUT_SECS: u64 = 30;

/// Resource profile controlling how aggressively the node uses system resources.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceProfile {
    #[default]
    Embedded,
    Standalone,
}

/// Privacy tier preference for the anonymous transport layer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrivacyTier {
    Stealth,
    #[default]
    Private,
    Fast,
}

/// Transport layer configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportConfig {
    #[serde(default)]
    pub default_tier: PrivacyTier,
    #[serde(default = "default_max_circuits")]
    pub max_circuits: usize,
    #[serde(default = "default_relay_discovery_interval")]
    pub relay_discovery_interval_secs: u64,
}
fn default_max_circuits() -> usize { 4 }
fn default_relay_discovery_interval() -> u64 { 300 }
impl Default for TransportConfig {
    fn default() -> Self {
        Self { default_tier: PrivacyTier::default(), max_circuits: 4, relay_discovery_interval_secs: 300 }
    }
}

/// Storage engine configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    #[serde(default = "default_max_storage")]
    pub max_storage_bytes: u64,
    #[serde(default = "default_gc_interval")]
    pub gc_interval_secs: u64,
    #[serde(default = "default_content_dir")]
    pub content_dir: String,
    #[serde(default = "default_metadata_db")]
    pub metadata_db: String,
}
fn default_max_storage() -> u64 { DEFAULT_MAX_STORAGE_BYTES }
fn default_gc_interval() -> u64 { DEFAULT_GC_INTERVAL_SECS }
fn default_content_dir() -> String { "content".into() }
fn default_metadata_db() -> String { "metadata.db".into() }
impl Default for StorageConfig {
    fn default() -> Self {
        Self { max_storage_bytes: DEFAULT_MAX_STORAGE_BYTES, gc_interval_secs: DEFAULT_GC_INTERVAL_SECS, content_dir: "content".into(), metadata_db: "metadata.db".into() }
    }
}

/// Gossip subsystem configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GossipConfig {
    #[serde(default = "default_max_gossip_peers")]
    pub max_peers: usize,
    #[serde(default = "default_gossip_heartbeat")]
    pub heartbeat_interval_secs: u64,
}
fn default_max_gossip_peers() -> usize { 8 }
fn default_gossip_heartbeat() -> u64 { 30 }
impl Default for GossipConfig {
    fn default() -> Self { Self { max_peers: 8, heartbeat_interval_secs: 30 } }
}

/// DHT subsystem configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DhtConfig {
    #[serde(default = "default_k")]
    pub k: usize,
    #[serde(default = "default_alpha")]
    pub alpha: usize,
    #[serde(default = "default_replication")]
    pub replication: u8,
}
fn default_k() -> usize { 20 }
fn default_alpha() -> usize { 3 }
fn default_replication() -> u8 { DEFAULT_DHT_REPLICATION }
impl Default for DhtConfig {
    fn default() -> Self { Self { k: 20, alpha: 3, replication: DEFAULT_DHT_REPLICATION } }
}

/// Top-level node configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    pub data_dir: PathBuf,
    #[serde(default)]
    pub profile: ResourceProfile,
    #[serde(default)]
    pub bandwidth_limit_bps: u64,
    #[serde(default)]
    pub bootstrap_nodes: Vec<String>,
    #[serde(default)]
    pub listen_addr: Option<SocketAddr>,
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
    #[serde(default = "default_max_message_size")]
    pub max_message_size: usize,
    #[serde(default = "default_connection_timeout")]
    pub connection_timeout_secs: u64,
    #[serde(default)]
    pub transport: TransportConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub gossip: GossipConfig,
    #[serde(default)]
    pub dht: DhtConfig,
}
fn default_max_connections() -> usize { DEFAULT_MAX_CONNECTIONS }
fn default_max_message_size() -> usize { DEFAULT_MAX_MESSAGE_SIZE }
fn default_connection_timeout() -> u64 { DEFAULT_CONNECTION_TIMEOUT_SECS }

impl NodeConfig {
    pub fn load(path: &Path) -> Result<Self, EphemeraError> {
        let content = std::fs::read_to_string(path)?;
        toml::from_str(&content).map_err(|e| EphemeraError::ConfigError {
            reason: format!("failed to parse config file: {e}"),
        })
    }

    pub fn load_or_create(data_dir: &Path) -> Result<Self, EphemeraError> {
        let config_path = data_dir.join("config.toml");
        if config_path.exists() {
            Self::load(&config_path)
        } else {
            let config = Self::default_for(data_dir);
            std::fs::create_dir_all(data_dir)?;
            let toml_str = toml::to_string_pretty(&config).map_err(|e| EphemeraError::ConfigError {
                reason: format!("failed to serialize default config: {e}"),
            })?;
            std::fs::write(&config_path, toml_str)?;
            Ok(config)
        }
    }

    #[must_use]
    pub fn default_for(data_dir: &Path) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            profile: ResourceProfile::default(),
            bandwidth_limit_bps: DEFAULT_BANDWIDTH_LIMIT_BPS,
            bootstrap_nodes: vec![],
            listen_addr: None,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            max_message_size: DEFAULT_MAX_MESSAGE_SIZE,
            connection_timeout_secs: DEFAULT_CONNECTION_TIMEOUT_SECS,
            transport: TransportConfig::default(),
            storage: StorageConfig::default(),
            gossip: GossipConfig::default(),
            dht: DhtConfig::default(),
        }
    }

    #[must_use]
    pub fn default_data_dir() -> Option<PathBuf> {
        dirs::data_dir().map(|d| d.join("ephemera"))
    }

    #[must_use]
    pub fn content_path(&self) -> PathBuf { self.data_dir.join(&self.storage.content_dir) }

    #[must_use]
    pub fn metadata_db_path(&self) -> PathBuf { self.data_dir.join(&self.storage.metadata_db) }

    #[must_use]
    pub fn keystore_path(&self) -> PathBuf { self.data_dir.join("keystore.enc") }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
