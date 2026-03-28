//! Adversarial input tests for the Ephemera platform.
//!
//! These tests try to break the system with malicious, malformed, or
//! out-of-spec inputs. Every test asserts a specific failure mode rather
//! than silently accepting errors.

use ephemera_abuse::{ActionType, RateLimiter};
use ephemera_crypto::{
    decrypt_xchacha20, encrypt_xchacha20, EpochKeyManager, MasterSecret, SigningKeyPair,
};
use ephemera_media::validation::validate_media;
use ephemera_media::video_validation::validate_video;
use ephemera_post::content::MAX_TEXT_BYTES;
use ephemera_post::validation::validate_post;
use ephemera_post::{Post, PostBuilder};
use ephemera_protocol::codec;
use ephemera_types::{ContentId, IdentityKey, Signature, Timestamp, Ttl};

// ---------------------------------------------------------------------------
// Post validation attacks
// ---------------------------------------------------------------------------

/// 100KB text post should be rejected (MAX_TEXT_BYTES is 16,384).
#[test]
fn test_oversized_text_rejected() {
    let kp = SigningKeyPair::generate();
    let oversized = "x".repeat(MAX_TEXT_BYTES + 1);
    let post = PostBuilder::new()
        .text(&oversized)
        .ttl(Ttl::from_secs(3600).unwrap())
        .build(&kp)
        .unwrap();

    let result = validate_post(&post);
    assert!(result.is_err(), "oversized text post should be rejected");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("bytes") || msg.contains("characters"),
        "error should mention size: {msg}"
    );
}

/// A post dated 1 year in the future must be rejected.
#[test]
fn test_future_timestamp_rejected() {
    let kp = SigningKeyPair::generate();
    let now = Timestamp::now().as_secs();
    let future_ts = now + 365 * 24 * 3600; // 1 year from now

    let mut post = PostBuilder::new()
        .text("from the future")
        .ttl(Ttl::from_secs(3600).unwrap())
        .build(&kp)
        .unwrap();

    // Manually set a future timestamp and re-sign.
    post.created_at = Timestamp::from_secs(future_ts);
    let canonical = ephemera_post::canonical_bytes(
        &post.content,
        &post.author,
        post.created_at.as_secs(),
        &post.ttl,
        &post.parent,
        &post.root,
        post.depth,
    )
    .unwrap();
    post.signature = kp.sign(&canonical);
    let id_hash = blake3::hash(&canonical);
    post.id = ContentId::from_digest(*id_hash.as_bytes());

    let result = validate_post(&post);
    assert!(result.is_err(), "future-dated post should be rejected");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("future"),
        "error should mention future timestamp: {msg}"
    );
}

/// TTL of 0 should be rejected by Ttl::from_secs.
#[test]
fn test_negative_ttl_rejected() {
    let result = Ttl::from_secs(0);
    assert!(result.is_err(), "TTL of 0 should be rejected");

    let result = Ttl::from_secs(1);
    assert!(result.is_err(), "TTL of 1 second should be rejected");
}

/// TTL of 31 days (exceeds 30-day maximum) should be rejected.
#[test]
fn test_ttl_exceeds_30_days_rejected() {
    let thirty_one_days = 31 * 24 * 3600;
    let result = Ttl::from_secs(thirty_one_days);
    assert!(result.is_err(), "TTL of 31 days should be rejected");
}

/// A post with a wrong signature must be rejected.
#[test]
fn test_invalid_signature_rejected() {
    let kp = SigningKeyPair::generate();
    let mut post = PostBuilder::new()
        .text("legitimate post")
        .ttl(Ttl::from_secs(3600).unwrap())
        .build(&kp)
        .unwrap();

    // Replace signature with garbage.
    post.signature = Signature::from_bytes([0xFF; 64]);

    let result = validate_post(&post);
    assert!(
        result.is_err(),
        "post with invalid signature should be rejected"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("signature"),
        "error should mention signature: {msg}"
    );
}

/// A post signed by key A but claiming author B must be rejected.
#[test]
fn test_forged_author_rejected() {
    let real_kp = SigningKeyPair::generate();
    let fake_kp = SigningKeyPair::generate();

    let mut post = PostBuilder::new()
        .text("I am not who I say I am")
        .ttl(Ttl::from_secs(3600).unwrap())
        .build(&real_kp)
        .unwrap();

    // Replace the author with the fake identity's public key
    // but keep the original (real_kp's) signature.
    post.author = fake_kp.public_key();

    let result = validate_post(&post);
    assert!(
        result.is_err(),
        "post with forged author should be rejected"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("signature"),
        "error should mention signature verification failure: {msg}"
    );
}

