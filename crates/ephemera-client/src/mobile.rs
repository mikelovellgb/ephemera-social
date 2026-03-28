//! Mobile platform configuration for Ephemera.
//!
//! Defines resource-constrained defaults for Android and iOS, including
//! storage caps, connection limits, and background-mode behaviour. The
//! [`MobileConfig`] can be converted into a [`NodeConfig`] that the
//! embedded node understands.

use ephemera_config::{
    DhtConfig, GossipConfig, NodeConfig, ResourceProfile, StorageConfig, TransportConfig,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Target platform for the running application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    /// Desktop (Windows, macOS, Linux).
    Desktop,
    /// Android (API 26+).
    Android,
    /// iOS (15+).
    #[serde(rename = "ios")]
    IOS,
}

impl Platform {
    /// Detect the platform at runtime based on compile-time target.
    #[must_use]
    pub fn detect() -> Self {
        #[cfg(target_os = "android")]
        {
            Self::Android
        }
        #[cfg(target_os = "ios")]
        {
            Self::IOS
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            Self::Desktop
        }
    }

    /// Whether the platform is a mobile device.
    #[must_use]
    pub fn is_mobile(self) -> bool {
        matches!(self, Self::Android | Self::IOS)
    }
}

/// How the node should behave when the app is backgrounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundMode {
    /// Full node with relay duties (desktop only).
    FullNode,
    /// Light node: no relay duties, sync only in foreground.
    LightNode,
    /// Sleep mode: disconnect entirely, reconnect on foreground.
    SleepMode,
}

/// Runtime resource limits derived from the mobile configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceLimits {
    /// Maximum number of gossip peers.
    pub max_peers: usize,
    /// Maximum storage in bytes.
    pub max_storage_bytes: u64,
    /// Whether relay duties are enabled.
    pub relay_enabled: bool,
    /// Bandwidth cap in bytes/sec (0 = unlimited).
    pub bandwidth_limit_bps: u64,
}

/// Configuration tuned for mobile platforms.
///
/// Provides sensible defaults for Android and iOS that respect mobile
/// constraints: limited storage, fewer connections, battery awareness,
/// and background-mode transitions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MobileConfig {
    /// Target platform.
    pub platform: Platform,
    /// Application data directory (persistent files, keystore, DB).
    pub data_dir: PathBuf,
    /// Cache directory (temporary, OS may purge).
    pub cache_dir: PathBuf,
    /// Maximum storage budget in megabytes.
    pub max_storage_mb: u64,
    /// Maximum concurrent gossip connections.
    pub max_connections: usize,
    /// Behaviour when the app enters the background.
    pub background_mode: BackgroundMode,
    /// When true, reduce resource usage when battery is low.
    pub battery_aware: bool,
}

/// Default storage cap for Android in megabytes.
const ANDROID_DEFAULT_STORAGE_MB: u64 = 256;
/// Default storage cap for iOS in megabytes.
const IOS_DEFAULT_STORAGE_MB: u64 = 200;
/// Default connection count for Android.
const ANDROID_DEFAULT_CONNECTIONS: usize = 4;
/// Default connection count for iOS.
const IOS_DEFAULT_CONNECTIONS: usize = 3;

/// Bandwidth limit applied in battery-aware mode (256 KiB/s).
const BATTERY_AWARE_BANDWIDTH_BPS: u64 = 256 * 1024;

impl MobileConfig {
    /// Sensible defaults for an Android device.
    #[must_use]
    pub fn android_defaults(data_dir: PathBuf, cache_dir: PathBuf) -> Self {
        Self {
            platform: Platform::Android,
            data_dir,
            cache_dir,
            max_storage_mb: ANDROID_DEFAULT_STORAGE_MB,
            max_connections: ANDROID_DEFAULT_CONNECTIONS,
            background_mode: BackgroundMode::LightNode,
            battery_aware: true,
        }
    }

    /// Sensible defaults for an iOS device.
    #[must_use]
    pub fn ios_defaults(data_dir: PathBuf, cache_dir: PathBuf) -> Self {
        Self {
            platform: Platform::IOS,
            data_dir,
            cache_dir,
            max_storage_mb: IOS_DEFAULT_STORAGE_MB,
            max_connections: IOS_DEFAULT_CONNECTIONS,
            background_mode: BackgroundMode::SleepMode,
            battery_aware: true,
        }
    }

    /// Convert this mobile config into a [`NodeConfig`] the node can use.
    #[must_use]
    pub fn to_node_config(&self) -> NodeConfig {
        let bandwidth_limit_bps = if self.battery_aware {
            BATTERY_AWARE_BANDWIDTH_BPS
        } else {
            0
        };

        NodeConfig {
            data_dir: self.data_dir.clone(),
            profile: ResourceProfile::Embedded,
            bandwidth_limit_bps,
            bootstrap_nodes: vec![],
            listen_addr: None,
            max_connections: self.max_connections,
            max_message_size: ephemera_config::DEFAULT_MAX_MESSAGE_SIZE,
            connection_timeout_secs: ephemera_config::DEFAULT_CONNECTION_TIMEOUT_SECS,
            transport: TransportConfig::default(),
            storage: StorageConfig {
                max_storage_bytes: self.max_storage_mb * 1024 * 1024,
                gc_interval_secs: 120, // less frequent GC on mobile
                ..StorageConfig::default()
            },
            gossip: GossipConfig {
                max_peers: self.max_connections,
                heartbeat_interval_secs: 45, // slower heartbeat on mobile
            },
            dht: DhtConfig::default(),
        }
    }

