use super::*;
use crate::prekey::validate_prekey_bundle;
use ephemera_crypto::signing::SigningKeyPair;

#[test]
fn test_x3dh_key_agreement() {
    let alice_signing = SigningKeyPair::generate();
    let alice_x25519 = X25519KeyPair::generate();

    let bob_signing = SigningKeyPair::generate();
    let bob_x25519 = X25519KeyPair::generate();

    // Bob publishes a prekey bundle (with one-time prekey).
    let bob_signed_prekey = X25519KeyPair::generate();
    let bob_otp = X25519KeyPair::generate();

    let bob_bundle = PrekeyBundle {
        identity_key: bob_signing.public_key(),
        identity_x25519: bob_x25519.public.clone(),
        signed_prekey: bob_signed_prekey.public.clone(),
        signed_prekey_signature: bob_signing.sign(bob_signed_prekey.public.as_bytes()),
        one_time_prekey: Some(bob_otp.public.clone()),
    };
    assert!(validate_prekey_bundle(&bob_bundle).is_ok());

    // Alice initiates X3DH.
    let alice_result = x3dh_initiate(
        &alice_x25519.secret,
        &alice_x25519.public,
        &alice_signing.public_key(),
        &bob_bundle,
    )
    .unwrap();

    // Bob responds to X3DH.
    let bob_shared = x3dh_respond(
        &bob_x25519.secret,
        &bob_signed_prekey.secret,
        Some(&bob_otp.secret),
        &alice_result.initial_message,
    )
    .unwrap();

    // Both sides must derive the same shared secret.
    assert_eq!(alice_result.shared_secret, bob_shared);
}

#[test]
fn test_x3dh_without_one_time_prekey() {
    let alice_signing = SigningKeyPair::generate();
    let alice_x25519 = X25519KeyPair::generate();

    let bob_signing = SigningKeyPair::generate();
    let bob_x25519 = X25519KeyPair::generate();

    let bob_signed_prekey = X25519KeyPair::generate();

    let bob_bundle = PrekeyBundle {
        identity_key: bob_signing.public_key(),
        identity_x25519: bob_x25519.public.clone(),
        signed_prekey: bob_signed_prekey.public.clone(),
        signed_prekey_signature: bob_signing.sign(bob_signed_prekey.public.as_bytes()),
        one_time_prekey: None,
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
        &bob_signed_prekey.secret,
        None,
        &alice_result.initial_message,
    )
    .unwrap();

    assert_eq!(alice_result.shared_secret, bob_shared);
}

#[test]
fn test_x3dh_different_initiators_get_different_secrets() {
    let alice_signing = SigningKeyPair::generate();
    let alice_x25519 = X25519KeyPair::generate();

    let carol_signing = SigningKeyPair::generate();
    let carol_x25519 = X25519KeyPair::generate();

    let bob_signing = SigningKeyPair::generate();
    let bob_x25519 = X25519KeyPair::generate();

    let bob_signed_prekey = X25519KeyPair::generate();

    let bob_bundle = PrekeyBundle {
        identity_key: bob_signing.public_key(),
        identity_x25519: bob_x25519.public.clone(),
        signed_prekey: bob_signed_prekey.public.clone(),
        signed_prekey_signature: bob_signing.sign(bob_signed_prekey.public.as_bytes()),
        one_time_prekey: None,
    };

    let alice_result = x3dh_initiate(
        &alice_x25519.secret,
        &alice_x25519.public,
        &alice_signing.public_key(),
        &bob_bundle,
    )
    .unwrap();

    let carol_result = x3dh_initiate(
        &carol_x25519.secret,
        &carol_x25519.public,
        &carol_signing.public_key(),
        &bob_bundle,
    )
    .unwrap();

    // Different initiators must produce different shared secrets.
    assert_ne!(alice_result.shared_secret, carol_result.shared_secret);
}
