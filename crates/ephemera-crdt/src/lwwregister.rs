//! Last-Writer-Wins Register (LWW-Register) CRDT.
//!
//! Stores a single value with a timestamp. On merge, the value with the
//! higher timestamp wins. Used in Ephemera for profile fields (display
//! name, bio, avatar) that can be updated from any node.

use ephemera_types::HlcTimestamp;
use serde::{Deserialize, Serialize};
use std::fmt;

/// A register that resolves concurrent writes by keeping the value with
/// the highest [`HlcTimestamp`].
///
/// The type parameter `V` is the stored value (e.g., `String` for a
/// profile display name).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LwwRegister<V: Clone> {
    value: V,
    timestamp: HlcTimestamp,
}

impl<V: Clone> LwwRegister<V> {
    /// Create a new register with an initial value and timestamp.
    pub fn new(value: V, timestamp: HlcTimestamp) -> Self {
        Self { value, timestamp }
    }

    /// Update the register if `timestamp` is strictly greater than the
    /// current one. Returns `true` if the value was replaced.
    pub fn set(&mut self, value: V, timestamp: HlcTimestamp) -> bool {
        if timestamp > self.timestamp {
            self.value = value;
            self.timestamp = timestamp;
            true
        } else {
            false
        }
    }

    /// Read the current value.
    #[must_use]
    pub fn value(&self) -> &V {
        &self.value
    }

    /// The timestamp of the current value.
    #[must_use]
    pub fn timestamp(&self) -> HlcTimestamp {
        self.timestamp
    }

    /// Merge another register into this one. The value with the higher
    /// timestamp survives. If timestamps are equal, `self` wins (stable
    /// tie-breaking favors the existing local state).
    pub fn merge(&mut self, other: &Self) {
        if other.timestamp > self.timestamp {
            self.value = other.value.clone();
            self.timestamp = other.timestamp;
        }
    }
}

impl<V: Clone + PartialEq> PartialEq for LwwRegister<V> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value && self.timestamp == other.timestamp
    }
}

impl<V: Clone + Eq> Eq for LwwRegister<V> {}

impl<V: Clone + fmt::Display> fmt::Display for LwwRegister<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} @{}", self.value, self.timestamp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(wall: u64, counter: u16) -> HlcTimestamp {
        HlcTimestamp::new(wall, counter)
    }

    #[test]
    fn new_and_read() {
        let reg = LwwRegister::new("hello".to_string(), ts(100, 0));
        assert_eq!(reg.value(), "hello");
        assert_eq!(reg.timestamp(), ts(100, 0));
    }

    #[test]
    fn set_with_newer_timestamp() {
        let mut reg = LwwRegister::new("old".to_string(), ts(100, 0));
        assert!(reg.set("new".to_string(), ts(200, 0)));
        assert_eq!(reg.value(), "new");
    }

    #[test]
    fn set_with_older_timestamp_is_no_op() {
        let mut reg = LwwRegister::new("current".to_string(), ts(200, 0));
        assert!(!reg.set("stale".to_string(), ts(100, 0)));
        assert_eq!(reg.value(), "current");
    }

    #[test]
    fn set_with_equal_timestamp_is_no_op() {
        let mut reg = LwwRegister::new("current".to_string(), ts(100, 0));
        assert!(!reg.set("same-time".to_string(), ts(100, 0)));
        assert_eq!(reg.value(), "current");
    }

    #[test]
    fn merge_commutativity() {
        let a = LwwRegister::new("a-val".to_string(), ts(100, 0));
        let b = LwwRegister::new("b-val".to_string(), ts(200, 0));

        let mut ab = a.clone();
        ab.merge(&b);

        let mut ba = b.clone();
        ba.merge(&a);

        assert_eq!(ab, ba);
        assert_eq!(ab.value(), "b-val");
    }

    #[test]
    fn merge_associativity() {
        let a = LwwRegister::new("a".to_string(), ts(100, 0));
        let b = LwwRegister::new("b".to_string(), ts(200, 0));
        let c = LwwRegister::new("c".to_string(), ts(150, 0));

        // (a merge b) merge c
        let mut ab = a.clone();
        ab.merge(&b);
        let mut abc1 = ab;
        abc1.merge(&c);

        // a merge (b merge c)
        let mut bc = b.clone();
        bc.merge(&c);
        let mut abc2 = a.clone();
        abc2.merge(&bc);

        assert_eq!(abc1, abc2);
        assert_eq!(abc1.value(), "b"); // 200 > 150 > 100
    }

    #[test]
    fn merge_idempotency() {
        let a = LwwRegister::new("val".to_string(), ts(100, 0));

        let mut merged = a.clone();
        merged.merge(&a);
        assert_eq!(merged, a);
    }

    #[test]
    fn merge_picks_higher_timestamp() {
        let mut local = LwwRegister::new("local".to_string(), ts(50, 0));
        let remote = LwwRegister::new("remote".to_string(), ts(100, 0));
        local.merge(&remote);
        assert_eq!(local.value(), "remote");
    }

    #[test]
    fn merge_keeps_local_on_equal_timestamp() {
        let mut local = LwwRegister::new("local".to_string(), ts(100, 0));
        let remote = LwwRegister::new("remote".to_string(), ts(100, 0));
        local.merge(&remote);
        assert_eq!(local.value(), "local");
    }

    #[test]
    fn counter_breaks_wall_clock_ties() {
        let mut reg = LwwRegister::new("old".to_string(), ts(100, 0));
        assert!(reg.set("new".to_string(), ts(100, 1)));
        assert_eq!(reg.value(), "new");
    }
}

#[cfg(test)]
mod proptest_tests {
    use super::*;
    use ephemera_types::HlcTimestamp;
    use proptest::prelude::*;

    fn arb_ts() -> impl Strategy<Value = HlcTimestamp> {
        (1u64..1_000_000u64, 0u16..1000u16).prop_map(|(w, c)| HlcTimestamp::new(w, c))
    }

    fn arb_register() -> impl Strategy<Value = LwwRegister<String>> {
        (any::<String>(), arb_ts()).prop_map(|(v, ts)| LwwRegister::new(v, ts))
    }

    proptest! {
        /// merge(a, b) == merge(b, a) (the winner is the same either way).
        #[test]
        fn prop_lww_merge_commutative(a in arb_register(), b in arb_register()) {
            let mut ab = a.clone();
            ab.merge(&b);
            let mut ba = b.clone();
            ba.merge(&a);
            prop_assert_eq!(ab.value(), ba.value());
            prop_assert_eq!(ab.timestamp(), ba.timestamp());
        }

        /// merge(merge(a, b), c) == merge(a, merge(b, c)).
        #[test]
        fn prop_lww_merge_associative(
            a in arb_register(),
            b in arb_register(),
            c in arb_register(),
        ) {
            let mut ab = a.clone();
            ab.merge(&b);
            let mut abc1 = ab;
            abc1.merge(&c);

            let mut bc = b.clone();
            bc.merge(&c);
            let mut abc2 = a.clone();
            abc2.merge(&bc);

            prop_assert_eq!(abc1.value(), abc2.value());
            prop_assert_eq!(abc1.timestamp(), abc2.timestamp());
        }

        /// merge(a, a) == a.
        #[test]
        fn prop_lww_merge_idempotent(a in arb_register()) {
            let before = a.clone();
            let mut merged = a;
            merged.merge(&before);
            prop_assert_eq!(merged.value(), before.value());
            prop_assert_eq!(merged.timestamp(), before.timestamp());
        }
    }
}
