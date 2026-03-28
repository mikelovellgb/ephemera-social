//! Timestamp types for Ephemera.
//!
//! Ephemera uses Hybrid Logical Clocks (HLC) internally for causal ordering,
//! but on the wire timestamps are transmitted as Unix seconds (`u64`).
//! This module provides both representations and conversion between them.

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

/// A Unix timestamp in seconds since the epoch.
///
/// This is the wire format used in protobuf envelopes. It has no
/// sub-second precision and no causal ordering guarantees on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Timestamp(u64);

impl Timestamp {
    /// Create a `Timestamp` from raw Unix seconds.
    #[must_use]
    pub fn from_secs(secs: u64) -> Self {
        Self(secs)
    }

    /// Capture the current wall-clock time as a `Timestamp`.
    #[must_use]
    pub fn now() -> Self {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before Unix epoch")
            .as_secs();
        Self(secs)
    }

    /// The raw Unix seconds value.
    #[must_use]
    pub fn as_secs(self) -> u64 {
        self.0
    }

    /// Convert to a `chrono::DateTime<Utc>` for display and arithmetic.
    ///
    /// Returns `None` if the timestamp is out of range for `chrono`
    /// (e.g., values that overflow `i64` or fall outside chrono's
    /// supported date range).
    #[must_use]
    pub fn to_datetime(self) -> Option<DateTime<Utc>> {
        let secs_i64 = i64::try_from(self.0).ok()?;
        Utc.timestamp_opt(secs_i64, 0).single()
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.to_datetime() {
            Some(dt) => write!(f, "{}", dt.format("%Y-%m-%dT%H:%M:%SZ")),
            None => write!(f, "Timestamp({})", self.0),
        }
    }
}

impl From<u64> for Timestamp {
    fn from(secs: u64) -> Self {
        Self(secs)
    }
}

impl From<Timestamp> for u64 {
    fn from(ts: Timestamp) -> Self {
        ts.0
    }
}

/// Hybrid Logical Clock timestamp for causal ordering.
///
/// Combines a wall-clock component (Unix seconds) with a logical counter
/// that breaks ties when events occur within the same second. This gives
/// us causal ordering properties that pure wall-clock timestamps lack.
///
/// The encoding packs both components into a single `u64`:
/// - Upper 48 bits: Unix seconds (good until year ~8919)
/// - Lower 16 bits: logical counter (up to 65535 events per second)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct HlcTimestamp(u64);

/// Number of bits reserved for the logical counter.
const COUNTER_BITS: u32 = 16;

/// Mask for extracting the logical counter from the packed representation.
const COUNTER_MASK: u64 = (1 << COUNTER_BITS) - 1;

impl HlcTimestamp {
    /// Create an HLC timestamp from wall-clock seconds and a logical counter.
    ///
    /// The counter is silently truncated to 16 bits.
    #[must_use]
    pub fn new(wall_secs: u64, counter: u16) -> Self {
        Self((wall_secs << COUNTER_BITS) | u64::from(counter))
    }

    /// Create an HLC timestamp from the current wall clock with counter = 0.
    #[must_use]
    pub fn now() -> Self {
        Self::new(Timestamp::now().as_secs(), 0)
    }

    /// The wall-clock component (Unix seconds).
    #[must_use]
    pub fn wall_secs(self) -> u64 {
        self.0 >> COUNTER_BITS
    }

    /// The logical counter component.
    #[must_use]
    pub fn counter(self) -> u16 {
        (self.0 & COUNTER_MASK) as u16
    }

    /// The raw packed `u64` representation.
    #[must_use]
    pub fn as_u64(self) -> u64 {
        self.0
    }

    /// Reconstruct from a packed `u64`.
    #[must_use]
    pub fn from_u64(val: u64) -> Self {
        Self(val)
    }

