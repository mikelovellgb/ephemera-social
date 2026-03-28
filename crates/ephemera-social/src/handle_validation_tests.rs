use super::*;

// ── Format validation ────────────────────────────────────────────

#[test]
fn test_valid_handle_format() {
    assert!(validate_handle_format("alice").is_ok());
    assert!(validate_handle_format("bob42").is_ok());
    assert!(validate_handle_format("quiet_fox").is_ok());
    assert!(validate_handle_format("abc").is_ok());
    assert!(validate_handle_format("a_b_c_d_e_f_g_h_i_j").is_ok()); // 20 chars
    assert!(validate_handle_format("zzzzzzzzzzzzzzzzzzzz").is_ok()); // exactly 20
}

#[test]
fn test_invalid_handle_too_short() {
    assert!(matches!(
        validate_handle_format("ab"),
        Err(HandleValidationError::TooShort { len: 2 })
    ));
    assert!(matches!(
        validate_handle_format(""),
        Err(HandleValidationError::TooShort { len: 0 })
    ));
    assert!(matches!(
        validate_handle_format("x"),
        Err(HandleValidationError::TooShort { len: 1 })
    ));
}

#[test]
fn test_invalid_handle_too_long() {
    let long = "a".repeat(21);
    assert!(matches!(
        validate_handle_format(&long),
        Err(HandleValidationError::TooLong { len: 21 })
    ));
}

#[test]
fn test_invalid_handle_uppercase() {
    assert!(matches!(
        validate_handle_format("Alice"),
        Err(HandleValidationError::InvalidCharacter { ch: 'A', pos: 0 })
    ));
}

#[test]
fn test_invalid_handle_special_chars() {
    assert!(matches!(
        validate_handle_format("al-ice"),
        Err(HandleValidationError::InvalidCharacter { ch: '-', .. })
    ));
    assert!(matches!(
        validate_handle_format("al.ice"),
        Err(HandleValidationError::InvalidCharacter { ch: '.', .. })
    ));
    assert!(matches!(
        validate_handle_format("al@ice"),
        Err(HandleValidationError::InvalidCharacter { ch: '@', .. })
    ));
}

#[test]
fn test_invalid_handle_underscore_rules() {
    assert!(matches!(
        validate_handle_format("_alice"),
        Err(HandleValidationError::StartsWithUnderscore)
    ));
    assert!(matches!(
        validate_handle_format("alice_"),
        Err(HandleValidationError::EndsWithUnderscore)
    ));
    assert!(matches!(
        validate_handle_format("al__ice"),
        Err(HandleValidationError::ConsecutiveUnderscores)
    ));
}

#[test]
fn test_invalid_handle_starts_with_digit() {
    assert!(matches!(
        validate_handle_format("42alice"),
        Err(HandleValidationError::StartsWithDigit)
    ));
}

#[test]
fn test_reserved_handles_blocked() {
    assert!(matches!(
        validate_handle_format("admin"),
        Err(HandleValidationError::Reserved { .. })
    ));
    assert!(matches!(
        validate_handle_format("support"),
        Err(HandleValidationError::Reserved { .. })
    ));
    assert!(matches!(
        validate_handle_format("ephemera"),
        Err(HandleValidationError::Reserved { .. })
    ));
    assert!(matches!(
        validate_handle_format("system"),
        Err(HandleValidationError::Reserved { .. })
    ));
}

// ── Difficulty calculation ────────────────────────────────────────

#[test]
fn test_short_handle_high_difficulty() {
    assert_eq!(calculate_pow_difficulty("abc"), PowDifficulty::High);
    assert_eq!(calculate_pow_difficulty("abcde"), PowDifficulty::High);
    assert_eq!(calculate_pow_difficulty("fox"), PowDifficulty::High);
}

#[test]
fn test_medium_handle_medium_difficulty() {
    assert_eq!(calculate_pow_difficulty("abcdef"), PowDifficulty::Medium);
    assert_eq!(
        calculate_pow_difficulty("abcdefghij"),
        PowDifficulty::Medium
    );
}

#[test]
fn test_long_handle_low_difficulty() {
    assert_eq!(calculate_pow_difficulty("abcdefghijk"), PowDifficulty::Low);
    assert_eq!(
        calculate_pow_difficulty("abcdefghijklmnopqrst"),
        PowDifficulty::Low
    );
}

// ── PoW challenge construction ───────────────────────────────────

