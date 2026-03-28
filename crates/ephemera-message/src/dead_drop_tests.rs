//! Tests for the dead drop mailbox system.

use super::*;
use crate::SealedEnvelope;
use ephemera_crypto::X25519KeyPair;
use ephemera_store::MetadataDb;
use ephemera_types::{IdentityKey, Ttl};

fn setup_test_db() -> MetadataDb {
    MetadataDb::open_in_memory().unwrap()
}

fn alice_identity() -> IdentityKey {
    IdentityKey::from_bytes([0xAA; 32])
}

fn bob_identity() -> IdentityKey {
    IdentityKey::from_bytes([0xBB; 32])
}

fn carol_identity() -> IdentityKey {
    IdentityKey::from_bytes([0xCC; 32])
}

fn make_sealed(
    sender: &IdentityKey,
    recipient: &IdentityKey,
    body: &[u8],
) -> (SealedEnvelope, X25519KeyPair) {
    let recipient_x25519 = X25519KeyPair::generate();
    let envelope = SealedEnvelope::seal(
        sender,
        recipient,
        &recipient_x25519.public,
        body,
        Ttl::one_day(),
    )
    .unwrap();
    (envelope, recipient_x25519)
}

// ── Required tests ──────────────────────────────────────────────

#[test]
fn test_deposit_and_retrieve() {
    let db = setup_test_db();
    let alice = alice_identity();
    let bob = bob_identity();

    let (envelope, _bob_keys) = make_sealed(&alice, &bob, b"hello offline bob");

    // Deposit a message for Bob.
    let msg_id = DeadDropService::deposit(&db, &bob, &envelope).unwrap();

    // Bob checks his mailbox.
    let pending = DeadDropService::check_mailbox(&db, &bob).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, msg_id);

    // Verify the sealed data round-trips.
    let recovered: SealedEnvelope =
        serde_json::from_slice(&pending[0].sealed_envelope).unwrap();
    assert_eq!(recovered.recipient, bob);
}

#[test]
fn test_mailbox_key_deterministic() {
    let bob = bob_identity();

    let key1 = DeadDropService::mailbox_key(&bob);
    let key2 = DeadDropService::mailbox_key(&bob);
    assert_eq!(key1, key2);

    // Different pubkeys produce different mailbox keys.
    let alice = alice_identity();
    let alice_key = DeadDropService::mailbox_key(&alice);
    assert_ne!(key1, alice_key);
}

#[test]
fn test_acknowledge_removes() {
    let db = setup_test_db();
    let alice = alice_identity();
    let bob = bob_identity();

    let (envelope, _bob_keys) = make_sealed(&alice, &bob, b"ack me");

    let msg_id = DeadDropService::deposit(&db, &bob, &envelope).unwrap();

    // Verify it exists.
    let pending = DeadDropService::check_mailbox(&db, &bob).unwrap();
    assert_eq!(pending.len(), 1);

    // Acknowledge receipt.
    DeadDropService::acknowledge(&db, &msg_id).unwrap();

    // Verify it is gone.
    let pending = DeadDropService::check_mailbox(&db, &bob).unwrap();
    assert!(pending.is_empty());
}

