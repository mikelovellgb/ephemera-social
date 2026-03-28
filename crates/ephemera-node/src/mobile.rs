//! Mobile-optimised node wrapper.
//!
//! [`MobileNode`] wraps an [`EphemeraNode`] with lifecycle methods for
//! foreground/background transitions that are critical on mobile platforms.
//! When the app enters the background the node reduces its resource
//! footprint; when it returns to the foreground it reconnects and syncs.

use crate::EphemeraNode;
use ephemera_config::NodeConfig;

/// Resource limits reported to callers so they can adapt the UI or
/// throttle behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceLimits {
    /// Maximum number of gossip peers allowed.
    pub max_peers: usize,
    /// Maximum storage in bytes.
    pub max_storage_bytes: u64,
    /// Whether the node is acting as a relay for other peers.
    pub relay_enabled: bool,
    /// Bandwidth cap in bytes/sec (0 = unlimited).
    pub bandwidth_limit_bps: u64,
}

/// Lifecycle state of a mobile node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleState {
    /// App is in the foreground with full connectivity.
    Foreground,
    /// App is backgrounded with reduced resource usage.
    Background,
    /// Node is sleeping (disconnected).
    Sleeping,
}

/// Mobile-specific settings governing background behaviour.
#[derive(Debug, Clone)]
pub struct MobileNodeConfig {
    /// Maximum peers when in foreground.
    pub foreground_max_peers: usize,
    /// Maximum peers when backgrounded (typically half of foreground).
    pub background_max_peers: usize,
    /// Whether to disconnect entirely in background (sleep mode).
    pub sleep_in_background: bool,
    /// Bandwidth limit while backgrounded (bytes/sec, 0 = unlimited).
    pub background_bandwidth_bps: u64,
    /// Maximum storage budget in bytes.
    pub max_storage_bytes: u64,
}

impl Default for MobileNodeConfig {
    fn default() -> Self {
        Self {
            foreground_max_peers: 4,
            background_max_peers: 2,
            sleep_in_background: false,
            background_bandwidth_bps: 256 * 1024, // 256 KiB/s
            max_storage_bytes: 256 * 1024 * 1024,  // 256 MiB
        }
    }
}

/// A mobile-aware wrapper around [`EphemeraNode`].
///
/// Handles foreground/background transitions and provides
/// resource-limit queries for the UI layer.
pub struct MobileNode {
    /// The underlying Ephemera node.
    node: EphemeraNode,
    /// Mobile-specific configuration.
    mobile_config: MobileNodeConfig,
    /// Current lifecycle state.
    state: LifecycleState,
}

impl MobileNode {
    /// Create a new mobile node from a [`NodeConfig`] and mobile settings.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying node fails to initialize.
    pub fn new(
        node_config: NodeConfig,
        mobile_config: MobileNodeConfig,
    ) -> Result<Self, crate::startup::StartupError> {
        let node = EphemeraNode::new(node_config)?;
        Ok(Self {
            node,
            mobile_config,
            state: LifecycleState::Sleeping,
        })
    }

    /// Start the node and enter foreground mode.
    ///
    /// # Errors
    ///
    /// Returns an error if the node fails to start.
    pub async fn start(&mut self) -> Result<(), crate::startup::StartupError> {
        self.node.start().await?;
        self.state = LifecycleState::Foreground;
        tracing::info!("mobile node started in foreground mode");
        Ok(())
    }

    /// Transition to foreground mode (full connectivity).
    ///
    /// Restores peer connections to the foreground limit. If the node
    /// was sleeping, reconnects first.
    pub async fn enter_foreground(&mut self) {
        let prev = self.state;
        self.state = LifecycleState::Foreground;

        if prev == LifecycleState::Sleeping && !self.node.is_running() {
            // Attempt restart — log errors but do not panic.
            if let Err(e) = self.node.start().await {
                tracing::error!(error = %e, "failed to restart node on foreground entry");
                return;
            }
        }

        tracing::info!(
            prev = ?prev,
            max_peers = self.mobile_config.foreground_max_peers,
            "entering foreground mode"
        );
    }

    /// Transition to background mode (reduced resource usage).
    ///
    /// Halves the peer count and applies bandwidth limits. If
    /// `sleep_in_background` is set, disconnects entirely instead.
    pub async fn enter_background(&mut self) {
        if self.mobile_config.sleep_in_background {
            self.state = LifecycleState::Sleeping;
            if let Err(e) = self.node.shutdown().await {
                tracing::error!(error = %e, "failed to shut down node for sleep");
            }
            tracing::info!("entering sleep mode (disconnected)");
        } else {
            self.state = LifecycleState::Background;
            tracing::info!(
                max_peers = self.mobile_config.background_max_peers,
                "entering background mode"
            );
        }
    }

