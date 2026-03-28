//! Direct message data model.
//!
//! A `DirectMessage` is an end-to-end encrypted 1:1 message between two
//! pseudonyms. Messages carry encrypted content, a TTL, and a signature
//! for integrity verification.

use ephemera_types::{ContentId, IdentityKey, Signature, Timestamp, Ttl};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Maximum wire size for a single message body (32 KB per spec).
pub const MAX_MESSAGE_WIRE_SIZE: usize = 32_768;

/// Unique identifier for a direct message, derived from its content hash.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(ContentId);

impl MessageId {
    /// Create a `MessageId` from a content hash.
    pub fn from_content_hash(hash: ContentId) -> Self {
        Self(hash)
    }

    /// Return the underlying content hash.
    pub fn as_content_hash(&self) -> &ContentId {
        &self.0
    }
}

impl fmt::Debug for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MessageId({:?})", self.0)
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Delivery / read status of a direct message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageStatus {
    /// Created locally, not yet delivered to the relay.
    Pending,
    /// Deposited at the dead drop; recipient has not acknowledged.
    Delivered,
    /// Recipient has decrypted and acknowledged the message.
    Read,
}

/// A direct message between two pseudonyms.
///
/// The `encrypted_content` field holds the XChaCha20-Poly1305 ciphertext
/// (nonce prepended). Only the sender and recipient can decrypt it.
#[derive(Clone, Serialize, Deserialize)]
pub struct DirectMessage {
    /// Unique message identifier (BLAKE3 of the plaintext envelope).
    pub id: MessageId,
    /// Sender pseudonym public key.
    pub sender: IdentityKey,
    /// Recipient pseudonym public key.
    pub recipient: IdentityKey,
    /// Encrypted message body (`nonce || ciphertext`).
    pub encrypted_content: Vec<u8>,
    /// When the message was created.
    pub timestamp: Timestamp,
    /// How long the message should be retained.
    pub ttl: Ttl,
    /// Ed25519 signature over the canonical message bytes.
    pub signature: Signature,
    /// Current delivery status (local tracking, not transmitted).
    #[serde(default = "default_status")]
    pub status: MessageStatus,
}

fn default_status() -> MessageStatus {
    MessageStatus::Pending
}

impl DirectMessage {
    /// Validate wire-level constraints on this message.
    ///
    /// Checks that the encrypted content does not exceed the protocol
    /// maximum. Signature verification requires the crypto crate and is
    /// handled by the caller.
    pub fn validate(&self) -> Result<(), crate::MessageError> {
        if self.encrypted_content.len() > MAX_MESSAGE_WIRE_SIZE {
            return Err(crate::MessageError::BodyTooLarge {
                got: self.encrypted_content.len(),
                max: MAX_MESSAGE_WIRE_SIZE,
            });
        }
        Ok(())
    }
}

impl fmt::Debug for DirectMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DirectMessage")
            .field("id", &self.id)
            .field("sender", &self.sender)
            .field("recipient", &self.recipient)
            .field("encrypted_len", &self.encrypted_content.len())
            .field("timestamp", &self.timestamp)
            .field("ttl", &self.ttl)
            .field("status", &self.status)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ephemera_types::Ttl;

    fn dummy_message(content_size: usize) -> DirectMessage {
        DirectMessage {
            id: MessageId::from_content_hash(ContentId::from_digest([0xAA; 32])),
            sender: IdentityKey::from_bytes([1; 32]),
            recipient: IdentityKey::from_bytes([2; 32]),
            encrypted_content: vec![0u8; content_size],
            timestamp: Timestamp::now(),
            ttl: Ttl::from_secs(86400).unwrap(),
            signature: Signature::from_bytes([0; 64]),
            status: MessageStatus::Pending,
        }
    }

    #[test]
    fn valid_message_passes() {
        let msg = dummy_message(1024);
        assert!(msg.validate().is_ok());
    }

    #[test]
    fn oversized_message_rejected() {
        let msg = dummy_message(MAX_MESSAGE_WIRE_SIZE + 1);
        assert!(msg.validate().is_err());
    }

    #[test]
    fn message_id_display() {
        let id = MessageId::from_content_hash(ContentId::from_digest([0xBB; 32]));
        let s = format!("{id}");
        assert!(!s.is_empty());
    }

    #[test]
    fn message_status_default() {
        assert_eq!(default_status(), MessageStatus::Pending);
    }
}
