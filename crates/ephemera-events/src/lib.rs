//! Internal event bus for decoupling Ephemera subsystems.
//!
//! Uses tokio broadcast channels to provide a publish-subscribe mechanism.
//! Any subsystem can emit events, and any number of subscribers (including
//! the Tauri event bridge) can receive them.

use ephemera_types::{ContentId, IdentityKey, NodeId};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// Default channel capacity for the event bus.
pub const DEFAULT_CHANNEL_CAPACITY: usize = 1024;

/// Internal events emitted by Ephemera subsystems.
///
/// These events flow through the event bus and can be consumed by
/// any subscriber, including the frontend event bridge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    // ── Content events ──────────────────────────────────────────────
    /// A new post was received from the network and stored locally.
    PostReceived {
        /// The content identifier of the new post.
        content_id: ContentId,
        /// The pseudonym that authored the post.
        author: IdentityKey,
    },

    /// A post was created locally and is ready for network publication.
    PostCreated {
        /// The content identifier of the created post.
        content_id: ContentId,
    },

    /// A post has expired and been garbage-collected.
    PostExpired {
        /// The content identifier of the expired post.
        content_id: ContentId,
    },

    /// A content deletion tombstone was received.
    ContentDeleted {
        /// The content identifier of the deleted content.
        content_id: ContentId,
    },

    // ── Messaging events ────────────────────────────────────────────
    /// A new direct message was received.
    MessageReceived {
        /// The pseudonym that sent the message.
        from: IdentityKey,
        /// An opaque message identifier for the frontend.
        message_id: String,
    },

    /// A direct message was sent successfully.
    MessageSent {
        /// The intended recipient.
        to: IdentityKey,
        /// The message identifier.
        message_id: String,
    },

    // ── Network events ──────────────────────────────────────────────
    /// A new peer connected to the node.
    PeerConnected {
        /// The peer that connected.
        peer_id: NodeId,
    },

    /// A peer disconnected from the node.
    PeerDisconnected {
        /// The peer that disconnected.
        peer_id: NodeId,
    },

    /// Network connectivity status changed.
    NetworkStatusChanged {
        /// Whether the node is currently connected to the network.
        connected: bool,
        /// Number of active peer connections.
        peer_count: usize,
    },

    // ── Social events ───────────────────────────────────────────────
    /// A connection request was received from another pseudonym.
    ConnectionRequestReceived {
        /// The pseudonym requesting connection.
        from: IdentityKey,
    },

    /// A connection was established with another pseudonym.
    ConnectionEstablished {
        /// The pseudonym we connected with.
        peer: IdentityKey,
    },

    /// A connection request was accepted.
    ConnectionAccepted {
        /// Hex-encoded pubkey of the accepted peer.
        peer: String,
    },

    /// A reaction was added to a post.
    ReactionReceived {
        /// The post that was reacted to.
        content_id: ContentId,
        /// The pseudonym that reacted.
        from: IdentityKey,
        /// The reaction (emoji) string.
        reaction: String,
    },

    /// A user was mentioned in a post.
    MentionReceived {
        /// The post containing the mention.
        content_id: ContentId,
        /// The pseudonym that authored the mentioning post.
        from: IdentityKey,
    },

    /// A new group chat message was received.
    GroupChatMessageReceived {
        /// The chat the message belongs to.
        chat_id: String,
        /// The sender of the message.
        from: IdentityKey,
        /// The message identifier.
        message_id: String,
    },

    // ── Handle events ───────────────────────────────────────────────
    /// Our handle was displaced by a conflicting registration that had
    /// priority (earlier timestamp or deterministic tiebreak).
    HandleConflictLost {
        /// The handle name we lost (without `@` prefix).
        handle_name: String,
        /// The identity that now owns the handle.
        new_owner: IdentityKey,
    },

    // ── Storage events ──────────────────────────────────────────────
    /// Garbage collection completed.
    GarbageCollectionCompleted {
        /// Number of items removed.
        items_removed: u64,
        /// Bytes freed.
        bytes_freed: u64,
    },

    /// Storage quota warning (approaching limit).
    StorageQuotaWarning {
        /// Current usage in bytes.
        used_bytes: u64,
        /// Maximum allowed bytes.
        max_bytes: u64,
    },
}

/// The event bus: a broadcast channel wrapper for internal event delivery.
///
/// Clone is cheap (just clones the sender handle).
#[derive(Debug, Clone)]
pub struct EventBus {
    sender: broadcast::Sender<Event>,
}

impl EventBus {
    /// Create a new event bus with the default channel capacity.
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CHANNEL_CAPACITY)
    }

    /// Create a new event bus with a custom channel capacity.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Publish an event to all current subscribers.
    ///
    /// Returns the number of receivers that received the event.
    /// Returns 0 if there are no active subscribers (this is not an error).
    pub fn emit(&self, event: Event) -> usize {
        // broadcast::send returns Err if there are no receivers,
        // which is a normal condition (no subscribers yet).
        self.sender.send(event).unwrap_or(0)
    }

    /// Subscribe to events on this bus.
    ///
    /// Returns a receiver that will yield all events published after
    /// this subscription was created. If the receiver falls behind by
    /// more than the channel capacity, it will receive a `Lagged` error
    /// and skip to the latest events.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.sender.subscribe()
    }

    /// The number of active subscribers.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ephemera_types::id::ED25519_PUBKEY_LEN;

    fn test_content_id() -> ContentId {
        ContentId::from_hash([0xAA; 32])
    }

    fn test_pseudonym() -> IdentityKey {
        IdentityKey::from_bytes([0xBB; ED25519_PUBKEY_LEN])
    }

    fn test_peer() -> NodeId {
        NodeId::from_bytes([0xCC; ED25519_PUBKEY_LEN])
    }

    #[tokio::test]
    async fn emit_and_receive() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        let event = Event::PostCreated {
            content_id: test_content_id(),
        };
        let count = bus.emit(event);
        assert_eq!(count, 1);

        let received = rx.recv().await.unwrap();
        match received {
            Event::PostCreated { content_id } => {
                assert_eq!(content_id, test_content_id());
            }
            _ => panic!("unexpected event variant"),
        }
    }

    #[tokio::test]
    async fn multiple_subscribers() {
        let bus = EventBus::new();
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        assert_eq!(bus.subscriber_count(), 2);

        bus.emit(Event::PeerConnected {
            peer_id: test_peer(),
        });

        let e1 = rx1.recv().await.unwrap();
        let e2 = rx2.recv().await.unwrap();
        assert!(matches!(e1, Event::PeerConnected { .. }));
        assert!(matches!(e2, Event::PeerConnected { .. }));
    }

    #[test]
    fn emit_with_no_subscribers() {
        let bus = EventBus::new();
        let count = bus.emit(Event::PostExpired {
            content_id: test_content_id(),
        });
        assert_eq!(count, 0);
    }

    #[test]
    fn default_capacity() {
        let bus = EventBus::default();
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[tokio::test]
    async fn event_variants_serialize() {
        let events = vec![
            Event::PostReceived {
                content_id: test_content_id(),
                author: test_pseudonym(),
            },
            Event::MessageReceived {
                from: test_pseudonym(),
                message_id: "msg-123".into(),
            },
            Event::NetworkStatusChanged {
                connected: true,
                peer_count: 5,
            },
            Event::GarbageCollectionCompleted {
                items_removed: 42,
                bytes_freed: 1024 * 1024,
            },
        ];

        for event in &events {
            let json = serde_json::to_string(event).unwrap();
            assert!(!json.is_empty());
        }
    }
}
