//! PlumTree-style gossip: hybrid eager push + lazy pull with O(log n) overhead.

use crate::fanout::FanoutConfig;
use crate::topic::{GossipMessage, GossipTopic, TopicSubscription};
use crate::GossipError;
use ephemera_types::NodeId;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use tokio::sync::mpsc;
use tracing;

/// State for a single gossip topic within the PlumTree protocol.
pub struct TopicState {
    topic: GossipTopic,
    eager_peers: HashSet<NodeId>,
    lazy_peers: HashSet<NodeId>,
    seen: HashSet<[u8; 32]>,
    subscriber_tx: Option<mpsc::Sender<GossipMessage>>,
}

impl TopicState {
    /// Create a new topic state with no peers.
    pub fn new(topic: GossipTopic) -> Self {
        Self {
            topic,
            eager_peers: HashSet::new(),
            lazy_peers: HashSet::new(),
            seen: HashSet::new(),
            subscriber_tx: None,
        }
    }

    /// Check if a message has already been seen.
    #[must_use]
    pub fn is_seen(&self, content_hash: &[u8; 32]) -> bool {
        self.seen.contains(content_hash)
    }

    /// Mark a message as seen.
    pub fn mark_seen(&mut self, content_hash: [u8; 32]) {
        self.seen.insert(content_hash);
    }

    /// Add a peer to the eager set.
    pub fn add_eager_peer(&mut self, peer: NodeId) {
        self.lazy_peers.remove(&peer);
        self.eager_peers.insert(peer);
    }

    /// Demote a peer from eager to lazy (prune).
    pub fn prune_to_lazy(&mut self, peer: &NodeId) {
        if self.eager_peers.remove(peer) {
            self.lazy_peers.insert(*peer);
            tracing::debug!(?peer, topic = %self.topic, "pruned peer to lazy");
        }
    }

    /// Promote a peer from lazy to eager (graft).
    pub fn graft_to_eager(&mut self, peer: &NodeId) {
        if self.lazy_peers.remove(peer) {
            self.eager_peers.insert(*peer);
            tracing::debug!(?peer, topic = %self.topic, "grafted peer to eager");
        }
    }

    /// Get the set of eager peers.
    #[must_use]
    pub fn eager_peers(&self) -> &HashSet<NodeId> {
        &self.eager_peers
    }

    /// Get the set of lazy peers.
    #[must_use]
    pub fn lazy_peers(&self) -> &HashSet<NodeId> {
        &self.lazy_peers
    }

    /// Total number of peers for this topic.
    #[must_use]
    pub fn peer_count(&self) -> usize {
        self.eager_peers.len() + self.lazy_peers.len()
    }
}

/// The PlumTree gossip engine, managing multiple topics.
pub struct PlumTreeEngine {
    local_id: NodeId,
    topics: HashMap<GossipTopic, TopicState>,
    _fanout: FanoutConfig,
    max_subscriptions: usize,
    dedup: MessageDedup,
}

impl PlumTreeEngine {
    /// Create a new PlumTree engine.
    pub fn new(local_id: NodeId, fanout: FanoutConfig, max_subscriptions: usize) -> Self {
        Self {
            local_id,
            topics: HashMap::new(),
            _fanout: fanout,
            max_subscriptions,
            dedup: MessageDedup::new(),
        }
    }

    /// Subscribe to a topic.
    pub fn subscribe(
        &mut self,
        topic: GossipTopic,
        buffer_size: usize,
    ) -> Result<TopicSubscription, GossipError> {
        if self.topics.len() >= self.max_subscriptions {
            return Err(GossipError::SubscriptionLimitReached {
                current: self.topics.len(),
                max: self.max_subscriptions,
            });
        }

        let (sub, tx) = TopicSubscription::new(topic, buffer_size);
        let mut state = TopicState::new(topic);
        state.subscriber_tx = Some(tx);
        self.topics.insert(topic, state);

        tracing::info!(%topic, "subscribed to gossip topic");
        Ok(sub)
    }

    /// Unsubscribe from a topic.
    pub fn unsubscribe(&mut self, topic: &GossipTopic) -> Result<(), GossipError> {
        if self.topics.remove(topic).is_none() {
            return Err(GossipError::NotSubscribed {
                topic: format!("{topic}"),
            });
        }
        tracing::info!(%topic, "unsubscribed from gossip topic");
        Ok(())
    }

