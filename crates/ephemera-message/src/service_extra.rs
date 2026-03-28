//! Additional `MessageService` methods: request management, conversation
//! queries, and helper functions used by the core send/receive flow.

use super::*;

impl MessageService {
    /// Accept a message request from a stranger, allowing future messages
    /// to go directly to the main inbox.
    pub fn accept_request(
        db: &MetadataDb,
        our_pubkey: &[u8],
        sender_pubkey: &[u8],
    ) -> Result<(), MessageError> {
        let updated = db.conn().execute(
            "UPDATE message_requests SET status = 'accepted'
             WHERE recipient_pubkey = ?1 AND sender_pubkey = ?2 AND status = 'pending'",
            rusqlite::params![our_pubkey, sender_pubkey],
        )?;

        if updated == 0 {
            return Err(MessageError::InvalidRequestState {
                expected: "Pending".into(),
                got: "no matching request found".into(),
            });
        }

        // Mark the conversation as no longer a request.
        db.conn().execute(
            "UPDATE conversations SET is_request = 0
             WHERE our_pubkey = ?1 AND their_pubkey = ?2",
            rusqlite::params![our_pubkey, sender_pubkey],
        )?;

        Ok(())
    }

    /// Reject a message request, preventing future messages from this sender.
    pub fn reject_request(
        db: &MetadataDb,
        our_pubkey: &[u8],
        sender_pubkey: &[u8],
    ) -> Result<(), MessageError> {
        let updated = db.conn().execute(
            "UPDATE message_requests SET status = 'rejected'
             WHERE recipient_pubkey = ?1 AND sender_pubkey = ?2 AND status = 'pending'",
            rusqlite::params![our_pubkey, sender_pubkey],
        )?;

        if updated == 0 {
            return Err(MessageError::InvalidRequestState {
                expected: "Pending".into(),
                got: "no matching request found".into(),
            });
        }

        Ok(())
    }

