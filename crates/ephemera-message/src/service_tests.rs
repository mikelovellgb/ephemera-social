use super::*;
use crate::{MessageEncryption, SealedEnvelope};
use ephemera_crypto::X25519KeyPair;
use ephemera_store::MetadataDb;
use ephemera_types::{IdentityKey, Timestamp, Ttl};

/// Set up a test database with all tables (including message_requests from v3 migration).
fn setup_test_db() -> MetadataDb {
    MetadataDb::open_in_memory().unwrap()
}

/// Create a "connected" relationship between two identities in the DB.
fn make_connected(db: &MetadataDb, local: &[u8], remote: &[u8]) {
    let now = Timestamp::now().as_secs() as i64;
    db.conn()
        .execute(
            "INSERT INTO connections (local_pubkey, remote_pubkey, status, created_at, updated_at)
             VALUES (?1, ?2, 'connected', ?3, ?3)",
            rusqlite::params![local, remote, now],
        )
        .unwrap();
}

fn alice_keys() -> (IdentityKey, X25519KeyPair) {
    let id = IdentityKey::from_bytes([0xAA; 32]);
    let x25519 = X25519KeyPair::generate();
    (id, x25519)
}

fn bob_keys() -> (IdentityKey, X25519KeyPair) {
    let id = IdentityKey::from_bytes([0xBB; 32]);
    let x25519 = X25519KeyPair::generate();
    (id, x25519)
}

fn carol_keys() -> (IdentityKey, X25519KeyPair) {
    let id = IdentityKey::from_bytes([0xCC; 32]);
    let x25519 = X25519KeyPair::generate();
    (id, x25519)
}

fn dave_keys() -> (IdentityKey, X25519KeyPair) {
    let id = IdentityKey::from_bytes([0xDD; 32]);
    let x25519 = X25519KeyPair::generate();
    (id, x25519)
}

// ── Required tests ──────────────────────────────────────────────

#[test]
fn test_encrypt_decrypt_roundtrip() {
    let recipient = X25519KeyPair::generate();
    let plaintext = b"Hello, this is a secret message for roundtrip testing!";

    let encrypted = MessageEncryption::encrypt_message(plaintext, &recipient.public).unwrap();

    let decrypted = MessageEncryption::decrypt_message(&encrypted, &recipient.secret).unwrap();

    assert_eq!(&decrypted, plaintext);
}

#[test]
fn test_wrong_key_fails_decrypt() {
    let alice = X25519KeyPair::generate();
    let bob = X25519KeyPair::generate();
    let plaintext = b"This message is for Alice only";

    let encrypted = MessageEncryption::encrypt_message(plaintext, &alice.public).unwrap();

    let result = MessageEncryption::decrypt_message(&encrypted, &bob.secret);
    assert!(result.is_err(), "decryption with wrong key must fail");
}

#[test]
fn test_sealed_sender() {
    let (alice_id, _alice_x25519) = alice_keys();
    let (bob_id, bob_x25519) = bob_keys();

    let message = b"Sealed secret message";
    let ttl = Ttl::one_day();

    let envelope =
        SealedEnvelope::seal(&alice_id, &bob_id, &bob_x25519.public, message, ttl).unwrap();

    assert_eq!(envelope.recipient, bob_id);
    assert_ne!(envelope.recipient, alice_id);

    let alice_bytes = alice_id.as_bytes();
    let found_in_ciphertext = envelope
        .ciphertext
        .windows(32)
        .any(|w| w == alice_bytes.as_slice());
    assert!(
        !found_in_ciphertext,
        "sender identity must not appear as plaintext in ciphertext"
    );

    let serialized = serde_json::to_vec(&envelope).unwrap();
    let alice_hex = hex::encode(alice_bytes);
    let serialized_str = String::from_utf8_lossy(&serialized);
    assert!(
        !serialized_str.contains(&alice_hex),
        "sender identity must not appear in serialized envelope"
    );

    let payload = envelope.open(&bob_x25519.secret).unwrap();
    assert_eq!(payload.sender, alice_id);
    assert_eq!(payload.body, message);
}

#[test]
fn test_message_request_flow() {
    let db = setup_test_db();
    let (alice_id, _alice_x25519) = alice_keys();
    let (bob_id, bob_x25519) = bob_keys();

    let alice_bytes = alice_id.as_bytes().to_vec();
    let bob_bytes = bob_id.as_bytes().to_vec();

    let result = MessageService::send_message(
        &db,
        &alice_id,
        &bob_id,
        &bob_x25519.public,
        b"hi bob",
        Ttl::one_day(),
    );
    assert!(
        result.is_err(),
        "stranger should not be able to send directly"
    );

    let envelope = SealedEnvelope::seal(
        &alice_id,
        &bob_id,
        &bob_x25519.public,
        b"hey, can we talk?",
        Ttl::one_day(),
    )
    .unwrap();

    let payload =
        MessageService::receive_message(&db, &envelope, &bob_id, &bob_x25519.secret).unwrap();
    assert_eq!(payload.sender, alice_id);
    assert_eq!(payload.body, b"hey, can we talk?");

    let req_status: String = db
        .conn()
        .query_row(
            "SELECT status FROM message_requests
             WHERE sender_pubkey = ?1 AND recipient_pubkey = ?2",
            rusqlite::params![alice_bytes, bob_bytes],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(req_status, "pending");

    MessageService::accept_request(&db, &bob_bytes, &alice_bytes).unwrap();

    let req_status: String = db
        .conn()
        .query_row(
            "SELECT status FROM message_requests
             WHERE sender_pubkey = ?1 AND recipient_pubkey = ?2",
            rusqlite::params![alice_bytes, bob_bytes],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(req_status, "accepted");

    let (_conv_id, envelope2) = MessageService::send_message(
        &db,
        &alice_id,
        &bob_id,
        &bob_x25519.public,
        b"thanks for accepting!",
        Ttl::one_day(),
    )
    .unwrap();

    let payload2 =
        MessageService::receive_message(&db, &envelope2, &bob_id, &bob_x25519.secret).unwrap();
    assert_eq!(payload2.sender, alice_id);
    assert_eq!(payload2.body, b"thanks for accepting!");
}

#[test]
fn test_message_request_reject() {
    let db = setup_test_db();
    let (alice_id, _alice_x25519) = alice_keys();
    let (bob_id, bob_x25519) = bob_keys();

    let alice_bytes = alice_id.as_bytes().to_vec();
    let bob_bytes = bob_id.as_bytes().to_vec();

    let envelope = SealedEnvelope::seal(
        &alice_id,
        &bob_id,
        &bob_x25519.public,
        b"hello stranger",
        Ttl::one_day(),
    )
    .unwrap();

    MessageService::receive_message(&db, &envelope, &bob_id, &bob_x25519.secret).unwrap();

    MessageService::reject_request(&db, &bob_bytes, &alice_bytes).unwrap();

    let envelope2 = SealedEnvelope::seal(
        &alice_id,
        &bob_id,
        &bob_x25519.public,
        b"please respond",
        Ttl::one_day(),
    )
    .unwrap();

    let result = MessageService::receive_message(&db, &envelope2, &bob_id, &bob_x25519.secret);
    assert!(
        result.is_err(),
        "messages from rejected sender must be denied"
    );
}

#[path = "service_tests_extra.rs"]
mod extra;
