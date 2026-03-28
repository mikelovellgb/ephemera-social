//! Feed assembly service.
//!
//! Builds chronological feeds from connected users' posts with
//! cursor-based pagination. No algorithmic ranking — pure
//! reverse-chronological order.

use ephemera_types::{ContentId, IdentityKey, Timestamp};
use serde::{Deserialize, Serialize};

use crate::SocialError;

/// A cursor for paginating through a feed.
///
/// Encodes the last-seen timestamp and content hash as a tiebreaker
/// for deterministic ordering when multiple posts share the same second.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedCursor {
    /// Timestamp of the last item on the previous page.
    pub created_at: Timestamp,
    /// Content hash of the last item (tiebreaker).
    pub content_hash: ContentId,
}

/// A single page of feed results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedPage {
    /// The feed items in this page (newest first).
    pub items: Vec<FeedItem>,
    /// Cursor for fetching the next page, `None` if this is the last page.
    pub next_cursor: Option<FeedCursor>,
    /// Whether there are more items after this page.
    pub has_more: bool,
}

/// Default number of items per feed page.
pub const DEFAULT_PAGE_SIZE: u32 = 50;

/// A single item in a feed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedItem {
    /// Content hash of the post.
    pub content_hash: ContentId,
    /// Author of the post.
    pub author: IdentityKey,
    /// When the post was created.
    pub created_at: Timestamp,
    /// Whether this is a reply.
    pub is_reply: bool,
    /// Parent content hash if this is a reply.
    pub parent: Option<ContentId>,
}

/// Type of feed to assemble.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedType {
    /// Posts from mutual connections (primary feed).
    Connections,
    /// Public posts from non-connected pseudonyms.
    Discover,
    /// All posts from a specific pseudonym.
    Profile,
}

/// Service trait for feed assembly.
///
/// Implementations query the metadata store to build feeds, applying
/// block/mute filters and pagination.
#[async_trait::async_trait]
pub trait FeedService: Send + Sync {
    /// Fetch a page of the connections feed for the given identity.
    async fn connections_feed(
        &self,
        identity: &IdentityKey,
        cursor: Option<&FeedCursor>,
        page_size: u32,
    ) -> Result<FeedPage, SocialError>;

    /// Fetch a page of a specific user's profile feed.
    async fn profile_feed(
        &self,
        viewer: &IdentityKey,
        profile_owner: &IdentityKey,
        cursor: Option<&FeedCursor>,
        page_size: u32,
    ) -> Result<FeedPage, SocialError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_cursor_serialization() {
        let cursor = FeedCursor {
            created_at: Timestamp::from_secs(1_700_000_000),
            content_hash: ContentId::from_digest([0xAA; 32]),
        };
        let json = serde_json::to_string(&cursor).unwrap();
        let decoded: FeedCursor = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.created_at, cursor.created_at);
        assert_eq!(decoded.content_hash, cursor.content_hash);
    }

    #[test]
    fn empty_feed_page() {
        let page = FeedPage {
            items: vec![],
            next_cursor: None,
            has_more: false,
        };
        assert!(page.items.is_empty());
        assert!(!page.has_more);
    }

    #[test]
    fn feed_item_reply() {
        let parent = ContentId::from_digest([0xBB; 32]);
        let item = FeedItem {
            content_hash: ContentId::from_digest([0xCC; 32]),
            author: IdentityKey::from_bytes([0x01; 32]),
            created_at: Timestamp::from_secs(1_700_000_000),
            is_reply: true,
            parent: Some(parent.clone()),
        };
        assert!(item.is_reply);
        assert_eq!(item.parent.as_ref(), Some(&parent));
    }
}