    /// List conversations for the given identity, ordered by most recent message.
    pub fn list_conversations(
        db: &MetadataDb,
        our_pubkey: &[u8],
    ) -> Result<Vec<ConversationSummary>, MessageError> {
        let mut stmt = db.conn().prepare(
            "SELECT conversation_id, our_pubkey, their_pubkey, last_message_at,
                    unread_count, is_request
             FROM conversations
             WHERE our_pubkey = ?1
             ORDER BY last_message_at DESC",
        )?;

        let rows = stmt
            .query_map(rusqlite::params![our_pubkey], |row| {
                Ok(ConversationSummary {
                    conversation_id: row.get(0)?,
                    our_pubkey: row.get(1)?,
                    their_pubkey: row.get(2)?,
                    last_message_at: row.get(3)?,
                    unread_count: row.get(4)?,
                    is_request: row.get::<_, i32>(5)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    /// Get paginated message history for a conversation.
    pub fn get_messages(
        db: &MetadataDb,
        conversation_id: &[u8],
        before: Option<i64>,
        limit: u32,
    ) -> Result<Vec<StoredMessage>, MessageError> {
        let before_val = before.unwrap_or(i64::MAX);

        let mut stmt = db.conn().prepare(
            "SELECT message_id, conversation_id, sender_pubkey, received_at,
                    expires_at, body_preview, is_read, encrypted_body, is_encrypted
             FROM messages
             WHERE conversation_id = ?1
               AND received_at < ?2
             ORDER BY received_at DESC
             LIMIT ?3",
        )?;

        let rows = stmt
            .query_map(
                rusqlite::params![conversation_id, before_val, limit],
                |row| {
                    Ok(StoredMessage {
                        id: row.get(0)?,
                        conversation_id: row.get(1)?,
                        sender_pubkey: row.get(2)?,
                        received_at: row.get(3)?,
                        expires_at: row.get(4)?,
                        body_preview: row.get(5)?,
                        is_read: row.get::<_, i32>(6)? != 0,
                        encrypted_body: row.get(7)?,
                        is_encrypted: row.get::<_, i32>(8).unwrap_or(0) != 0,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    /// Run message garbage collection: delete expired messages and clean up
    /// empty conversations.
    ///
    /// This is intended to be called periodically alongside the post GC.
    pub fn gc_expired_messages(db: &MetadataDb) -> Result<u64, MessageError> {
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_secs() as i64;

        // Delete expired messages.
        let deleted = db.conn().execute(
            "DELETE FROM messages WHERE expires_at < ?1",
            rusqlite::params![now_secs],
        )?;

        // Clean up conversations that have no remaining messages.
        db.conn().execute(
            "DELETE FROM conversations
             WHERE conversation_id NOT IN (SELECT DISTINCT conversation_id FROM messages)",
            [],
        )?;

        Ok(deleted as u64)
    }

    // ── Internal helpers ─────────────────────────────────────────────

    /// Check if two identities are connected (mutual connection).
    pub(crate) fn is_connected(
        db: &MetadataDb,
        local_pubkey: &[u8],
        remote_pubkey: &[u8],
    ) -> Result<bool, MessageError> {
        let count: i64 = db.conn().query_row(
            "SELECT COUNT(*) FROM connections
             WHERE local_pubkey = ?1 AND remote_pubkey = ?2 AND status = 'connected'",
            rusqlite::params![local_pubkey, remote_pubkey],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Check if a message request from sender to recipient has been accepted.
    pub(crate) fn is_request_accepted(
        db: &MetadataDb,
        recipient_pubkey: &[u8],
        sender_pubkey: &[u8],
    ) -> Result<bool, MessageError> {
        // Check both the message_requests table and the conversations table.
        let count: i64 = db.conn().query_row(
            "SELECT COUNT(*) FROM message_requests
             WHERE recipient_pubkey = ?1 AND sender_pubkey = ?2 AND status = 'accepted'",
            rusqlite::params![recipient_pubkey, sender_pubkey],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Check if a message request from sender to recipient has been rejected.
    pub(crate) fn is_request_rejected(
        db: &MetadataDb,
        recipient_pubkey: &[u8],
        sender_pubkey: &[u8],
    ) -> Result<bool, MessageError> {
        let count: i64 = db.conn().query_row(
            "SELECT COUNT(*) FROM message_requests
             WHERE recipient_pubkey = ?1 AND sender_pubkey = ?2 AND status = 'rejected'",
            rusqlite::params![recipient_pubkey, sender_pubkey],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Get or create a conversation between two identities.
    ///
    /// The conversation_id is the BLAKE3 hash of (our_pubkey || their_pubkey),
    /// sorted to ensure the same conversation regardless of who initiates.
    pub fn get_or_create_conversation(
        db: &MetadataDb,
        our_pubkey: &[u8],
        their_pubkey: &[u8],
        is_request: bool,
    ) -> Result<Vec<u8>, MessageError> {
        let conversation_id = Self::compute_conversation_id(our_pubkey, their_pubkey);

        let now = Timestamp::now().as_secs() as i64;

        // Upsert: insert if not exists.
        db.conn().execute(
            "INSERT OR IGNORE INTO conversations
             (conversation_id, our_pubkey, their_pubkey, last_message_at,
              unread_count, is_request, created_at)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?4)",
            rusqlite::params![
                conversation_id,
                our_pubkey,
                their_pubkey,
                now,
                is_request as i32
            ],
        )?;

        Ok(conversation_id)
    }

    /// Compute a deterministic conversation ID from two public keys.
    ///
    /// The keys are sorted lexicographically before hashing to ensure
    /// both parties derive the same conversation ID.
    pub fn compute_conversation_id(pubkey_a: &[u8], pubkey_b: &[u8]) -> Vec<u8> {
        let mut input = Vec::with_capacity(pubkey_a.len() + pubkey_b.len());
        if pubkey_a <= pubkey_b {
            input.extend_from_slice(pubkey_a);
            input.extend_from_slice(pubkey_b);
        } else {
            input.extend_from_slice(pubkey_b);
            input.extend_from_slice(pubkey_a);
        }
        blake3::hash(&input).as_bytes().to_vec()
    }

    /// Store a pending message request from a stranger.
    pub(crate) fn store_message_request(
        db: &MetadataDb,
        sender_pubkey: &[u8],
        recipient_pubkey: &[u8],
    ) -> Result<(), MessageError> {
        let now = Timestamp::now().as_secs() as i64;

        db.conn().execute(
            "INSERT OR IGNORE INTO message_requests
             (sender_pubkey, recipient_pubkey, status, created_at)
             VALUES (?1, ?2, 'pending', ?3)",
            rusqlite::params![sender_pubkey, recipient_pubkey, now],
        )?;

        Ok(())
    }
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
