//! Content classification types.
//!
//! Defines the kinds of content that can exist on the Ephemera network,
//! along with metadata structures for content items.

use crate::id::ContentId;
use crate::identity::IdentityKey;
use crate::timestamp::HlcTimestamp;
use crate::ttl::Ttl;
use serde::{Deserialize, Serialize};

/// The kind of content carried by a post or message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ContentKind {
    /// Plain text (constrained Markdown).
    Text,
    /// A photo (WebP after processing).
    Photo,
    /// A short video (H.264 after processing).
    Video,
    /// An audio clip (Opus after processing).
    Audio,
    /// A location check-in (no coordinates stored, only place name).
    CheckIn,
    /// A reply to another post.
    Reply,
    /// A reaction (emoji) to another post.
    Reaction,
    /// A repost / boost of another post.
    Repost,
    /// A profile update event.
    ProfileUpdate,
    /// A connection request or acceptance.
    ConnectionEvent,
}

impl ContentKind {
    /// Whether this content kind can carry media attachments.
    #[must_use]
    pub fn supports_media(self) -> bool {
        matches!(
            self,
            Self::Text | Self::Photo | Self::Video | Self::Audio | Self::Reply
        )
    }

    /// Whether this content kind is a social-graph event rather than user content.
    #[must_use]
    pub fn is_social_event(self) -> bool {
        matches!(self, Self::ProfileUpdate | Self::ConnectionEvent)
    }
}

/// Audience visibility for a piece of content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Audience {
    /// Visible to anyone with the epoch key (public feed).
    Public,
    /// Visible only to the author's direct connections.
    Connections,
    /// Visible only to explicitly listed pseudonyms.
    Listed,
    /// Visible only to the intended recipient (DM).
    Direct,
}

/// Content sensitivity labels for content warnings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SensitivityLabel {
    /// No sensitivity label.
    None,
    /// Nudity or sexual content.
    Nsfw,
    /// Graphic violence.
    Violence,
    /// Potential spoiler for media.
    Spoiler,
    /// Other user-defined sensitivity.
    Other,
}

/// Metadata describing a piece of content, stored alongside the ciphertext.
///
/// This is the cleartext metadata that lives in SQLite for indexing.
/// The actual content body is encrypted and stored separately in fjall.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentMetadata {
    /// The content-addressed identifier.
    pub id: ContentId,
    /// The pseudonym that authored this content.
    pub author: IdentityKey,
    /// What kind of content this is.
    pub kind: ContentKind,
    /// When this content was created (HLC for causal ordering).
    pub created_at: HlcTimestamp,
    /// How long this content should live.
    pub ttl: Ttl,
    /// Who can see this content.
    pub audience: Audience,
    /// Content warning label, if any.
    pub sensitivity: SensitivityLabel,
    /// If this is a reply, the parent content ID.
    pub parent_id: Option<ContentId>,
    /// Size of the encrypted content blob in bytes.
    pub blob_size: u64,
    /// Whether this content has media attachments.
    pub has_media: bool,
}

/// Classification of media types in the processing pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaType {
    /// Still image (will be converted to WebP).
    Image,
    /// Video clip (will be transcoded to H.264).
    Video,
    /// Audio clip (will be transcoded to Opus).
    Audio,
}

/// Quality tier for media encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Quality {
    /// Thumbnail / preview (very small).
    Thumbnail,
    /// Low quality (bandwidth-constrained).
    Low,
    /// Standard quality (default).
    Standard,
    /// High quality (Wi-Fi or wired connection).
    High,
}

impl Quality {
    /// Maximum width in pixels for image resizing at this quality tier.
    #[must_use]
    pub fn max_image_width(self) -> u32 {
        match self {
            Self::Thumbnail => 150,
            Self::Low => 480,
            Self::Standard => 1080,
            Self::High => 2048,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_kind_media_support() {
        assert!(ContentKind::Text.supports_media());
        assert!(ContentKind::Photo.supports_media());
        assert!(!ContentKind::Reaction.supports_media());
        assert!(!ContentKind::ProfileUpdate.supports_media());
    }

    #[test]
    fn content_kind_social_event() {
        assert!(ContentKind::ProfileUpdate.is_social_event());
        assert!(ContentKind::ConnectionEvent.is_social_event());
        assert!(!ContentKind::Text.is_social_event());
    }

    #[test]
    fn quality_image_widths() {
        assert!(Quality::Thumbnail.max_image_width() < Quality::Low.max_image_width());
        assert!(Quality::Low.max_image_width() < Quality::Standard.max_image_width());
        assert!(Quality::Standard.max_image_width() < Quality::High.max_image_width());
    }

    #[test]
    fn audience_serde_round_trip() {
        let json = serde_json::to_string(&Audience::Connections).unwrap();
        let recovered: Audience = serde_json::from_str(&json).unwrap();
        assert_eq!(recovered, Audience::Connections);
    }
}
