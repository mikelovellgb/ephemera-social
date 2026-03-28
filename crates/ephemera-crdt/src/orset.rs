//! Observed-Remove Set (OR-Set) CRDT.
//!
//! An OR-Set allows both additions and removals. Each add is tagged with a
//! globally unique marker so that concurrent add/remove operations resolve
//! correctly: an element is in the set if it has at least one add-tag that
//! has not been removed.
//!
//! Used in Ephemera for managing follower/connection sets where adds and
//! removes can happen concurrently on different nodes.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// A unique tag assigned to each add operation.
///
/// Composed of the actor that performed the add and a per-actor sequence
/// number, making it globally unique without coordination.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct UniqueTag<A: Ord + Clone> {
    /// The actor (node) that created this tag.
    pub actor: A,
    /// Per-actor monotonic sequence number.
    pub seq: u64,
}

/// An Observed-Remove Set.
///
/// Elements are present if they have at least one live (non-removed) tag.
/// Adding an element creates a fresh unique tag. Removing an element
/// discards all *currently observed* tags for that element (but not tags
/// created concurrently on other nodes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrSet<A: Ord + Clone, V: Ord + Clone> {
    /// Map from values to their set of live unique tags.
    entries: BTreeMap<V, BTreeSet<UniqueTag<A>>>,
    /// Per-actor sequence counter (next value to assign).
    counters: BTreeMap<A, u64>,
}

impl<A: Ord + Clone, V: Ord + Clone> OrSet<A, V> {
    /// Create an empty OR-Set.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            counters: BTreeMap::new(),
        }
    }

    /// Add an element to the set, returning the unique tag created.
    pub fn add(&mut self, actor: A, value: V) -> UniqueTag<A> {
        let seq = self.counters.entry(actor.clone()).or_insert(0);
        *seq += 1;
        let tag = UniqueTag { actor, seq: *seq };
        self.entries.entry(value).or_default().insert(tag.clone());
        tag
    }

    /// Remove an element by discarding all its currently observed tags.
    ///
    /// Returns `true` if the element was present (and thus removed).
    pub fn remove(&mut self, value: &V) -> bool {
        self.entries.remove(value).is_some()
    }

    /// Check whether a value is in the set.
    #[must_use]
    pub fn contains(&self, value: &V) -> bool {
        self.entries.get(value).is_some_and(|tags| !tags.is_empty())
    }

    /// Return all values currently in the set.
    #[must_use]
    pub fn elements(&self) -> Vec<&V> {
        self.entries
            .iter()
            .filter(|(_, tags)| !tags.is_empty())
            .map(|(v, _)| v)
            .collect()
    }

    /// Number of elements currently in the set.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries
            .values()
            .filter(|tags| !tags.is_empty())
            .count()
    }

    /// Whether the set is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Merge another OR-Set into this one.
    ///
    /// For each element, the live tags after merge are the union of tags
    /// from both sides. Tags removed on one side (absent from that side)
    /// will not be present, while tags added concurrently on the other
    /// side are preserved.
    pub fn merge(&mut self, other: &Self) {
        // Collect all keys as owned values to avoid borrowing self.entries
        // immutably while we need to mutate it.
        let all_values: BTreeSet<V> = self
            .entries
            .keys()
            .chain(other.entries.keys())
            .cloned()
            .collect();

        let empty = BTreeSet::new();
        for value in &all_values {
            let local_tags = self.entries.get(value).unwrap_or(&empty);
            let remote_tags = other.entries.get(value).unwrap_or(&empty);
            let merged: BTreeSet<UniqueTag<A>> = local_tags.union(remote_tags).cloned().collect();

            if merged.is_empty() {
                self.entries.remove(value);
            } else {
                self.entries.insert(value.clone(), merged);
            }
        }

        // Merge counters (take max per actor).
        for (actor, &remote_seq) in &other.counters {
            let local_seq = self.counters.entry(actor.clone()).or_insert(0);
            *local_seq = (*local_seq).max(remote_seq);
        }
    }
}