    /// Compute the runtime resource limits, factoring in background mode
    /// and battery awareness.
    #[must_use]
    pub fn resource_limits(&self) -> ResourceLimits {
        let (max_peers, relay_enabled, bandwidth_limit_bps) = match self.background_mode {
            BackgroundMode::FullNode => (self.max_connections, true, 0),
            BackgroundMode::LightNode => (self.max_connections / 2, false, BATTERY_AWARE_BANDWIDTH_BPS),
            BackgroundMode::SleepMode => (0, false, 0),
        };

        // Apply further restrictions when battery-aware.
        let max_peers = if self.battery_aware {
            max_peers.min(self.max_connections)
        } else {
            max_peers
        };

        ResourceLimits {
            max_peers,
            max_storage_bytes: self.max_storage_mb * 1024 * 1024,
            relay_enabled,
            bandwidth_limit_bps,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_data_dir() -> PathBuf {
        PathBuf::from("/tmp/ephemera-test-data")
    }

    fn test_cache_dir() -> PathBuf {
        PathBuf::from("/tmp/ephemera-test-cache")
    }

    #[test]
    fn test_mobile_config_android_defaults() {
        let cfg = MobileConfig::android_defaults(test_data_dir(), test_cache_dir());
        assert_eq!(cfg.platform, Platform::Android);
        assert_eq!(cfg.max_storage_mb, 256);
        assert_eq!(cfg.max_connections, 4);
        assert!(cfg.battery_aware);
        assert_eq!(cfg.background_mode, BackgroundMode::LightNode);
    }

    #[test]
    fn test_mobile_config_ios_defaults() {
        let cfg = MobileConfig::ios_defaults(test_data_dir(), test_cache_dir());
        assert_eq!(cfg.platform, Platform::IOS);
        assert_eq!(cfg.max_storage_mb, 200);
        assert_eq!(cfg.max_connections, 3);
        assert!(cfg.battery_aware);
        assert_eq!(cfg.background_mode, BackgroundMode::SleepMode);
    }

    #[test]
    fn test_to_node_config_storage_conversion() {
        let cfg = MobileConfig::android_defaults(test_data_dir(), test_cache_dir());
        let node_cfg = cfg.to_node_config();

        // 256 MB -> bytes
        assert_eq!(node_cfg.storage.max_storage_bytes, 256 * 1024 * 1024);
        assert_eq!(node_cfg.gossip.max_peers, 4);
        assert_eq!(node_cfg.profile, ResourceProfile::Embedded);
    }

    #[test]
    fn test_resource_limits_full_node() {
        let mut cfg = MobileConfig::android_defaults(test_data_dir(), test_cache_dir());
        cfg.background_mode = BackgroundMode::FullNode;
        cfg.battery_aware = false;

        let limits = cfg.resource_limits();
        assert_eq!(limits.max_peers, 4);
        assert!(limits.relay_enabled);
        assert_eq!(limits.bandwidth_limit_bps, 0);
    }

    #[test]
    fn test_resource_limits_light_node() {
        let mut cfg = MobileConfig::android_defaults(test_data_dir(), test_cache_dir());
        cfg.background_mode = BackgroundMode::LightNode;

        let limits = cfg.resource_limits();
        // LightNode halves the connections.
        assert_eq!(limits.max_peers, 2);
        assert!(!limits.relay_enabled);
        assert_eq!(limits.bandwidth_limit_bps, BATTERY_AWARE_BANDWIDTH_BPS);
    }

    #[test]
    fn test_resource_limits_sleep_mode() {
        let cfg = MobileConfig::ios_defaults(test_data_dir(), test_cache_dir());

        let limits = cfg.resource_limits();
        assert_eq!(limits.max_peers, 0);
        assert!(!limits.relay_enabled);
    }

    #[test]
    fn test_resource_limits_battery_aware_and_platform() {
        let mut cfg = MobileConfig::android_defaults(test_data_dir(), test_cache_dir());
        cfg.background_mode = BackgroundMode::FullNode;

        cfg.battery_aware = true;
        let limits_aware = cfg.resource_limits();
        cfg.battery_aware = false;
        let limits_unaware = cfg.resource_limits();
        assert!(limits_aware.max_peers <= cfg.max_connections);
        assert!(limits_unaware.max_peers <= cfg.max_connections);

        // Platform detection and is_mobile.
        assert_eq!(Platform::detect(), Platform::Desktop);
        assert!(Platform::Android.is_mobile());
        assert!(Platform::IOS.is_mobile());
        assert!(!Platform::Desktop.is_mobile());
    }
}
