//! Tests for the SessionManager: ratchet session persistence and wiring.

use crate::prekey::PrekeyBundle;
use crate::session::SessionManager;
use crate::x3dh::x3dh_respond;
use ephemera_crypto::signing::SigningKeyPair;
use ephemera_crypto::X25519KeyPair;
use ephemera_store::MetadataDb;

/// Helper: set up an in-memory database with all migrations applied.
fn setup_db() -> MetadataDb {
    MetadataDb::open_in_memory().unwrap()
}

/// Helper: create full identity materials for a user.
struct TestIdentity {
    signing: SigningKeyPair,
    x25519: X25519KeyPair,
}

impl TestIdentity {
    fn generate() -> Self {
        Self {
            signing: SigningKeyPair::generate(),
            x25519: X25519KeyPair::generate(),
        }
    }

    fn identity_key(&self) -> ephemera_types::IdentityKey {
        self.signing.public_key()
    }
}

/// Helper: build a PrekeyBundle with a separate signed prekey.
fn make_bundle(identity: &TestIdentity, spk: &X25519KeyPair) -> PrekeyBundle {
    PrekeyBundle {
        identity_key: identity.signing.public_key(),
        identity_x25519: identity.x25519.public.clone(),
        signed_prekey: spk.public.clone(),
        signed_prekey_signature: identity.signing.sign(spk.public.as_bytes()),
        one_time_prekey: None,
    }
}

// ── Test: first message creates session ──────────────────────────

