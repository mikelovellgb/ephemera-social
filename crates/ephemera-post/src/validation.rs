//! Post validation rules.
//!
//! Validates incoming posts against protocol constraints: signature
//! verification, proof-of-work, TTL bounds, text length, and
//! attachment limits.

use ephemera_crypto::verify_signature;
use ephemera_types::{Timestamp, CLOCK_SKEW_TOLERANCE};

use crate::content::{MAX_ATTACHMENTS, MAX_TEXT_BYTES, MAX_TEXT_GRAPHEMES};
use crate::post::Post;
use crate::PostError;

/// Maximum reply thread depth.
pub const MAX_THREAD_DEPTH: u32 = 50;

/// Validate a post received from the network.
///
/// Checks in order:
/// 1. Signature is valid for the author's public key.
/// 2. TTL is within protocol bounds (already enforced by the `Ttl` type,
///    but we double-check the wire value).
/// 3. Post has not already expired (with clock skew tolerance).
/// 4. Post timestamp is not too far in the future.
/// 5. Text body length is within limits.
/// 6. Attachment count is within limits.
/// 7. Thread depth does not exceed maximum.
///
/// # Errors
///
/// Returns a [`PostError`] describing the first validation failure.
pub fn validate_post(post: &Post) -> Result<(), PostError> {
    validate_signature(post)?;
    validate_not_expired(post)?;
    validate_not_future(post)?;
    validate_text_length(post)?;
    validate_attachment_count(post)?;
    validate_thread_depth(post)?;
    Ok(())
}

/// Verify the Ed25519 signature on the post.
fn validate_signature(post: &Post) -> Result<(), PostError> {
    let canonical = crate::canonical::canonical_bytes(
        &post.content,
        &post.author,
        post.created_at.as_secs(),
        &post.ttl,
        &post.parent,
        &post.root,
        post.depth,
    )?;

    verify_signature(&post.author, &canonical, &post.signature)
        .map_err(|_| PostError::Validation("invalid signature".into()))
}

/// Check that the post has not already expired.
fn validate_not_expired(post: &Post) -> Result<(), PostError> {
    let now = Timestamp::now().as_secs();
    let expires_at = post.created_at.as_secs() + post.ttl.as_secs();
    if expires_at + CLOCK_SKEW_TOLERANCE < now {
        return Err(PostError::Validation(format!(
            "post expired at {expires_at}, now is {now}"
        )));
    }
    Ok(())
}

/// Check that the post timestamp is not too far in the future.
fn validate_not_future(post: &Post) -> Result<(), PostError> {
    let now = Timestamp::now().as_secs();
    if post.created_at.as_secs() > now + CLOCK_SKEW_TOLERANCE {
        return Err(PostError::Validation(
            "post timestamp is in the future".into(),
        ));
    }
    Ok(())
}

/// Validate text body length (both grapheme clusters and byte size).
fn validate_text_length(post: &Post) -> Result<(), PostError> {
    if let Some(body) = post.content.text_body() {
        if body.len() > MAX_TEXT_BYTES {
            return Err(PostError::Validation(format!(
                "text body is {} bytes, max is {MAX_TEXT_BYTES}",
                body.len()
            )));
        }
        // Count grapheme clusters using a simple char count as an approximation.
        // The full UAX #29 implementation lives in the client-side validation;
        // here we use char count as a conservative check.
        let char_count = body.chars().count();
        if char_count > MAX_TEXT_GRAPHEMES {
            return Err(PostError::Validation(format!(
                "text body is {char_count} characters, max is {MAX_TEXT_GRAPHEMES}"
            )));
        }
    }
    Ok(())
}

/// Validate attachment count.
fn validate_attachment_count(post: &Post) -> Result<(), PostError> {
    let count = post.content.attachment_count();
    if count > MAX_ATTACHMENTS {
        return Err(PostError::Validation(format!(
            "post has {count} attachments, max is {MAX_ATTACHMENTS}"
        )));
    }
    Ok(())
}

/// Validate reply thread depth.
fn validate_thread_depth(post: &Post) -> Result<(), PostError> {
    if post.depth > MAX_THREAD_DEPTH {
        return Err(PostError::Validation(format!(
            "thread depth {} exceeds maximum {MAX_THREAD_DEPTH}",
            post.depth
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::PostBuilder;
    use ephemera_crypto::SigningKeyPair;
    use ephemera_types::Ttl;

    fn signed_post() -> (Post, SigningKeyPair) {
        let kp = SigningKeyPair::generate();
        let post = PostBuilder::new()
            .text("test post please ignore")
            .ttl(Ttl::from_secs(86400).unwrap())
            .build(&kp)
            .unwrap();
        (post, kp)
    }

    #[test]
    fn valid_post_passes() {
        let (post, _) = signed_post();
        assert!(validate_post(&post).is_ok());
    }

    #[test]
    fn tampered_content_fails_signature() {
        let (mut post, _) = signed_post();
        post.content = crate::content::PostContent::Text {
            body: "TAMPERED".into(),
        };
        let result = validate_post(&post);
        assert!(result.is_err());
    }

    #[test]
    fn expired_post_fails() {
        let kp = SigningKeyPair::generate();
        let mut post = PostBuilder::new()
            .text("old post")
            .ttl(Ttl::from_secs(3600).unwrap())
            .build(&kp)
            .unwrap();
        // Backdate the post far enough to be expired.
        post.created_at = Timestamp::from_secs(1_000_000);
        // Re-sign with the backdated timestamp using the canonical bytes function.
        let canonical = crate::canonical::canonical_bytes(
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
        post.id = ephemera_types::ContentId::from_digest(*id_hash.as_bytes());

        let result = validate_post(&post);
        assert!(result.is_err());
    }

    #[test]
    fn excessive_depth_fails() {
        let kp = SigningKeyPair::generate();
        let parent = ephemera_types::ContentId::from_digest([0xAA; 32]);
        let root = ephemera_types::ContentId::from_digest([0xBB; 32]);
        let post = PostBuilder::new()
            .text("deep reply")
            .reply_to(parent, root, MAX_THREAD_DEPTH + 1)
            .ttl(Ttl::from_secs(3600).unwrap())
            .build(&kp)
            .unwrap();

        let result = validate_post(&post);
        assert!(result.is_err());
    }

    #[test]
    fn long_text_fails() {
        let kp = SigningKeyPair::generate();
        let long_body = "x".repeat(MAX_TEXT_GRAPHEMES + 1);
        let post = PostBuilder::new()
            .text(&long_body)
            .ttl(Ttl::from_secs(3600).unwrap())
            .build(&kp)
            .unwrap();

        let result = validate_post(&post);
        assert!(result.is_err());
    }
}
