use super::*;

#[test]
fn test_time_partitioned_cleanup() {
    let db = MetadataDb::open_in_memory().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let content = ContentStore::open(dir.path()).unwrap();

    // Create posts in a day directory.
    let date = "2025-01-01";
    let h1 = content.put_partitioned(b"old content 1", date).unwrap();
    let h2 = content.put_partitioned(b"old content 2", date).unwrap();

    assert!(content.day_partition_exists(date));

    // Delete the individual blobs (simulating GC phase 1).
    content.delete_partitioned(date, &h1).unwrap();
    content.delete_partitioned(date, &h2).unwrap();

    // The directory should now be empty.
    assert!(content.is_day_partition_empty(date).unwrap());

    // Run GC to clean up the empty partition.
    let gc = GarbageCollector::with_defaults();
    let report = gc.sweep(&db, &content).unwrap();

    assert_eq!(report.day_partitions_removed, 1);
    assert!(
        !content.day_partition_exists(date),
        "empty day partition should be removed"
    );
}

#[test]
fn test_gc_emits_events() {
    let db = MetadataDb::open_in_memory().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let content = ContentStore::open(dir.path()).unwrap();

    let event_bus = EventBus::new();
    let mut rx = event_bus.subscribe();

    // Insert an expired post.
    let blob_data = b"event test content";
    let blob_hash = content.put(blob_data).unwrap();
    let hash_bytes = hex::decode(&blob_hash).unwrap();
    let author = vec![0xAAu8; 32];
    let sig = vec![0xBBu8; 64];
    db.conn()
        .execute(
            "INSERT INTO posts (
                content_hash, author_pubkey, sequence_number, created_at,
                expires_at, ttl_seconds, received_at, epoch_number, signature
             ) VALUES (?1, ?2, 1, 1000000, 1086400, 86400, 1000000, 1, ?3)",
            rusqlite::params![hash_bytes, author, sig],
        )
        .unwrap();

    let gc = GarbageCollector::with_defaults();
    gc.sweep_with_events(&db, &content, Some(&event_bus))
        .unwrap();

    // Collect events (non-blocking -- they should already be buffered).
    let mut got_post_expired = false;
    let mut got_gc_completed = false;

    // We expect at least PostExpired + GarbageCollectionCompleted.
    while let Ok(event) = rx.try_recv() {
        match event {
            Event::PostExpired { .. } => got_post_expired = true,
            Event::GarbageCollectionCompleted { items_removed, .. } => {
                got_gc_completed = true;
                assert!(items_removed > 0);
            }
            _ => {}
        }
    }

    assert!(got_post_expired, "should emit PostExpired event");
    assert!(
        got_gc_completed,
        "should emit GarbageCollectionCompleted event"
    );
}

#[test]
fn default_config() {
    let gc = GarbageCollector::with_defaults();
    assert_eq!(gc.interval(), Duration::from_secs(60));
}

#[test]
fn sweep_deletes_expired_posts() {
    let db = MetadataDb::open_in_memory().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let content = ContentStore::open(dir.path()).unwrap();

    // Store a blob.
    let blob_data = b"expired post content";
    let blob_hash = content.put(blob_data).unwrap();
    let hash_bytes = hex::decode(&blob_hash).unwrap();

    // Insert an expired post referencing this blob.
    let past = 1_000_000i64; // way in the past
    let author = vec![0xAAu8; 32];
    let sig = vec![0xBBu8; 64];
    db.conn()
        .execute(
            "INSERT INTO posts (
                content_hash, author_pubkey, sequence_number, created_at,
                expires_at, ttl_seconds, received_at, epoch_number, signature
             ) VALUES (?1, ?2, 1, ?3, ?4, 86400, ?3, 1, ?5)",
            rusqlite::params![hash_bytes, author, past, past + 86400, sig],
        )
        .unwrap();

    let gc = GarbageCollector::with_defaults();
    let report = gc.sweep(&db, &content).unwrap();

    // The post was created at t=1000000 and expired at t=1086400, which is
    // way in the past. The blob should be deleted and the post tombstoned.
    assert_eq!(report.posts_deleted, 1);
    assert!(!content.exists(&blob_hash));
}

// ── Fake clock tests ───────────────────────────────────────────

/// A fake clock fixed at t = 2_000_000 (well in the past).
fn fake_clock_2m() -> i64 {
    2_000_000
}

/// A fake clock fixed at t = 500_000 (before the post expires).
fn fake_clock_500k() -> i64 {
    500_000
}

#[test]
fn fake_clock_does_not_expire_future_post() {
    let db = MetadataDb::open_in_memory().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let content = ContentStore::open(dir.path()).unwrap();

    let blob_data = b"not expired yet";
    let blob_hash = content.put(blob_data).unwrap();
    let hash_bytes = hex::decode(&blob_hash).unwrap();

    // Post created at t=100_000, expires at t=1_100_000.
    let author = vec![0xAAu8; 32];
    let sig = vec![0xBBu8; 64];
    db.conn()
        .execute(
            "INSERT INTO posts (
                content_hash, author_pubkey, sequence_number, created_at,
                expires_at, ttl_seconds, received_at, epoch_number, signature
             ) VALUES (?1, ?2, 1, 100000, 1100000, 1000000, 100000, 1, ?3)",
            rusqlite::params![hash_bytes, author, sig],
        )
        .unwrap();

    // Clock is at 500_000 -- before expiry at 1_100_000.
    let gc = GarbageCollector::new(GcConfig {
        clock: fake_clock_500k,
        ..GcConfig::default()
    });
    let report = gc.sweep(&db, &content).unwrap();
    assert_eq!(report.posts_deleted, 0, "post should NOT be expired yet");
    assert!(content.exists(&blob_hash), "blob should still exist");
}

#[test]
fn fake_clock_expires_post_at_correct_time() {
    let db = MetadataDb::open_in_memory().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let content = ContentStore::open(dir.path()).unwrap();

    let blob_data = b"will expire at t=1_100_000";
    let blob_hash = content.put(blob_data).unwrap();
    let hash_bytes = hex::decode(&blob_hash).unwrap();

    // Post created at t=100_000, expires at t=1_100_000.
    let author = vec![0xAAu8; 32];
    let sig = vec![0xBBu8; 64];
    db.conn()
        .execute(
            "INSERT INTO posts (
                content_hash, author_pubkey, sequence_number, created_at,
                expires_at, ttl_seconds, received_at, epoch_number, signature
             ) VALUES (?1, ?2, 1, 100000, 1100000, 1000000, 100000, 1, ?3)",
            rusqlite::params![hash_bytes, author, sig],
        )
        .unwrap();

    // Clock is at 2_000_000 -- well after expiry at 1_100_000.
    let gc = GarbageCollector::new(GcConfig {
        clock: fake_clock_2m,
        ..GcConfig::default()
    });
    let report = gc.sweep(&db, &content).unwrap();
    assert_eq!(report.posts_deleted, 1, "post should be expired");
    assert!(!content.exists(&blob_hash), "blob should be deleted");
}