#[test]
fn test_first_message_creates_session() {
    let db = setup_db();
    let alice = TestIdentity::generate();
    let bob = TestIdentity::generate();
    let bob_spk = X25519KeyPair::generate();

    let conv_id = hex::encode(blake3::hash(b"first-msg-test").as_bytes());
    let bob_bundle = make_bundle(&bob, &bob_spk);

    // No session should exist yet.
    assert!(!SessionManager::has_session(&db, &conv_id).unwrap());

    // Alice encrypts a first message (triggers X3DH + session creation).
    let result = SessionManager::encrypt_for_peer(
        &db,
        &conv_id,
        bob.identity_key().as_bytes(),
        &alice.x25519,
        &alice.identity_key(),
        Some(&bob_bundle),
        b"Hello Bob!",
    )
    .unwrap();

    // X3DH header should be present on first message.
    assert!(result.x3dh_header.is_some());

    // Session should now exist in SQLite.
    assert!(SessionManager::has_session(&db, &conv_id).unwrap());

    // Verify the ratchet_sessions row exists.
    let count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM ratchet_sessions WHERE conversation_id = ?1",
            rusqlite::params![conv_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

// ── Test: subsequent messages advance ratchet ────────────────────

#[test]
fn test_subsequent_messages_advance_ratchet() {
    let db = setup_db();
    let alice = TestIdentity::generate();
    let bob = TestIdentity::generate();
    let bob_spk = X25519KeyPair::generate();

    let conv_id = hex::encode(blake3::hash(b"advance-test").as_bytes());
    let bob_bundle = make_bundle(&bob, &bob_spk);

    // Send first message.
    SessionManager::encrypt_for_peer(
        &db,
        &conv_id,
        bob.identity_key().as_bytes(),
        &alice.x25519,
        &alice.identity_key(),
        Some(&bob_bundle),
        b"msg 0",
    )
    .unwrap();

    // Send 4 more messages (no X3DH header needed).
    for i in 1..5 {
        let msg = format!("msg {i}");
        let result = SessionManager::encrypt_for_peer(
            &db,
            &conv_id,
            bob.identity_key().as_bytes(),
            &alice.x25519,
            &alice.identity_key(),
            None,
            msg.as_bytes(),
        )
        .unwrap();

        // Subsequent messages should not have X3DH header.
        assert!(result.x3dh_header.is_none());
    }

    // Verify send_count has advanced to 5.
    let send_count: u32 = db
        .conn()
        .query_row(
            "SELECT send_count FROM ratchet_sessions WHERE conversation_id = ?1",
            rusqlite::params![conv_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(send_count, 5);
}

// ── Test: bidirectional ratchet ──────────────────────────────────
//
// Each party has its own database (mirrors real-world node separation).
// Alice's encrypt_for_peer performs X3DH internally; Bob uses the
// X3DH initial message from Alice's result to derive the shared secret.

#[test]
fn test_bidirectional_ratchet() {
    let alice_db = setup_db();
    let bob_db = setup_db();

    let alice = TestIdentity::generate();
    let bob = TestIdentity::generate();
    let bob_spk = X25519KeyPair::generate();

    let bob_bundle = make_bundle(&bob, &bob_spk);
    let bob_peer = bob.identity_key().as_bytes().to_vec();
    let alice_peer = alice.identity_key().as_bytes().to_vec();
    let conv_id = hex::encode(blake3::hash(b"bidirectional-test").as_bytes());

    // Alice sends first message (X3DH happens internally).
    let enc1 = SessionManager::encrypt_for_peer(
        &alice_db,
        &conv_id,
        &bob_peer,
        &alice.x25519,
        &alice.identity_key(),
        Some(&bob_bundle),
        b"Hello from Alice",
    )
    .unwrap();
    let x3dh_header = enc1.x3dh_header.as_ref().expect("first message must have X3DH header");

    // Bob derives the shared secret from Alice's X3DH initial message.
    let shared_secret = x3dh_respond(
        &bob.x25519.secret,
        &bob_spk.secret,
        None,
        x3dh_header,
    )
    .unwrap();

    // Bob decrypts (his DB, creates session from X3DH).
    let bob_rk = X25519KeyPair::from_secret_bytes(bob_spk.secret.as_bytes());
    let pt1 = SessionManager::decrypt_from_peer(
        &bob_db,
        &conv_id,
        &alice_peer,
        Some(bob_rk),
        Some(&shared_secret),
        &enc1.header,
        &enc1.ciphertext,
    )
    .unwrap();
    assert_eq!(pt1, b"Hello from Alice");

    // Bob replies (his DB, triggers DH ratchet step).
    let enc2 = SessionManager::encrypt_for_peer(
        &bob_db,
        &conv_id,
        &alice_peer,
        &bob.x25519,
        &bob.identity_key(),
        None,
        b"Hello from Bob",
    )
    .unwrap();
    assert!(enc2.x3dh_header.is_none());

    // Alice decrypts Bob's reply (her DB).
    let pt2 = SessionManager::decrypt_from_peer(
        &alice_db,
        &conv_id,
        &bob_peer,
        None,
        None,
        &enc2.header,
        &enc2.ciphertext,
    )
    .unwrap();
    assert_eq!(pt2, b"Hello from Bob");

    // Alice sends again (another DH ratchet step).
    let enc3 = SessionManager::encrypt_for_peer(
        &alice_db,
        &conv_id,
        &bob_peer,
        &alice.x25519,
        &alice.identity_key(),
        None,
        b"Alice again",
    )
    .unwrap();

    // Bob decrypts.
    let pt3 = SessionManager::decrypt_from_peer(
        &bob_db,
        &conv_id,
        &alice_peer,
        None,
        None,
        &enc3.header,
        &enc3.ciphertext,
    )
    .unwrap();
    assert_eq!(pt3, b"Alice again");
}

// ── Test: session persistence across reload ──────────────────────

#[test]
fn test_session_persistence() {
    let alice_db = setup_db();
    let bob_db = setup_db();

    let alice = TestIdentity::generate();
    let bob = TestIdentity::generate();
    let bob_spk = X25519KeyPair::generate();

    let bob_bundle = make_bundle(&bob, &bob_spk);
    let bob_peer = bob.identity_key().as_bytes().to_vec();
    let alice_peer = alice.identity_key().as_bytes().to_vec();
    let conv_id = hex::encode(blake3::hash(b"persistence-test").as_bytes());

    // Alice sends first message (X3DH done internally).
    let enc1 = SessionManager::encrypt_for_peer(
        &alice_db,
        &conv_id,
        &bob_peer,
        &alice.x25519,
        &alice.identity_key(),
        Some(&bob_bundle),
        b"Persisted message 1",
    )
    .unwrap();
    let x3dh_header = enc1.x3dh_header.as_ref().unwrap();

    // Bob computes shared secret from Alice's X3DH header.
    let shared_secret = x3dh_respond(
        &bob.x25519.secret,
        &bob_spk.secret,
        None,
        x3dh_header,
    )
    .unwrap();

    // Bob decrypts.
    let bob_rk = X25519KeyPair::from_secret_bytes(bob_spk.secret.as_bytes());
    let pt1 = SessionManager::decrypt_from_peer(
        &bob_db,
        &conv_id,
        &alice_peer,
        Some(bob_rk),
        Some(&shared_secret),
        &enc1.header,
        &enc1.ciphertext,
    )
    .unwrap();
    assert_eq!(pt1, b"Persisted message 1");

    // Verify sessions were persisted to both databases.
    assert!(
        SessionManager::has_session(&alice_db, &conv_id).unwrap(),
        "Alice's session must be persisted"
    );
    assert!(
        SessionManager::has_session(&bob_db, &conv_id).unwrap(),
        "Bob's session must be persisted"
    );

    // Verify Alice's session can be loaded independently.
    let loaded = SessionManager::load_session(&alice_db, &conv_id).unwrap();
    assert!(loaded.is_some(), "loaded session must exist");

    // Alice sends a second message (session loaded from SQLite).
    let enc2 = SessionManager::encrypt_for_peer(
        &alice_db,
        &conv_id,
        &bob_peer,
        &alice.x25519,
        &alice.identity_key(),
        None,
        b"Persisted message 2",
    )
    .unwrap();

    // Bob decrypts (his session also loaded from SQLite).
    let pt2 = SessionManager::decrypt_from_peer(
        &bob_db,
        &conv_id,
        &alice_peer,
        None,
        None,
        &enc2.header,
        &enc2.ciphertext,
    )
    .unwrap();
    assert_eq!(pt2, b"Persisted message 2");

    // Verify send_count advanced for Alice (2 messages sent).
    let alice_sc: u32 = alice_db
        .conn()
        .query_row(
            "SELECT send_count FROM ratchet_sessions WHERE conversation_id = ?1",
            rusqlite::params![conv_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(alice_sc, 2, "Alice's send_count must be 2");
}
