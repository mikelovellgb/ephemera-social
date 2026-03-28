//! Gossip ingest: receives posts from the gossip network, validates them,
//! and stores them locally.
//!
//! This module bridges the gossip subsystem with the storage layer.
//! When a gossip message arrives on the public feed topic:
//! 1. Deserialize the post from the payload
//! 2. Validate signature, PoW, TTL
//! 3. Store the blob in ContentStore
//! 4. Write metadata to SQLite
//! 5. Emit a PostReceived event

use ephemera_abuse::{ActionType, FingerprintStore, RateLimiter};
use ephemera_events::{Event, EventBus};
use ephemera_gossip::TopicSubscription;
use ephemera_mod::{ContentFilter, FilterResult};
use ephemera_post::{validate_post, Post};
use ephemera_store::{ContentStore, MetadataDb};
use ephemera_types::{ContentId, IdentityKey};

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs()
}

/// Process incoming gossip messages from the public feed subscription.
///
/// This function runs as a background task. It reads messages from the
/// gossip subscription, validates them as posts, and stores valid ones.
///
/// Exits when the subscription channel closes or the shutdown signal fires.
pub async fn gossip_ingest_loop(
    mut subscription: TopicSubscription,
    content_store: ContentStore,
    metadata_db: Mutex<MetadataDb>,
    event_bus: EventBus,
    rate_limiter: Mutex<RateLimiter>,
    fingerprint_store: Mutex<FingerprintStore>,
    content_filter: Mutex<ContentFilter>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            msg = subscription.recv() => {
                let msg = match msg {
                    Some(m) => m,
                    None => {
                        tracing::debug!("gossip ingest: subscription channel closed");
                        break;
                    }
                };

                // Attempt to deserialize the gossip payload as a Post.
                let post: Post = match serde_json::from_slice(&msg.payload) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::trace!(
                            error = %e,
                            "gossip ingest: payload is not a valid Post, skipping"
                        );
                        continue;
                    }
                };

                // Validate the post (signature, expiry, text length, etc.)
                if let Err(e) = validate_post(&post) {
                    tracing::warn!(
                        error = %e,
                        hash = %post.id,
                        "gossip ingest: post failed validation"
                    );
                    continue;
                }

                // --- Abuse prevention checks ---

                // Rate limiting on the author identity.
                {
                    let mut limiter = match rate_limiter.lock() {
                        Ok(l) => l,
                        Err(_) => continue,
                    };
                    if limiter.check(&post.author, ActionType::Post).is_err() {
                        tracing::warn!(
                            author = %hex::encode(post.author.as_bytes()),
                            "gossip ingest: rate limited incoming post"
                        );
                        continue;
                    }
                }

                // Spam detection (near-duplicate text).
                if let Some(text) = post.content.text_body() {
                    let mut fps = match fingerprint_store.lock() {
                        Ok(f) => f,
                        Err(_) => continue,
                    };
                    if fps.check_and_record(text) {
                        tracing::warn!("gossip ingest: near-duplicate spam detected, dropping");
                        continue;
                    }
                }

                // Content filter (blocklist + heuristics).
                if let Some(text) = post.content.text_body() {
                    let filter = match content_filter.lock() {
                        Ok(f) => f,
                        Err(_) => continue,
                    };
                    match filter.check_text(text) {
                        FilterResult::Block(reason) => {
                            tracing::warn!(
                                reason = %reason,
                                "gossip ingest: content blocked by filter"
                            );
                            continue;
                        }
                        FilterResult::RequireReview(reason) => {
                            tracing::info!(
                                reason = %reason,
                                "gossip ingest: content flagged for review"
                            );
                        }
                        FilterResult::Allow => {}
                    }
                }

                // Store the post.
                match store_received_post(&post, &msg.payload, &content_store, &metadata_db) {
                    Ok(()) => {
                        tracing::debug!(
                            hash = %post.id,
                            author = %post.author,
                            "gossip ingest: stored post from network"
                        );

                        // Emit PostReceived event.
                        let content_id = ContentId::from_hash(*post.id.hash_bytes());
                        let author = IdentityKey::from_bytes(*post.author.as_bytes());
                        event_bus.emit(Event::PostReceived {
                            content_id,
                            author,
                        });
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            hash = %post.id,
                            "gossip ingest: failed to store post"
                        );
                    }
                }
            }
            _ = shutdown_rx.changed() => {
                tracing::debug!("gossip ingest: received shutdown signal");
                break;
            }
        }
    }
}

/// Store a validated post received from the gossip network.
///
/// Writes the serialized post blob to the content store and inserts
/// metadata into SQLite. Duplicate posts (same content hash) are
/// silently ignored.
fn store_received_post(
    post: &Post,
    post_bytes: &[u8],
    content_store: &ContentStore,
    metadata_db: &Mutex<MetadataDb>,
) -> Result<(), String> {
    // Store the blob.
    let blob_hash = content_store
        .put(post_bytes)
        .map_err(|e| format!("content store put: {e}"))?;

    let now = now_secs() as i64;
    let content_hash_wire = post.id.to_wire_bytes();
    let author_bytes = post.author.as_bytes().to_vec();
    let sig_bytes = post.signature.as_slice().to_vec();
    let ttl_secs = post.ttl.as_secs() as i64;
    let expires_at = post.created_at.as_secs() as i64 + ttl_secs;
    let body_preview = post
        .content
        .text_body()
        .map(|b| b.chars().take(280).collect::<String>());
    let parent_wire = post.parent.as_ref().map(|p| p.to_wire_bytes());
    let root_wire = post.root.as_ref().map(|r| r.to_wire_bytes());
    let media_count = post.content.attachment_count() as i64;
    let has_media = if media_count > 0 { 1i64 } else { 0i64 };

    let db = metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
    db.conn()
        .execute(
            "INSERT OR IGNORE INTO posts (
                content_hash, author_pubkey, sequence_number, created_at,
                expires_at, ttl_seconds, parent_hash, root_hash, depth,
                body_preview, media_count, has_media, pow_difficulty,
                received_at, epoch_number, signature, blob_hash
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
            rusqlite::params![
                content_hash_wire,
                author_bytes,
                0i64,
                post.created_at.as_secs() as i64,
                expires_at,
                ttl_secs,
                parent_wire,
                root_wire,
                post.depth as i64,
                body_preview,
                media_count,
                has_media,
                post.pow_proof.as_bytes().len() as i64,
                now,
                1i64,
                sig_bytes,
                blob_hash,
            ],
        )
        .map_err(|e| format!("insert post: {e}"))?;

    Ok(())
}