#[test]
fn test_gc_cleans_expired() {
    let db = setup_test_db();
    let alice = alice_identity();
    let bob = bob_identity();

    // Insert a message with an expiry in the past by using deposit_raw.
    let (envelope, _bob_keys) = make_sealed(&alice, &bob, b"expired message");
    let sealed_data = serde_json::to_vec(&envelope).unwrap();
    let msg_hash = blake3::hash(&sealed_data);
    let msg_id = ContentId::from_digest(*msg_hash.as_bytes());
    let mailbox = DeadDropService::mailbox_key(&bob);

    // Insert directly with past expiry (bypass the expired check in deposit_raw).
    let mailbox_bytes = mailbox.hash_bytes().to_vec();
    let msg_id_bytes = msg_id.hash_bytes().to_vec();
    db.conn()
        .execute(
            "INSERT INTO dead_drops
             (message_id, mailbox_key, sealed_data, deposited_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![msg_id_bytes, mailbox_bytes, sealed_data, 1000, 1001],
        )
        .unwrap();

    // Also deposit a valid (non-expired) message.
    let (envelope2, _) = make_sealed(&alice, &bob, b"fresh message");
    DeadDropService::deposit(&db, &bob, &envelope2).unwrap();

    // GC should remove the expired one.
    let removed = DeadDropService::gc(&db).unwrap();
    assert_eq!(removed, 1);

    // Only the fresh message should remain.
    let pending = DeadDropService::check_mailbox(&db, &bob).unwrap();
    assert_eq!(pending.len(), 1);
}

#[test]
fn test_offline_message_flow() {
    let db = setup_test_db();
    let alice = alice_identity();
    let bob = bob_identity();
    let bob_x25519 = X25519KeyPair::generate();

    // Alice sends a message while Bob is offline.
    let envelope = SealedEnvelope::seal(
        &alice,
        &bob,
        &bob_x25519.public,
        b"are you there?",
        Ttl::one_day(),
    )
    .unwrap();

    // Deposit into dead drop.
    let msg_id = DeadDropService::deposit(&db, &bob, &envelope).unwrap();

    // Alice sends a second message.
    let envelope2 = SealedEnvelope::seal(
        &alice,
        &bob,
        &bob_x25519.public,
        b"still waiting...",
        Ttl::one_day(),
    )
    .unwrap();
    let msg_id2 = DeadDropService::deposit(&db, &bob, &envelope2).unwrap();

    // Bob comes online and checks his mailbox.
    let pending = DeadDropService::check_mailbox(&db, &bob).unwrap();
    assert_eq!(pending.len(), 2);

    // Bob decrypts each message.
    for pm in &pending {
        let recovered: SealedEnvelope =
            serde_json::from_slice(&pm.sealed_envelope).unwrap();
        let payload = recovered.open(&bob_x25519.secret).unwrap();
        assert_eq!(payload.sender, alice);
    }

    // Bob acknowledges both messages.
    DeadDropService::acknowledge(&db, &msg_id).unwrap();
    DeadDropService::acknowledge(&db, &msg_id2).unwrap();

    // Mailbox should be empty now.
    let pending = DeadDropService::check_mailbox(&db, &bob).unwrap();
    assert!(pending.is_empty());
}

#[test]
fn test_multiple_recipients() {
    let db = setup_test_db();
    let alice = alice_identity();
    let bob = bob_identity();
    let carol = carol_identity();

    let (env_bob, _) = make_sealed(&alice, &bob, b"for bob");
    let (env_carol, _) = make_sealed(&alice, &carol, b"for carol");

    DeadDropService::deposit(&db, &bob, &env_bob).unwrap();
    DeadDropService::deposit(&db, &carol, &env_carol).unwrap();

    // Bob's mailbox should only have his message.
    let bob_pending = DeadDropService::check_mailbox(&db, &bob).unwrap();
    assert_eq!(bob_pending.len(), 1);

    // Carol's mailbox should only have her message.
    let carol_pending = DeadDropService::check_mailbox(&db, &carol).unwrap();
    assert_eq!(carol_pending.len(), 1);
}

#[test]
fn test_acknowledge_nonexistent_fails() {
    let db = setup_test_db();
    let fake_id = ContentId::from_digest([0xFF; 32]);
    let result = DeadDropService::acknowledge(&db, &fake_id);
    assert!(result.is_err());
}

#[test]
fn test_mailbox_count() {
    let db = setup_test_db();
    let alice = alice_identity();
    let bob = bob_identity();

    assert_eq!(DeadDropService::mailbox_count(&db, &bob).unwrap(), 0);

    let (env1, _) = make_sealed(&alice, &bob, b"msg 1");
    let (env2, _) = make_sealed(&alice, &bob, b"msg 2");
    DeadDropService::deposit(&db, &bob, &env1).unwrap();
    DeadDropService::deposit(&db, &bob, &env2).unwrap();

    assert_eq!(DeadDropService::mailbox_count(&db, &bob).unwrap(), 2);
}

#[test]
fn test_deposit_raw_rejects_expired() {
    let db = setup_test_db();
    let mailbox = ContentId::from_digest([0x01; 32]);
    let msg_id = ContentId::from_digest([0x02; 32]);

    let result = DeadDropService::deposit_raw(
        &db, &mailbox, &msg_id, b"data", 1000, 1001,
    );
    assert!(result.is_err());
}
