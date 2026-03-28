//! SQLite-backed implementation of [`FeedService`].

use super::feed_discover::get_excluded_keys;
use super::*;

#[async_trait::async_trait]
impl FeedService for SqliteSocialServices {
    async fn connections_feed(
        &self,
        identity: &IdentityKey,
        cursor: Option<&FeedCursor>,
        page_size: u32,
    ) -> Result<FeedPage, SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;
        let local_bytes = identity.as_bytes().to_vec();
        let limit = page_size.min(200) as i64 + 1; // fetch one extra to detect has_more

        // Get connected peers + self.
        let mut connected_keys: Vec<Vec<u8>> = vec![local_bytes.clone()];
        {
            let mut stmt = db
                .conn()
                .prepare(
                    "SELECT remote_pubkey FROM connections
                     WHERE local_pubkey = ?1 AND status = 'connected'",
                )
                .map_err(|e| SocialError::Storage(e.to_string()))?;
            let rows = stmt
                .query_map(rusqlite::params![local_bytes], |row| {
                    row.get::<_, Vec<u8>>(0)
                })
                .map_err(|e| SocialError::Storage(e.to_string()))?;
            for pk in rows.flatten() {
                connected_keys.push(pk);
            }
        }

        // Get blocked + muted identities to exclude.
        let excluded = get_excluded_keys(&db, &local_bytes);

        // Build placeholders for IN clause.
        let placeholders: Vec<String> = connected_keys
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect();
        let in_clause = placeholders.join(", ");

        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match cursor {
            Some(c) => {
                let cursor_ts = c.created_at.as_secs() as i64;
                let offset = connected_keys.len() + 1;
                let limit_idx = offset + 1;
                (
                    format!(
                        "SELECT content_hash, author_pubkey, created_at, parent_hash
                         FROM posts
                         WHERE author_pubkey IN ({in_clause})
                           AND is_tombstone = 0
                           AND created_at < ?{offset}
                         ORDER BY created_at DESC
                         LIMIT ?{limit_idx}"
                    ),
                    {
                        let mut p: Vec<Box<dyn rusqlite::types::ToSql>> = connected_keys
                            .iter()
                            .map(|k| Box::new(k.clone()) as Box<dyn rusqlite::types::ToSql>)
                            .collect();
                        p.push(Box::new(cursor_ts));
                        p.push(Box::new(limit));
                        p
                    },
                )
            }
            None => {
                let limit_idx = connected_keys.len() + 1;
                (
                    format!(
                        "SELECT content_hash, author_pubkey, created_at, parent_hash
                         FROM posts
                         WHERE author_pubkey IN ({in_clause})
                           AND is_tombstone = 0
                         ORDER BY created_at DESC
                         LIMIT ?{limit_idx}"
                    ),
                    {
                        let mut p: Vec<Box<dyn rusqlite::types::ToSql>> = connected_keys
                            .iter()
                            .map(|k| Box::new(k.clone()) as Box<dyn rusqlite::types::ToSql>)
                            .collect();
                        p.push(Box::new(limit));
                        p
                    },
                )
            }
        };

        let mut stmt = db
            .conn()
            .prepare(&sql)
            .map_err(|e| SocialError::Storage(e.to_string()))?;
        let params_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| &**p).collect();
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

            // Skip excluded authors.
            if excluded.contains(&author) {
                continue;
            }

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

        let has_more = items.len() > page_size as usize;
        if has_more {
            items.truncate(page_size as usize);
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

    async fn profile_feed(
        &self,
        _viewer: &IdentityKey,
        profile_owner: &IdentityKey,
        cursor: Option<&FeedCursor>,
        page_size: u32,
    ) -> Result<FeedPage, SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;
        let owner_bytes = profile_owner.as_bytes().to_vec();
        let limit = page_size.min(200) as i64 + 1;

        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match cursor {
            Some(c) => {
                let cursor_ts = c.created_at.as_secs() as i64;
                (
                    "SELECT content_hash, author_pubkey, created_at, parent_hash
                     FROM posts
                     WHERE author_pubkey = ?1 AND is_tombstone = 0 AND created_at < ?2
                     ORDER BY created_at DESC LIMIT ?3"
                        .to_string(),
                    vec![
                        Box::new(owner_bytes) as Box<dyn rusqlite::types::ToSql>,
                        Box::new(cursor_ts),
                        Box::new(limit),
                    ],
                )
            }
            None => (
                "SELECT content_hash, author_pubkey, created_at, parent_hash
                 FROM posts
                 WHERE author_pubkey = ?1 AND is_tombstone = 0
                 ORDER BY created_at DESC LIMIT ?2"
                    .to_string(),
                vec![
                    Box::new(owner_bytes) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(limit),
                ],
            ),
        };

        let mut stmt = db
            .conn()
            .prepare(&sql)
            .map_err(|e| SocialError::Storage(e.to_string()))?;
        let params_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| &**p).collect();
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

        let has_more = items.len() > page_size as usize;
        if has_more {
            items.truncate(page_size as usize);
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
