//! SQLite-backed topic room storage.
//!
//! Implements topic room persistence using the `topic_rooms`,
//! `topic_subscriptions`, and `topic_posts` tables.

use super::*;
use crate::topic_room::{topic_id_from_name, TopicRoom};

impl SqliteSocialServices {
    /// Create a new topic room and subscribe the creator.
    pub fn create_topic(
        &self,
        name: &str,
        description: Option<&str>,
        creator_pubkey: &str,
        created_at: i64,
    ) -> Result<TopicRoom, SocialError> {
        if name.is_empty() || name.len() > 128 {
            return Err(SocialError::Validation(
                "topic name must be 1-128 characters".into(),
            ));
        }

        let topic_id = topic_id_from_name(name);

        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        // Insert the room (fail if already exists).
        let inserted = db
            .conn()
            .execute(
                "INSERT OR IGNORE INTO topic_rooms (topic_id, name, description, created_by, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![topic_id, name, description, creator_pubkey, created_at],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        if inserted == 0 {
            return Err(SocialError::Validation(format!(
                "topic room '{name}' already exists"
            )));
        }

        // Auto-subscribe the creator.
        db.conn()
            .execute(
                "INSERT OR IGNORE INTO topic_subscriptions (topic_id, user_pubkey, subscribed_at)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![topic_id, creator_pubkey, created_at],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(TopicRoom {
            topic_id,
            name: name.to_string(),
            description: description.map(String::from),
            created_by: creator_pubkey.to_string(),
            created_at: Timestamp::from_secs(created_at as u64),
        })
    }

    /// Subscribe a user to a topic room.
    pub fn join_topic(
        &self,
        topic_id: &str,
        user_pubkey: &str,
        subscribed_at: i64,
    ) -> Result<(), SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        // Verify the topic exists.
        let exists: bool = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM topic_rooms WHERE topic_id = ?1",
                rusqlite::params![topic_id],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !exists {
            return Err(SocialError::NotFound(format!(
                "topic room {topic_id} not found"
            )));
        }

        db.conn()
            .execute(
                "INSERT OR IGNORE INTO topic_subscriptions (topic_id, user_pubkey, subscribed_at)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![topic_id, user_pubkey, subscribed_at],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(())
    }

    /// Unsubscribe a user from a topic room.
    pub fn leave_topic(
        &self,
        topic_id: &str,
        user_pubkey: &str,
    ) -> Result<(), SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        db.conn()
            .execute(
                "DELETE FROM topic_subscriptions WHERE topic_id = ?1 AND user_pubkey = ?2",
                rusqlite::params![topic_id, user_pubkey],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(())
    }

    /// List all known topic rooms.
    pub fn list_topics(&self) -> Result<Vec<TopicRoom>, SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let mut stmt = db
            .conn()
            .prepare(
                "SELECT topic_id, name, description, created_by, created_at
                 FROM topic_rooms ORDER BY created_at DESC",
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                let topic_id: String = row.get(0)?;
                let name: String = row.get(1)?;
                let description: Option<String> = row.get(2)?;
                let created_by: String = row.get(3)?;
                let created_at: i64 = row.get(4)?;
                Ok(TopicRoom {
                    topic_id,
                    name,
                    description,
                    created_by,
                    created_at: Timestamp::from_secs(created_at as u64),
                })
            })
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let mut topics = Vec::new();
        for row in rows {
            topics.push(row.map_err(|e| SocialError::Storage(e.to_string()))?);
        }

        Ok(topics)
    }

    /// Link a post to a topic room.
    pub fn post_to_topic(
        &self,
        topic_id: &str,
        content_hash: &[u8],
        created_at: i64,
    ) -> Result<(), SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        db.conn()
            .execute(
                "INSERT OR IGNORE INTO topic_posts (topic_id, content_hash, created_at)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![topic_id, content_hash, created_at],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(())
    }

    /// Get paginated feed for a topic room.
    pub fn get_topic_feed(
        &self,
        topic_id: &str,
        cursor: Option<&FeedCursor>,
        limit: u32,
    ) -> Result<FeedPage, SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;
        let fetch_limit = limit.min(200) as i64 + 1;

        let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match cursor {
            Some(c) => {
                let cursor_ts = c.created_at.as_secs() as i64;
                (
                    "SELECT tp.content_hash, p.author_pubkey, tp.created_at, p.parent_hash
                     FROM topic_posts tp
                     JOIN posts p ON tp.content_hash = p.content_hash
                     WHERE tp.topic_id = ?1
                       AND tp.created_at < ?2
                       AND p.is_tombstone = 0
                     ORDER BY tp.created_at DESC
                     LIMIT ?3",
                    vec![
                        Box::new(topic_id.to_string()) as Box<dyn rusqlite::types::ToSql>,
                        Box::new(cursor_ts),
                        Box::new(fetch_limit),
                    ],
                )
            }
            None => (
                "SELECT tp.content_hash, p.author_pubkey, tp.created_at, p.parent_hash
                 FROM topic_posts tp
                 JOIN posts p ON tp.content_hash = p.content_hash
                 WHERE tp.topic_id = ?1
                   AND p.is_tombstone = 0
                 ORDER BY tp.created_at DESC
                 LIMIT ?2",
                vec![
                    Box::new(topic_id.to_string()) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(fetch_limit),
                ],
            ),
        };

        let mut stmt = db
            .conn()
            .prepare(sql)
            .map_err(|e| SocialError::Storage(e.to_string()))?;
        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| &**p).collect();
        let rows = stmt
            .query_map(params_refs.as_slice(), |row| {
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
            items.push(FeedItem {
                content_hash,
                author: author_key,
                created_at: Timestamp::from_secs(created_at as u64),
                is_reply,
                parent: parent_hash,
            });
        }

        let has_more = items.len() > limit as usize;
        if has_more {
            items.truncate(limit as usize);
        }

        let next_cursor = if has_more {
            items.last().map(|item| FeedCursor {
                created_at: item.created_at,
                content_hash: item.content_hash.clone(),
            })
        } else {
            None
        };

        Ok(FeedPage {
            items,
            next_cursor,
            has_more,
        })
    }
}
