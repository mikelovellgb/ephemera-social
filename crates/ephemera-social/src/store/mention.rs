//! SQLite-backed mention storage.
//!
//! Queries the `post_mentions` table for mentions. Mention insertion
//! happens in the post creation flow; this module provides read queries.

use super::*;
use crate::mention::Mention;

impl SqliteSocialServices {
    /// Store a mention (called during post creation).
    pub fn store_mention(
        &self,
        content_hash: &[u8],
        mentioned_pubkey: &[u8],
        display_hint: &str,
        byte_start: usize,
        byte_end: usize,
    ) -> Result<(), SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        db.conn()
            .execute(
                "INSERT OR IGNORE INTO post_mentions (content_hash, mentioned_key, display_hint, byte_start, byte_end)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    content_hash,
                    mentioned_pubkey,
                    display_hint,
                    byte_start as i64,
                    byte_end as i64
                ],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(())
    }

    /// List posts where a given pubkey is mentioned, newest first.
    ///
    /// Excludes mentions from users the mentioned user has blocked.
    pub fn list_mentions(
        &self,
        mentioned_pubkey: &[u8],
        limit: u32,
    ) -> Result<Vec<crate::feed::FeedItem>, SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;
        let fetch_limit = limit.min(200) as i64;

        let mut stmt = db
            .conn()
            .prepare(
                "SELECT p.content_hash, p.author_pubkey, p.created_at, p.parent_hash
                 FROM post_mentions pm
                 JOIN posts p ON pm.content_hash = p.content_hash
                 WHERE pm.mentioned_key = ?1
                   AND p.is_tombstone = 0
                   AND p.author_pubkey NOT IN (
                       SELECT blocked_pubkey FROM blocks WHERE blocker_pubkey = ?1
                   )
                 ORDER BY p.created_at DESC
                 LIMIT ?2",
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params![mentioned_pubkey, fetch_limit], |row| {
                let hash: Vec<u8> = row.get(0)?;
                let author: Vec<u8> = row.get(1)?;
                let created_at: i64 = row.get(2)?;
                let parent: Option<Vec<u8>> = row.get(3)?;
                Ok((hash, author, created_at, parent))
            })
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let mut items = Vec::new();
        for row in rows {
            let (hash, author, created_at, parent) =
                row.map_err(|e| SocialError::Storage(e.to_string()))?;
            let content_hash = bytes_to_content_hash(&hash);
            let author_key = bytes_to_identity_key(&author);
            let parent_hash = parent.as_deref().map(bytes_to_content_hash);
            let is_reply = parent_hash.is_some();
            items.push(crate::feed::FeedItem {
                content_hash,
                author: author_key,
                created_at: Timestamp::from_secs(created_at as u64),
                is_reply,
                parent: parent_hash,
            });
        }

        Ok(items)
    }

    /// Get mentions for a specific post.
    pub fn get_post_mentions(
        &self,
        content_hash: &[u8],
    ) -> Result<Vec<Mention>, SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let mut stmt = db
            .conn()
            .prepare(
                "SELECT mentioned_key, display_hint, byte_start, byte_end
                 FROM post_mentions
                 WHERE content_hash = ?1
                 ORDER BY byte_start ASC",
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params![content_hash], |row| {
                let key: Vec<u8> = row.get(0)?;
                let display_hint: Option<String> = row.get(1)?;
                let byte_start: i64 = row.get(2)?;
                let byte_end: i64 = row.get(3)?;
                Ok(Mention {
                    mentioned_pubkey: hex::encode(key),
                    display_hint: display_hint.unwrap_or_default(),
                    byte_start: byte_start as usize,
                    byte_end: byte_end as usize,
                })
            })
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let mut mentions = Vec::new();
        for row in rows {
            mentions.push(row.map_err(|e| SocialError::Storage(e.to_string()))?);
        }
        Ok(mentions)
    }
}