    /// Process an incoming message on a topic.
    ///
    /// Returns the list of peers that should receive eager pushes
    /// and the list that should receive lazy IHave announcements.
    pub fn receive_message(
        &mut self,
        topic: &GossipTopic,
        content_hash: [u8; 32],
        payload: Vec<u8>,
        source: NodeId,
    ) -> Option<(Vec<NodeId>, Vec<NodeId>)> {
        // Global dedup check.
        if self.dedup.is_duplicate(&content_hash) {
            return None;
        }
        self.dedup.mark_seen(content_hash);

        let state = self.topics.get_mut(topic)?;

        if state.is_seen(&content_hash) {
            return None;
        }
        state.mark_seen(content_hash);

        // Deliver to local subscriber.
        if let Some(tx) = &state.subscriber_tx {
            let msg = GossipMessage {
                topic: *topic,
                payload: payload.clone(),
                content_hash,
                source_node: *source.as_bytes(),
            };
            // Best-effort delivery -- don't block if subscriber is slow.
            let _ = tx.try_send(msg);
        }

        let exclude = |p: &&NodeId| **p != source && **p != self.local_id;
        let eager: Vec<NodeId> = state
            .eager_peers()
            .iter()
            .filter(exclude)
            .copied()
            .collect();
        let lazy: Vec<NodeId> = state.lazy_peers().iter().filter(exclude).copied().collect();
        Some((eager, lazy))
    }

    /// List all subscribed topics.
    #[must_use]
    pub fn subscriptions(&self) -> Vec<GossipTopic> {
        self.topics.keys().copied().collect()
    }
}

/// Message deduplication using a time-rotated seen set (two epoch HashSets).
pub struct MessageDedup {
    current: HashSet<[u8; 32]>,
    previous: HashSet<[u8; 32]>,
    epoch_start: Instant,
    epoch_duration: std::time::Duration,
}

impl MessageDedup {
    /// Create a new deduplicator with a 5-minute epoch.
    pub fn new() -> Self {
        Self {
            current: HashSet::new(),
            previous: HashSet::new(),
            epoch_start: Instant::now(),
            epoch_duration: std::time::Duration::from_secs(300),
        }
    }

    /// Check if a content hash has been seen recently.
    pub fn is_duplicate(&mut self, hash: &[u8; 32]) -> bool {
        self.maybe_rotate();
        self.current.contains(hash) || self.previous.contains(hash)
    }

    /// Mark a hash as seen.
    pub fn mark_seen(&mut self, hash: [u8; 32]) {
        self.current.insert(hash);
    }

    /// Rotate epochs if enough time has passed.
    fn maybe_rotate(&mut self) {
        if self.epoch_start.elapsed() >= self.epoch_duration {
            self.previous = std::mem::take(&mut self.current);
            self.epoch_start = Instant::now();
        }
    }
}

impl Default for MessageDedup {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fanout::FanoutConfig;

    fn test_engine() -> PlumTreeEngine {
        PlumTreeEngine::new(NodeId::from_bytes([0; 32]), FanoutConfig::default(), 100)
    }

    #[test]
    fn subscribe_and_unsubscribe() {
        let mut engine = test_engine();
        let topic = GossipTopic::public_feed();
        let _sub = engine.subscribe(topic, 32).unwrap();
        assert_eq!(engine.subscriptions().len(), 1);
        engine.unsubscribe(&topic).unwrap();
        assert_eq!(engine.subscriptions().len(), 0);
    }

    #[test]
    fn subscription_limit() {
        let mut engine =
            PlumTreeEngine::new(NodeId::from_bytes([0; 32]), FanoutConfig::default(), 2);
        let _a = engine.subscribe(GossipTopic::public_feed(), 8).unwrap();
        let _b = engine.subscribe(GossipTopic::moderation(), 8).unwrap();
        let err = engine
            .subscribe(GossipTopic::bloom_updates(), 8)
            .unwrap_err();
        assert!(matches!(err, GossipError::SubscriptionLimitReached { .. }));
    }

    #[test]
    fn dedup_prevents_reprocessing() {
        let mut dedup = MessageDedup::new();
        let hash = [0xAA; 32];
        assert!(!dedup.is_duplicate(&hash));
        dedup.mark_seen(hash);
        assert!(dedup.is_duplicate(&hash));
    }

    #[test]
    fn topic_state_peer_management() {
        let mut state = TopicState::new(GossipTopic::public_feed());
        let peer = NodeId::from_bytes([1; 32]);
        state.add_eager_peer(peer);
        assert!(state.eager_peers().contains(&peer));
        assert_eq!(state.peer_count(), 1);

        state.prune_to_lazy(&peer);
        assert!(!state.eager_peers().contains(&peer));
        assert!(state.lazy_peers().contains(&peer));
        assert_eq!(state.peer_count(), 1);

        state.graft_to_eager(&peer);
        assert!(state.eager_peers().contains(&peer));
        assert!(!state.lazy_peers().contains(&peer));
    }
}
