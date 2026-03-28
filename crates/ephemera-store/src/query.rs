//! Query helpers for assembling feeds, messages, and connection lists.
//!
//! These operate on the SQLite metadata layer and return lightweight
//! result structs that can be hydrated with content blobs separately.

use crate::metadata::MetadataDb;
use crate::StoreError;
use serde::{Deserialize, Serialize};

/// A row from the posts table, carrying only the metadata needed for
/// feed display (the actual encrypted body is in the content store).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostMeta {
    /// Content hash (33 bytes, hex-encoded for convenience).
    pub content_hash: Vec<u8>,
    /// Author public key.
    pub author_pubkey: Vec<u8>,
    /// When the post was created (Unix seconds).
    pub created_at: i64,
    /// When the post expires (Unix seconds).
    pub expires_at: i64,
    /// First 280 characters of plaintext body (for local search).
    pub body_preview: Option<String>,
    /// Whether the post has media attachments.
    pub has_media: bool,
}

/// A row from the messages table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageMeta {
    /// Message ID (content hash of encrypted payload).
    pub message_id: Vec<u8>,
    /// Conversation this message belongs to.
    pub conversation_id: Vec<u8>,
    /// Sender public key.
    pub sender_pubkey: Vec<u8>,
    /// When the message was received (Unix seconds).
    pub received_at: i64,
    /// First 100 characters of decrypted body preview.
    pub body_preview: Option<String>,
    /// Whether this message has been read.
    pub is_read: bool,
}

/// A row from the connections table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionRow {
    /// The remote peer's public key.
    pub remote_pubkey: Vec<u8>,
    /// Connection status.
    pub status: String,
    /// Display name (cached).
    pub display_name: Option<String>,
    /// When the connection was last updated.
    pub updated_at: i64,
}

/// High-level query engine for feed assembly and metadata lookups.
///
/// All queries use cursor-based pagination: pass an optional `before`
/// timestamp and a `limit` to get the next page.
pub struct QueryEngine;

impl QueryEngine {
    /// Query the feed: posts from a set of authors, ordered by
    /// `created_at DESC`, with cursor-based pagination.
    pub fn feed_query(
        db: &MetadataDb,
        author_pubkeys: &[Vec<u8>],
        before: Option<i64>,
        limit: u32,
    ) -> Result<Vec<PostMeta>, StoreError> {
        if author_pubkeys.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: Vec<String> = (1..=author_pubkeys.len())
            .map(|i| format!("?{i}"))
            .collect();
        let in_clause = placeholders.join(", ");

        let before_val = before.unwrap_or(i64::MAX);
        let limit_param_idx = author_pubkeys.len() + 1;
        let before_param_idx = author_pubkeys.len() + 2;

        let sql = format!(
            "SELECT content_hash, author_pubkey, created_at, expires_at,
                    body_preview, has_media
             FROM posts
             WHERE author_pubkey IN ({in_clause})
               AND is_tombstone = 0
               AND created_at < ?{before_param_idx}
             ORDER BY created_at DESC
             LIMIT ?{limit_param_idx}"
        );

        let mut stmt = db.conn().prepare(&sql)?;

        // Build parameter vector: all author pubkeys + limit + before.
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        for key in author_pubkeys {
            params.push(Box::new(key.clone()));
        }
        params.push(Box::new(limit));
        params.push(Box::new(before_val));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok(PostMeta {
                    content_hash: row.get(0)?,
                    author_pubkey: row.get(1)?,
                    created_at: row.get(2)?,
                    expires_at: row.get(3)?,
                    body_preview: row.get(4)?,
                    has_media: row.get::<_, i32>(5)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    /// Query messages in a conversation, ordered by `received_at DESC`.
    pub fn messages_query(
        db: &MetadataDb,
        conversation_id: &[u8],
        before: Option<i64>,
        limit: u32,
    ) -> Result<Vec<MessageMeta>, StoreError> {
        let before_val = before.unwrap_or(i64::MAX);
        let mut stmt = db.conn().prepare(
            "SELECT message_id, conversation_id, sender_pubkey, received_at,
                    body_preview, is_read
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
                    Ok(MessageMeta {
                        message_id: row.get(0)?,
                        conversation_id: row.get(1)?,
                        sender_pubkey: row.get(2)?,
                        received_at: row.get(3)?,
                        body_preview: row.get(4)?,
                        is_read: row.get::<_, i32>(5)? != 0,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    /// Query connections for a local pseudonym.
    pub fn connections_query(
        db: &MetadataDb,
        local_pubkey: &[u8],
        status_filter: Option<&str>,
    ) -> Result<Vec<ConnectionRow>, StoreError> {
        let (sql, has_status) = if status_filter.is_some() {
            (
                "SELECT remote_pubkey, status, display_name, updated_at
                 FROM connections
                 WHERE local_pubkey = ?1 AND status = ?2
                 ORDER BY updated_at DESC",
                true,
            )
        } else {
            (
                "SELECT remote_pubkey, status, display_name, updated_at
                 FROM connections
                 WHERE local_pubkey = ?1
                 ORDER BY updated_at DESC",
                false,
            )
        };

        let mut stmt = db.conn().prepare(sql)?;

        let row_mapper = |row: &rusqlite::Row<'_>| {
            Ok(ConnectionRow {
                remote_pubkey: row.get(0)?,
                status: row.get(1)?,
                display_name: row.get(2)?,
                updated_at: row.get(3)?,
            })
        };

        let rows: Vec<ConnectionRow> = if has_status {
            let status = status_filter.unwrap_or_default();
            stmt.query_map(rusqlite::params![local_pubkey, status], row_mapper)?
                .collect::<Result<Vec<_>, _>>()?
        } else {
            stmt.query_map(rusqlite::params![local_pubkey], row_mapper)?
                .collect::<Result<Vec<_>, _>>()?
        };

        Ok(rows)
    }
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;
