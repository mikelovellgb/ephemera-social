use super::*;
use crate::StoreError;

#[test]
fn put_get_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(dir.path()).unwrap();

    let data = b"hello ephemera";
    let hash = store.put(data).unwrap();

    let retrieved = store.get(&hash).unwrap();
    assert_eq!(retrieved, data);
}

#[test]
fn put_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(dir.path()).unwrap();

    let data = b"duplicate";
    let h1 = store.put(data).unwrap();
    let h2 = store.put(data).unwrap();
    assert_eq!(h1, h2);
}

#[test]
fn get_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(dir.path()).unwrap();

    let fake = "0".repeat(64);
    let result = store.get(&fake);
    assert!(matches!(result, Err(StoreError::NotFound(_))));
}

#[test]
fn exists_check() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(dir.path()).unwrap();

    let hash = store.put(b"check me").unwrap();
    assert!(store.exists(&hash));
    assert!(!store.exists(&"0".repeat(64)));
}

#[test]
fn delete_removes_blob() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(dir.path()).unwrap();

    let hash = store.put(b"delete me").unwrap();
    assert!(store.exists(&hash));

    assert!(store.delete(&hash).unwrap());
    assert!(!store.exists(&hash));
}

#[test]
fn delete_nonexistent_returns_false() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(dir.path()).unwrap();

    assert!(!store.delete("no_such_hash").unwrap());
}

#[test]
fn path_structure_uses_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(dir.path()).unwrap();

    let hash = store.put(b"path test").unwrap();
    let prefix = &hash[..2];
    let rest = &hash[2..];

    let expected = dir.path().join(prefix).join(format!("{rest}.blob"));
    assert!(
        expected.exists(),
        "blob should be at {}",
        expected.display()
    );
}

// ── Time-partitioned storage tests ──────────────────────────────

#[test]
fn partitioned_put_get_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(dir.path()).unwrap();

    let data = b"partitioned content";
    let date = "2026-03-26";
    let hash = store.put_partitioned(data, date).unwrap();

    let retrieved = store.get_partitioned(date, &hash).unwrap();
    assert_eq!(retrieved, data);
}

#[test]
fn partitioned_exists() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(dir.path()).unwrap();

    let hash = store.put_partitioned(b"exists test", "2026-03-26").unwrap();
    assert!(store.exists_partitioned("2026-03-26", &hash));
    assert!(!store.exists_partitioned("2026-03-25", &hash));
}

#[test]
fn delete_day_partition() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(dir.path()).unwrap();

    let date = "2026-03-20";
    store.put_partitioned(b"content 1", date).unwrap();
    store.put_partitioned(b"content 2", date).unwrap();
    assert!(store.day_partition_exists(date));

    assert!(store.delete_day_partition(date).unwrap());
    assert!(!store.day_partition_exists(date));
}

#[test]
fn list_day_partitions() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(dir.path()).unwrap();

    store.put_partitioned(b"a", "2026-03-24").unwrap();
    store.put_partitioned(b"b", "2026-03-25").unwrap();
    store.put_partitioned(b"c", "2026-03-26").unwrap();

    let partitions = store.list_day_partitions().unwrap();
    assert_eq!(partitions, vec!["2026-03-24", "2026-03-25", "2026-03-26"]);
}

#[test]
fn date_str_from_timestamp_works() {
    // 2026-03-26 00:00:00 UTC = 1774483200
    let ts = 1_774_483_200u64;
    let date = ContentStore::date_str_from_timestamp(ts);
    assert_eq!(date, "2026-03-26");
}

#[test]
fn empty_partition_check() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(dir.path()).unwrap();

    // Non-existent partition is considered empty.
    assert!(store.is_day_partition_empty("2026-01-01").unwrap());

    let hash = store.put_partitioned(b"data", "2026-01-01").unwrap();
    assert!(!store.is_day_partition_empty("2026-01-01").unwrap());

    // Delete the blob, but the directory remains.
    store.delete_partitioned("2026-01-01", &hash).unwrap();
    assert!(store.is_day_partition_empty("2026-01-01").unwrap());
}

// ── Encrypted content store tests ──────────────────────────────

fn test_encryption_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut key);
    key
}

#[test]
fn encrypted_put_get_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let key = test_encryption_key();
    let store = ContentStore::open_encrypted(dir.path(), key).unwrap();

    let data = b"secret content at rest";
    let hash = store.put(data).unwrap();

    // Data on disk must not be plaintext.
    let raw_on_disk = std::fs::read(store.hash_to_path(&hash)).unwrap();
    assert_ne!(
        raw_on_disk, data,
        "stored bytes must be ciphertext, not plaintext"
    );

    // Reading through the store must return plaintext.
    let retrieved = store.get(&hash).unwrap();
    assert_eq!(retrieved, data);
}

#[test]
fn encrypted_partitioned_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let key = test_encryption_key();
    let store = ContentStore::open_encrypted(dir.path(), key).unwrap();

    let data = b"encrypted partitioned blob";
    let date = "2026-03-26";
    let hash = store.put_partitioned(data, date).unwrap();
    let retrieved = store.get_partitioned(date, &hash).unwrap();
    assert_eq!(retrieved, data);
}

#[test]
fn encrypted_wrong_key_fails() {
    let dir = tempfile::tempdir().unwrap();
    let key1 = test_encryption_key();
    let key2 = test_encryption_key();

    let store1 = ContentStore::open_encrypted(dir.path(), key1).unwrap();
    let hash = store1.put(b"only for key1").unwrap();
    drop(store1);

    let store2 = ContentStore::open_encrypted(dir.path(), key2).unwrap();
    let result = store2.get(&hash);
    assert!(result.is_err(), "decryption with wrong key must fail");
}
