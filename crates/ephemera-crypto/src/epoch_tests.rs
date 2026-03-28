use super::*;
use crate::encryption;

fn test_master() -> MasterSecret {
    MasterSecret::from_bytes([0x42; 32])
}

#[test]
fn test_epoch_id_computation() {
    // Epoch 0 starts at Unix time 0.
    assert_eq!(epoch_id_for_timestamp(0), 0);
    // 86400 seconds = epoch 1.
    assert_eq!(epoch_id_for_timestamp(86400), 1);
    // Midday on day 1.
    assert_eq!(epoch_id_for_timestamp(86400 + 43200), 1);
    // Day 2.
    assert_eq!(epoch_id_for_timestamp(86400 * 2), 2);
}

#[test]
fn test_current_epoch_key() {
    let master = test_master();
    let mut mgr = EpochKeyManager::new(master);

    let (epoch_id, key) = mgr.current_epoch_key().unwrap();
    assert_eq!(epoch_id, current_epoch_id());
    assert_ne!(key, [0u8; 32]); // Should not be all zeros.

    // Calling again should return the same key (cached).
    let (epoch_id2, key2) = mgr.current_epoch_key().unwrap();
    assert_eq!(epoch_id, epoch_id2);
    assert_eq!(key, key2);
}

#[test]
fn test_epoch_key_for_returns_key() {
    let master = test_master();
    let mut mgr = EpochKeyManager::new(master);

    let epoch = 12345u64;
    let key = mgr.epoch_key_for(epoch).unwrap();
    assert!(key.is_some());

    // Same key when called again.
    let key2 = mgr.epoch_key_for(epoch).unwrap();
    assert_eq!(key, key2);
}

#[test]
fn test_epoch_key_rotation() {
    let master = test_master();
    let mut mgr = EpochKeyManager::new(master);

    // Pre-populate with an old epoch key.
    let old_epoch = 1u64; // Day 1 (extremely old).
    mgr.ensure_key(old_epoch).unwrap();

    let result = mgr.rotate().unwrap();
    assert_eq!(result.new_epoch_id, current_epoch_id());

    // The old epoch should be marked for destruction since it's way older
    // than 30 days.
    assert!(
        result.marked_for_destruction.contains(&old_epoch),
        "old epoch should be marked for destruction"
    );
}

#[test]
fn test_epoch_key_destruction() {
    let master = test_master();
    let mut mgr = EpochKeyManager::new(master);

    // Derive a key for epoch 100.
    let epoch = 100u64;
    let key = mgr.epoch_key_for(epoch).unwrap();
    assert!(key.is_some());

    // Destroy it.
    assert!(mgr.destroy_key(epoch));
    assert!(mgr.is_destroyed(epoch));

    // Trying to get the key again should return None.
    let key2 = mgr.epoch_key_for(epoch).unwrap();
    assert!(key2.is_none(), "destroyed key should not be retrievable");
}

#[test]
fn test_content_unreadable_after_key_destruction() {
    let master = test_master();
    let mut mgr = EpochKeyManager::new(master);

    // Create and encrypt content under a specific epoch.
    let epoch = 500u64;
    let key = mgr.epoch_key_for(epoch).unwrap().unwrap();
    let plaintext = b"this content must disappear forever";
    let sealed = encryption::seal(&key, plaintext).unwrap();

    // Verify we can still decrypt it.
    let decrypted = mgr.decrypt_with_epoch_key(epoch, &sealed).unwrap();
    assert_eq!(decrypted.as_deref(), Some(plaintext.as_ref()));

    // Destroy the epoch key.
    mgr.destroy_key(epoch);

    // Attempt to decrypt -- should return None (content is shredded).
    let result = mgr.decrypt_with_epoch_key(epoch, &sealed).unwrap();
    assert!(
        result.is_none(),
        "content must be unreadable after key destruction"
    );
}

#[test]
fn test_destroy_expired_keys_at() {
    let master = test_master();
    let mut mgr = EpochKeyManager::new(master);

    // Create keys for epochs 10, 20, 30.
    mgr.ensure_key(10).unwrap();
    mgr.ensure_key(20).unwrap();
    mgr.ensure_key(30).unwrap();
    assert_eq!(mgr.active_key_count(), 3);

    // Simulate "now" as epoch 70 (day 70).
    // Cutoff = 70 - 30 = epoch 40. Epochs < 40 should be destroyed.
    let now_secs = 70 * EPOCH_DURATION_SECS;
    let result = mgr.destroy_expired_keys_at(now_secs);

    assert_eq!(result.count, 3);
    assert!(result.destroyed_epoch_ids.contains(&10));
    assert!(result.destroyed_epoch_ids.contains(&20));
    assert!(result.destroyed_epoch_ids.contains(&30));
    assert_eq!(mgr.active_key_count(), 0);
}

#[test]
fn test_encrypt_decrypt_with_epoch() {
    let master = test_master();
    let mut mgr = EpochKeyManager::new(master);

    let plaintext = b"ephemeral secret content";
    let (epoch_id, sealed) = mgr.encrypt_with_current_epoch(plaintext).unwrap();

    let decrypted = mgr
        .decrypt_with_epoch_key(epoch_id, &sealed)
        .unwrap()
        .expect("should decrypt successfully");
    assert_eq!(&decrypted, plaintext);
}

#[test]
fn test_different_epochs_different_keys() {
    let master = test_master();
    let mut mgr = EpochKeyManager::new(master);

    let key1 = mgr.epoch_key_for(100).unwrap().unwrap();
    let key2 = mgr.epoch_key_for(101).unwrap().unwrap();
    assert_ne!(key1, key2, "different epochs must have different keys");
}

#[test]
fn test_destroyed_key_cannot_be_rederived() {
    let master = test_master();
    let mut mgr = EpochKeyManager::new(master);

    let epoch = 42u64;
    let key = mgr.epoch_key_for(epoch).unwrap().unwrap();
    assert_ne!(key, [0u8; 32]);

    // Destroy it.
    mgr.destroy_key(epoch);

    // Attempting to get it via epoch_key_for should return None.
    assert!(mgr.epoch_key_for(epoch).unwrap().is_none());

    // Attempting to use ensure_key directly should fail.
    assert!(mgr.ensure_key(epoch).is_err());
}

#[test]
fn test_destroy_nonexistent_key() {
    let master = test_master();
    let mut mgr = EpochKeyManager::new(master);

    // Destroying a key that was never derived should return false but
    // still mark it as destroyed.
    assert!(!mgr.destroy_key(999));
    assert!(mgr.is_destroyed(999));
}
