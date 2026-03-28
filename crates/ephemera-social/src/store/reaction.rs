//! SQLite-backed reaction storage.
//!
//! Implements reaction persistence using the `reactions` table. Each user
//! can have at most one reaction per post (the latest wins).

use super::*;
use crate::interaction::{ReactionEmoji, ReactionSummary};

impl SqliteSocialServices {
    /// Store a reaction. Replaces any existing reaction by the same user on
    /// the same post (one reaction per user per post).
    pub fn react(
        &self,
        post_hash: &str,
        reactor_pubkey: &str,
        emoji: ReactionEmoji,
        created_at: i64,
    ) -> Result<(), SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;
        db.conn()
            .execute(
                "INSERT INTO reactions (post_hash, reactor_pubkey, emoji, created_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(post_hash, reactor_pubkey) DO UPDATE SET
                    emoji = ?3, created_at = ?4",
                rusqlite::params![post_hash, reactor_pubkey, emoji.to_string(), created_at],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;
        Ok(())
    }

    /// Remove a reaction by a user on a post.
    pub fn unreact(
        &self,
        post_hash: &str,
        reactor_pubkey: &str,
    ) -> Result<(), SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;
        db.conn()
            .execute(
                "DELETE FROM reactions WHERE post_hash = ?1 AND reactor_pubkey = ?2",
                rusqlite::params![post_hash, reactor_pubkey],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;
        Ok(())
    }

    /// Get the reaction summary for a post, including counts per emoji and
    /// the current user's reaction (if any).
    pub fn get_reactions(
        &self,
        post_hash: &str,
        current_user_pubkey: Option<&str>,
    ) -> Result<ReactionSummary, SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        // Count reactions per emoji type.
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT emoji, COUNT(*) FROM reactions
                 WHERE post_hash = ?1
                 GROUP BY emoji",
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params![post_hash], |row| {
                let emoji_str: String = row.get(0)?;
                let count: u32 = row.get(1)?;
                Ok((emoji_str, count))
            })
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let mut counts = Vec::new();
        for row in rows {
            let (emoji_str, count) =
                row.map_err(|e| SocialError::Storage(e.to_string()))?;
            if let Ok(emoji) = emoji_str.parse::<ReactionEmoji>() {
                counts.push((emoji, count));
            }
        }

        // Check if the current user has reacted.
        let my_emoji = if let Some(pubkey) = current_user_pubkey {
            let result: Option<String> = db
                .conn()
                .query_row(
                    "SELECT emoji FROM reactions
                     WHERE post_hash = ?1 AND reactor_pubkey = ?2",
                    rusqlite::params![post_hash, pubkey],
                    |row| row.get(0),
                )
                .ok();
            result.and_then(|s| s.parse::<ReactionEmoji>().ok())
        } else {
            None
        };

        Ok(ReactionSummary { counts, my_emoji })
    }
}
