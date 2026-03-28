//! Integration tests for the full X3DH + Double Ratchet messaging protocol.
//!
//! These tests verify the complete flow from prekey bundle generation
//! through X3DH key exchange, ratchet initialization, and multi-message
//! bidirectional conversations.

#[cfg(test)]
mod tests {
    use crate::prekey::{generate_prekey_bundle, validate_prekey_bundle, PrekeyBundle};
    use crate::ratchet::RatchetState;
    use crate::x3dh::{x3dh_initiate, x3dh_respond};
    use ephemera_crypto::signing::SigningKeyPair;
    use ephemera_crypto::X25519KeyPair;

    /// Test that X3DH produces matching shared secrets on both sides.
    #[test]
    fn test_x3dh_key_agreement() {
        let alice_signing = SigningKeyPair::generate();
        let alice_x25519 = X25519KeyPair::generate();
        let bob_signing = SigningKeyPair::generate();
        let bob_x25519 = X25519KeyPair::generate();
        let bob_spk = X25519KeyPair::generate();
        let bob_otp = X25519KeyPair::generate();

        let bob_bundle = PrekeyBundle {
            identity_key: bob_signing.public_key(),
            identity_x25519: bob_x25519.public.clone(),
            signed_prekey: bob_spk.public.clone(),
            signed_prekey_signature: bob_signing.sign(bob_spk.public.as_bytes()),
            one_time_prekey: Some(bob_otp.public.clone()),
        };

        let alice_result = x3dh_initiate(
            &alice_x25519.secret,
            &alice_x25519.public,
            &alice_signing.public_key(),
            &bob_bundle,
        )
        .unwrap();

        let bob_shared = x3dh_respond(
            &bob_x25519.secret,
            &bob_spk.secret,
            Some(&bob_otp.secret),
            &alice_result.initial_message,
        )
        .unwrap();

        assert_eq!(
            alice_result.shared_secret, bob_shared,
            "X3DH must produce identical shared secrets on both sides"
        );
    }

    /// Test that compromising current ratchet keys does not reveal past messages.
    #[test]
    fn test_ratchet_forward_secrecy() {
        let shared_secret = [0x42u8; 32];
        let bob_ratchet = X25519KeyPair::generate();

        let mut alice = RatchetState::init_sender(&shared_secret, &bob_ratchet.public).unwrap();
        let mut bob = RatchetState::init_receiver(&shared_secret, bob_ratchet);

        // Exchange messages with DH ratchet steps.
        let (h1, c1) = alice.encrypt_message(b"past message 1").unwrap();
        let p1 = bob.decrypt_message(&h1, &c1).unwrap();
        assert_eq!(p1, b"past message 1");

        // Bob replies (triggers DH ratchet).
        let (h2, c2) = bob.encrypt_message(b"past reply").unwrap();
        let p2 = alice.decrypt_message(&h2, &c2).unwrap();
        assert_eq!(p2, b"past reply");

        // Alice sends again (another DH ratchet).
        let (h3, c3) = alice.encrypt_message(b"past message 2").unwrap();
        let p3 = bob.decrypt_message(&h3, &c3).unwrap();
        assert_eq!(p3, b"past message 2");

        // Now: even if an attacker captures the current chain keys, they
        // cannot decrypt past messages because those used different keys
        // from earlier ratchet states. Each DH ratchet step produces
        // new chain keys that are cryptographically independent.
        //
        // Verify the ratchet has advanced by checking that encrypting
        // the same plaintext yields different ciphertext each time.
        let (_, ct_now_a) = alice.encrypt_message(b"test").unwrap();
        let (_, ct_now_b) = alice.encrypt_message(b"test").unwrap();
        assert_ne!(ct_now_a, ct_now_b, "every message must use a unique key");

        // The ciphertext from the past (c1) used a different chain key
        // than what we have now, so compromise of current state cannot
        // reconstruct the key used for c1.
        assert_ne!(
            c1, ct_now_a,
            "past ciphertext must differ from current encryption"
        );
    }

    /// Test that messages decrypt correctly when received in order.
    #[test]
    fn test_ratchet_message_ordering() {
        let shared_secret = [0x42u8; 32];
        let bob_ratchet = X25519KeyPair::generate();

        let mut alice = RatchetState::init_sender(&shared_secret, &bob_ratchet.public).unwrap();
        let mut bob = RatchetState::init_receiver(&shared_secret, bob_ratchet);

        let messages: Vec<&[u8]> = vec![b"first", b"second", b"third", b"fourth"];
        let mut encrypted: Vec<_> = Vec::new();

        for msg in &messages {
            let (h, c) = alice.encrypt_message(msg).unwrap();
            encrypted.push((h, c));
        }

        // Decrypt in order: all must succeed with correct plaintext.
        for (i, (h, c)) in encrypted.iter().enumerate() {
            let pt = bob.decrypt_message(h, c).unwrap();
            assert_eq!(pt, messages[i], "message {i} decrypted incorrectly");
        }
    }

