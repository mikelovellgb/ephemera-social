//! Fluent builder for constructing signed posts.
//!
//! The [`PostBuilder`] accumulates content, TTL, and reply metadata,
//! then signs and hashes the post in [`build`](PostBuilder::build).

use ephemera_crypto::SigningKeyPair;
use ephemera_types::{ContentId, Timestamp, Ttl};

use crate::canonical::canonical_bytes;
use crate::content::PostContent;
use crate::post::{Post, PowProof};
use crate::PostError;

/// Builder for constructing a [`Post`] with a fluent API.
///
/// # Example
///
/// ```ignore
/// let post = PostBuilder::new()
///     .text("Hello, world!")
///     .ttl(Ttl::from_secs(86400)?)
///     .build(&keypair)?;
/// ```
pub struct PostBuilder {
    text: Option<String>,
    photo: Option<ContentId>,
    caption: Option<String>,
    attachments: Vec<ContentId>,
    ttl: Option<Ttl>,
    parent: Option<ContentId>,
    root: Option<ContentId>,
    depth: u32,
    pow_proof: PowProof,
}

impl PostBuilder {
    /// Create a new empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self {
            text: None,
            photo: None,
            caption: None,
            attachments: Vec::new(),
            ttl: None,
            parent: None,
            root: None,
            depth: 0,
            pow_proof: PowProof::empty_internal(),
        }
    }

    /// Set the text body.
    #[must_use]
    pub fn text(mut self, body: &str) -> Self {
        self.text = Some(body.to_string());
        self
    }

    /// Attach a single photo by its blob content hash.
    #[must_use]
    pub fn photo(mut self, blob_id: ContentId) -> Self {
        self.photo = Some(blob_id);
        self
    }

    /// Set a caption for a photo post.
    #[must_use]
    pub fn caption(mut self, cap: &str) -> Self {
        self.caption = Some(cap.to_string());
        self
    }

    /// Add a media attachment by its blob content hash.
    #[must_use]
    pub fn attachment(mut self, blob_id: ContentId) -> Self {
        self.attachments.push(blob_id);
        self
    }

    /// Set the time-to-live.
    #[must_use]
    pub fn ttl(mut self, ttl: Ttl) -> Self {
        self.ttl = Some(ttl);
        self
    }

    /// Mark this post as a reply to the given parent.
    #[must_use]
    pub fn reply_to(mut self, parent: ContentId, root: ContentId, depth: u32) -> Self {
        self.parent = Some(parent);
        self.root = Some(root);
        self.depth = depth;
        self
    }

    /// Set a pre-computed proof-of-work stamp.
    #[must_use]
    pub fn pow(mut self, proof: PowProof) -> Self {
        self.pow_proof = proof;
        self
    }

    /// Build and sign the post.
    ///
    /// # Errors
    ///
    /// Returns [`PostError`] if the builder state is invalid (e.g. no
    /// content, too many attachments, missing TTL).
    pub fn build(self, identity: &SigningKeyPair) -> Result<Post, PostError> {
        let content = self.resolve_content()?;
        let ttl = self
            .ttl
            .unwrap_or_else(|| Ttl::from_secs(86400).expect("default TTL valid"));
        let created_at = Timestamp::now();
        let author = identity.public_key();

        // Serialize for hashing and signing (includes all identity-defining fields).
        let canonical = canonical_bytes(
            &content,
            &author,
            created_at.as_secs(),
            &ttl,
            &self.parent,
            &self.root,
            self.depth,
        )?;

        let hash = blake3::hash(&canonical);
        let id = ContentId::from_digest(*hash.as_bytes());
        let signature = identity.sign(&canonical);

        Ok(Post {
            id,
            author,
            content,
            created_at,
            ttl,
            signature,
            pow_proof: self.pow_proof,
            parent: self.parent,
            root: self.root,
            depth: self.depth,
        })
    }

    /// Resolve the content from the various builder fields.
    fn resolve_content(&self) -> Result<PostContent, PostError> {
        match (&self.text, &self.photo, self.attachments.is_empty()) {
            // Text-only post.
            (Some(body), None, true) => Ok(PostContent::Text { body: body.clone() }),
            // Photo-only post (with optional caption).
            (None, Some(blob_id), true) => Ok(PostContent::Photo {
                blob_id: blob_id.clone(),
                caption: self.caption.clone(),
            }),
            // Text + photo: treat as combined with single attachment.
            (Some(text), Some(blob_id), true) => Ok(PostContent::Combined {
                text: text.clone(),
                attachments: vec![blob_id.clone()],
            }),
            // Text + multiple attachments.
            (Some(text), None, false) => Ok(PostContent::Combined {
                text: text.clone(),
                attachments: self.attachments.clone(),
            }),
            // No content at all.
            (None, None, true) => Err(PostError::Build("post has no content".into())),
            // Conflicting: photo + extra attachments.
            _ => Err(PostError::Build(
                "cannot combine photo() and attachment() — use attachment() only".into(),
            )),
        }
    }
}

impl Default for PostBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_keypair() -> SigningKeyPair {
        SigningKeyPair::generate()
    }

    #[test]
    fn build_text_post() {
        let kp = test_keypair();
        let post = PostBuilder::new()
            .text("hello ephemera")
            .ttl(Ttl::from_secs(3600).unwrap())
            .build(&kp)
            .unwrap();

        assert!(post.is_top_level());
        assert!(matches!(post.content, PostContent::Text { .. }));
    }

    #[test]
    fn build_photo_post() {
        let kp = test_keypair();
        let blob = ContentId::from_digest([0xBB; 32]);
        let post = PostBuilder::new()
            .photo(blob)
            .caption("sunset")
            .ttl(Ttl::from_secs(86400).unwrap())
            .build(&kp)
            .unwrap();

        assert!(matches!(post.content, PostContent::Photo { .. }));
    }

    #[test]
    fn build_combined_post() {
        let kp = test_keypair();
        let blob = ContentId::from_digest([0xCC; 32]);
        let post = PostBuilder::new()
            .text("check this out")
            .attachment(blob)
            .ttl(Ttl::from_secs(86400).unwrap())
            .build(&kp)
            .unwrap();

        assert!(matches!(post.content, PostContent::Combined { .. }));
    }

    #[test]
    fn build_reply() {
        let kp = test_keypair();
        let parent = ContentId::from_digest([0xDD; 32]);
        let root = ContentId::from_digest([0xEE; 32]);
        let post = PostBuilder::new()
            .text("great post!")
            .reply_to(parent.clone(), root, 1)
            .ttl(Ttl::from_secs(3600).unwrap())
            .build(&kp)
            .unwrap();

        assert!(post.is_reply());
        assert_eq!(post.parent.as_ref(), Some(&parent));
        assert_eq!(post.depth, 1);
    }

    #[test]
    fn empty_builder_fails() {
        let kp = test_keypair();
        let result = PostBuilder::new()
            .ttl(Ttl::from_secs(3600).unwrap())
            .build(&kp);
        assert!(result.is_err());
    }
}
