//! Social graph and feed assembly for the Ephemera platform.
//!
//! This crate provides the domain logic for connections (mutual social
//! graph), feed assembly, user profiles, reactions/replies,
//! block/mute management, groups, group chats, mentions,
//! and the human-readable handle system.

pub mod block;
pub mod connection;
pub mod feed;
pub mod group;
pub mod group_chat;
pub mod handle;
pub mod handle_validation;
pub mod interaction;
pub mod invite;
pub mod mention;
pub mod profile;
pub mod store;
pub mod topic_room;

pub use block::{Block, BlockService, Mute};
pub use connection::{Connection, ConnectionError, ConnectionService, ConnectionStatus};
pub use feed::{FeedCursor, FeedItem, FeedPage, FeedService, FeedType};
pub use group::{
    group_id_from_name, validate_group_description, validate_group_name, Group, GroupInfo,
    GroupMember, GroupRole, GroupVisibility,
};
pub use group_chat::{
    generate_chat_id, GroupChat, GroupChatMember, GroupChatMessage, GroupChatSummary,
};
pub use handle::{ConflictResult, Handle, HandleError, HandleRegistry, InsertOutcome};
pub use handle_validation::{
    calculate_pow_difficulty, validate_handle_format, verify_handle_pow, verify_handle_signature,
    HandleValidationError, PowDifficulty, HANDLE_REGISTRATION_COOLDOWN_SECS,
    HANDLE_RELEASE_COOLDOWN_SECS, HANDLE_TTL_SECS,
};
pub use interaction::{
    InteractionService, Reaction, ReactionAction, ReactionEmoji, ReactionSummary, Reply,
};
pub use invite::{
    generate_invite_link, generate_qr_payload, parse_invite_link, parse_qr_payload, InviteError,
};
pub use mention::{extract_mention_patterns, Mention};
pub use profile::{ProfileService, ProfileUpdate, UserProfile};
pub use store::{discover_feed, receive_connection_request, SqliteSocialServices};
pub use topic_room::{topic_id_from_name, TopicRoom, TopicRoomService, TopicSubscription};

/// Errors from social operations.
#[derive(Debug, thiserror::Error)]
pub enum SocialError {
    /// A validation constraint was violated.
    #[error("validation error: {0}")]
    Validation(String),

    /// The requested resource was not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// A connection-layer error.
    #[error("connection error: {0}")]
    Connection(#[from] ConnectionError),

    /// Storage backend error.
    #[error("storage error: {0}")]
    Storage(String),

    /// The caller lacks permission for the requested action.
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    /// The action was rate-limited.
    #[error("rate limited: {action} (retry after {retry_after_secs}s)")]
    RateLimited {
        /// The action that was denied.
        action: String,
        /// Seconds until the next attempt is allowed.
        retry_after_secs: u64,
    },
}
