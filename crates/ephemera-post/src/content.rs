//! Post content types.
//!
//! A post can carry text, a single photo, or a combination of text with
//! media attachments. The [`PostContent`] enum captures these variants.

use ephemera_types::ContentId;
use serde::{Deserialize, Serialize};

/// Maximum length of a text post body in grapheme clusters.
pub const MAX_TEXT_GRAPHEMES: usize = 2_000;

/// Maximum wire size of a text post body in bytes.
pub const MAX_TEXT_BYTES: usize = 16_384;

/// Maximum number of media attachments per post.
pub const MAX_ATTACHMENTS: usize = 4;

/// The content payload of a post.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PostContent {
    /// A text-only post.
    Text {
        /// The post body (constrained Markdown subset).
        body: String,
    },
    /// A single photo with an optional caption.
    Photo {
        /// Content hash of the processed image blob.
        blob_id: ContentId,
        /// Optional caption text.
        caption: Option<String>,
    },
    /// Text combined with one or more media attachments.
    Combined {
        /// The post body text.
        text: String,
        /// Content hashes of the attached media blobs (max 4).
        attachments: Vec<ContentId>,
    },
}

impl PostContent {
    /// Return the text body if present, regardless of variant.
    #[must_use]
    pub fn text_body(&self) -> Option<&str> {
        match self {
            Self::Text { body } => Some(body),
            Self::Photo { caption, .. } => caption.as_deref(),
            Self::Combined { text, .. } => Some(text),
        }
    }

    /// Return references to all attachment content hashes.
    #[must_use]
    pub fn attachment_ids(&self) -> Vec<&ContentId> {
        match self {
            Self::Text { .. } => vec![],
            Self::Photo { blob_id, .. } => vec![blob_id],
            Self::Combined { attachments, .. } => attachments.iter().collect(),
        }
    }

    /// Count the number of media attachments.
    #[must_use]
    pub fn attachment_count(&self) -> usize {
        match self {
            Self::Text { .. } => 0,
            Self::Photo { .. } => 1,
            Self::Combined { attachments, .. } => attachments.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_hash() -> ContentId {
        ContentId::from_digest([0xAA; 32])
    }

    #[test]
    fn text_body_extraction() {
        let text = PostContent::Text {
            body: "hello".into(),
        };
        assert_eq!(text.text_body(), Some("hello"));

        let photo = PostContent::Photo {
            blob_id: dummy_hash(),
            caption: Some("sunset".into()),
        };
        assert_eq!(photo.text_body(), Some("sunset"));

        let photo_no_cap = PostContent::Photo {
            blob_id: dummy_hash(),
            caption: None,
        };
        assert_eq!(photo_no_cap.text_body(), None);

        let combined = PostContent::Combined {
            text: "look!".into(),
            attachments: vec![dummy_hash()],
        };
        assert_eq!(combined.text_body(), Some("look!"));
    }

    #[test]
    fn attachment_counts() {
        let text = PostContent::Text { body: "hi".into() };
        assert_eq!(text.attachment_count(), 0);

        let photo = PostContent::Photo {
            blob_id: dummy_hash(),
            caption: None,
        };
        assert_eq!(photo.attachment_count(), 1);

        let combined = PostContent::Combined {
            text: "x".into(),
            attachments: vec![dummy_hash(), dummy_hash(), dummy_hash()],
        };
        assert_eq!(combined.attachment_count(), 3);
    }
}
