//! Message service: orchestrates sending, receiving, and querying messages.
//!
//! The `MessageService` provides the high-level API for direct messaging:
//! - Encrypting and storing outgoing messages
//! - Validating, decrypting, and storing incoming messages
//! - Managing message requests from strangers
//! - Listing conversations ordered by recency
//! - Paginated message history retrieval
//! - TTL enforcement via garbage collection integration

use ephemera_crypto::{X25519PublicKey, X25519SecretKey};
use ephemera_store::MetadataDb;
use ephemera_types::{IdentityKey, Timestamp, Ttl};

use crate::sealed::{SealedEnvelope, SealedPayload};
use crate::MessageError;

/// Maximum TTL for messages (30 days, same as posts per spec).
const MAX_MESSAGE_TTL_SECS: u64 = 30 * 24 * 60 * 60;

/// A stored message record retrieved from the database.
#[derive(Debug, Clone)]
pub struct StoredMessage {
    /// The message ID.
    pub id: Vec<u8>,
    /// The conversation this message belongs to.
    pub conversation_id: Vec<u8>,
    /// The sender's public key.
    pub sender_pubkey: Vec<u8>,
    /// When the message was received (Unix seconds).
    pub received_at: i64,
    /// When the message expires (Unix seconds).
    pub expires_at: i64,
    /// Preview of the decrypted body.
    pub body_preview: Option<String>,
    /// Whether the message has been read.
    pub is_read: bool,
    /// The encrypted message body (ciphertext), if available.
    pub encrypted_body: Option<Vec<u8>>,
    /// Whether this message is E2E encrypted.
    pub is_encrypted: bool,
}

/// A conversation summary for listing.
#[derive(Debug, Clone)]
pub struct ConversationSummary {
    /// The conversation ID.
    pub conversation_id: Vec<u8>,
    /// Our public key in this conversation.
    pub our_pubkey: Vec<u8>,
    /// The peer's public key.
    pub their_pubkey: Vec<u8>,
    /// Timestamp of the last message.
    pub last_message_at: Option<i64>,
    /// Number of unread messages.
    pub unread_count: i64,
    /// Whether this conversation started as a request.
    pub is_request: bool,
}

/// The message service, providing the complete DM API.
///
/// This struct is the main entry point for all messaging operations. It
/// handles encryption, storage, message requests, and conversation
/// management.
pub struct MessageService;

