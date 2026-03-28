//! Time-to-live types for ephemeral content.
//!
//! Every piece of content in Ephemera has a bounded lifetime. The TTL is
//! a protocol-level invariant enforced at every layer: type system, network,
//! storage, and cryptography.

use crate::error::EphemeraError;
use crate::timestamp::Timestamp;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

/// Minimum TTL: 1 hour (3600 seconds).
pub const MIN_TTL_SECS: u64 = 3_600;

/// Maximum TTL: 30 days (2,592,000 seconds).
pub const MAX_TTL_SECS: u64 = 30 * 24 * 3_600;

/// Duration of one epoch for key rotation (24 hours).
pub const EPOCH_DURATION_SECS: u64 = 24 * 3_600;

/// Clock skew tolerance when validating incoming timestamps (5 minutes).
pub const CLOCK_SKEW_TOLERANCE_SECS: u64 = 5 * 60;

/// How long tombstones are retained beyond the original TTL (3x).
pub const TOMBSTONE_RETENTION_MULTIPLIER: u64 = 3;

/// A validated time-to-live duration, guaranteed to be within
/// [`MIN_TTL_SECS`]..=[`MAX_TTL_SECS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Ttl(u64);

impl Ttl {
    /// Create a `Ttl` from a number of seconds, validating the range.
    ///
    /// # Errors
    ///
    /// Returns [`EphemeraError::InvalidTtl`] if the value is outside the
    /// allowed range.
    pub fn from_secs(secs: u64) -> Result<Self, EphemeraError> {
        if !(MIN_TTL_SECS..=MAX_TTL_SECS).contains(&secs) {
            return Err(EphemeraError::InvalidTtl {
                value_secs: secs,
                min_secs: MIN_TTL_SECS,
                max_secs: MAX_TTL_SECS,
            });
        }
        Ok(Self(secs))
    }

    /// Create a `Ttl` of 1 hour (the minimum).
    #[must_use]
    pub fn one_hour() -> Self {
        Self(MIN_TTL_SECS)
    }

    /// Create a `Ttl` of 24 hours.
    #[must_use]
    pub fn one_day() -> Self {
        Self(24 * 3_600)
    }

    /// Create a `Ttl` of 7 days.
    #[must_use]
    pub fn one_week() -> Self {
        Self(7 * 24 * 3_600)
    }

    /// Create a `Ttl` of 30 days (the maximum).
    #[must_use]
    pub fn max() -> Self {
        Self(MAX_TTL_SECS)
    }

    /// The TTL in seconds.
    #[must_use]
    pub fn as_secs(self) -> u64 {
        self.0
    }

    /// Convert to a `std::time::Duration`.
    #[must_use]
    pub fn as_duration(self) -> Duration {
        Duration::from_secs(self.0)
    }
}

impl fmt::Display for Ttl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hours = self.0 / 3_600;
        if hours >= 24 && hours.is_multiple_of(24) {
            write!(f, "{}d", hours / 24)
        } else {
            write!(f, "{}h", hours)
        }
    }
}

/// The computed expiry instant for a piece of content.
///
/// Combines the creation timestamp with the TTL to determine when
/// the content should be garbage-collected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Expiry {
    /// When the content was created.
    created_at: Timestamp,
    /// The content's time-to-live.
    ttl: Ttl,
}

impl Expiry {
    /// Compute the expiry for content created at `created_at` with the given `ttl`.
    #[must_use]
    pub fn new(created_at: Timestamp, ttl: Ttl) -> Self {
        Self { created_at, ttl }
    }

    /// The Unix timestamp (seconds) at which this content expires.
    #[must_use]
    pub fn expires_at_secs(&self) -> u64 {
        self.created_at.as_secs() + self.ttl.as_secs()
    }

    /// Check whether this content has expired relative to the given timestamp.
    #[must_use]
    pub fn is_expired_at(&self, now: Timestamp) -> bool {
        now.as_secs() > self.expires_at_secs()
    }

    /// Check whether this content has expired right now.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        self.is_expired_at(Timestamp::now())
    }

    /// The Unix timestamp at which the tombstone for this content should
    /// itself be deleted (`expires_at + TOMBSTONE_RETENTION_MULTIPLIER * ttl`).
    #[must_use]
    pub fn tombstone_expires_at_secs(&self) -> u64 {
        self.expires_at_secs() + TOMBSTONE_RETENTION_MULTIPLIER * self.ttl.as_secs()
    }

    /// The creation timestamp.
    #[must_use]
    pub fn created_at(&self) -> Timestamp {
        self.created_at
    }

    /// The TTL.
    #[must_use]
    pub fn ttl(&self) -> Ttl {
        self.ttl
    }
}

impl fmt::Display for Expiry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let expires = Timestamp::from_secs(self.expires_at_secs());
        write!(f, "expires {expires}")
    }
}

/// Validate that an incoming timestamp is not too far in the future.
///
/// Returns `true` if the timestamp is within the acceptable clock skew tolerance
/// compared to the local wall clock.
#[must_use]
pub fn is_timestamp_acceptable(remote_ts: Timestamp) -> bool {
    let now = Timestamp::now().as_secs();
    remote_ts.as_secs() <= now + CLOCK_SKEW_TOLERANCE_SECS
}

