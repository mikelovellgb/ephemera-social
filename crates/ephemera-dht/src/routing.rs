//! Kademlia routing table with k-buckets (256 buckets, LRU eviction).

use ephemera_types::NodeId;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::Instant;

/// Number of bits in a node ID.
const ID_BITS: usize = 256;

/// Entry in a k-bucket representing a known node.
#[derive(Debug, Clone)]
pub struct NodeEntry {
    /// The node's identifier.
    pub id: NodeId,
    /// Known socket addresses.
    pub addresses: Vec<SocketAddr>,
    /// When we last heard from this node.
    pub last_seen: Instant,
    /// Last round-trip time.
    pub last_rtt: Option<std::time::Duration>,
    /// Consecutive ping failures.
    pub failures: u32,
}

impl NodeEntry {
    /// Create a new node entry.
    pub fn new(id: NodeId, addresses: Vec<SocketAddr>) -> Self {
        Self {
            id,
            addresses,
            last_seen: Instant::now(),
            last_rtt: None,
            failures: 0,
        }
    }

    /// Mark this node as recently seen.
    pub fn mark_seen(&mut self) {
        self.last_seen = Instant::now();
        self.failures = 0;
    }

    /// Record a failed contact attempt.
    pub fn record_failure(&mut self) {
        self.failures += 1;
    }

    /// Whether this node should be considered stale.
    pub fn is_stale(&self, timeout: std::time::Duration) -> bool {
        self.last_seen.elapsed() > timeout
    }
}

/// A single k-bucket in the routing table.
#[derive(Debug)]
pub struct KBucket {
    entries: VecDeque<NodeEntry>,
    replacement_cache: VecDeque<NodeEntry>,
    k: usize,
}

impl KBucket {
    /// Create an empty bucket.
    pub fn new(k: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(k),
            replacement_cache: VecDeque::with_capacity(k),
            k,
        }
    }

    /// Number of entries in this bucket.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether this bucket is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether this bucket is full.
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.entries.len() >= self.k
    }

    /// Try to insert or update a node entry.
    ///
    /// Returns `InsertResult` indicating what action should be taken.
    pub fn insert(&mut self, entry: NodeEntry) -> InsertResult {
        // If the node is already in the bucket, move it to the tail (most recent).
        if let Some(pos) = self.entries.iter().position(|e| e.id == entry.id) {
            self.entries.remove(pos);
            self.entries.push_back(entry);
            return InsertResult::Updated;
        }

        // If the bucket is not full, insert at the tail.
        if !self.is_full() {
            self.entries.push_back(entry);
            return InsertResult::Inserted;
        }

        // Bucket is full. Add to replacement cache and signal a ping needed.
        if self.replacement_cache.len() >= self.k {
            self.replacement_cache.pop_front();
        }
        self.replacement_cache.push_back(entry);

        // The head (least-recently-seen) node should be pinged.
        let lrs_id = self.entries.front().map(|e| e.id);
        InsertResult::BucketFull {
            ping_target: lrs_id,
        }
    }

    /// Evict the oldest entry (after it failed to respond to a ping)
    /// and promote the newest from the replacement cache.
    pub fn evict_oldest(&mut self) -> Option<NodeEntry> {
        let evicted = self.entries.pop_front();
        if let Some(replacement) = self.replacement_cache.pop_back() {
            self.entries.push_back(replacement);
        }
        evicted
    }

    /// Get all entries as a slice.
    pub fn entries(&self) -> &VecDeque<NodeEntry> {
        &self.entries
    }

    /// Get the N closest entries to a target ID.
    pub fn closest_to(&self, target: &NodeId, n: usize) -> Vec<&NodeEntry> {
        let mut v: Vec<_> = self.entries.iter().collect();
        v.sort_by_key(|e| e.id.xor_distance(target));
        v.truncate(n);
        v
    }
}

/// Result of attempting to insert into a k-bucket.
#[derive(Debug)]
pub enum InsertResult {
    /// The entry was inserted into the bucket.
    Inserted,
    /// An existing entry was updated (moved to tail).
    Updated,
    /// The bucket is full; ping the indicated node to check liveness.
    BucketFull {
        /// The least-recently-seen node that should be pinged.
        ping_target: Option<NodeId>,
    },
}

/// The Kademlia routing table (256 k-buckets).
pub struct RoutingTable {
    local_id: NodeId,
    buckets: Vec<KBucket>,
    _k: usize,
}

impl RoutingTable {
    /// Create a new routing table for the given local node ID.
    pub fn new(local_id: NodeId, k: usize) -> Self {
        let buckets = (0..ID_BITS).map(|_| KBucket::new(k)).collect();
        Self {
            local_id,
            buckets,
            _k: k,
        }
    }

    /// The local node ID.
    #[must_use]
    pub fn local_id(&self) -> &NodeId {
        &self.local_id
    }

    /// Determine which bucket a node belongs in based on XOR distance.
    ///
    /// Returns `None` if the node ID is our own.
    fn bucket_index(&self, id: &NodeId) -> Option<usize> {
        self.local_id.leading_zeros_distance(id)
    }

    /// Insert or update a node in the routing table.
    pub fn insert(&mut self, entry: NodeEntry) -> InsertResult {
        if entry.id == self.local_id {
            return InsertResult::Updated; // Ignore ourselves
        }

        let idx = match self.bucket_index(&entry.id) {
            Some(idx) => idx,
            None => return InsertResult::Updated,
        };

        self.buckets[idx].insert(entry)
    }

    /// Find the `count` closest nodes to a target ID.
    pub fn closest(&self, target: &NodeId, count: usize) -> Vec<NodeEntry> {
        let mut all: Vec<&NodeEntry> = self
            .buckets
            .iter()
            .flat_map(|b| b.entries().iter())
            .collect();
        all.sort_by_key(|e| e.id.xor_distance(target));
        all.into_iter().take(count).cloned().collect()
    }

    /// Total number of entries across all buckets.
    #[must_use]
    pub fn total_entries(&self) -> usize {
        self.buckets.iter().map(KBucket::len).sum()
    }

    /// Get a reference to a specific bucket.
    pub fn bucket(&self, index: usize) -> Option<&KBucket> {
        self.buckets.get(index)
    }
}

#[cfg(test)]
#[path = "routing_tests.rs"]
mod tests;
