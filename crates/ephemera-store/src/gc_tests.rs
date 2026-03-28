use super::*;

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[test]
fn sweep_on_empty_db() {
    let db = MetadataDb::open_in_memory().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let content = ContentStore::open(dir.path()).unwrap();
    let gc = GarbageCollector::with_defaults();

    let report = gc.sweep(&db, &content).unwrap();
    assert_eq!(report.posts_deleted, 0);
    assert_eq!(report.messages_deleted, 0);
    assert_eq!(report.tombstones_purged, 0);
    assert_eq!(report.epoch_keys_destroyed, 0);
}

#[test]
fn test_gc_deletes_expired_posts() {
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

    // The blob should be deleted and the post tombstoned.
    assert_eq!(report.posts_deleted, 1);
    assert!(!content.exists(&blob_hash));
}

#[test]
fn test_gc_creates_tombstones() {
    let db = MetadataDb::open_in_memory().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let content = ContentStore::open(dir.path()).unwrap();

    let blob_data = b"tombstone test content";
    let blob_hash = content.put(blob_data).unwrap();
    let hash_bytes = hex::decode(&blob_hash).unwrap();

    let past = 1_000_000i64;
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
    gc.sweep(&db, &content).unwrap();

    // Verify the post is now a tombstone.
    let is_tombstone: bool = db
        .conn()
        .query_row(
            "SELECT is_tombstone FROM posts WHERE content_hash = ?1",
            rusqlite::params![hash_bytes],
            |row| row.get::<_, i32>(0).map(|v| v != 0),
        )
        .unwrap();
    assert!(is_tombstone, "post should be marked as tombstone");

    // Verify tombstone_at is set.
    let tombstone_at: Option<i64> = db
        .conn()
        .query_row(
            "SELECT tombstone_at FROM posts WHERE content_hash = ?1",
            rusqlite::params![hash_bytes],
            |row| row.get(0),
        )
        .unwrap();
    assert!(tombstone_at.is_some(), "tombstone_at should be set");
}

#[test]
fn test_gc_cleans_tombstones() {
    let db = MetadataDb::open_in_memory().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let content = ContentStore::open(dir.path()).unwrap();

    // Insert a tombstoned post with a very old tombstone_at.
    // ttl_seconds = 1, tombstone_at = 1000000
    // Tombstone should be purged when: tombstone_at + (ttl_seconds * 3) < now
    // 1000000 + 3 < now => yes, so it should be purged.
    let hash_bytes = vec![0x01u8; 32];
    let author = vec![0xAAu8; 32];
    let sig = vec![0xBBu8; 64];
    db.conn()
        .execute(
            "INSERT INTO posts (
                content_hash, author_pubkey, sequence_number, created_at,
                expires_at, ttl_seconds, received_at, epoch_number, signature,
                is_tombstone, tombstone_at
             ) VALUES (?1, ?2, 1, 1000000, 1000001, 1, 1000000, 1, ?3, 1, 1000000)",
            rusqlite::params![hash_bytes, author, sig],
        )
        .unwrap();

    // Verify the tombstone exists.
    let count: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM posts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);

    let gc = GarbageCollector::with_defaults();
    let report = gc.sweep(&db, &content).unwrap();

    // Tombstone should be purged (tombstone_at + ttl_seconds * 3 = 1000003 < now).
    assert_eq!(report.tombstones_purged, 1);

    let count_after: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM posts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count_after, 0, "tombstone metadata should be fully deleted");
}

#[test]
fn test_gc_destroys_epoch_keys() {
    let db = MetadataDb::open_in_memory().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let content = ContentStore::open(dir.path()).unwrap();

    let now = now_secs();

    // Insert an epoch key that expired way in the past.
    db.conn()
        .execute(
            "INSERT INTO epoch_keys (epoch_number, created_at, expires_at, is_deleted)
             VALUES (?1, ?2, ?3, 0)",
            rusqlite::params![100, 1_000_000, 1_000_000 + 86400],
        )
        .unwrap();

    // Insert an epoch key that has NOT expired yet.
    db.conn()
        .execute(
            "INSERT INTO epoch_keys (epoch_number, created_at, expires_at, is_deleted)
             VALUES (?1, ?2, ?3, 0)",
            rusqlite::params![20600, now, now + 86400 * 30],
        )
        .unwrap();

    let gc = GarbageCollector::with_defaults();
    let report = gc.sweep(&db, &content).unwrap();

    assert_eq!(
        report.epoch_keys_destroyed, 1,
        "only the old epoch key should be destroyed"
    );

    // Verify the old key is marked deleted.
    let is_deleted: bool = db
        .conn()
        .query_row(
            "SELECT is_deleted FROM epoch_keys WHERE epoch_number = 100",
            [],
            |row| row.get::<_, i32>(0).map(|v| v != 0),
        )
        .unwrap();
    assert!(is_deleted);

    // Verify the new key is still active.
    let new_is_deleted: bool = db
        .conn()
        .query_row(
            "SELECT is_deleted FROM epoch_keys WHERE epoch_number = 20600",
            [],
            |row| row.get::<_, i32>(0).map(|v| v != 0),
        )
        .unwrap();
    assert!(!new_is_deleted);
}

#[test]
fn test_gc_cleans_orphaned_content_from_destroyed_epochs() {
    let db = MetadataDb::open_in_memory().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let content = ContentStore::open(dir.path()).unwrap();

    // Insert a destroyed epoch key.
    db.conn()
        .execute(
            "INSERT INTO epoch_keys (epoch_number, created_at, expires_at, is_deleted, deleted_at)
             VALUES (42, 1000000, 1000001, 1, 2000000)",
            [],
        )
        .unwrap();

    // Insert a post from that destroyed epoch with a blob.
    let blob_data = b"orphaned content from destroyed epoch";
    let blob_hash = content.put(blob_data).unwrap();
    let hash_bytes = hex::decode(&blob_hash).unwrap();
    let author = vec![0xAAu8; 32];
    let sig = vec![0xBBu8; 64];
    db.conn()
        .execute(
            "INSERT INTO posts (
                content_hash, author_pubkey, sequence_number, created_at,
                expires_at, ttl_seconds, received_at, epoch_number, signature
             ) VALUES (?1, ?2, 1, 1000000, 1086400, 86400, 1000000, 42, ?3)",
            rusqlite::params![hash_bytes, author, sig],
        )
        .unwrap();

    assert!(content.exists(&blob_hash));

    let gc = GarbageCollector::with_defaults();
    let report = gc.sweep(&db, &content).unwrap();

    // The orphaned blob should be deleted.
    assert!(report.orphaned_blobs_deleted > 0 || report.posts_deleted > 0);
    assert!(!content.exists(&blob_hash), "orphaned blob must be deleted");

    // The post metadata should also be deleted.
    let count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM posts WHERE epoch_number = 42",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0, "orphaned post metadata should be deleted");
}

// Remaining tests in the _extra module.
#[path = "gc_tests_extra.rs"]
mod extra;