/// Submitting the same signed post twice should be detectable.
/// The content ID (hash) should be identical for both submissions,
/// so a dedup check (which exists in the gossip layer) would catch it.
#[test]
fn test_replay_attack() {
    let kp = SigningKeyPair::generate();
    let post1 = PostBuilder::new()
        .text("one-time message")
        .ttl(Ttl::from_secs(3600).unwrap())
        .build(&kp)
        .unwrap();

    // Simulate a second submission of the exact same post.
    // In a real system the gossip dedup layer would reject it.
    // Here we verify the content hash is deterministic, which is
    // the prerequisite for dedup to work.
    let post1_bytes = serde_json::to_vec(&post1).unwrap();
    let post2: Post = serde_json::from_slice(&post1_bytes).unwrap();

    assert_eq!(
        post1.id, post2.id,
        "replayed post should have the same content ID"
    );
}

/// An empty post (no text, no media) should be rejected at build time.
#[test]
fn test_empty_post_rejected() {
    let kp = SigningKeyPair::generate();
    let result = PostBuilder::new()
        .ttl(Ttl::from_secs(3600).unwrap())
        .build(&kp);

    assert!(result.is_err(), "empty post should be rejected");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("no content"),
        "error should mention missing content: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Media attacks
// ---------------------------------------------------------------------------

/// Trying to upload a .exe-like file as image should be rejected.
#[test]
fn test_non_media_file_rejected() {
    // MZ header (Windows PE executable).
    let mut exe_data = vec![0u8; 1024];
    exe_data[0] = b'M';
    exe_data[1] = b'Z';

    let result = validate_media(&exe_data);
    assert!(
        result.is_err(),
        "executable file should be rejected by media validation"
    );
}

/// Image with valid JPEG header but truncated body should be rejected.
#[test]
fn test_truncated_image_rejected() {
    // Valid JPEG header bytes followed by truncated data.
    let mut truncated = vec![0xFF, 0xD8, 0xFF, 0xE0];
    truncated.extend_from_slice(&[0x00; 50]); // Some data but not a valid image

    let result = validate_media(&truncated);
    assert!(
        result.is_err(),
        "truncated JPEG should be rejected by media validation"
    );
}

/// 50MB image should be rejected (limit is 10 MiB for images).
#[test]
fn test_oversized_image_rejected() {
    // Create data that exceeds MAX_INPUT_SIZE (10 MiB).
    let oversized = vec![0xFF; 11 * 1024 * 1024]; // 11 MiB
    let result = validate_media(&oversized);
    assert!(
        result.is_err(),
        "oversized image should be rejected by media validation"
    );
}

/// Video over 3 minutes (4-minute video) should be rejected.
#[test]
fn test_video_over_3_minutes_rejected() {
    let four_minutes_ms = 4 * 60 * 1000;
    let input = ephemera_media::test_mp4::build_long_duration_mp4(four_minutes_ms);
    let result = validate_video(&input);

    // The test MP4 builder may produce a file that the mp4 crate rejects
    // for structural reasons, or it may parse and fail on duration.
    // Either way, the result MUST be an error -- it must NOT succeed.
    assert!(
        result.is_err(),
        "4-minute video should be rejected (either by parser or duration check)"
    );
}

/// Video over 50MB should be rejected.
#[test]
fn test_video_over_50mb_rejected() {
    // Build minimal MP4 header then pad to 60MB.
    let target_size = 60 * 1024 * 1024;
    let mut data = vec![0u8; target_size];
    data[0..4].copy_from_slice(&8u32.to_be_bytes());
    data[4..8].copy_from_slice(b"ftyp");

    let result = validate_video(&data);
    assert!(result.is_err(), "60MB video should be rejected");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("exceeds maximum"),
        "error should mention size limit: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Crypto attacks
// ---------------------------------------------------------------------------

/// Trying to unlock a keystore with the wrong passphrase must fail.
#[test]
fn test_wrong_passphrase_keystore() {
    use ephemera_crypto::keystore::{load_keystore, save_keystore, KeystoreContents};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("adversarial.keystore");

    let contents = KeystoreContents {
        master_secret: [0xAA; 32],
        node_secret: [0xBB; 32],
        pseudonym_secrets: vec![],
    };

    save_keystore(&path, b"correct-horse-battery-staple", &contents).unwrap();

    let result = load_keystore(&path, b"wrong-passphrase");
    assert!(result.is_err(), "wrong passphrase must be rejected");
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("decryption failed")
            || msg.contains("wrong passphrase")
            || msg.contains("corrupt"),
        "error should mention decryption failure: {msg}"
    );
}

/// Corrupting random bytes in a keystore file must fail to decrypt.
#[test]
fn test_corrupted_keystore_file() {
    use ephemera_crypto::keystore::{load_keystore, save_keystore, KeystoreContents};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corrupted.keystore");

    let contents = KeystoreContents {
        master_secret: [0xAA; 32],
        node_secret: [0xBB; 32],
        pseudonym_secrets: vec![],
    };

    save_keystore(&path, b"my-passphrase", &contents).unwrap();

    // Read, corrupt a byte in the ciphertext region, write back.
    let mut data = std::fs::read(&path).unwrap();
    // Corrupt a byte well past the header (version + salt = 20 bytes).
    let corrupt_pos = data.len() / 2;
    data[corrupt_pos] ^= 0xFF;
    std::fs::write(&path, &data).unwrap();

    let result = load_keystore(&path, b"my-passphrase");
    assert!(result.is_err(), "corrupted keystore must fail to decrypt");
}

/// Content encrypted with a destroyed epoch key must be undecryptable.
#[test]
fn test_expired_epoch_key_decrypt_fails() {
    let master = MasterSecret::from_bytes([0x42; 32]);
    let mut mgr = EpochKeyManager::new(master);

    // Encrypt some content with the current epoch.
    let plaintext = b"ephemeral secret content";
    let (epoch_id, sealed) = mgr.encrypt_with_current_epoch(plaintext).unwrap();

    // Verify decryption works before destruction.
    let decrypted = mgr.decrypt_with_epoch_key(epoch_id, &sealed).unwrap();
    assert_eq!(decrypted.as_deref(), Some(plaintext.as_slice()));

    // Destroy the key.
    let removed = mgr.destroy_key(epoch_id);
    assert!(removed, "key should have been present and destroyed");

    // Now decryption must return None (key destroyed).
    let result = mgr.decrypt_with_epoch_key(epoch_id, &sealed);
    match result {
        Ok(None) => {} // Expected: key destroyed, cannot decrypt.
        Ok(Some(_)) => panic!("must not decrypt after key destruction"),
        Err(e) => {
            // Also acceptable: an error indicating the key was destroyed.
            let msg = e.to_string();
            assert!(
                msg.contains("destroyed"),
                "error should mention key destruction: {msg}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Rate limiting
// ---------------------------------------------------------------------------

/// Trying to post 100 times per second should be throttled.
#[test]
fn test_rapid_posting_throttled() {
    let mut limiter = RateLimiter::new();
    let identity = IdentityKey::from_bytes([0x99; 32]);

    // Post capacity is 10 per bucket.
    let mut allowed = 0;
    let mut denied = 0;
    for _ in 0..100 {
        match limiter.check(&identity, ActionType::Post) {
            Ok(()) => allowed += 1,
            Err(_) => denied += 1,
        }
    }

    assert!(
        allowed <= 10,
        "at most 10 rapid posts should be allowed, got {allowed}"
    );
    assert!(
        denied >= 90,
        "at least 90 rapid posts should be denied, got {denied}"
    );
}

/// Trying to create 100 identities (connection requests) rapidly
/// should be throttled.
#[test]
fn test_rapid_identity_creation_throttled() {
    let mut limiter = RateLimiter::new();
    let identity = IdentityKey::from_bytes([0x77; 32]);

    // ConnectionRequest capacity is 5.
    let mut allowed = 0;
    let mut denied = 0;
    for _ in 0..100 {
        match limiter.check(&identity, ActionType::ConnectionRequest) {
            Ok(()) => allowed += 1,
            Err(_) => denied += 1,
        }
    }

    assert!(
        allowed <= 5,
        "at most 5 rapid connection requests should be allowed, got {allowed}"
    );
    assert!(
        denied >= 95,
        "at least 95 rapid connection requests should be denied, got {denied}"
    );
}

// ---------------------------------------------------------------------------
// Protocol codec attacks
// ---------------------------------------------------------------------------

/// Random bytes fed to the protocol decoder should not panic.
#[test]
fn test_protocol_decode_random_bytes() {
    use bytes::BytesMut;

    // Various adversarial byte patterns.
    let patterns: Vec<Vec<u8>> = vec![
        vec![0u8; 0],
        vec![0xFF; 4],
        vec![0x00, 0x00, 0x00, 0x01, 0xFF], // length=1, invalid JSON
        vec![0x00, 0x00, 0x00, 0x04, b'{', b'}', 0x00, 0x00], // length=4, minimal JSON
        vec![0xFF, 0xFF, 0xFF, 0xFF],       // max u32 length prefix
        (0..256).map(|i| i as u8).collect(), // all byte values
    ];

    for pattern in &patterns {
        let mut buf = BytesMut::from(pattern.as_slice());
        // Must not panic. Errors are fine, panics are not.
        let _ = codec::decode(&mut buf);
    }
}

/// Random bytes fed to bare protocol decoder should not panic.
#[test]
fn test_protocol_decode_bare_random_bytes() {
    let patterns: Vec<Vec<u8>> = vec![
        vec![],
        vec![0xFF],
        b"not json".to_vec(),
        b"{}".to_vec(),
        b"null".to_vec(),
        vec![0x00; 1024],
    ];

    for pattern in &patterns {
        let _ = codec::decode_bare(pattern);
    }
}

/// Verify that XChaCha20 rejects ciphertext tampered with.
#[test]
fn test_tampered_ciphertext_rejected() {
    let key = [0x42u8; 32];
    let plaintext = b"sensitive data";
    let mut sealed = encrypt_xchacha20(&key, plaintext).unwrap();

    // Flip the last byte of the ciphertext (which is in the auth tag region).
    let last = sealed.len() - 1;
    sealed[last] ^= 0xFF;

    let result = decrypt_xchacha20(&key, &sealed);
    assert!(result.is_err(), "tampered ciphertext must be rejected");
}
