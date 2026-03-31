//! Gossip topics and subscriptions.
//!
//! Topics are identified by 32-byte BLAKE3 hashes derived from well-known
//! domain strings. This module provides constructors for all standard
//! Ephemera topics and the [`TopicSubscription`] handle.

use serde::{Deserialize, Serialize};
use std::fmt;
use tokio::sync::mpsc;

/// A gossip topic identifier (32-byte BLAKE3 hash).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GossipTopic([u8; 32]);

impl GossipTopic {
    /// Create a topic from raw bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Return the raw bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Global public feed (all public posts).
    #[must_use]
    pub fn public_feed() -> Self {
        Self(*blake3::hash(b"ephemera-topic-public-feed-v1").as_bytes())
    }

    /// Per-pseudonym feed (posts from a specific author).
    #[must_use]
    pub fn author_feed(author_pubkey: &[u8; 32]) -> Self {
        let mut input = Vec::with_capacity(64);
        input.extend_from_slice(b"ephemera-topic-author-v1");
        input.extend_from_slice(author_pubkey);
        Self(*blake3::hash(&input).as_bytes())
    }

    /// User-created topic room.
    #[must_use]
    pub fn topic_room(room_id: &[u8]) -> Self {
        let mut input = Vec::with_capacity(32 + room_id.len());
        input.extend_from_slice(b"ephemera-topic-room-v1");
        input.extend_from_slice(room_id);
        Self(*blake3::hash(&input).as_bytes())
    }

    /// Moderation events (reports, votes, tombstones).
    #[must_use]
    pub fn moderation() -> Self {
        Self(*blake3::hash(b"ephemera-topic-moderation-v1").as_bytes())
    }

    /// CRDT sync for a neighborhood.
    #[must_use]
    pub fn crdt_sync(neighborhood_id: &[u8]) -> Self {
        let mut input = Vec::with_capacity(32 + neighborhood_id.len());
        input.extend_from_slice(b"ephemera-topic-crdt-v1");
        input.extend_from_slice(neighborhood_id);
        Self(*blake3::hash(&input).as_bytes())
    }

    /// Bloom filter updates (CSAM hash database).
    #[must_use]
    pub fn bloom_updates() -> Self {
        Self(*blake3::hash(b"ephemera-topic-bloom-v1").as_bytes())
    }

    /// Reactions gossip topic (reaction add/remove events).
    #[must_use]
    pub fn reactions() -> Self {
        Self(*blake3::hash(b"ephemera-topic-reactions-v1").as_bytes())
    }

    /// Direct message delivery topic.
    ///
    /// Dead drop envelopes are published on this topic so that recipient
    /// nodes (or relay nodes) can ingest and store them. The payload is a
    /// serialized [`DeadDropEnvelope`](crate::topic::GossipMessage) containing
    /// the mailbox key, message ID, sealed data, and TTL metadata.
    #[must_use]
    pub fn direct_messages() -> Self {
        Self(*blake3::hash(b"ephemera-topic-dm-delivery-v1").as_bytes())
    }

    /// DHT lookup topic for network-wide DHT queries and responses.
    ///
    /// When a local DHT lookup misses, a query is published on this topic.
    /// Peers that hold the requested key respond on the same topic with the
    /// value. This turns the local-only DHT into a network-queryable store.
    #[must_use]
    pub fn dht_lookup() -> Self {
        Self(*blake3::hash(b"ephemera-topic-dht-lookup-v1").as_bytes())
    }
}

impl fmt::Debug for GossipTopic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GossipTopic({})", hex::encode(&self.0[..8]))
    }
}

impl fmt::Display for GossipTopic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(&self.0[..8]))
    }
}

/// A message received on a gossip topic.
#[derive(Debug, Clone)]
pub struct GossipMessage {
    /// The topic this message arrived on.
    pub topic: GossipTopic,
    /// The message payload.
    pub payload: Vec<u8>,
    /// BLAKE3 hash of the payload (for deduplication).
    pub content_hash: [u8; 32],
    /// The node ID that forwarded this message to us.
    pub source_node: [u8; 32],
}

/// Handle for receiving messages on a subscribed topic.
///
/// Drop this handle to unsubscribe.
pub struct TopicSubscription {
    /// The subscribed topic.
    topic: GossipTopic,
    /// Channel for receiving messages.
    receiver: mpsc::Receiver<GossipMessage>,
}

impl std::fmt::Debug for TopicSubscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TopicSubscription")
            .field("topic", &self.topic)
            .finish()
    }
}

impl TopicSubscription {
    /// Create a new topic subscription with a bounded channel.
    pub fn new(topic: GossipTopic, buffer_size: usize) -> (Self, mpsc::Sender<GossipMessage>) {
        let (tx, rx) = mpsc::channel(buffer_size);
        (
            Self {
                topic,
                receiver: rx,
            },
            tx,
        )
    }

    /// The topic this subscription is for.
    #[must_use]
    pub fn topic(&self) -> &GossipTopic {
        &self.topic
    }

    /// Receive the next message on this topic.
    pub async fn recv(&mut self) -> Option<GossipMessage> {
        self.receiver.recv().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_feed_deterministic() {
        let a = GossipTopic::public_feed();
        let b = GossipTopic::public_feed();
        assert_eq!(a, b);
    }

    #[test]
    fn different_authors_different_topics() {
        let a = GossipTopic::author_feed(&[1u8; 32]);
        let b = GossipTopic::author_feed(&[2u8; 32]);
        assert_ne!(a, b);
    }

    #[test]
    fn standard_topics_are_distinct() {
        let topics = [
            GossipTopic::public_feed(),
            GossipTopic::moderation(),
            GossipTopic::bloom_updates(),
            GossipTopic::reactions(),
            GossipTopic::direct_messages(),
            GossipTopic::dht_lookup(),
        ];
        for (i, a) in topics.iter().enumerate() {
            for (j, b) in topics.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b);
                }
            }
        }
    }

    #[tokio::test]
    async fn subscription_channel() {
        let topic = GossipTopic::public_feed();
        let (mut sub, tx) = TopicSubscription::new(topic, 8);

        let msg = GossipMessage {
            topic,
            payload: vec![1, 2, 3],
            content_hash: *blake3::hash(&[1, 2, 3]).as_bytes(),
            source_node: [0; 32],
        };

        tx.send(msg.clone()).await.unwrap();
        let received = sub.recv().await.unwrap();
        assert_eq!(received.payload, vec![1, 2, 3]);
    }
}
