//! Dead drop mailbox system for offline message delivery.
//!
//! A dead drop is a DHT-based mailbox keyed by the BLAKE3 hash of the
//! recipient's public key. Senders deposit sealed envelopes into the dead
//! drop when the recipient is offline. When the recipient comes online,
//! they check their mailbox and retrieve pending messages.
//!
//! Dead drop messages have a maximum TTL of 14 days (shorter than the
//! general 30-day content TTL) to limit storage burden on relay nodes.

use ephemera_store::MetadataDb;
use ephemera_types::{ContentId, IdentityKey, Timestamp};
use serde::{Deserialize, Serialize};

use crate::sealed::SealedEnvelope;
use crate::MessageError;

/// Maximum TTL for dead drop messages: 14 days in seconds.
pub const DEAD_DROP_MAX_TTL_SECS: u64 = 14 * 24 * 60 * 60;

/// Wire envelope for dead drop messages published on the gossip network.
///
/// When a sender deposits a dead drop, the node also publishes this envelope
/// on the `dm_delivery` gossip topic so that other nodes (including the
/// recipient's node) can ingest and store it in their local dead drop table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadDropEnvelope {
    /// The mailbox key (BLAKE3 hash of recipient's pubkey).
    pub mailbox_key: [u8; 32],
    /// Content-addressed message identifier.
    pub message_id: [u8; 32],
    /// The serialized sealed envelope bytes.
    pub sealed_data: Vec<u8>,
    /// Unix timestamp when the message was deposited.
    pub deposited_at: u64,
    /// Unix timestamp when the message expires.
    pub expires_at: u64,
}

/// A pending message waiting in a dead drop mailbox.
#[derive(Debug, Clone)]
pub struct PendingMessage {
    /// Content-addressed message identifier.
    pub id: ContentId,
    /// The serialized sealed envelope bytes.
    pub sealed_envelope: Vec<u8>,
    /// Unix timestamp when the message was deposited.
    pub deposited_at: u64,
    /// Unix timestamp when the message expires from the dead drop.
    pub expires_at: u64,
}

/// A dead drop mailbox for a single recipient.
#[derive(Debug, Clone)]
pub struct DeadDrop {
    /// The mailbox key (BLAKE3 hash of recipient's pubkey).
    pub mailbox_key: ContentId,
    /// Pending messages for this recipient.
    pub messages: Vec<PendingMessage>,
}

/// Service for managing dead drop mailboxes backed by SQLite.
///
/// Dead drops provide store-and-forward delivery for offline recipients.
/// Messages are deposited by senders and retrieved when the recipient
/// polls their mailbox.
pub struct DeadDropService;

impl DeadDropService {
    /// Compute the mailbox key for a given public key.
    ///
    /// The mailbox key is the BLAKE3 hash of a domain-separated input
    /// to prevent collisions with other key derivations.
    pub fn mailbox_key(pubkey: &IdentityKey) -> ContentId {
        let hash = blake3::hash(
            &[b"ephemera-dead-drop-mailbox-v1\x00", pubkey.as_bytes().as_slice()].concat(),
        );
        ContentId::from_digest(*hash.as_bytes())
    }

