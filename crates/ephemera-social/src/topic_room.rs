//! Topic rooms: user-created discussion spaces.
//!
//! A topic room is a named channel where posts can be tagged and filtered.
//! Each room has its own gossip topic for P2P distribution.

use ephemera_types::Timestamp;
use serde::{Deserialize, Serialize};

use crate::feed::{FeedCursor, FeedPage};
use crate::SocialError;

/// A topic room that users can create, join, and post to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicRoom {
    /// BLAKE3 hash of the topic name, used as the unique identifier.
    pub topic_id: String,
    /// Human-readable name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Hex-encoded pubkey of the creator.
    pub created_by: String,
    /// When the room was created (Unix seconds).
    pub created_at: Timestamp,
}

/// A user's subscription to a topic room.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicSubscription {
    /// The topic room ID.
    pub topic_id: String,
    /// Hex-encoded pubkey of the subscriber.
    pub user_pubkey: String,
    /// When the subscription was created.
    pub subscribed_at: Timestamp,
}

/// Service trait for topic room operations.
#[async_trait::async_trait]
pub trait TopicRoomService: Send + Sync {
    /// Create a new topic room. The creator is auto-subscribed.
    async fn create_topic(
        &self,
        name: &str,
        description: Option<&str>,
        creator_pubkey: &str,
    ) -> Result<TopicRoom, SocialError>;

    /// Join (subscribe to) a topic room.
    async fn join_topic(
        &self,
        topic_id: &str,
        user_pubkey: &str,
    ) -> Result<(), SocialError>;

    /// Leave (unsubscribe from) a topic room.
    async fn leave_topic(
        &self,
        topic_id: &str,
        user_pubkey: &str,
    ) -> Result<(), SocialError>;

    /// List all known topic rooms.
    async fn list_topics(&self) -> Result<Vec<TopicRoom>, SocialError>;

    /// Get the feed of posts tagged with this topic.
    async fn get_topic_feed(
        &self,
        topic_id: &str,
        cursor: Option<&FeedCursor>,
        limit: u32,
    ) -> Result<FeedPage, SocialError>;

    /// Post to a topic room (link a post hash to a topic).
    async fn post_to_topic(
        &self,
        topic_id: &str,
        content_hash: &[u8],
        created_at: i64,
    ) -> Result<(), SocialError>;
}

/// Compute the topic ID from a room name.
///
/// Returns a hex-encoded BLAKE3 hash of the name.
#[must_use]
pub fn topic_id_from_name(name: &str) -> String {
    let hash = blake3::hash(name.as_bytes());
    hex::encode(hash.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_id_deterministic() {
        let a = topic_id_from_name("rust-lang");
        let b = topic_id_from_name("rust-lang");
        assert_eq!(a, b);
    }

    #[test]
    fn different_names_different_ids() {
        let a = topic_id_from_name("rust-lang");
        let b = topic_id_from_name("go-lang");
        assert_ne!(a, b);
    }

    #[test]
    fn topic_id_is_64_hex_chars() {
        let id = topic_id_from_name("test");
        assert_eq!(id.len(), 64);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