impl MessageService {
    /// Send an encrypted message to a recipient.
    ///
    /// This function:
    /// 1. Checks whether messaging is allowed (connection or accepted request).
    /// 2. Encrypts the plaintext using sealed sender envelope.
    /// 3. Stores the message metadata in SQLite.
    /// 4. Updates the conversation list.
    ///
    /// Returns the conversation ID and the sealed envelope for delivery.
    pub fn send_message(
        db: &MetadataDb,
        sender_identity: &IdentityKey,
        recipient_identity: &IdentityKey,
        recipient_x25519: &X25519PublicKey,
        plaintext: &[u8],
        ttl: Ttl,
    ) -> Result<(Vec<u8>, SealedEnvelope), MessageError> {
        // Validate TTL.
        if ttl.as_secs() > MAX_MESSAGE_TTL_SECS {
            return Err(MessageError::InvalidTtl(
                ephemera_types::EphemeraError::InvalidTtl {
                    value_secs: ttl.as_secs(),
                    min_secs: ephemera_types::ttl::MIN_TTL_SECS,
                    max_secs: MAX_MESSAGE_TTL_SECS,
                },
            ));
        }

        // Check if messaging is allowed (connection or accepted request).
        let sender_bytes = sender_identity.as_bytes().to_vec();
        let recipient_bytes = recipient_identity.as_bytes().to_vec();

        let is_connected = Self::is_connected(db, &sender_bytes, &recipient_bytes)?;
        // Check if the recipient has accepted a request FROM the sender.
        // The request record has sender=us, recipient=them, so we check
        // with recipient_pubkey=recipient_bytes and sender_pubkey=sender_bytes.
        let is_request_accepted = Self::is_request_accepted(db, &recipient_bytes, &sender_bytes)?;

        if !is_connected && !is_request_accepted {
            return Err(MessageError::NotAllowed {
                reason: "no connection or accepted message request with recipient".into(),
            });
        }

        // Create sealed envelope (sender is inside the ciphertext).
        let envelope = SealedEnvelope::seal(
            sender_identity,
            recipient_identity,
            recipient_x25519,
            plaintext,
            ttl,
        )?;

        // Get or create conversation.
        let conversation_id =
            Self::get_or_create_conversation(db, &sender_bytes, &recipient_bytes, false)?;

        // Compute message ID from ciphertext content hash.
        let msg_hash = blake3::hash(&envelope.ciphertext);
        let msg_id = msg_hash.as_bytes().to_vec();

        let now = Timestamp::now().as_secs() as i64;
        let expires_at = now + ttl.as_secs() as i64;

        // Truncate plaintext for preview (first 100 chars of UTF-8 if valid).
        let preview = String::from_utf8(plaintext.to_vec())
            .ok()
            .map(|s| s.chars().take(100).collect::<String>());

        // Store message metadata.
        db.conn().execute(
            "INSERT INTO messages (message_id, conversation_id, sender_pubkey,
             received_at, expires_at, is_read, body_preview, has_media)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, 0)",
            rusqlite::params![
                msg_id,
                conversation_id,
                sender_bytes,
                now,
                expires_at,
                preview
            ],
        )?;

        // Update conversation last_message_at.
        db.conn().execute(
            "UPDATE conversations SET last_message_at = ?1 WHERE conversation_id = ?2",
            rusqlite::params![now, conversation_id],
        )?;

        Ok((conversation_id, envelope))
    }

    /// Receive and process an incoming encrypted message.
    ///
    /// This function:
    /// 1. Decrypts the sealed envelope to reveal the sender.
    /// 2. Checks if sender is connected or has an accepted request.
    /// 3. If sender is a stranger, routes to message requests.
    /// 4. Validates the message TTL.
    /// 5. Stores the message and updates the conversation.
    ///
    /// Returns the sender identity and decrypted body.
    pub fn receive_message(
        db: &MetadataDb,
        envelope: &SealedEnvelope,
        our_identity: &IdentityKey,
        our_x25519_secret: &X25519SecretKey,
    ) -> Result<SealedPayload, MessageError> {
        // Check TTL.
        if envelope.is_expired() {
            return Err(MessageError::Expired);
        }

        // Decrypt to reveal sender and body.
        let payload = envelope.open(our_x25519_secret)?;

        let our_bytes = our_identity.as_bytes().to_vec();
        let sender_bytes = payload.sender.as_bytes().to_vec();

        let is_connected = Self::is_connected(db, &our_bytes, &sender_bytes)?;
        let is_request_accepted = Self::is_request_accepted(db, &our_bytes, &sender_bytes)?;

        let is_request = !is_connected && !is_request_accepted;

        // If sender is a stranger and has a rejected request, deny.
        if is_request && Self::is_request_rejected(db, &our_bytes, &sender_bytes)? {
            return Err(MessageError::NotAllowed {
                reason: "message request was rejected".into(),
            });
        }

        // Get or create conversation.
        let conversation_id =
            Self::get_or_create_conversation(db, &our_bytes, &sender_bytes, is_request)?;

        // Store message.
        let msg_hash = blake3::hash(&envelope.ciphertext);
        let msg_id = msg_hash.as_bytes().to_vec();

        let now = Timestamp::now().as_secs() as i64;
        let expires_at = envelope.timestamp.as_secs() as i64 + envelope.ttl.as_secs() as i64;

        let preview = String::from_utf8(payload.body.clone())
            .ok()
            .map(|s| s.chars().take(100).collect::<String>());

        db.conn().execute(
            "INSERT OR IGNORE INTO messages (message_id, conversation_id, sender_pubkey,
             received_at, expires_at, is_read, body_preview, has_media)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, 0)",
            rusqlite::params![
                msg_id,
                conversation_id,
                sender_bytes,
                now,
                expires_at,
                preview
            ],
        )?;

        // Update conversation and bump unread.
        db.conn().execute(
            "UPDATE conversations SET last_message_at = ?1, unread_count = unread_count + 1
             WHERE conversation_id = ?2",
            rusqlite::params![now, conversation_id],
        )?;

        // If this is a message request from a stranger, store the request.
        if is_request {
            Self::store_message_request(db, &sender_bytes, &our_bytes)?;
        }

        Ok(payload)
    }
}

#[path = "service_extra.rs"]
pub mod service_extra;

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
