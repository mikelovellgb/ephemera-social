use super::*;

#[test]
fn test_ratchet_basic_send_receive() {
    let shared_secret = [0x42u8; 32];
    let bob_ratchet = X25519KeyPair::generate();

    let mut alice = RatchetState::init_sender(&shared_secret, &bob_ratchet.public).unwrap();
    let mut bob = RatchetState::init_receiver(&shared_secret, bob_ratchet);

    // Alice sends a message to Bob.
    let (header, ct) = alice.encrypt_message(b"hello bob").unwrap();
    let pt = bob.decrypt_message(&header, &ct).unwrap();
    assert_eq!(pt, b"hello bob");
}

#[test]
fn test_ratchet_multiple_messages() {
    let shared_secret = [0x42u8; 32];
    let bob_ratchet = X25519KeyPair::generate();

    let mut alice = RatchetState::init_sender(&shared_secret, &bob_ratchet.public).unwrap();
    let mut bob = RatchetState::init_receiver(&shared_secret, bob_ratchet);

    // Alice sends several messages.
    for i in 0..5 {
        let msg = format!("message {i}");
        let (header, ct) = alice.encrypt_message(msg.as_bytes()).unwrap();
        let pt = bob.decrypt_message(&header, &ct).unwrap();
        assert_eq!(pt, msg.as_bytes());
    }
}

#[test]
fn test_ratchet_bidirectional() {
    let shared_secret = [0x42u8; 32];
    let bob_ratchet = X25519KeyPair::generate();

    let mut alice = RatchetState::init_sender(&shared_secret, &bob_ratchet.public).unwrap();
    let mut bob = RatchetState::init_receiver(&shared_secret, bob_ratchet);

    // Alice -> Bob.
    let (h1, c1) = alice.encrypt_message(b"hello bob").unwrap();
    let p1 = bob.decrypt_message(&h1, &c1).unwrap();
    assert_eq!(p1, b"hello bob");

    // Bob -> Alice (triggers DH ratchet on both sides).
    let (h2, c2) = bob.encrypt_message(b"hello alice").unwrap();
    let p2 = alice.decrypt_message(&h2, &c2).unwrap();
    assert_eq!(p2, b"hello alice");

    // Alice -> Bob again (another ratchet step).
    let (h3, c3) = alice.encrypt_message(b"how are you?").unwrap();
    let p3 = bob.decrypt_message(&h3, &c3).unwrap();
    assert_eq!(p3, b"how are you?");
}

#[test]
fn test_ratchet_forward_secrecy() {
    let shared_secret = [0x42u8; 32];
    let bob_ratchet = X25519KeyPair::generate();

    let mut alice = RatchetState::init_sender(&shared_secret, &bob_ratchet.public).unwrap();
    let mut bob = RatchetState::init_receiver(&shared_secret, bob_ratchet);

    // Exchange several messages with ratchet steps.
    let (h1, c1) = alice.encrypt_message(b"message 1").unwrap();
    bob.decrypt_message(&h1, &c1).unwrap();

    let (h2, c2) = bob.encrypt_message(b"reply 1").unwrap();
    alice.decrypt_message(&h2, &c2).unwrap();

    let (h3, c3) = alice.encrypt_message(b"message 2").unwrap();
    bob.decrypt_message(&h3, &c3).unwrap();

    // Verify chain keys have advanced beyond the initial state.
    assert_ne!(alice.send_chain_key, shared_secret);
    assert_ne!(alice.recv_chain_key, shared_secret);

    // Each message uses a unique key derived from the chain.
    let (_, ct_a) = alice.encrypt_message(b"same").unwrap();
    let (_, ct_b) = alice.encrypt_message(b"same").unwrap();
    assert_ne!(ct_a, ct_b, "each message must use a unique key");
}

#[test]
fn test_ratchet_message_ordering() {
    let shared_secret = [0x42u8; 32];
    let bob_ratchet = X25519KeyPair::generate();

    let mut alice = RatchetState::init_sender(&shared_secret, &bob_ratchet.public).unwrap();
    let mut bob = RatchetState::init_receiver(&shared_secret, bob_ratchet);

    // Alice sends three messages in order.
    let (h0, c0) = alice.encrypt_message(b"msg 0").unwrap();
    let (h1, c1) = alice.encrypt_message(b"msg 1").unwrap();
    let (h2, c2) = alice.encrypt_message(b"msg 2").unwrap();

    // Bob decrypts in order -- all should succeed.
    assert_eq!(bob.decrypt_message(&h0, &c0).unwrap(), b"msg 0");
    assert_eq!(bob.decrypt_message(&h1, &c1).unwrap(), b"msg 1");
    assert_eq!(bob.decrypt_message(&h2, &c2).unwrap(), b"msg 2");
}

#[test]
fn test_wrong_key_fails_ratchet() {
    let shared_secret_a = [0x42u8; 32];
    let shared_secret_b = [0x99u8; 32];
    let bob_ratchet = X25519KeyPair::generate();

    let mut alice = RatchetState::init_sender(&shared_secret_a, &bob_ratchet.public).unwrap();
    let mut bob = RatchetState::init_receiver(&shared_secret_b, bob_ratchet);

    let (header, ct) = alice.encrypt_message(b"secret").unwrap();
    let result = bob.decrypt_message(&header, &ct);
    assert!(result.is_err(), "mismatched secrets must fail");
}
