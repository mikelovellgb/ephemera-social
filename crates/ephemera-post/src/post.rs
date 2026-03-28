//! Core post data structure.
//!
//! A [`Post`] is the fundamental content unit in Ephemera. It carries
//! authored content with a cryptographic signature, proof-of-work stamp,
//! and a bounded time-to-live.

use ephemera_types::{ContentId, IdentityKey, Signature, Timestamp, Ttl};
use serde::{Deserialize, Serialize};

use crate::content::PostContent;

/// Proof-of-work stamp that demonstrates computational effort.
///
/// The exact algorithm (Equihash) is implemented in `ephemera-abuse`.
/// Here we store the serialized proof bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowProof(Vec<u8>);

impl PowProof {
    /// Create a `PowProof` from raw proof bytes.
    #[must_use]
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Return the raw proof bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Create an empty (placeholder) proof for testing.
    ///
    /// This is intentionally restricted to crate-internal and test use only.
    /// Production code must supply a real proof via [`PowProof::from_bytes`].
    #[must_use]
    #[cfg(any(test, feature = "test-utils"))]
    pub fn empty() -> Self {
        Self(Vec::new())
    }

    /// Create an empty (placeholder) proof (crate-internal only).
    #[must_use]
    pub(crate) fn empty_internal() -> Self {
        Self(Vec::new())
    }

    /// Whether this proof is empty (no actual PoW was performed).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A signed, ephemeral post.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Post {
    /// Content-addressed identifier (BLAKE3 hash of the CBOR-encoded body).
    pub id: ContentId,
    /// The pseudonym that authored this post.
    pub author: IdentityKey,
    /// The content payload (text, photo, or combined).
    pub content: PostContent,
    /// When the post was created (wall-clock seconds).
    pub created_at: Timestamp,
    /// How long the post should live.
    pub ttl: Ttl,
    /// Ed25519 signature over the canonical post bytes.
    pub signature: Signature,
    /// Proof-of-work stamp.
    pub pow_proof: PowProof,
    /// Parent post hash if this is a reply.
    pub parent: Option<ContentId>,
    /// Root of the reply thread (equal to `id` for top-level posts).
    pub root: Option<ContentId>,
    /// Depth in the reply thread (0 for top-level).
    pub depth: u32,
}

impl Post {
    /// Whether this post is a reply to another post.
    #[must_use]
    pub fn is_reply(&self) -> bool {
        self.parent.is_some()
    }

    /// Whether this post is a top-level post (not a reply).
    #[must_use]
    pub fn is_top_level(&self) -> bool {
        self.parent.is_none()
    }

    /// Compute the expiry timestamp for this post.
    #[must_use]
    pub fn expires_at(&self) -> Timestamp {
        Timestamp::from_secs(self.created_at.as_secs() + self.ttl.as_secs())
    }

    /// Check whether this post has expired at the given point in time.
    #[must_use]
    pub fn is_expired_at(&self, now: Timestamp) -> bool {
        now.as_secs() > self.expires_at().as_secs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::PostContent;

    fn make_post(parent: Option<ContentId>) -> Post {
        Post {
            id: ContentId::from_digest([0x01; 32]),
            author: IdentityKey::from_bytes([0x02; 32]),
            content: PostContent::Text {
                body: "hello world".into(),
            },
            created_at: Timestamp::from_secs(1_700_000_000),
            ttl: Ttl::from_secs(86400).unwrap(),
            signature: Signature::from_bytes([0x03; 64]),
            pow_proof: PowProof::empty(),
            parent,
            root: None,
            depth: 0,
        }
    }

    #[test]
    fn top_level_post() {
        let post = make_post(None);
        assert!(post.is_top_level());
        assert!(!post.is_reply());
    }

    #[test]
    fn reply_post() {
        let parent_hash = ContentId::from_digest([0xFF; 32]);
        let post = make_post(Some(parent_hash));
        assert!(post.is_reply());
        assert!(!post.is_top_level());
    }

    #[test]
    fn expiry() {
        let post = make_post(None);
        let expected = 1_700_000_000 + 86400;
        assert_eq!(post.expires_at().as_secs(), expected);
        assert!(!post.is_expired_at(Timestamp::from_secs(1_700_000_000)));
        assert!(post.is_expired_at(Timestamp::from_secs(expected + 1)));
    }
}
