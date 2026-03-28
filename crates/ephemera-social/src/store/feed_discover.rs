//! Discovery feed query: returns posts from authors the viewer is NOT
//! connected to, providing content from outside the social graph.

use super::*;

/// Build a "discover" feed showing posts from authors the viewer is NOT connected to.
pub async fn discover_feed(
    services: &SqliteSocialServices,
    viewer: &IdentityKey,
    cursor: Option<&FeedCursor>,
    page_size: u32,
) -> Result<FeedPage, SocialError> {
    let db = services
        .db
        .lock()
        .map_err(|e| SocialError::Storage(e.to_string()))?;
    let viewer_bytes = viewer.as_bytes().to_vec();
    let limit = page_size.min(200) as i64 + 1;

    // Get connected peers + self (to exclude).
    let mut excluded_keys: Vec<Vec<u8>> = vec![viewer_bytes.clone()];
    {
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT remote_pubkey FROM connections
                 WHERE local_pubkey = ?1 AND status = 'connected'",
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;
        let rows = stmt
            .query_map(rusqlite::params![viewer_bytes], |row| {
                row.get::<_, Vec<u8>>(0)
            })
            .map_err(|e| SocialError::Storage(e.to_string()))?;
        for pk in rows.flatten() {
            excluded_keys.push(pk);
        }
    }

    // Also add blocked keys.
    let blocked = get_excluded_keys(&db, &viewer_bytes);
    excluded_keys.extend(blocked);

    // Build NOT IN clause.
    let placeholders: Vec<String> = excluded_keys
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect();
    let not_in_clause = placeholders.join(", ");

    let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match cursor {
        Some(c) => {
            let cursor_ts = c.created_at.as_secs() as i64;
            let offset = excluded_keys.len() + 1;
            let limit_idx = offset + 1;
            (
                format!(
                    "SELECT content_hash, author_pubkey, created_at, parent_hash
                     FROM posts
                     WHERE author_pubkey NOT IN ({not_in_clause})
                       AND is_tombstone = 0
                       AND created_at < ?{offset}
                     ORDER BY created_at DESC
                     LIMIT ?{limit_idx}"
                ),
                {
                    let mut p: Vec<Box<dyn rusqlite::types::ToSql>> = excluded_keys
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
            let limit_idx = excluded_keys.len() + 1;
            (
                format!(
                    "SELECT content_hash, author_pubkey, created_at, parent_hash
                     FROM posts
                     WHERE author_pubkey NOT IN ({not_in_clause})
                       AND is_tombstone = 0
                     ORDER BY created_at DESC
                     LIMIT ?{limit_idx}"
                ),
                {
                    let mut p: Vec<Box<dyn rusqlite::types::ToSql>> = excluded_keys
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

/// Helper: get keys that should be excluded from feeds (blocked + muted).
pub(super) fn get_excluded_keys(db: &MetadataDb, local_bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut excluded = Vec::new();

    // Blocked users.
    if let Ok(mut stmt) = db
        .conn()
        .prepare("SELECT blocked_pubkey FROM blocks WHERE blocker_pubkey = ?1")
    {
        if let Ok(rows) = stmt.query_map(rusqlite::params![local_bytes], |row| {
            row.get::<_, Vec<u8>>(0)
        }) {
            for row in rows.flatten() {
                excluded.push(row);
            }
        }
    }

    // Muted users (permanent or not yet expired).
    let now_secs = Timestamp::now().as_secs() as i64;
    if let Ok(mut stmt) = db.conn().prepare(
        "SELECT muted_pubkey FROM mutes
         WHERE muter_pubkey = ?1
           AND (expires_at IS NULL OR expires_at > ?2)",
    ) {
        if let Ok(rows) = stmt.query_map(rusqlite::params![local_bytes, now_secs], |row| {
            row.get::<_, Vec<u8>>(0)
        }) {
            for row in rows.flatten() {
                excluded.push(row);
            }
        }
    }

    excluded
}

/// Helper to insert a test post (only used in tests).
#[cfg(test)]
pub(crate) fn insert_test_post(db: &MetadataDb, author: &IdentityKey, created_at: i64, ttl: i64) {
    let author_bytes = author.as_bytes().to_vec();
    let hash = ephemera_crypto::blake3_hash(&created_at.to_le_bytes());
    let mut content_hash = vec![0x01u8]; // 1-byte type prefix
    content_hash.extend_from_slice(&hash);
    let sig = vec![0xBBu8; 64];
    db.conn()
        .execute(
            "INSERT INTO posts (content_hash, author_pubkey, sequence_number, created_at,
             expires_at, ttl_seconds, received_at, epoch_number, signature)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?4, 1, ?7)",
            rusqlite::params![
                content_hash,
                author_bytes,
                created_at,
                created_at,
                created_at + ttl,
                ttl,
                sig
            ],
        )
        .unwrap();
}