#[test]
fn test_pow_challenge_deterministic() {
    let owner = IdentityKey::from_bytes([0xAA; 32]);
    let c1 = build_pow_challenge("alice", &owner, 1000);
    let c2 = build_pow_challenge("alice", &owner, 1000);
    assert_eq!(c1, c2);
}

#[test]
fn test_pow_challenge_varies_with_name() {
    let owner = IdentityKey::from_bytes([0xAA; 32]);
    let c1 = build_pow_challenge("alice", &owner, 1000);
    let c2 = build_pow_challenge("bob", &owner, 1000);
    assert_ne!(c1, c2);
}

#[test]
fn test_pow_challenge_varies_with_owner() {
    let owner1 = IdentityKey::from_bytes([0xAA; 32]);
    let owner2 = IdentityKey::from_bytes([0xBB; 32]);
    let c1 = build_pow_challenge("alice", &owner1, 1000);
    let c2 = build_pow_challenge("alice", &owner2, 1000);
    assert_ne!(c1, c2);
}

#[test]
fn test_pow_challenge_varies_with_timestamp() {
    let owner = IdentityKey::from_bytes([0xAA; 32]);
    let c1 = build_pow_challenge("alice", &owner, 1000);
    let c2 = build_pow_challenge("alice", &owner, 2000);
    assert_ne!(c1, c2);
}

// ── Signature message construction ───────────────────────────────

#[test]
fn test_sig_message_deterministic() {
    let owner = IdentityKey::from_bytes([0xAA; 32]);
    let m1 = build_signature_message("alice", &owner, 1000);
    let m2 = build_signature_message("alice", &owner, 1000);
    assert_eq!(m1, m2);
}

#[test]
fn test_sig_and_pow_challenges_differ() {
    // Ensure the domain separators keep sig and pow challenges distinct.
    let owner = IdentityKey::from_bytes([0xAA; 32]);
    let pow_c = build_pow_challenge("alice", &owner, 1000);
    let sig_m = build_signature_message("alice", &owner, 1000);
    assert_ne!(pow_c, sig_m);
}

// ── Signature verification ───────────────────────────────────────

#[test]
fn test_handle_signature_verification() {
    let kp = ephemera_crypto::signing::SigningKeyPair::generate();
    let owner = kp.public_key();
    let registered_at = 1_700_000_000u64;
    let msg = build_signature_message("alice", &owner, registered_at);
    let sig = kp.sign(&msg);

    assert!(verify_handle_signature("alice", &owner, registered_at, &sig).is_ok());
}

#[test]
fn test_handle_signature_wrong_name_fails() {
    let kp = ephemera_crypto::signing::SigningKeyPair::generate();
    let owner = kp.public_key();
    let registered_at = 1_700_000_000u64;
    let msg = build_signature_message("alice", &owner, registered_at);
    let sig = kp.sign(&msg);

    // Verify against a different name should fail
    assert!(verify_handle_signature("bob", &owner, registered_at, &sig).is_err());
}

#[test]
fn test_handle_signature_wrong_key_fails() {
    let kp1 = ephemera_crypto::signing::SigningKeyPair::generate();
    let kp2 = ephemera_crypto::signing::SigningKeyPair::generate();
    let owner = kp1.public_key();
    let registered_at = 1_700_000_000u64;
    let msg = build_signature_message("alice", &owner, registered_at);
    let sig = kp1.sign(&msg);

    // Verify against the wrong key should fail
    assert!(verify_handle_signature("alice", &kp2.public_key(), registered_at, &sig).is_err());
}

// ── PoW verification ─────────────────────────────────────────────

#[test]
fn test_handle_pow_verification() {
    let owner = IdentityKey::from_bytes([0xAA; 32]);
    let registered_at = 1_700_000_000u64;
    let challenge = build_pow_challenge("long_handle_name", &owner, registered_at);
    // Use a very low difficulty for testing speed
    let stamp = ephemera_crypto::pow::generate_pow(&challenge, 4);

    // Should pass with difficulty <= 4
    assert!(verify_handle_pow(
        "long_handle_name",
        &owner,
        registered_at,
        &stamp,
        PowDifficulty::Renewal, // We override the check below
    )
    .is_err()); // Renewal is 18 bits, our stamp only has 4

    // Regenerate with enough difficulty
    let stamp = ephemera_crypto::pow::generate_pow(&challenge, POW_DIFFICULTY_RENEWAL);
    assert!(verify_handle_pow(
        "long_handle_name",
        &owner,
        registered_at,
        &stamp,
        PowDifficulty::Renewal,
    )
    .is_ok());
}
