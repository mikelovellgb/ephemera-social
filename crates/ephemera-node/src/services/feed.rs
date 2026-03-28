//! Feed assembly from connections, discovery, topics, and tags.
//!
//! Provides connection-filtered, discovery, topic, and tag-based feed
//! queries. Falls back to an unfiltered reverse-chronological feed when
//! the identity is locked.

use super::identity::IdentityService;
use ephemera_social::store::SqliteSocialServices;
use ephemera_social::FeedService as _;
use ephemera_store::MetadataDb;
use serde_json::Value;
use std::sync::Mutex;

/// Assembles content feeds from connections, discovery, topics, and tags.
///
/// When an unlocked identity and social services are available, the
/// connections feed filters to posts from the user's social graph. Otherwise
/// it falls back to an unfiltered reverse-chronological query.
pub struct FeedService;

impl FeedService {
    /// Get connections feed: posts from connected users and self, ordered chronologically.
    ///
    /// When the identity is unlocked, this filters posts to only those from
    /// the user's connections. When locked (no identity), falls back to the
    /// unfiltered query for backward compatibility.
    pub async fn connections(
        &self,
        limit: u64,
        cursor: Option<i64>,
        metadata_db: &Mutex<MetadataDb>,
        identity: Option<&IdentityService>,
        social_services: Option<&SqliteSocialServices>,
    ) -> Result<Value, String> {
        // If we have an identity and social services, use the real connection-filtered feed.
        // After fetching feed items (which have content_hash + author + created_at),
        // enrich each post with body_preview, expires_at, and ttl_seconds from the
        // posts table so the frontend has enough data to render cards.
        if let (Some(id_svc), Some(social_svc)) = (identity, social_services) {
            if let Ok(signing_kp) = id_svc.get_signing_keypair() {
                let local = signing_kp.public_key();
                let local_hex = hex::encode(local.as_bytes());
                let page_size = limit.min(200) as u32;
                let feed_cursor = cursor.map(|ts| ephemera_social::FeedCursor {
                    created_at: ephemera_types::Timestamp::from_secs(ts as u64),
                    content_hash: ephemera_types::ContentId::from_digest([0u8; 32]),
                });

                let page = social_svc
                    .connections_feed(&local, feed_cursor.as_ref(), page_size)
                    .await
                    .map_err(|e| format!("connections feed: {e}"))?;

                // Enrich feed items with post body and expiry from metadata db.
                let db = metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
                let posts: Vec<Value> = page
                    .items
                    .iter()
                    .map(|item| {
                        let content_hash_hex = hex::encode(item.content_hash.hash_bytes());
                        let author_hex = hex::encode(item.author.as_bytes());
                        let wire = item.content_hash.to_wire_bytes();
                        let is_own = author_hex == local_hex;

                        // Look up body_preview, expires_at, ttl_seconds, media_count from posts table.
                        let enriched = db.conn().query_row(
                            "SELECT body_preview, expires_at, ttl_seconds, media_count
                             FROM posts WHERE content_hash = ?1 AND is_tombstone = 0",
                            rusqlite::params![wire],
                            |row| {
                                let preview: Option<String> = row.get(0)?;
                                let expires: i64 = row.get(1)?;
                                let ttl: i64 = row.get(2)?;
                                let media_count: i64 = row.get(3)?;
                                Ok((preview, expires, ttl, media_count))
                            },
                        );

                        // Look up author display_name and avatar_cid from profiles table (F-03).
                        let (author_display_name, author_avatar_url): (Option<String>, Option<String>) = db
                            .conn()
                            .query_row(
                                "SELECT display_name, avatar_cid FROM profiles WHERE pubkey = ?1",
                                rusqlite::params![item.author.as_bytes().to_vec()],
                                |row| {
                                    let name: Option<String> = row.get(0)?;
                                    let avatar_cid: Option<Vec<u8>> = row.get(1)?;
                                    let avatar_url = avatar_cid.map(|cid| format!("/media/{}", hex::encode(cid)));
                                    Ok((name, avatar_url))
                                },
                            )
                            .unwrap_or((None, None));

                        // Look up media attachments for this post (F-06).
                        let media_items: Vec<Value> = if let Ok(attachments) =
                            ephemera_store::list_media_for_post(&db, &wire)
                        {
                            attachments
                                .iter()
                                .map(|a| {
                                    let media_url = format!("/media/{}", a.id);
                                    let thumbnail_url = if a.thumbnail_hash.is_some() {
                                        Some(format!("/media/{}/thumbnail", a.id))
                                    } else {
                                        None
                                    };
                                    serde_json::json!({
                                        "id": a.id,
                                        "media_type": a.media_type,
                                        "mime_type": a.mime_type,
                                        "width": a.width,
                                        "height": a.height,
                                        "duration_ms": a.duration_ms,
                                        "url": media_url,
                                        "thumbnail_url": thumbnail_url,
                                        "variants": [{
                                            "url": media_url,
                                            "mime_type": a.mime_type,
                                            "width": a.width,
                                            "height": a.height,
                                        }],
                                    })
                                })
                                .collect()
                        } else {
                            Vec::new()
                        };

                        match enriched {
                            Ok((body_preview, expires_at, ttl_seconds, media_count)) => {
                                serde_json::json!({
                                    "content_hash": content_hash_hex,
                                    "author": author_hex,
                                    "author_display_name": author_display_name,
                                    "author_avatar_url": author_avatar_url,
                                    "body": body_preview.unwrap_or_default(),
                                    "created_at": item.created_at.as_secs(),
                                    "expires_at": expires_at,
                                    "ttl_seconds": ttl_seconds,
                                    "is_reply": item.is_reply,
                                    "is_own": is_own,
                                    "media_count": media_count,
                                    "media": media_items,
                                })
                            }
                            Err(_) => {
                                // Post may have been GC'd between feed query and enrichment.
                                serde_json::json!({
                                    "content_hash": content_hash_hex,
                                    "author": author_hex,
                                    "author_display_name": author_display_name,
                                    "author_avatar_url": author_avatar_url,
                                    "body": "",
                                    "created_at": item.created_at.as_secs(),
                                    "is_reply": item.is_reply,
                                    "is_own": is_own,
                                    "media_count": 0,
                                    "media": [],
                                })
                            }
                        }
                    })
                    .collect();
                drop(db);

                let next_cursor = page
                    .next_cursor
                    .as_ref()
                    .map(|c| c.created_at.as_secs() as i64);

                return Ok(serde_json::json!({
                    "posts": posts,
                    "next_cursor": next_cursor,
                    "has_more": page.has_more,
                }));
            }
        }

        // Fallback: unfiltered query (backward compatible for when identity is locked).
        self.connections_unfiltered(limit, cursor, metadata_db)
            .await
    }