    /// Deposit a sealed message for an offline recipient.
    ///
    /// The message is stored in the dead drop table with a 14-day TTL.
    /// Returns the content-addressed message ID.
    pub fn deposit(
        db: &MetadataDb,
        recipient_pubkey: &IdentityKey,
        sealed: &SealedEnvelope,
    ) -> Result<ContentId, MessageError> {
        let mailbox = Self::mailbox_key(recipient_pubkey);
        let mailbox_bytes = mailbox.hash_bytes().to_vec();

        // Serialize the sealed envelope.
        let sealed_data = serde_json::to_vec(sealed)
            .map_err(|e| MessageError::Deserialization(e.to_string()))?;

        // Compute message ID from the sealed data.
        let msg_hash = blake3::hash(&sealed_data);
        let msg_id = ContentId::from_digest(*msg_hash.as_bytes());
        let msg_id_bytes = msg_id.hash_bytes().to_vec();

        let now = Timestamp::now().as_secs();
        let expires_at = now + DEAD_DROP_MAX_TTL_SECS;

        db.conn().execute(
            "INSERT OR IGNORE INTO dead_drops
             (message_id, mailbox_key, sealed_data, deposited_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![msg_id_bytes, mailbox_bytes, sealed_data, now, expires_at],
        )?;

        Ok(msg_id)
    }

    /// Check our mailbox for pending (non-expired) messages.
    ///
    /// Returns all messages deposited for the given public key that
    /// have not yet expired.
    pub fn check_mailbox(
        db: &MetadataDb,
        our_pubkey: &IdentityKey,
    ) -> Result<Vec<PendingMessage>, MessageError> {
        let mailbox = Self::mailbox_key(our_pubkey);
        let mailbox_bytes = mailbox.hash_bytes().to_vec();
        let now = Timestamp::now().as_secs() as i64;

        let mut stmt = db.conn().prepare(
            "SELECT message_id, sealed_data, deposited_at, expires_at
             FROM dead_drops
             WHERE mailbox_key = ?1 AND expires_at > ?2
             ORDER BY deposited_at ASC",
        )?;

        let rows = stmt
            .query_map(rusqlite::params![mailbox_bytes, now], |row| {
                let id_bytes: Vec<u8> = row.get(0)?;
                let sealed_data: Vec<u8> = row.get(1)?;
                let deposited_at: i64 = row.get(2)?;
                let expires_at: i64 = row.get(3)?;

                let mut hash = [0u8; 32];
                if id_bytes.len() == 32 {
                    hash.copy_from_slice(&id_bytes);
                }

                Ok(PendingMessage {
                    id: ContentId::from_digest(hash),
                    sealed_envelope: sealed_data,
                    deposited_at: deposited_at as u64,
                    expires_at: expires_at as u64,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    /// Acknowledge receipt of a message, removing it from the dead drop.
    ///
    /// Called after the recipient has successfully decrypted and stored
    /// a message from their mailbox.
    pub fn acknowledge(
        db: &MetadataDb,
        message_id: &ContentId,
    ) -> Result<(), MessageError> {
        let id_bytes = message_id.hash_bytes().to_vec();

        let deleted = db.conn().execute(
            "DELETE FROM dead_drops WHERE message_id = ?1",
            rusqlite::params![id_bytes],
        )?;

        if deleted == 0 {
            return Err(MessageError::ConversationNotFound {
                peer_id: format!("dead drop message {}", message_id),
            });
        }

        Ok(())
    }

    /// Garbage-collect expired dead drop messages (older than 14 days).
    ///
    /// Returns the number of messages removed.
    pub fn gc(db: &MetadataDb) -> Result<usize, MessageError> {
        let now = Timestamp::now().as_secs() as i64;

        let deleted = db.conn().execute(
            "DELETE FROM dead_drops WHERE expires_at <= ?1",
            rusqlite::params![now],
        )?;

        Ok(deleted)
    }

    /// Deposit a raw sealed envelope (already serialized) into a specific
    /// mailbox. Used when relaying dead drop records from gossip or DHT.
    pub fn deposit_raw(
        db: &MetadataDb,
        mailbox_key: &ContentId,
        message_id: &ContentId,
        sealed_data: &[u8],
        deposited_at: u64,
        expires_at: u64,
    ) -> Result<(), MessageError> {
        let mailbox_bytes = mailbox_key.hash_bytes().to_vec();
        let msg_id_bytes = message_id.hash_bytes().to_vec();

        // Reject if already expired.
        let now = Timestamp::now().as_secs();
        if expires_at <= now {
            return Err(MessageError::Expired);
        }

        // Clamp expires_at to max dead drop TTL from now.
        let clamped_expires = expires_at.min(now + DEAD_DROP_MAX_TTL_SECS);

        db.conn().execute(
            "INSERT OR IGNORE INTO dead_drops
             (message_id, mailbox_key, sealed_data, deposited_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                msg_id_bytes,
                mailbox_bytes,
                sealed_data,
                deposited_at,
                clamped_expires,
            ],
        )?;

        Ok(())
    }

    /// Count the number of pending messages in a mailbox.
    pub fn mailbox_count(
        db: &MetadataDb,
        pubkey: &IdentityKey,
    ) -> Result<u64, MessageError> {
        let mailbox = Self::mailbox_key(pubkey);
        let mailbox_bytes = mailbox.hash_bytes().to_vec();
        let now = Timestamp::now().as_secs() as i64;

        let count: i64 = db.conn().query_row(
            "SELECT COUNT(*) FROM dead_drops
             WHERE mailbox_key = ?1 AND expires_at > ?2",
            rusqlite::params![mailbox_bytes, now],
            |row| row.get(0),
        )?;

        Ok(count as u64)
    }
}

#[cfg(test)]
#[path = "dead_drop_tests.rs"]
mod tests;