/// Validate that a TTL has not already expired given the content's creation time.
///
/// Returns `true` if the content is still alive, accounting for clock skew.
#[must_use]
pub fn is_ttl_valid(created_at: Timestamp, ttl: Ttl) -> bool {
    let now = Timestamp::now().as_secs();
    let deadline = created_at.as_secs() + ttl.as_secs() + CLOCK_SKEW_TOLERANCE_SECS;
    now <= deadline
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttl_valid_range() {
        assert!(Ttl::from_secs(3_600).is_ok());
        assert!(Ttl::from_secs(MAX_TTL_SECS).is_ok());
        assert!(Ttl::from_secs(100_000).is_ok());
    }

    #[test]
    fn ttl_rejects_too_small() {
        let err = Ttl::from_secs(60).unwrap_err();
        assert!(matches!(err, EphemeraError::InvalidTtl { .. }));
    }

    #[test]
    fn ttl_rejects_too_large() {
        let err = Ttl::from_secs(MAX_TTL_SECS + 1).unwrap_err();
        assert!(matches!(err, EphemeraError::InvalidTtl { .. }));
    }

    #[test]
    fn ttl_display() {
        assert_eq!(Ttl::one_hour().to_string(), "1h");
        assert_eq!(Ttl::one_day().to_string(), "1d");
        assert_eq!(Ttl::one_week().to_string(), "7d");
        assert_eq!(Ttl::max().to_string(), "30d");
    }

    #[test]
    fn expiry_fresh_content_not_expired() {
        let now = Timestamp::now();
        let ttl = Ttl::one_day();
        let expiry = Expiry::new(now, ttl);
        assert!(!expiry.is_expired());
    }

    #[test]
    fn expiry_old_content_is_expired() {
        let created = Timestamp::from_secs(Timestamp::now().as_secs() - 2 * 24 * 3_600);
        let ttl = Ttl::one_hour();
        let expiry = Expiry::new(created, ttl);
        assert!(expiry.is_expired());
    }

    #[test]
    fn tombstone_retention() {
        let created = Timestamp::from_secs(1_000_000);
        let ttl = Ttl::one_day();
        let expiry = Expiry::new(created, ttl);
        let content_expires = expiry.expires_at_secs();
        let tombstone_expires = expiry.tombstone_expires_at_secs();
        assert_eq!(
            tombstone_expires - content_expires,
            TOMBSTONE_RETENTION_MULTIPLIER * ttl.as_secs()
        );
    }

    #[test]
    fn timestamp_acceptability() {
        let now = Timestamp::now();
        assert!(is_timestamp_acceptable(now));

        // 10 minutes in the future should be rejected.
        let future = Timestamp::from_secs(now.as_secs() + 600);
        assert!(!is_timestamp_acceptable(future));
    }
}

#[cfg(test)]
mod proptest_tests {
    use super::*;
    use crate::timestamp::HlcTimestamp;
    use proptest::prelude::*;

    proptest! {
        /// Any value in [MIN_TTL_SECS, MAX_TTL_SECS] should round-trip.
        #[test]
        fn prop_ttl_round_trip(secs in MIN_TTL_SECS..=MAX_TTL_SECS) {
            let ttl = Ttl::from_secs(secs).unwrap();
            prop_assert_eq!(ttl.as_secs(), secs);
            prop_assert_eq!(ttl.as_duration().as_secs(), secs);
        }

        /// Any value outside [MIN_TTL_SECS, MAX_TTL_SECS] should be rejected.
        #[test]
        fn prop_ttl_rejects_out_of_range(
            secs in prop::num::u64::ANY.prop_filter(
                "must be out of range",
                |&s| !(MIN_TTL_SECS..=MAX_TTL_SECS).contains(&s),
            )
        ) {
            prop_assert!(Ttl::from_secs(secs).is_err());
        }

        /// Timestamp round-trips through as_secs/from_secs.
        #[test]
        fn prop_timestamp_round_trip(secs in 0u64..=4_000_000_000u64) {
            let ts = Timestamp::from_secs(secs);
            prop_assert_eq!(ts.as_secs(), secs);
        }

        /// HlcTimestamp pack/unpack is lossless for valid wall/counter.
        #[test]
        fn prop_hlc_round_trip(
            wall in 0u64..=(1u64 << 48) - 1,
            counter in 0u16..=u16::MAX,
        ) {
            let hlc = HlcTimestamp::new(wall, counter);
            prop_assert_eq!(hlc.wall_secs(), wall);
            prop_assert_eq!(hlc.counter(), counter);
            let raw = hlc.as_u64();
            let recovered = HlcTimestamp::from_u64(raw);
            prop_assert_eq!(recovered, hlc);
        }

        /// Expiry round-trip: created_at + ttl matches expires_at_secs.
        #[test]
        fn prop_expiry_math(
            created_secs in 1_000_000_000u64..2_000_000_000u64,
            ttl_secs in MIN_TTL_SECS..=MAX_TTL_SECS,
        ) {
            let created = Timestamp::from_secs(created_secs);
            let ttl = Ttl::from_secs(ttl_secs).unwrap();
            let expiry = Expiry::new(created, ttl);
            prop_assert_eq!(expiry.expires_at_secs(), created_secs + ttl_secs);
            prop_assert_eq!(expiry.created_at().as_secs(), created_secs);
            prop_assert_eq!(expiry.ttl().as_secs(), ttl_secs);
        }
    }
}
