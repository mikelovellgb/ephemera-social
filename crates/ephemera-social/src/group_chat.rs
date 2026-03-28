//! Group chats: both group-linked and standalone private group conversations.
//!
//! Group-linked chats share membership and moderation with their parent group.
//! Private group chats are ad-hoc, egalitarian (no owner/admin), and any
//! member can add others or leave.

use ephemera_types::Timestamp;
use serde::{Deserialize, Serialize};

/// Maximum number of members in a private group chat.
pub const MAX_PRIVATE_CHAT_MEMBERS: usize = 50;

/// Maximum name length for a group chat.
pub const MAX_CHAT_NAME_LEN: usize = 100;

/// A group chat record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupChat {
    /// Unique identifier for the chat.
    pub chat_id: String,
    /// Optional display name for the chat.
    pub name: Option<String>,
    /// Whether this chat is linked to a group.
    pub is_group_linked: bool,
    /// The group ID if this is a group-linked chat.
    pub group_id: Option<String>,
    /// When the chat was created.
    pub created_at: Timestamp,
}

/// A member of a group chat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupChatMember {
    /// The chat this member belongs to.
    pub chat_id: String,
    /// Hex-encoded pubkey.
    pub member_pubkey: String,
    /// When the member joined.
    pub joined_at: Timestamp,
}

/// A message in a group chat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupChatMessage {
    /// Unique message identifier.
    pub message_id: String,
    /// The chat this message belongs to.
    pub chat_id: String,
    /// Hex-encoded pubkey of the sender.
    pub sender_pubkey: String,
    /// Message body text.
    pub body: Option<String>,
    /// When the message was created.
    pub created_at: Timestamp,
    /// When the message expires.
    pub expires_at: Timestamp,
}

/// Summary of a group chat (for list views).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupChatSummary {
    /// The chat record.
    pub chat: GroupChat,
    /// Number of members.
    pub member_count: u32,
    /// Last message preview.
    pub last_message: Option<String>,
    /// Timestamp of the last message.
    pub last_message_at: Option<Timestamp>,
}

/// Generate a unique chat ID from the creator and a timestamp.
#[must_use]
pub fn generate_chat_id(creator_pubkey: &str, timestamp_secs: u64) -> String {
    let input = format!("{creator_pubkey}:{timestamp_secs}");
    let hash = blake3::hash(input.as_bytes());
    hex::encode(hash.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_id_deterministic() {
        let a = generate_chat_id("abc123", 1000);
        let b = generate_chat_id("abc123", 1000);
        assert_eq!(a, b);
    }

    #[test]
    fn different_inputs_different_ids() {
        let a = generate_chat_id("abc123", 1000);
        let b = generate_chat_id("abc123", 1001);
        assert_ne!(a, b);
    }
}