impl<A: Ord + Clone, V: Ord + Clone> Default for OrSet<A, V> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_contains() {
        let mut s: OrSet<String, String> = OrSet::new();
        s.add("node1".into(), "alice".into());
        assert!(s.contains(&"alice".into()));
        assert!(!s.contains(&"bob".into()));
    }

    #[test]
    fn remove_element() {
        let mut s: OrSet<String, String> = OrSet::new();
        s.add("node1".into(), "alice".into());
        assert!(s.remove(&"alice".into()));
        assert!(!s.contains(&"alice".into()));
    }

    #[test]
    fn remove_nonexistent_returns_false() {
        let mut s: OrSet<String, String> = OrSet::new();
        assert!(!s.remove(&"ghost".into()));
    }

    #[test]
    fn merge_commutativity() {
        let mut a: OrSet<String, String> = OrSet::new();
        a.add("n1".into(), "x".into());
        a.add("n1".into(), "y".into());

        let mut b: OrSet<String, String> = OrSet::new();
        b.add("n2".into(), "y".into());
        b.add("n2".into(), "z".into());

        let mut ab = a.clone();
        ab.merge(&b);

        let mut ba = b.clone();
        ba.merge(&a);

        assert_eq!(ab.elements().len(), ba.elements().len());
        for elem in ab.elements() {
            assert!(ba.contains(elem));
        }
        for elem in ba.elements() {
            assert!(ab.contains(elem));
        }
    }

    #[test]
    fn merge_associativity() {
        let mut a: OrSet<String, i32> = OrSet::new();
        a.add("n1".into(), 1);

        let mut b: OrSet<String, i32> = OrSet::new();
        b.add("n2".into(), 2);

        let mut c: OrSet<String, i32> = OrSet::new();
        c.add("n3".into(), 3);

        let mut ab = a.clone();
        ab.merge(&b);
        let mut abc1 = ab;
        abc1.merge(&c);

        let mut bc = b.clone();
        bc.merge(&c);
        let mut abc2 = a.clone();
        abc2.merge(&bc);

        assert_eq!(abc1.len(), abc2.len());
        for elem in abc1.elements() {
            assert!(abc2.contains(elem));
        }
    }

    #[test]
    fn merge_idempotency() {
        let mut a: OrSet<String, String> = OrSet::new();
        a.add("n1".into(), "x".into());
        a.add("n1".into(), "y".into());

        let snapshot = a.clone();
        a.merge(&snapshot);

        assert_eq!(a.len(), snapshot.len());
        for elem in snapshot.elements() {
            assert!(a.contains(elem));
        }
    }

    #[test]
    fn concurrent_add_and_remove() {
        // Node A adds "x", node B independently adds "x" then removes it.
        // After merge, "x" should be present because A's add was concurrent
        // with B's remove (B never saw A's tag).
        let mut a: OrSet<String, String> = OrSet::new();
        a.add("nodeA".into(), "x".into());

        let mut b: OrSet<String, String> = OrSet::new();
        b.add("nodeB".into(), "x".into());
        b.remove(&"x".into());

        a.merge(&b);
        assert!(a.contains(&"x".into()));
    }

    #[test]
    fn len_and_is_empty() {
        let mut s: OrSet<String, i32> = OrSet::new();
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);

        s.add("n".into(), 1);
        s.add("n".into(), 2);
        assert_eq!(s.len(), 2);
        assert!(!s.is_empty());

        s.remove(&1);
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn elements_returns_all_present() {
        let mut s: OrSet<String, i32> = OrSet::new();
        s.add("n".into(), 10);
        s.add("n".into(), 20);
        s.add("n".into(), 30);
        s.remove(&20);

        let mut elems: Vec<i32> = s.elements().into_iter().copied().collect();
        elems.sort();
        assert_eq!(elems, vec![10, 30]);
    }
}

#[cfg(test)]
mod proptest_tests {
    use super::*;
    use proptest::prelude::*;

    /// Generate a random OrSet by applying a sequence of add/remove operations.
    fn arb_orset(max_ops: usize) -> impl Strategy<Value = OrSet<String, i32>> {
        proptest::collection::vec(
            (
                prop::bool::ANY,
                prop::string::string_regex("[a-z]{1,4}").unwrap(),
                -100i32..100i32,
            ),
            0..max_ops,
        )
        .prop_map(|ops| {
            let mut set = OrSet::new();
            for (is_add, actor, value) in ops {
                if is_add {
                    set.add(actor, value);
                } else {
                    set.remove(&value);
                }
            }
            set
        })
    }

    proptest! {
        /// merge(a, b) produces the same elements as merge(b, a).
        #[test]
        fn prop_orset_merge_commutative(
            a in arb_orset(20),
            b in arb_orset(20),
        ) {
            let mut ab = a.clone();
            ab.merge(&b);

            let mut ba = b.clone();
            ba.merge(&a);

            let mut elems_ab: Vec<&i32> = ab.elements();
            elems_ab.sort();
            let mut elems_ba: Vec<&i32> = ba.elements();
            elems_ba.sort();

            prop_assert_eq!(elems_ab, elems_ba);
        }

        /// merge(merge(a, b), c) == merge(a, merge(b, c)).
        #[test]
        fn prop_orset_merge_associative(
            a in arb_orset(10),
            b in arb_orset(10),
            c in arb_orset(10),
        ) {
            let mut ab = a.clone();
            ab.merge(&b);
            let mut abc1 = ab;
            abc1.merge(&c);

            let mut bc = b.clone();
            bc.merge(&c);
            let mut abc2 = a.clone();
            abc2.merge(&bc);

            let mut e1: Vec<&i32> = abc1.elements();
            e1.sort();
            let mut e2: Vec<&i32> = abc2.elements();
            e2.sort();

            prop_assert_eq!(e1, e2);
        }

        /// merge(a, a) == a (idempotency).
        #[test]
        fn prop_orset_merge_idempotent(a in arb_orset(20)) {
            let before = a.clone();
            let mut merged = a;
            merged.merge(&before);

            let mut e_before: Vec<&i32> = before.elements();
            e_before.sort();
            let mut e_merged: Vec<&i32> = merged.elements();
            e_merged.sort();

            prop_assert_eq!(e_before, e_merged);
        }
    }
}
