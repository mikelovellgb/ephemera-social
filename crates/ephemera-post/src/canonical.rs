//! Canonical byte serialization for post signing and hashing.
//!
//! The canonical form binds ALL fields that define a post's identity:
//! content, author, created_at, ttl, parent, root, and depth. This
//! ensures that the content hash and signature cover the full post
//! structure, preventing signature bypass attacks (e.g. repackaging
//! a signed post as a reply or stripping the author attribution).

use crate::content::PostContent;
use crate::PostError;
use ephemera_types::{ContentId, IdentityKey, Ttl};

/// Serialize a post's identity-defining fields into canonical bytes
/// suitable for hashing (to produce the content ID) and signing.
///
/// Fields included:
/// - `content` -- the post payload
/// - `author` -- the 32-byte Ed25519 public key
/// - `created_at` -- creation timestamp in seconds
/// - `ttl` -- time-to-live in seconds
/// - `parent` -- optional parent content hash (for replies)
/// - `root` -- optional root content hash (for replies)
/// - `depth` -- thread depth (0 for top-level posts)
///
/// # Errors
///
/// Returns a [`PostError`] if JSON serialization fails.
pub fn canonical_bytes(
    content: &PostContent,
    author: &IdentityKey,
    created_at: u64,
    ttl: &Ttl,
    parent: &Option<ContentId>,
    root: &Option<ContentId>,
    depth: u32,
) -> Result<Vec<u8>, PostError> {
    serde_json::to_vec(&(
        content,
        author.as_bytes(),
        created_at,
        ttl.as_secs(),
        parent,
        root,
        depth,
    ))
    .map_err(|e| PostError::Build(format!("serialization failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ephemera_types::Timestamp;

    #[test]
    fn canonical_bytes_deterministic() {
        let content = PostContent::Text {
            body: "hello".into(),
        };
        let author = IdentityKey::from_bytes([0xAA; 32]);
        let ts = Timestamp::from_secs(1_700_000_000).as_secs();
        let ttl = Ttl::from_secs(3600).unwrap();

        let a = canonical_bytes(&content, &author, ts, &ttl, &None, &None, 0).unwrap();
        let b = canonical_bytes(&content, &author, ts, &ttl, &None, &None, 0).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn different_author_different_bytes() {
        let content = PostContent::Text {
            body: "hello".into(),
        };
        let author_a = IdentityKey::from_bytes([0xAA; 32]);
        let author_b = IdentityKey::from_bytes([0xBB; 32]);
        let ttl = Ttl::from_secs(3600).unwrap();

        let a = canonical_bytes(&content, &author_a, 1000, &ttl, &None, &None, 0).unwrap();
        let b = canonical_bytes(&content, &author_b, 1000, &ttl, &None, &None, 0).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn different_parent_different_bytes() {
        let content = PostContent::Text {
            body: "hello".into(),
        };
        let author = IdentityKey::from_bytes([0xAA; 32]);
        let ttl = Ttl::from_secs(3600).unwrap();
        let parent = Some(ContentId::from_digest([0xDD; 32]));

        let a = canonical_bytes(&content, &author, 1000, &ttl, &None, &None, 0).unwrap();
        let b = canonical_bytes(&content, &author, 1000, &ttl, &parent, &None, 1).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn different_depth_different_bytes() {
        let content = PostContent::Text {
            body: "hello".into(),
        };
        let author = IdentityKey::from_bytes([0xAA; 32]);
        let ttl = Ttl::from_secs(3600).unwrap();
        let parent = Some(ContentId::from_digest([0xDD; 32]));
        let root = Some(ContentId::from_digest([0xEE; 32]));

        let a = canonical_bytes(&content, &author, 1000, &ttl, &parent, &root, 1).unwrap();
        let b = canonical_bytes(&content, &author, 1000, &ttl, &parent, &root, 2).unwrap();
        assert_ne!(a, b);
    }
}
