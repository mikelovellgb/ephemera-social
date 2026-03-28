//! Grow-only counter (G-Counter) CRDT.
//!
//! Each node increments its own slot; the global value is the sum of all
//! slots. Used in Ephemera for counting likes and reactions across nodes
//! without coordination.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A grow-only counter keyed by node identifier.
///
/// Each node may only increment its own slot. The total value is the sum
/// of all per-node counts. Merge takes the pointwise maximum.
///
/// The type parameter `A` is the actor/node identifier (typically a
/// `String` or fixed-size key).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GCounter<A: Ord + Clone> {
    counts: BTreeMap<A, u64>,
}

impl<A: Ord + Clone> GCounter<A> {
    /// Create an empty counter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            counts: BTreeMap::new(),
        }
    }

    /// Increment the counter for `actor` by `amount`.
    pub fn increment(&mut self, actor: A, amount: u64) {
        let entry = self.counts.entry(actor).or_insert(0);
        *entry = entry.saturating_add(amount);
    }

    /// The total value across all actors.
    #[must_use]
    pub fn value(&self) -> u64 {
        self.counts.values().sum()
    }

    /// The count attributed to a specific actor.
    #[must_use]
    pub fn actor_value(&self, actor: &A) -> u64 {
        self.counts.get(actor).copied().unwrap_or(0)
    }

    /// Merge another counter into this one (pointwise max).
    pub fn merge(&mut self, other: &Self) {
        for (actor, &count) in &other.counts {
            let entry = self.counts.entry(actor.clone()).or_insert(0);
            *entry = (*entry).max(count);
        }
    }

    /// Return an iterator over `(actor, count)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&A, &u64)> {
        self.counts.iter()
    }

    /// Number of distinct actors that have contributed.
    #[must_use]
    pub fn actor_count(&self) -> usize {
        self.counts.len()
    }
}

impl<A: Ord + Clone> Default for GCounter<A> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_counter_is_zero() {
        let c: GCounter<String> = GCounter::new();
        assert_eq!(c.value(), 0);
    }

    #[test]
    fn increment_and_read() {
        let mut c = GCounter::new();
        c.increment("alice", 3);
        c.increment("bob", 2);
        c.increment("alice", 1);
        assert_eq!(c.value(), 6);
        assert_eq!(c.actor_value(&"alice"), 4);
        assert_eq!(c.actor_value(&"bob"), 2);
    }

    #[test]
    fn merge_commutativity() {
        let mut a = GCounter::new();
        a.increment("x", 5);
        a.increment("y", 3);

        let mut b = GCounter::new();
        b.increment("y", 7);
        b.increment("z", 2);

        let mut ab = a.clone();
        ab.merge(&b);

        let mut ba = b.clone();
        ba.merge(&a);

        assert_eq!(ab, ba);
    }

    #[test]
    fn merge_associativity() {
        let mut a = GCounter::new();
        a.increment("x", 1);

        let mut b = GCounter::new();
        b.increment("y", 2);

        let mut c = GCounter::new();
        c.increment("z", 3);

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
    }

    #[test]
    fn merge_idempotency() {
        let mut a = GCounter::new();
        a.increment("x", 5);
        a.increment("y", 3);

        let before = a.clone();
        a.merge(&before);
        assert_eq!(a, before);
    }

    #[test]
    fn merge_takes_max() {
        let mut a = GCounter::new();
        a.increment("node1", 10);

        let mut b = GCounter::new();
        b.increment("node1", 7);
        b.increment("node2", 4);

        a.merge(&b);
        assert_eq!(a.actor_value(&"node1"), 10); // max(10, 7)
        assert_eq!(a.actor_value(&"node2"), 4);
        assert_eq!(a.value(), 14);
    }

    #[test]
    fn saturating_increment() {
        let mut c = GCounter::new();
        c.increment("x", u64::MAX);
        c.increment("x", 1);
        assert_eq!(c.actor_value(&"x"), u64::MAX);
    }

    #[test]
    fn actor_count() {
        let mut c = GCounter::new();
        assert_eq!(c.actor_count(), 0);
        c.increment("a", 1);
        c.increment("b", 1);
        assert_eq!(c.actor_count(), 2);
    }
}

#[cfg(test)]
mod proptest_tests {
    use super::*;
    use proptest::prelude::*;

    /// Generate a random GCounter by incrementing various actors.
    fn arb_gcounter(max_ops: usize) -> impl Strategy<Value = GCounter<String>> {
        proptest::collection::vec(
            (
                prop::string::string_regex("[a-z]{1,3}").unwrap(),
                0u64..1000,
            ),
            0..max_ops,
        )
        .prop_map(|ops| {
            let mut counter = GCounter::new();
            for (actor, amount) in ops {
                counter.increment(actor, amount);
            }
            counter
        })
    }

    proptest! {
        /// merge(a, b) == merge(b, a).
        #[test]
        fn prop_gcounter_merge_commutative(
            a in arb_gcounter(20),
            b in arb_gcounter(20),
        ) {
            let mut ab = a.clone();
            ab.merge(&b);
            let mut ba = b.clone();
            ba.merge(&a);
            prop_assert_eq!(ab, ba);
        }

        /// merge(merge(a, b), c) == merge(a, merge(b, c)).
        #[test]
        fn prop_gcounter_merge_associative(
            a in arb_gcounter(10),
            b in arb_gcounter(10),
            c in arb_gcounter(10),
        ) {
            let mut ab = a.clone();
            ab.merge(&b);
            let mut abc1 = ab;
            abc1.merge(&c);

            let mut bc = b.clone();
            bc.merge(&c);
            let mut abc2 = a.clone();
            abc2.merge(&bc);

            prop_assert_eq!(abc1, abc2);
        }

        /// merge(a, a) == a.
        #[test]
        fn prop_gcounter_merge_idempotent(a in arb_gcounter(20)) {
            let before = a.clone();
            let mut merged = a;
            merged.merge(&before);
            prop_assert_eq!(merged, before);
        }

        /// The total value after merge is >= max of the two individual totals.
        #[test]
        fn prop_gcounter_merge_monotone(
            a in arb_gcounter(20),
            b in arb_gcounter(20),
        ) {
            let val_a = a.value();
            let val_b = b.value();
            let mut merged = a;
            merged.merge(&b);
            prop_assert!(merged.value() >= val_a);
            prop_assert!(merged.value() >= val_b);
        }
    }
}