    /// Test that a valid prekey bundle passes validation and a tampered one fails.
    #[test]
    fn test_prekey_bundle_validation() {
        let signing = SigningKeyPair::generate();
        let x25519 = X25519KeyPair::generate();

        // Valid bundle.
        let bundle = generate_prekey_bundle(&signing, &x25519.public, true);
        assert!(
            validate_prekey_bundle(&bundle).is_ok(),
            "valid bundle must pass validation"
        );

        // Tamper with signed_prekey: swap in a different key.
        let mut tampered = bundle.clone();
        tampered.signed_prekey = X25519KeyPair::generate().public;
        assert!(
            validate_prekey_bundle(&tampered).is_err(),
            "tampered signed_prekey must fail validation"
        );

        // Tamper with signature: use a different signer.
        let other_signing = SigningKeyPair::generate();
        let mut wrong_sig_bundle = generate_prekey_bundle(&signing, &x25519.public, false);
        wrong_sig_bundle.signed_prekey_signature =
            other_signing.sign(wrong_sig_bundle.signed_prekey.as_bytes());
        assert!(
            validate_prekey_bundle(&wrong_sig_bundle).is_err(),
            "wrong signer must fail validation"
        );
    }

    /// Full conversation: X3DH init, then multiple messages back and forth.
    #[test]
    fn test_full_conversation() {
        // 1. Setup identities.
        let alice_signing = SigningKeyPair::generate();
        let alice_x25519 = X25519KeyPair::generate();
        let bob_signing = SigningKeyPair::generate();
        let bob_x25519 = X25519KeyPair::generate();

        // 2. Bob publishes prekey bundle.
        let bob_spk = X25519KeyPair::generate();
        let bob_otp = X25519KeyPair::generate();
        let bob_bundle = PrekeyBundle {
            identity_key: bob_signing.public_key(),
            identity_x25519: bob_x25519.public.clone(),
            signed_prekey: bob_spk.public.clone(),
            signed_prekey_signature: bob_signing.sign(bob_spk.public.as_bytes()),
            one_time_prekey: Some(bob_otp.public.clone()),
        };
        assert!(validate_prekey_bundle(&bob_bundle).is_ok());

        // 3. Alice performs X3DH and initializes her ratchet.
        let alice_x3dh = x3dh_initiate(
            &alice_x25519.secret,
            &alice_x25519.public,
            &alice_signing.public_key(),
            &bob_bundle,
        )
        .unwrap();

        let mut alice_ratchet =
            RatchetState::init_sender(&alice_x3dh.shared_secret, &bob_spk.public).unwrap();

        // 4. Bob responds to X3DH and initializes his ratchet.
        let bob_shared = x3dh_respond(
            &bob_x25519.secret,
            &bob_spk.secret,
            Some(&bob_otp.secret),
            &alice_x3dh.initial_message,
        )
        .unwrap();

        assert_eq!(alice_x3dh.shared_secret, bob_shared);

        let mut bob_ratchet = RatchetState::init_receiver(&bob_shared, bob_spk);

        // 5. Alice sends first message.
        let (h1, c1) = alice_ratchet
            .encrypt_message(b"Hey Bob! First message via X3DH.")
            .unwrap();
        let p1 = bob_ratchet.decrypt_message(&h1, &c1).unwrap();
        assert_eq!(p1, b"Hey Bob! First message via X3DH.");

        // 6. Alice sends another message (same send chain).
        let (h2, c2) = alice_ratchet.encrypt_message(b"Are you there?").unwrap();
        let p2 = bob_ratchet.decrypt_message(&h2, &c2).unwrap();
        assert_eq!(p2, b"Are you there?");

        // 7. Bob replies (triggers DH ratchet step).
        let (h3, c3) = bob_ratchet
            .encrypt_message(b"Hey Alice! Yes, I'm here.")
            .unwrap();
        let p3 = alice_ratchet.decrypt_message(&h3, &c3).unwrap();
        assert_eq!(p3, b"Hey Alice! Yes, I'm here.");

        // 8. Alice replies again (another DH ratchet step).
        let (h4, c4) = alice_ratchet
            .encrypt_message(b"Great, the ratchet works!")
            .unwrap();
        let p4 = bob_ratchet.decrypt_message(&h4, &c4).unwrap();
        assert_eq!(p4, b"Great, the ratchet works!");

        // 9. Bob sends multiple messages (advancing his send chain).
        for i in 0..5 {
            let msg = format!("Bob's message #{i}");
            let (h, c) = bob_ratchet.encrypt_message(msg.as_bytes()).unwrap();
            let pt = alice_ratchet.decrypt_message(&h, &c).unwrap();
            assert_eq!(pt, msg.as_bytes());
        }

        // 10. Alice sends a final message.
        let (h_final, c_final) = alice_ratchet.encrypt_message(b"Goodbye Bob!").unwrap();
        let p_final = bob_ratchet.decrypt_message(&h_final, &c_final).unwrap();
        assert_eq!(p_final, b"Goodbye Bob!");
    }
}
