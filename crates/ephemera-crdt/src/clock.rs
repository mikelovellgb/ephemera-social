//! Hybrid Logical Clock (HLC) for causal ordering across nodes.
//!
//! An HLC combines a physical wall-clock component with a logical counter
//! to produce timestamps that respect both real time and causal ordering.
//! This is the clock used by all CRDTs in this crate for merge decisions.

use ephemera_types::HlcTimestamp;
use serde::{Deserialize, Serialize};

/// A hybrid logical clock that can generate causally-ordered timestamps.
///
/// Each node maintains its own `HybridClock`. When a local event occurs,
/// call [`tick`](Self::tick). When a remote timestamp is received, call
/// [`update`](Self::update) to merge the remote time into the local clock.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridClock {
    latest: HlcTimestamp,
}

impl HybridClock {
    /// Create a new clock seeded from the current wall time.
    #[must_use]
    pub fn new() -> Self {
        Self {
            latest: HlcTimestamp::now(),
        }
    }

    /// Create a clock from a known starting timestamp (useful in tests).
    #[must_use]
    pub fn from_timestamp(ts: HlcTimestamp) -> Self {
        Self { latest: ts }
    }

    /// Generate a new timestamp for a local event.
    ///
    /// Advances the clock by at least one logical tick, ensuring the
    /// returned timestamp is strictly greater than any previously issued.
    pub fn tick(&mut self) -> HlcTimestamp {
        let now_wall = ephemera_types::Timestamp::now().as_secs();
        let local_wall = self.latest.wall_secs();

        if now_wall > local_wall {
            self.latest = HlcTimestamp::new(now_wall, 0);
        } else {
            // Wall clock hasn't advanced; bump the counter.
            self.latest = HlcTimestamp::new(local_wall, self.latest.counter() + 1);
        }
        self.latest
    }

    /// Merge a received remote timestamp into this clock and return the
    /// new local time. The result is guaranteed to be strictly greater
    /// than both the previous local time and the remote time.
    pub fn update(&mut self, remote: HlcTimestamp) -> HlcTimestamp {
        self.latest = self.latest.update(remote);
        self.latest
    }

    /// The most recent timestamp issued by this clock.
    #[must_use]
    pub fn latest(&self) -> HlcTimestamp {
        self.latest
    }
}

impl Default for HybridClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_monotonically_increases() {
        let mut clock = HybridClock::from_timestamp(HlcTimestamp::new(1000, 0));
        let t1 = clock.tick();
        let t2 = clock.tick();
        let t3 = clock.tick();
        assert!(t1 < t2);
        assert!(t2 < t3);
    }

    #[test]
    fn update_advances_past_remote() {
        let mut clock = HybridClock::from_timestamp(HlcTimestamp::new(100, 5));
        let remote = HlcTimestamp::new(200, 10);
        let result = clock.update(remote);
        assert!(result > remote);
    }

    #[test]
    fn update_advances_past_local() {
        let mut clock = HybridClock::from_timestamp(HlcTimestamp::new(300, 5));
        let remote = HlcTimestamp::new(100, 0);
        let result = clock.update(remote);
        assert!(result > HlcTimestamp::new(300, 5));
    }

    #[test]
    fn default_is_now() {
        let clock = HybridClock::default();
        // Should be a recent wall time (after 2024)
        assert!(clock.latest().wall_secs() > 1_704_067_200);
    }
}
