//! Integration tests for the Ephemera core loop:
//! Create Identity -> Create Post -> Store to SQLite+Filesystem -> Read Feed
//!
//! These tests prove the end-to-end flow actually works with real storage.

use ephemera_config::NodeConfig;
use ephemera_events::EventBus;
use ephemera_node::services::ServiceContainer;
use std::sync::Arc;

/// Helper: create a ServiceContainer backed by a real temp directory.
fn make_services() -> (Arc<ServiceContainer>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let config = NodeConfig::default_for(dir.path());
    let event_bus = EventBus::new();
    let svc = Arc::new(ServiceContainer::new(&config, event_bus).unwrap());
    (svc, dir)
}

/// Test 1: Create an identity, verify the keystore file exists on disk.
#[tokio::test]
async fn test_create_identity_stores_keys() {
    let dir = tempfile::tempdir().unwrap();
    let config = NodeConfig::default_for(dir.path());
    let keystore_path = config.keystore_path();
    let event_bus = EventBus::new();
    let svc = Arc::new(ServiceContainer::new(&config, event_bus).unwrap());

    // Before creation, keystore should not exist.
    assert!(
        !keystore_path.exists(),
        "keystore should not exist before create"
    );

    // Create identity.
    let result = svc.identity.create("test-passphrase").await.unwrap();
    let pubkey = result["pseudonym_pubkey"].as_str().unwrap();

    // Pubkey should be 64 hex chars (32 bytes).
    assert_eq!(pubkey.len(), 64, "pubkey should be 64 hex chars");

    // Keystore file should now exist.
    assert!(
        keystore_path.exists(),
        "keystore file should exist after create"
    );

    // The keystore should be non-trivially sized (encrypted data).
    let keystore_size = std::fs::metadata(&keystore_path).unwrap().len();
    assert!(keystore_size > 50, "keystore should contain encrypted data");

    // We should be able to get the active identity.
    let active = svc.identity.get_active().await.unwrap();
    assert_eq!(active["pubkey"].as_str().unwrap(), pubkey);
    assert_eq!(active["index"].as_u64().unwrap(), 0);
}

/// Test 2: Create a post, read it back, verify content matches.
#[tokio::test]
async fn test_create_post_roundtrip() {
    let (svc, _dir) = make_services();

    // Create identity first (required for signing).
    svc.identity.create("test-pass").await.unwrap();

    // Create a post.
    let create_result = svc
        .posts
        .create(
            "Hello, Ephemera! This is a test post.",
            vec![],
            Some(86400),
            None,
            &svc.identity,
            svc.content_store(),
            &svc.metadata_db,
            &svc.rate_limiter,
            &svc.reputation,
            &svc.fingerprint_store,
            &svc.content_filter,
            None,
        )
        .await
        .unwrap();

    let content_hash = create_result["content_hash"].as_str().unwrap().to_string();
    assert_eq!(
        content_hash.len(),
        64,
        "content hash should be 64 hex chars"
    );

    // The blob should exist in the content store.
    let blob_hash = create_result["blob_hash"].as_str().unwrap();
    assert!(
        svc.content_store().exists(blob_hash),
        "blob should exist in content store"
    );

    // Read the post back by its content hash.
    let get_result = svc
        .posts
        .get(&content_hash, svc.content_store(), &svc.metadata_db)
        .await
        .unwrap();

    assert_eq!(get_result["content_hash"].as_str().unwrap(), content_hash);
    assert_eq!(
        get_result["body_preview"].as_str().unwrap(),
        "Hello, Ephemera! This is a test post."
    );

    // Verify created_at and expires_at are sane.
    let created_at = get_result["created_at"].as_i64().unwrap();
    let expires_at = get_result["expires_at"].as_i64().unwrap();
    assert!(created_at > 0);
    assert_eq!(expires_at - created_at, 86400);
}

