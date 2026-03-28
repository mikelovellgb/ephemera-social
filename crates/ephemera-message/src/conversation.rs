//! Conversation tracking for 1:1 messaging.
//!
//! A `Conversation` represents an ongoing message thread between two
//! pseudonyms. `ConversationList` maintains the set of active conversations
//! ordered by last activity.

use ephemera_types::{IdentityKey, Timestamp};
use serde::{Deserialize, Serialize};

/// A 1:1 conversation with a single peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    /// The remote peer's pseudonym public key.
    pub peer_id: IdentityKey,
    /// Timestamp of the most recent message (sent or received).
    pub last_message_at: Timestamp,
    /// Number of unread messages from this peer.
    pub unread_count: u32,
}

impl Conversation {
    /// Create a new conversation record.
    pub fn new(peer_id: IdentityKey, last_message_at: Timestamp) -> Self {
        Self {
            peer_id,
            last_message_at,
            unread_count: 0,
        }
    }

    /// Record an incoming message, bumping the unread counter.
    pub fn record_incoming(&mut self, timestamp: Timestamp) {
        self.last_message_at = timestamp;
        self.unread_count = self.unread_count.saturating_add(1);
    }

    /// Record an outgoing message (no unread bump).
    pub fn record_outgoing(&mut self, timestamp: Timestamp) {
        self.last_message_at = timestamp;
    }

    /// Mark all messages as read, resetting the unread counter.
    pub fn mark_read(&mut self) {
        self.unread_count = 0;
    }
}

/// An ordered collection of conversations, sorted by most-recent activity.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConversationList {
    conversations: Vec<Conversation>,
}

impl ConversationList {
    /// Create an empty conversation list.
    pub fn new() -> Self {
        Self {
            conversations: Vec::new(),
        }
    }

    /// Return conversations ordered by `last_message_at` descending.
    pub fn list(&self) -> &[Conversation] {
        &self.conversations
    }

    /// Return the total number of conversations.
    pub fn len(&self) -> usize {
        self.conversations.len()
    }

    /// Whether the list is empty.
    pub fn is_empty(&self) -> bool {
        self.conversations.is_empty()
    }

    /// Find a conversation by peer identity, returning a mutable reference.
    pub fn get_mut(&mut self, peer_id: &IdentityKey) -> Option<&mut Conversation> {
        self.conversations
            .iter_mut()
            .find(|c| c.peer_id == *peer_id)
    }

    /// Find a conversation by peer identity.
    pub fn get(&self, peer_id: &IdentityKey) -> Option<&Conversation> {
        self.conversations.iter().find(|c| c.peer_id == *peer_id)
    }

    /// Insert or update a conversation for the given peer.
    ///
    /// If a conversation already exists, it is updated with the new
    /// timestamp and incremented unread count. Otherwise a new
    /// conversation is created. The list is re-sorted after mutation.
    pub fn upsert_incoming(&mut self, peer_id: IdentityKey, timestamp: Timestamp) {
        if let Some(conv) = self.get_mut(&peer_id) {
            conv.record_incoming(timestamp);
        } else {
            let mut conv = Conversation::new(peer_id, timestamp);
            conv.unread_count = 1;
            self.conversations.push(conv);
        }
        self.sort();
    }

    /// Record an outgoing message, creating the conversation if absent.
    pub fn upsert_outgoing(&mut self, peer_id: IdentityKey, timestamp: Timestamp) {
        if let Some(conv) = self.get_mut(&peer_id) {
            conv.record_outgoing(timestamp);
        } else {
            self.conversations
                .push(Conversation::new(peer_id, timestamp));
        }
        self.sort();
    }

    /// Remove a conversation entirely.
    pub fn remove(&mut self, peer_id: &IdentityKey) {
        self.conversations.retain(|c| c.peer_id != *peer_id);
    }

    /// Sort conversations by `last_message_at` descending.
    fn sort(&mut self) {
        self.conversations.sort_by(|a, b| {
            b.last_message_at
                .as_secs()
                .cmp(&a.last_message_at.as_secs())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(n: u8) -> IdentityKey {
        IdentityKey::from_bytes([n; 32])
    }

    #[test]
    fn new_conversation_has_zero_unread() {
        let conv = Conversation::new(peer(1), Timestamp::from_secs(1000));
        assert_eq!(conv.unread_count, 0);
    }

    #[test]
    fn record_incoming_increments_unread() {
        let mut conv = Conversation::new(peer(1), Timestamp::from_secs(1000));
        conv.record_incoming(Timestamp::from_secs(2000));
        assert_eq!(conv.unread_count, 1);
        conv.record_incoming(Timestamp::from_secs(3000));
        assert_eq!(conv.unread_count, 2);
    }

    #[test]
    fn mark_read_resets_unread() {
        let mut conv = Conversation::new(peer(1), Timestamp::from_secs(1000));
        conv.record_incoming(Timestamp::from_secs(2000));
        conv.mark_read();
        assert_eq!(conv.unread_count, 0);
    }

    #[test]
    fn conversation_list_ordering() {
        let mut list = ConversationList::new();
        list.upsert_incoming(peer(1), Timestamp::from_secs(100));
        list.upsert_incoming(peer(2), Timestamp::from_secs(200));
        list.upsert_incoming(peer(3), Timestamp::from_secs(150));

        let ordered = list.list();
        assert_eq!(ordered[0].peer_id, peer(2));
        assert_eq!(ordered[1].peer_id, peer(3));
        assert_eq!(ordered[2].peer_id, peer(1));
    }

    #[test]
    fn upsert_existing_updates_timestamp() {
        let mut list = ConversationList::new();
        list.upsert_incoming(peer(1), Timestamp::from_secs(100));
        list.upsert_incoming(peer(2), Timestamp::from_secs(200));
        // peer(1) sends a newer message, should move to top
        list.upsert_incoming(peer(1), Timestamp::from_secs(300));

        assert_eq!(list.list()[0].peer_id, peer(1));
        assert_eq!(list.list()[0].unread_count, 2);
    }

    #[test]
    fn remove_conversation() {
        let mut list = ConversationList::new();
        list.upsert_incoming(peer(1), Timestamp::from_secs(100));
        list.upsert_incoming(peer(2), Timestamp::from_secs(200));
        assert_eq!(list.len(), 2);

        list.remove(&peer(1));
        assert_eq!(list.len(), 1);
        assert!(list.get(&peer(1)).is_none());
    }
}