    /// Unfiltered feed query (original implementation, used when identity is locked).
    async fn connections_unfiltered(
        &self,
        limit: u64,
        cursor: Option<i64>,
        metadata_db: &Mutex<MetadataDb>,
    ) -> Result<Value, String> {
        let before_rowid = cursor.unwrap_or(i64::MAX);
        let db = metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT p.rowid, lower(hex(p.content_hash)), lower(hex(p.author_pubkey)), p.body_preview,
                        p.created_at, p.expires_at, p.ttl_seconds, pr.display_name, pr.avatar_cid
                 FROM posts p
                 LEFT JOIN profiles pr ON p.author_pubkey = pr.pubkey
                 WHERE p.is_tombstone = 0 AND p.rowid < ?1
                 ORDER BY p.rowid DESC LIMIT ?2",
            )
            .map_err(|e| format!("prepare feed query: {e}"))?;

        let rows: Vec<Value> = stmt
            .query_map(rusqlite::params![before_rowid, limit as i64], |row| {
                let rid: i64 = row.get(0)?;
                let hash: String = row.get(1)?;
                let author: String = row.get(2)?;
                let preview: Option<String> = row.get(3)?;
                let created: i64 = row.get(4)?;
                let expires: i64 = row.get(5)?;
                let ttl: i64 = row.get::<_, Option<i64>>(6)?.unwrap_or(86400);
                let display_name: Option<String> = row.get(7)?;
                let avatar_cid: Option<Vec<u8>> = row.get(8)?;
                let avatar_url = avatar_cid.map(|cid| format!("/media/{}", hex::encode(cid)));
                Ok(serde_json::json!({
                    "rowid": rid, "content_hash": hash, "author": author,
                    "author_display_name": display_name,
                    "author_avatar_url": avatar_url,
                    "body": preview, "body_preview": preview,
                    "created_at": created, "expires_at": expires,
                    "ttl_seconds": ttl,
                }))
            })
            .map_err(|e| format!("execute feed query: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("map feed row: {e}"))?;

        let next_cursor = rows.last().and_then(|r| r["rowid"].as_i64());
        let has_more = rows.len() == limit as usize;

        Ok(serde_json::json!({
            "posts": rows, "next_cursor": next_cursor, "has_more": has_more,
        }))
    }

    /// Discover feed: posts from non-connected users.
    pub async fn discover(
        &self,
        limit: u64,
        metadata_db: &Mutex<MetadataDb>,
        identity: Option<&IdentityService>,
        social_services: Option<&SqliteSocialServices>,
    ) -> Result<Value, String> {
        // If we have identity and social services, use the real discover feed.
        if let (Some(id_svc), Some(social_svc)) = (identity, social_services) {
            if let Ok(signing_kp) = id_svc.get_signing_keypair() {
                let local = signing_kp.public_key();
                let page_size = limit.min(200) as u32;

                let page =
                    ephemera_social::store::discover_feed(social_svc, &local, None, page_size)
                        .await
                        .map_err(|e| format!("discover feed: {e}"))?;

                let posts: Vec<Value> = page
                    .items
                    .iter()
                    .map(|item| {
                        serde_json::json!({
                            "content_hash": hex::encode(item.content_hash.hash_bytes()),
                            "author": hex::encode(item.author.as_bytes()),
                            "created_at": item.created_at.as_secs(),
                            "is_reply": item.is_reply,
                        })
                    })
                    .collect();

                return Ok(serde_json::json!({
                    "posts": posts,
                    "has_more": page.has_more,
                }));
            }
        }

        // Fallback to unfiltered.
        self.connections_unfiltered(limit, None, metadata_db).await
    }

    /// Get posts for a topic room (placeholder -- returns empty for now).
    pub async fn topic(&self, _room_id: &str, _limit: u64) -> Result<Value, String> {
        Ok(serde_json::json!({ "posts": [], "has_more": false }))
    }

    /// Search posts by hashtag.
    pub async fn search_tags(
        &self,
        tag: &str,
        limit: u64,
        metadata_db: &Mutex<MetadataDb>,
    ) -> Result<Value, String> {
        let db = metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT lower(hex(p.content_hash)), p.body_preview, p.created_at
                 FROM posts p
                 INNER JOIN post_tags t ON p.content_hash = t.content_hash
                 WHERE t.tag = ?1 AND p.is_tombstone = 0
                 ORDER BY p.created_at DESC LIMIT ?2",
            )
            .map_err(|e| format!("prepare tag search query: {e}"))?;

        let rows: Vec<Value> = stmt
            .query_map(rusqlite::params![tag, limit as i64], |row| {
                let hash: String = row.get(0)?;
                let preview: Option<String> = row.get(1)?;
                let created: i64 = row.get(2)?;
                Ok(serde_json::json!({
                    "content_hash": hash, "body_preview": preview, "created_at": created,
                }))
            })
            .map_err(|e| format!("execute tag search query: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("map tag search row: {e}"))?;

        Ok(serde_json::json!({ "posts": rows, "has_more": false }))
    }
}