/// Test 3: Create 3 posts, verify feed returns them in insertion order (newest first).
#[tokio::test]
async fn test_feed_returns_posts_chronologically() {
    let (svc, _dir) = make_services();

    // Create identity.
    svc.identity.create("test-pass").await.unwrap();

    // Create 3 posts with distinct content.
    let bodies = ["First post", "Second post", "Third post"];

    for body in &bodies {
        svc.posts
            .create(
                body,
                vec![],
                Some(86400),
                None,
                &svc.identity,
                svc.content_store(),
                &svc.metadata_db,
                &svc.rate_limiter,
                &svc.reputation,
                &svc.fingerprint_store,
                &svc.content_filter,
                None,
            )
            .await
            .unwrap();
    }

    // Query the feed (ordered by rowid DESC = insertion order DESC).
    // Use unfiltered mode (None for identity/social) for backward-compatible testing.
    let feed = svc
        .feed
        .connections(50, None, &svc.metadata_db, None, None)
        .await
        .unwrap();

    let posts = feed["posts"].as_array().unwrap();
    assert_eq!(posts.len(), 3, "feed should contain 3 posts");

    // The last inserted post (Third post) should be first in the feed (highest rowid).
    assert_eq!(posts[0]["body_preview"].as_str().unwrap(), "Third post");
    assert_eq!(posts[1]["body_preview"].as_str().unwrap(), "Second post");
    assert_eq!(posts[2]["body_preview"].as_str().unwrap(), "First post");

    // Rowids should be strictly decreasing.
    let rowids: Vec<i64> = posts.iter().map(|p| p["rowid"].as_i64().unwrap()).collect();

    for i in 0..rowids.len() - 1 {
        assert!(
            rowids[i] > rowids[i + 1],
            "rowids should be strictly decreasing: {} should be > {}",
            rowids[i],
            rowids[i + 1]
        );
    }
}

/// Test 4: Create a post with short TTL, run GC, verify it's gone.
#[tokio::test]
async fn test_post_gc_deletes_expired() {
    let (svc, _dir) = make_services();

    // Create identity.
    svc.identity.create("test-pass").await.unwrap();

    // Create a post with 1-hour TTL (the minimum).
    let result = svc
        .posts
        .create(
            "This will expire",
            vec![],
            Some(3600),
            None,
            &svc.identity,
            svc.content_store(),
            &svc.metadata_db,
            &svc.rate_limiter,
            &svc.reputation,
            &svc.fingerprint_store,
            &svc.content_filter,
            None,
        )
        .await
        .unwrap();

    let content_hash = result["content_hash"].as_str().unwrap().to_string();

    // Verify the post exists.
    let get_result = svc
        .posts
        .get(&content_hash, svc.content_store(), &svc.metadata_db)
        .await;
    assert!(get_result.is_ok(), "post should exist before GC");

    // Manually backdate the expires_at to the past so GC will pick it up.
    {
        let hash_bytes = hex::decode(&content_hash).unwrap();
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&hash_bytes);
        let content_id = ephemera_types::ContentId::from_digest(arr);
        let wire = content_id.to_wire_bytes();

        let db = svc.metadata_db.lock().unwrap();
        db.conn()
            .execute(
                "UPDATE posts SET expires_at = 1000000, created_at = 900000 WHERE content_hash = ?1",
                rusqlite::params![wire],
            )
            .unwrap();
    }

    // Run GC.
    let (posts_deleted, _tombstones) = svc.run_gc().unwrap();
    assert!(posts_deleted >= 1, "GC should have deleted at least 1 post");

    // The post should now be a tombstone -- reading it back should fail
    // because our get() filters out tombstones.
    let get_after_gc = svc
        .posts
        .get(&content_hash, svc.content_store(), &svc.metadata_db)
        .await;
    assert!(get_after_gc.is_err(), "post should be tombstoned after GC");
}

/// Test 5: Identity unlock/lock round-trip.
#[tokio::test]
async fn test_identity_unlock_lock_roundtrip() {
    let (svc, _dir) = make_services();

    // Create identity with passphrase.
    let create_result = svc.identity.create("my-secret-passphrase").await.unwrap();
    let original_pubkey = create_result["pseudonym_pubkey"]
        .as_str()
        .unwrap()
        .to_string();

    // Lock the identity.
    svc.identity.lock().await.unwrap();

    // Attempting to get active should fail.
    let active_err = svc.identity.get_active().await;
    assert!(active_err.is_err(), "get_active should fail when locked");

    // Unlock with the correct passphrase.
    let unlock = svc.identity.unlock("my-secret-passphrase").await.unwrap();
    assert!(unlock["unlocked"].as_bool().unwrap());
    assert_eq!(unlock["pseudonym_count"].as_u64().unwrap(), 1);

    // Get active should now succeed and return the same pubkey.
    let active = svc.identity.get_active().await.unwrap();
    assert_eq!(active["pubkey"].as_str().unwrap(), original_pubkey);
}
