//! Test data generators for the Ephemera platform.
//!
//! Provides functions that produce randomized but valid test data
//! for posts, identities, media, and social graph operations.

use ephemera_crypto::{
    identity::PseudonymIdentity, keys::MasterSecret, NodeIdentity, SigningKeyPair,
};
use ephemera_types::IdentityKey;
use rand::Rng;
use serde_json::Value;

/// Generate a random text post body with realistic content.
///
/// Returns a JSON object suitable for passing to `posts.create`:
/// ```json
/// { "body": "...", "ttl_seconds": 86400 }
/// ```
#[must_use]
pub fn random_text_post() -> Value {
    let mut rng = rand::thread_rng();

    let adjectives = [
        "ephemeral",
        "anonymous",
        "decentralized",
        "private",
        "encrypted",
        "fleeting",
        "transient",
        "secure",
        "distributed",
        "free",
    ];
    let nouns = [
        "thought",
        "message",
        "note",
        "idea",
        "whisper",
        "signal",
        "post",
        "moment",
        "reflection",
        "echo",
    ];
    let tags = ["#ephemera", "#decentralized", "#privacy", "#p2p", "#test"];

    let adj = adjectives[rng.gen_range(0..adjectives.len())];
    let noun = nouns[rng.gen_range(0..nouns.len())];
    let tag = tags[rng.gen_range(0..tags.len())];
    let num: u32 = rng.gen_range(1..10000);

    let body = format!("This is a {adj} {noun} #{num} {tag}");
    let ttl = [3600, 7200, 14400, 43200, 86400][rng.gen_range(0..5)];

    serde_json::json!({
        "body": body,
        "ttl_seconds": ttl,
    })
}

/// Generate a random small PNG image as raw bytes.
///
/// Creates a real, valid PNG image (8x8 pixels with random colors)
/// suitable for testing media upload paths.
#[must_use]
pub fn random_photo_post() -> Vec<u8> {
    let mut rng = rand::thread_rng();
    let width = 8u32;
    let height = 8u32;

    let mut img = image::RgbImage::new(width, height);
    for pixel in img.pixels_mut() {
        *pixel = image::Rgb([rng.gen::<u8>(), rng.gen::<u8>(), rng.gen::<u8>()]);
    }

    let mut buf = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buf);
    img.write_to(&mut cursor, image::ImageFormat::Png)
        .expect("encoding a small PNG should not fail");

    buf
}

/// Generate small, valid video-like data for testing.
///
/// This produces a minimal byte sequence with a recognizable header
/// that can pass basic "is this plausibly video?" checks. It is NOT
/// a valid video container, but it is suitable for storage and hash
/// tests.
#[must_use]
pub fn random_video_data() -> Vec<u8> {
    let mut rng = rand::thread_rng();
    let size: usize = rng.gen_range(256..1024);

    // Start with an ftyp-style header so content-type sniffing
    // can recognize it as video-like data.
    let mut data = Vec::with_capacity(size);
    // Minimal MP4/ftyp box header (12 bytes).
    data.extend_from_slice(&[
        0x00, 0x00, 0x00, 0x14, // box size = 20
        0x66, 0x74, 0x79, 0x70, // "ftyp"
        0x69, 0x73, 0x6F, 0x6D, // "isom"
        0x00, 0x00, 0x02, 0x00, // minor version
        0x69, 0x73, 0x6F, 0x6D, // compatible brand "isom"
    ]);

    // Fill the rest with random bytes.
    while data.len() < size {
        data.push(rng.gen::<u8>());
    }

    data
}

/// Generate a random test identity (master secret + pseudonym).
///
/// Returns a tuple of `(MasterSecret, PseudonymIdentity)` for the
/// first derived pseudonym (index 0).
pub fn random_identity() -> (MasterSecret, PseudonymIdentity) {
    let master = MasterSecret::generate();
    let pseudonym = PseudonymIdentity::derive(&master, 0)
        .expect("deriving pseudonym index 0 should never fail");
    (master, pseudonym)
}

/// Generate a random `SigningKeyPair` for ad-hoc signing.
#[must_use]
pub fn random_signing_keypair() -> SigningKeyPair {
    SigningKeyPair::generate()
}

/// Generate a random `NodeIdentity` for network-layer testing.
#[must_use]
pub fn random_node_identity() -> NodeIdentity {
    NodeIdentity::generate()
}

/// Generate a connection request payload between two identities.
///
/// Returns a JSON object representing a connection request:
/// ```json
/// {
///   "from": "<hex pubkey>",
///   "to": "<hex pubkey>",
///   "message": "...",
///   "timestamp_ms": 1234567890
/// }
/// ```
#[must_use]
pub fn random_connection_request(from: &IdentityKey, to: &IdentityKey) -> Value {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is before UNIX epoch")
        .as_millis() as u64;

    let greetings = [
        "Hey, let's connect!",
        "Would love to follow your ephemeral thoughts.",
        "Connecting in the decentralized void.",
        "Anonymous but friendly.",
        "Privacy-first friendship request.",
    ];

    let mut rng = rand::thread_rng();
    let msg = greetings[rng.gen_range(0..greetings.len())];

    serde_json::json!({
        "from": from.to_hex(),
        "to": to.to_hex(),
        "message": msg,
        "timestamp_ms": now_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_post_has_body_and_ttl() {
        let post = random_text_post();
        assert!(post.get("body").and_then(|v| v.as_str()).is_some());
        assert!(post.get("ttl_seconds").and_then(|v| v.as_u64()).is_some());
    }

    #[test]
    fn text_post_body_is_nonempty() {
        for _ in 0..20 {
            let post = random_text_post();
            let body = post["body"].as_str().unwrap();
            assert!(!body.is_empty());
        }
    }

    #[test]
    fn photo_post_is_valid_png() {
        let png = random_photo_post();
        // PNG magic bytes: 0x89 P N G
        assert!(png.len() > 8);
        assert_eq!(&png[..4], &[0x89, 0x50, 0x4E, 0x47]);
    }

    #[test]
    fn video_data_has_ftyp_header() {
        let video = random_video_data();
        assert!(video.len() >= 20);
        // Check for "ftyp" at offset 4.
        assert_eq!(&video[4..8], b"ftyp");
    }

    #[test]
    fn random_identity_produces_valid_keypair() {
        let (master, pseudo) = random_identity();
        let msg = b"test message for signature";
        let sig = pseudo.sign(msg).unwrap();
        let pubkey = pseudo.identity_key();
        assert!(ephemera_crypto::identity::verify_signature(pubkey.as_bytes(), msg, &sig).is_ok());
        // Master secret should be 32 bytes.
        assert_eq!(master.as_bytes().len(), 32);
    }

    #[test]
    fn connection_request_has_required_fields() {
        let (_, from) = random_identity();
        let (_, to) = random_identity();
        let req = random_connection_request(&from.identity_key(), &to.identity_key());

        assert!(req.get("from").is_some());
        assert!(req.get("to").is_some());
        assert!(req.get("message").is_some());
        assert!(req.get("timestamp_ms").is_some());
    }

    #[test]
    fn two_random_identities_are_different() {
        let (_, a) = random_identity();
        let (_, b) = random_identity();
        assert_ne!(a.identity_key(), b.identity_key());
    }
}
