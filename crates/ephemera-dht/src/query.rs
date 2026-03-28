//! DHT query operations: iterative FIND_NODE and FIND_VALUE lookups.
//!
//! Implements the Kademlia iterative lookup algorithm:
//! 1. Select alpha closest nodes from the routing table.
//! 2. Send queries in parallel.
//! 3. Incorporate responses, selecting the next closest unqueried nodes.
//! 4. Repeat until convergence (no closer nodes discovered).

use crate::routing::RoutingTable;
use crate::{DhtConfig, DhtRecord};
use ephemera_types::NodeId;
use std::collections::{BTreeMap, HashSet};

/// State of a single iterative query.
pub struct DhtQuery {
    /// The target key we're looking for.
    target: [u8; 32],
    /// The local node ID (to avoid including ourselves in results).
    local_id: NodeId,
    /// Alpha parameter (concurrent lookups).
    alpha: usize,
    /// k parameter (result set size).
    k: usize,
    /// Nodes we've already queried.
    queried: HashSet<NodeId>,
    /// Candidate nodes ordered by distance to target.
    /// Key = XOR distance bytes, Value = NodeId.
    candidates: BTreeMap<[u8; 32], NodeId>,
    /// Whether the query is complete.
    finished: bool,
    /// If we found a value during FIND_VALUE.
    found_value: Option<DhtRecord>,
}

impl DhtQuery {
    /// Create a new query for the given target key.
    ///
    /// Seeds the candidate set from the routing table.
    pub fn new(target: [u8; 32], config: &DhtConfig, routing_table: &RoutingTable) -> Self {
        let target_node = NodeId::from_bytes(target);
        let seeds = routing_table.closest(&target_node, config.k);

        let mut candidates = BTreeMap::new();
        for entry in seeds {
            let dist = entry.id.xor_distance(&target_node);
            candidates.insert(dist, entry.id);
        }

        Self {
            target,
            local_id: *routing_table.local_id(),
            alpha: config.alpha,
            k: config.k,
            queried: HashSet::new(),
            candidates,
            finished: false,
            found_value: None,
        }
    }

    /// Get the next batch of nodes to query.
    ///
    /// Returns up to `alpha` unqueried nodes, closest to the target.
    pub fn next_to_query(&self) -> Vec<NodeId> {
        self.candidates
            .values()
            .filter(|id| !self.queried.contains(id))
            .take(self.alpha)
            .copied()
            .collect()
    }

    /// Record that we queried a node and received closer nodes in response.
    ///
    /// Returns `true` if we learned about any new, closer nodes.
    pub fn process_find_node_response(&mut self, from: NodeId, closer_nodes: &[NodeId]) -> bool {
        self.queried.insert(from);
        let target_node = NodeId::from_bytes(self.target);

        let mut learned_closer = false;
        for node in closer_nodes {
            if *node == self.local_id {
                continue; // Skip ourselves
            }
            let dist = node.xor_distance(&target_node);
            if let std::collections::btree_map::Entry::Vacant(entry) = self.candidates.entry(dist) {
                entry.insert(*node);
                learned_closer = true;
            }
        }

        // Check if we should terminate.
        let next = self.next_to_query();
        if next.is_empty() {
            self.finished = true;
        }

        learned_closer
    }

    /// Record that we found the value during a FIND_VALUE query.
    pub fn set_found_value(&mut self, record: DhtRecord) {
        self.found_value = Some(record);
        self.finished = true;
    }

    /// Whether the query has finished (converged or found a value).
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Get the found value, if any.
    pub fn take_found_value(&mut self) -> Option<DhtRecord> {
        self.found_value.take()
    }

    /// Get the k closest nodes discovered so far.
    pub fn closest_results(&self) -> Vec<NodeId> {
        self.candidates.values().take(self.k).copied().collect()
    }

    /// The target key.
    #[must_use]
    pub fn target(&self) -> &[u8; 32] {
        &self.target
    }

    /// How many nodes have been queried so far.
    #[must_use]
    pub fn queried_count(&self) -> usize {
        self.queried.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DhtConfig;

    fn make_routing_table() -> RoutingTable {
        let local = NodeId::from_bytes([0; 32]);
        let mut rt = RoutingTable::new(local, 20);
        for i in 1..=10u8 {
            use crate::routing::NodeEntry;
            rt.insert(NodeEntry::new(
                NodeId::from_bytes([i; 32]),
                vec!["127.0.0.1:4433".parse().unwrap()],
            ));
        }
        rt
    }

    #[test]
    fn query_initialization() {
        let rt = make_routing_table();
        let config = DhtConfig::default();
        let query = DhtQuery::new([5; 32], &config, &rt);
        assert!(!query.is_finished());
        assert_eq!(query.queried_count(), 0);
    }

    #[test]
    fn next_to_query_returns_alpha() {
        let rt = make_routing_table();
        let config = DhtConfig::default();
        let query = DhtQuery::new([5; 32], &config, &rt);
        let next = query.next_to_query();
        assert!(next.len() <= config.alpha);
        assert!(!next.is_empty());
    }

    #[test]
    fn query_processes_responses() {
        let rt = make_routing_table();
        let config = DhtConfig::default();
        let mut query = DhtQuery::new([5; 32], &config, &rt);

        let next = query.next_to_query();
        let from = next[0];
        let new_nodes = vec![NodeId::from_bytes([20; 32]), NodeId::from_bytes([21; 32])];
        let learned = query.process_find_node_response(from, &new_nodes);
        assert!(learned);
        assert_eq!(query.queried_count(), 1);
    }

    #[test]
    fn query_terminates_when_no_more_candidates() {
        let rt = make_routing_table();
        let config = DhtConfig {
            k: 3,
            alpha: 3,
            ..DhtConfig::default()
        };
        let mut query = DhtQuery::new([5; 32], &config, &rt);

        // Query all candidates without learning new nodes.
        for _ in 0..20 {
            let next = query.next_to_query();
            if next.is_empty() {
                break;
            }
            for node in next {
                query.process_find_node_response(node, &[]);
            }
        }
        assert!(query.is_finished());
    }
}
