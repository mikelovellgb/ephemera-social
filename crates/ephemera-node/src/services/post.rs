//! Post creation, retrieval, deletion, and listing.
//!
//! Wraps [`PostBuilder`], content store, and SQLite metadata into a
//! single service that the JSON-RPC API layer calls.

use super::identity::IdentityService;
use super::media::{self, MediaFile};
use crate::network::NetworkSubsystem;
use ephemera_abuse::{ActionType, Capability, FingerprintStore, RateLimiter, ReputationScore};
use ephemera_crypto::generate_pow;
use ephemera_mod::{ContentFilter, FilterResult};
use ephemera_post::PostBuilder;
use ephemera_store::{ContentStore, MetadataDb};
use ephemera_types::{IdentityKey, Ttl};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Return the current wall-clock time as Unix seconds.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs()
}

/// Handles post creation, retrieval, deletion, and listing.
///
/// All database interactions go through a [`Mutex<MetadataDb>`] passed by
/// the caller (the [`ServiceContainer`](super::ServiceContainer)).
pub struct PostService;

impl PostService {
    /// Create a post: build, sign, compute PoW, store blob + metadata + media.
    ///
    /// Enforces rate limiting, spam detection, content filtering, and
    /// reputation-based media gating before allowing the post to be created.
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        &self,
        body: &str,
        media_files: Vec<MediaFile>,
        ttl_seconds: Option<u64>,
        parent: Option<&str>,
        identity: &IdentityService,
        content_store: &ContentStore,
        metadata_db: &Mutex<MetadataDb>,
        rate_limiter: &Mutex<RateLimiter>,
        reputation: &Mutex<HashMap<IdentityKey, ReputationScore>>,
        fingerprint_store: &Mutex<FingerprintStore>,
        content_filter: &Mutex<ContentFilter>,
        epoch_key_manager: Option<&Mutex<Option<ephemera_crypto::EpochKeyManager>>>,
    ) -> Result<Value, String> {
        let signing_kp = identity.get_signing_keypair()?;
        let identity_key = signing_kp.public_key();

        // --- Abuse prevention checks ---

        // 1. Rate limiting
        {
            let mut limiter = rate_limiter.lock().map_err(|e| format!("lock: {e}"))?;
            limiter.check(&identity_key, ActionType::Post).map_err(|e| {
                format!("Rate limited: {e}")
            })?;
        }

        // 2. Reputation-based media gating
        if !media_files.is_empty() {
            let rep_map = reputation.lock().map_err(|e| format!("lock: {e}"))?;
            let rep = rep_map.get(&identity_key);
            let default_rep = ReputationScore::new();
            let score = rep.unwrap_or(&default_rep);
            if !score.has_capability(Capability::AttachPhotos) {
                return Err(
                    "Your account is too new to post media. Text-only for the first 7 days."
                        .to_string(),
                );
            }
        }

        // 3. Spam detection (near-duplicate text)
        {
            let mut fps = fingerprint_store.lock().map_err(|e| format!("lock: {e}"))?;
            if fps.check_and_record(body) {
                return Err("near-duplicate content detected, post rejected".to_string());
            }
        }

        // 4. Content filter (blocklist + heuristics)
        {
            let filter = content_filter.lock().map_err(|e| format!("lock: {e}"))?;
            match filter.check_text(body) {
                FilterResult::Block(reason) => {
                    return Err(format!("content blocked: {reason}"));
                }
                FilterResult::RequireReview(reason) => {
                    tracing::info!(reason = %reason, "post flagged for review, allowing for now");
                }
                FilterResult::Allow => {}
            }
        }

        let ttl_secs = ttl_seconds.unwrap_or(86400);
        let ttl = Ttl::from_secs(ttl_secs).map_err(|e| format!("invalid TTL: {e}"))?;
        let mut builder = PostBuilder::new().text(body).ttl(ttl);

        if let Some(parent_hex) = parent {
            let parent_bytes =
                hex::decode(parent_hex).map_err(|e| format!("bad parent hash: {e}"))?;
            if parent_bytes.len() != 32 {
                return Err("parent hash must be 32 bytes".to_string());
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&parent_bytes);
            let parent_hash = ephemera_types::ContentId::from_digest(arr);
            builder = builder.reply_to(parent_hash.clone(), parent_hash, 1);
        }

        // Pre-process media to get content hashes for the post signature.
        let mut media_hashes: Vec<ephemera_types::ContentId> = Vec::new();
        for file in &media_files {
            let processed = ephemera_media::MediaPipeline::process_auto(&file.data)
                .map_err(|e| format!("media processing failed: {e}"))?;
            let hash = match &processed {
                ephemera_media::ProcessedContent::Image(img) => img.content_hash.clone(),
                ephemera_media::ProcessedContent::Video(vid) => vid.content_hash.clone(),
            };
            media_hashes.push(hash);
        }

        for hash in &media_hashes {
            builder = builder.attachment(hash.clone());
        }

        let post = builder
            .build(&signing_kp)
            .map_err(|e| format!("build post: {e}"))?;
        let pow_stamp = generate_pow(post.id.hash_bytes(), 8);
        let post_bytes = serde_json::to_vec(&post).map_err(|e| format!("serialize: {e}"))?;
        let blob_hash = content_store
            .put(&post_bytes)
            .map_err(|e| format!("store blob: {e}"))?;

        // Epoch encryption: encrypt the post blob for cryptographic shredding.
        // When the epoch key is destroyed (after 30 days), the encrypted blob
        // becomes permanently undecryptable.
        let (epoch_encrypted_blob, post_epoch_id): (Option<Vec<u8>>, Option<i64>) =
            if let Some(ekm_mutex) = epoch_key_manager {
                if let Ok(mut ekm_guard) = ekm_mutex.lock() {
                    if let Some(ref mut ekm) = *ekm_guard {
                        match ekm.encrypt_with_current_epoch(&post_bytes) {
                            Ok((epoch_id, sealed)) => {
                                tracing::debug!(epoch_id, "post encrypted with epoch key");
                                (Some(sealed), Some(epoch_id as i64))
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "epoch encryption failed, storing unencrypted");
                                (None, None)
                            }
                        }
                    } else {
                        (None, None)
                    }
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            };

        let now = now_secs() as i64;
        let content_hash_wire = post.id.to_wire_bytes();
        let author_bytes = post.author.as_bytes().to_vec();
        let sig_bytes = post.signature.as_slice().to_vec();
        let expires_at = now + ttl_secs as i64;
        let body_preview = body.chars().take(280).collect::<String>();
        let parent_wire = post.parent.as_ref().map(|p| p.to_wire_bytes());
        let root_wire = post.root.as_ref().map(|r| r.to_wire_bytes());
        let media_count = media_files.len() as i64;
        let has_media = if media_files.is_empty() { 0i64 } else { 1i64 };

        {
            let db = metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
            db.conn()
                .execute(
                    "INSERT INTO posts (
                        content_hash, author_pubkey, sequence_number, created_at,
                        expires_at, ttl_seconds, parent_hash, root_hash, depth,
                        body_preview, media_count, has_media, pow_difficulty,
                        received_at, epoch_number, signature, blob_hash,
                        encrypted_blob, post_epoch_id
                    ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
                    rusqlite::params![
                        content_hash_wire,
                        author_bytes,
                        0i64,
                        now,
                        expires_at,
                        ttl_secs as i64,
                        parent_wire,
                        root_wire,
                        post.depth as i64,
                        body_preview,
                        media_count,
                        has_media,
                        pow_stamp.difficulty as i64,
                        now,
                        1i64,
                        sig_bytes,
                        blob_hash,
                        epoch_encrypted_blob,
                        post_epoch_id,
                    ],
                )
                .map_err(|e| format!("insert post: {e}"))?;

            // Process and store each media file (chunk + metadata).
            for (i, file) in media_files.iter().enumerate() {
                media::process_and_store(file, &content_hash_wire, i, content_store, &db)?;
            }
        }

        let content_hash_hex = hex::encode(post.id.hash_bytes());
        Ok(serde_json::json!({
            "content_hash": content_hash_hex,
            "blob_hash": blob_hash,
            "created_at": now,
            "expires_at": expires_at,
            "media_count": media_count,
        }))
    }

    /// Create a post and publish it to the gossip network.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_and_publish(
        &self,
        body: &str,
        media_files: Vec<MediaFile>,
        ttl_seconds: Option<u64>,
        parent: Option<&str>,
        identity: &IdentityService,
        content_store: &ContentStore,
        metadata_db: &Mutex<MetadataDb>,
        network: &NetworkSubsystem,
        rate_limiter: &Mutex<RateLimiter>,
        reputation: &Mutex<HashMap<IdentityKey, ReputationScore>>,
        fingerprint_store: &Mutex<FingerprintStore>,
        content_filter: &Mutex<ContentFilter>,
        epoch_key_manager: Option<&Mutex<Option<ephemera_crypto::EpochKeyManager>>>,
    ) -> Result<Value, String> {
        let result = self
            .create(
                body,
                media_files,
                ttl_seconds,
                parent,
                identity,
                content_store,
                metadata_db,
                rate_limiter,
                reputation,
                fingerprint_store,
                content_filter,
                epoch_key_manager,
            )
            .await?;

        let blob_hash = result["blob_hash"].as_str().unwrap_or_default().to_string();
        if let Ok(post_bytes) = content_store.get(&blob_hash) {
            if let Err(e) = network.publish_post(post_bytes).await {
                tracing::warn!(error = %e, "failed to publish post to gossip network");
            }
        }

        let mut result = result;
        result["published"] = serde_json::json!(true);
        Ok(result)
    }

    /// Read a post by its content hash (hex of 32-byte digest).
    pub async fn get(
        &self,
        hash_hex: &str,
        _content_store: &ContentStore,
        metadata_db: &Mutex<MetadataDb>,
    ) -> Result<Value, String> {
        let hash_bytes = hex::decode(hash_hex).map_err(|e| format!("bad hash: {e}"))?;
        if hash_bytes.len() != 32 {
            return Err("content hash must be 32 bytes hex".to_string());
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&hash_bytes);
        let content_id = ephemera_types::ContentId::from_digest(arr);
        let wire = content_id.to_wire_bytes();

        let (body_preview, created_at, expires_at, author_hex, media_count): (
            Option<String>,
            i64,
            i64,
            String,
            i64,
        ) = {
            let db = metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
            db.conn()
                .query_row(
                    "SELECT body_preview, created_at, expires_at, lower(hex(author_pubkey)), media_count
                     FROM posts WHERE content_hash = ?1 AND is_tombstone = 0",
                    rusqlite::params![wire],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
                )
                .map_err(|e| format!("post not found: {e}"))?
        };

        Ok(serde_json::json!({
            "content_hash": hash_hex,
            "author": author_hex,
            "body_preview": body_preview,
            "created_at": created_at,
            "expires_at": expires_at,
            "media_count": media_count,
        }))
    }

    /// Delete a post (mark as tombstone).
    pub async fn delete(
        &self,
        hash_hex: &str,
        metadata_db: &Mutex<MetadataDb>,
    ) -> Result<Value, String> {
        let hash_bytes = hex::decode(hash_hex).map_err(|e| format!("bad hash: {e}"))?;
        if hash_bytes.len() != 32 {
            return Err("content hash must be 32 bytes hex".to_string());
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&hash_bytes);
        let content_id = ephemera_types::ContentId::from_digest(arr);
        let wire = content_id.to_wire_bytes();

        let now = now_secs() as i64;
        let db = metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
        let rows = db
            .conn()
            .execute(
                "UPDATE posts SET is_tombstone = 1, tombstone_at = ?1 WHERE content_hash = ?2",
                rusqlite::params![now, wire],
            )
            .map_err(|e| format!("delete: {e}"))?;

        if rows == 0 {
            return Err(format!("post not found: {hash_hex}"));
        }

        Ok(serde_json::json!({ "deleted": true, "content_hash": hash_hex }))
    }

    /// Delete a post and propagate the tombstone via gossip.
    ///
    /// This is the full deletion path used by the node API. After marking
    /// the post as tombstoned locally, it publishes a tombstone message on
    /// the moderation gossip topic so that other nodes can clean up too.
    pub async fn delete_and_propagate(
        &self,
        hash_hex: &str,
        identity: &IdentityService,
        metadata_db: &Mutex<MetadataDb>,
        network: Option<&crate::network::NetworkSubsystem>,
    ) -> Result<Value, String> {
        let result = self.delete(hash_hex, metadata_db).await?;

        // Publish tombstone to the moderation gossip topic so peers clean up.
        if let Some(net) = network {
            let signing_kp = identity.get_signing_keypair()?;
            let author_hex = hex::encode(signing_kp.public_key().as_bytes());
            let tombstone = serde_json::json!({
                "type": "tombstone",
                "content_hash": hash_hex,
                "author": author_hex,
                "tombstoned_at": now_secs(),
            });
            if let Ok(payload) = serde_json::to_vec(&tombstone) {
                let topic = ephemera_gossip::GossipTopic::moderation();
                if let Err(e) = net.publish(&topic, payload).await {
                    tracing::warn!(error = %e, "failed to publish tombstone to gossip");
                } else {
                    tracing::debug!(hash = hash_hex, "tombstone published to moderation topic");
                }
            }
        }

        Ok(result)
    }

    /// List posts by author public key (hex).
    pub async fn list_by_author(
        &self,
        author_hex: &str,
        limit: u64,
        metadata_db: &Mutex<MetadataDb>,
    ) -> Result<Value, String> {
        let author_bytes = hex::decode(author_hex).map_err(|e| format!("bad author: {e}"))?;
        let db = metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT lower(hex(content_hash)), body_preview, created_at, expires_at
                 FROM posts
                 WHERE author_pubkey = ?1 AND is_tombstone = 0
                 ORDER BY created_at DESC LIMIT ?2",
            )
            .map_err(|e| format!("prepare list_by_author query: {e}"))?;

        let rows: Vec<Value> = stmt
            .query_map(rusqlite::params![author_bytes, limit as i64], |row| {
                let hash: String = row.get(0)?;
                let preview: Option<String> = row.get(1)?;
                let created: i64 = row.get(2)?;
                let expires: i64 = row.get(3)?;
                Ok(serde_json::json!({
                    "content_hash": hash, "body_preview": preview,
                    "created_at": created, "expires_at": expires,
                }))
            })
            .map_err(|e| format!("execute list_by_author query: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("map list_by_author row: {e}"))?;

        Ok(serde_json::json!({ "posts": rows, "has_more": false }))
    }

    /// List replies to a parent post.
    pub async fn replies(
        &self,
        parent_hex: &str,
        limit: u64,
        metadata_db: &Mutex<MetadataDb>,
    ) -> Result<Value, String> {
        let parent_bytes = hex::decode(parent_hex).map_err(|e| format!("bad hash: {e}"))?;
        if parent_bytes.len() != 32 {
            return Err("parent hash must be 32 bytes hex".to_string());
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&parent_bytes);
        let parent_id = ephemera_types::ContentId::from_digest(arr);
        let parent_wire = parent_id.to_wire_bytes();

        let db = metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT lower(hex(content_hash)), body_preview, created_at
                 FROM posts
                 WHERE parent_hash = ?1 AND is_tombstone = 0
                 ORDER BY created_at DESC LIMIT ?2",
            )
            .map_err(|e| format!("prepare replies query: {e}"))?;

        let rows: Vec<Value> = stmt
            .query_map(rusqlite::params![parent_wire, limit as i64], |row| {
                let hash: String = row.get(0)?;
                let preview: Option<String> = row.get(1)?;
                let created: i64 = row.get(2)?;
                Ok(serde_json::json!({
                    "content_hash": hash, "body_preview": preview, "created_at": created,
                }))
            })
            .map_err(|e| format!("execute replies query: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("map replies row: {e}"))?;

        Ok(serde_json::json!({ "posts": rows, "has_more": false }))
    }
}