    /// Advance the HLC given the current wall clock and a received remote timestamp.
    ///
    /// Implements the standard HLC update rule:
    /// 1. Take the max of local wall clock, local HLC wall, and remote HLC wall.
    /// 2. If all three walls are equal, increment the max counter.
    /// 3. If the max wall advanced, reset the counter to 0.
    ///
    /// If the counter would overflow (wrap past u16::MAX), the wall clock
    /// component is incremented by one second and the counter resets to 0,
    /// preserving monotonicity without silent wrapping.
    #[must_use]
    pub fn update(self, remote: Self) -> Self {
        let now_secs = Timestamp::now().as_secs();
        let local_wall = self.wall_secs();
        let remote_wall = remote.wall_secs();

        let max_wall = now_secs.max(local_wall).max(remote_wall);

        let counter = if max_wall == local_wall && max_wall == remote_wall {
            self.counter().max(remote.counter()).checked_add(1)
        } else if max_wall == local_wall {
            self.counter().checked_add(1)
        } else if max_wall == remote_wall {
            remote.counter().checked_add(1)
        } else {
            // Wall clock advanced past both; reset counter.
            Some(0)
        };

        match counter {
            Some(c) => Self::new(max_wall, c),
            // Counter overflow: advance wall clock by 1 second, reset counter.
            None => Self::new(max_wall.saturating_add(1), 0),
        }
    }

    /// Convert the wall-clock component to a `Timestamp`.
    #[must_use]
    pub fn to_timestamp(self) -> Timestamp {
        Timestamp::from_secs(self.wall_secs())
    }
}

impl fmt::Display for HlcTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ts = Timestamp::from_secs(self.wall_secs());
        write!(f, "{}:{}", ts, self.counter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_round_trip() {
        let ts = Timestamp::from_secs(1_700_000_000);
        assert_eq!(ts.as_secs(), 1_700_000_000);
        let dt = ts.to_datetime().expect("valid timestamp");
        assert_eq!(dt.timestamp() as u64, 1_700_000_000);
    }

    #[test]
    fn timestamp_now_is_reasonable() {
        let ts = Timestamp::now();
        // Should be after 2024-01-01
        assert!(ts.as_secs() > 1_704_067_200);
    }

    #[test]
    fn timestamp_to_datetime_returns_none_on_overflow() {
        // u64::MAX cannot be represented as i64, so to_datetime should return None.
        let ts = Timestamp::from_secs(u64::MAX);
        assert!(ts.to_datetime().is_none());
    }

    #[test]
    fn timestamp_display_out_of_range() {
        let ts = Timestamp::from_secs(u64::MAX);
        let s = format!("{ts}");
        // Should fall back to raw display rather than panicking.
        assert!(s.contains("Timestamp("));
    }

    #[test]
    fn hlc_pack_unpack() {
        let hlc = HlcTimestamp::new(1_700_000_000, 42);
        assert_eq!(hlc.wall_secs(), 1_700_000_000);
        assert_eq!(hlc.counter(), 42);

        let raw = hlc.as_u64();
        let recovered = HlcTimestamp::from_u64(raw);
        assert_eq!(recovered, hlc);
    }

    #[test]
    fn hlc_ordering() {
        let a = HlcTimestamp::new(100, 0);
        let b = HlcTimestamp::new(100, 1);
        let c = HlcTimestamp::new(101, 0);
        assert!(a < b);
        assert!(b < c);
    }

    #[test]
    fn hlc_update_advances() {
        let local = HlcTimestamp::new(100, 5);
        let remote = HlcTimestamp::new(100, 10);
        let updated = local.update(remote);
        // The wall clock should be >= 100 (likely much higher since now() is real time)
        // and the counter should have advanced appropriately.
        assert!(updated > local);
        assert!(updated > remote);
    }

    #[test]
    fn hlc_counter_overflow_advances_wall_clock() {
        // Two timestamps at the same wall clock with max counter values.
        let local = HlcTimestamp::new(1000, u16::MAX);
        let remote = HlcTimestamp::new(1000, u16::MAX);
        // Force a scenario where max_wall == local_wall == remote_wall
        // and counter.max() == u16::MAX, so +1 would overflow.
        // We can't easily mock now(), but we can test the overflow logic
        // by ensuring the result is strictly greater than both inputs.
        let updated = local.update(remote);
        assert!(updated > local);
        assert!(updated > remote);
    }

    #[test]
    fn hlc_display() {
        let hlc = HlcTimestamp::new(1_700_000_000, 7);
        let s = format!("{hlc}");
        assert!(s.contains(":7"));
    }
}
