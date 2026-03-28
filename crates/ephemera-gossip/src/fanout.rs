//! Fanout configuration and peer selection for gossip.
//!
//! Controls how many peers receive eager pushes vs. lazy IHave announcements,
//! based on the PlumTree protocol parameters from the architecture spec.

use ephemera_types::NodeId;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Configuration for gossip fanout parameters.
///
/// Based on 02_network_protocol.md Section 4.5.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FanoutConfig {
    /// Maximum message size in gossip (larger content uses chunked transfer).
    pub max_message_size: usize,
    /// Number of peers for eager push (PlumTree parameter).
    pub eager_push_peers: usize,
    /// Number of peers for lazy push / IHave (PlumTree parameter).
    pub lazy_push_peers: usize,
    /// Interval between IHave batches.
    pub ihave_interval: Duration,
    /// Timeout before switching from lazy to eager for a message.
    pub lazy_timeout: Duration,
    /// Maximum number of topics a node can subscribe to.
    pub max_subscriptions: usize,
}

impl Default for FanoutConfig {
    fn default() -> Self {
        Self {
            max_message_size: 64 * 1024, // 64 KiB
            eager_push_peers: 3,
            lazy_push_peers: 6,
            ihave_interval: Duration::from_millis(500),
            lazy_timeout: Duration::from_secs(2),
            max_subscriptions: 100,
        }
    }
}

impl FanoutConfig {
    /// Total fanout (eager + lazy).
    #[must_use]
    pub fn total_fanout(&self) -> usize {
        self.eager_push_peers + self.lazy_push_peers
    }
}

/// Select peers for eager and lazy push from a candidate set.
///
/// Randomly partitions the candidate peers into an eager set (of size
/// `eager_push_peers`) and a lazy set (the remainder, capped at
/// `lazy_push_peers`).
pub fn select_fanout(candidates: &[NodeId], config: &FanoutConfig) -> (Vec<NodeId>, Vec<NodeId>) {
    if candidates.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let mut shuffled: Vec<NodeId> = candidates.to_vec();
    let mut rng = rand::thread_rng();
    shuffled.shuffle(&mut rng);

    let eager_count = config.eager_push_peers.min(shuffled.len());
    let eager = shuffled[..eager_count].to_vec();

    let remaining = &shuffled[eager_count..];
    let lazy_count = config.lazy_push_peers.min(remaining.len());
    let lazy = remaining[..lazy_count].to_vec();

    (eager, lazy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let config = FanoutConfig::default();
        assert_eq!(config.eager_push_peers, 3);
        assert_eq!(config.lazy_push_peers, 6);
        assert_eq!(config.total_fanout(), 9);
        assert_eq!(config.max_message_size, 64 * 1024);
    }

    #[test]
    fn select_fanout_empty() {
        let config = FanoutConfig::default();
        let (eager, lazy) = select_fanout(&[], &config);
        assert!(eager.is_empty());
        assert!(lazy.is_empty());
    }

    #[test]
    fn select_fanout_fewer_than_eager() {
        let config = FanoutConfig::default();
        let candidates = vec![NodeId::from_bytes([1; 32])];
        let (eager, lazy) = select_fanout(&candidates, &config);
        assert_eq!(eager.len(), 1);
        assert!(lazy.is_empty());
    }

    #[test]
    fn select_fanout_normal() {
        let config = FanoutConfig::default();
        let candidates: Vec<NodeId> = (0..20u8).map(|i| NodeId::from_bytes([i; 32])).collect();
        let (eager, lazy) = select_fanout(&candidates, &config);
        assert_eq!(eager.len(), 3);
        assert_eq!(lazy.len(), 6);
        // Eager and lazy should not overlap.
        for e in &eager {
            assert!(!lazy.contains(e));
        }
    }
}