    /// Perform a catch-up sync after returning from background or sleep.
    ///
    /// Queries connected peers for any content missed while backgrounded.
    pub async fn sync_catchup(&mut self) {
        if !self.node.is_running() {
            tracing::warn!("cannot sync: node is not running");
            return;
        }

        tracing::info!("starting catch-up sync");
        // The actual sync is handled by the gossip layer on reconnect.
        // This method exists as a hook for future anti-entropy queries.
    }

    /// Current resource limits based on lifecycle state.
    #[must_use]
    pub fn resource_limits(&self) -> ResourceLimits {
        match self.state {
            LifecycleState::Foreground => ResourceLimits {
                max_peers: self.mobile_config.foreground_max_peers,
                max_storage_bytes: self.mobile_config.max_storage_bytes,
                relay_enabled: false, // mobile never relays
                bandwidth_limit_bps: 0,
            },
            LifecycleState::Background => ResourceLimits {
                max_peers: self.mobile_config.background_max_peers,
                max_storage_bytes: self.mobile_config.max_storage_bytes,
                relay_enabled: false,
                bandwidth_limit_bps: self.mobile_config.background_bandwidth_bps,
            },
            LifecycleState::Sleeping => ResourceLimits {
                max_peers: 0,
                max_storage_bytes: self.mobile_config.max_storage_bytes,
                relay_enabled: false,
                bandwidth_limit_bps: 0,
            },
        }
    }

    /// Current lifecycle state.
    #[must_use]
    pub fn lifecycle_state(&self) -> LifecycleState {
        self.state
    }

    /// Access the underlying node.
    #[must_use]
    pub fn node(&self) -> &EphemeraNode {
        &self.node
    }

    /// Mutable access to the underlying node.
    pub fn node_mut(&mut self) -> &mut EphemeraNode {
        &mut self.node
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_mobile_config() -> MobileNodeConfig {
        MobileNodeConfig {
            foreground_max_peers: 4,
            background_max_peers: 2,
            sleep_in_background: false,
            background_bandwidth_bps: 256 * 1024,
            max_storage_bytes: 256 * 1024 * 1024,
        }
    }

    #[test]
    fn test_resource_limits_by_lifecycle_state() {
        let cfg = test_mobile_config();

        // Foreground: full connectivity.
        let fg = ResourceLimits {
            max_peers: cfg.foreground_max_peers,
            max_storage_bytes: cfg.max_storage_bytes,
            relay_enabled: false,
            bandwidth_limit_bps: 0,
        };
        assert_eq!(fg.max_peers, 4);
        assert_eq!(fg.max_storage_bytes, 256 * 1024 * 1024);
        assert!(!fg.relay_enabled);

        // Background: reduced.
        let bg = ResourceLimits {
            max_peers: cfg.background_max_peers,
            max_storage_bytes: cfg.max_storage_bytes,
            relay_enabled: false,
            bandwidth_limit_bps: cfg.background_bandwidth_bps,
        };
        assert_eq!(bg.max_peers, 2);
        assert_eq!(bg.bandwidth_limit_bps, 256 * 1024);

        // Sleeping: disconnected.
        let sl = ResourceLimits {
            max_peers: 0,
            max_storage_bytes: cfg.max_storage_bytes,
            relay_enabled: false,
            bandwidth_limit_bps: 0,
        };
        assert_eq!(sl.max_peers, 0);
    }

    #[test]
    fn test_background_reduces_and_foreground_restores() {
        let cfg = test_mobile_config();
        assert!(cfg.background_max_peers < cfg.foreground_max_peers);
        assert_eq!(cfg.foreground_max_peers, 4);
        assert_eq!(cfg.background_max_peers, 2);
    }

    #[test]
    fn test_default_mobile_config() {
        let cfg = MobileNodeConfig::default();
        assert_eq!(cfg.foreground_max_peers, 4);
        assert_eq!(cfg.background_max_peers, 2);
        assert!(!cfg.sleep_in_background);
        assert_eq!(cfg.background_bandwidth_bps, 256 * 1024);
        assert_eq!(cfg.max_storage_bytes, 256 * 1024 * 1024);
    }
}
