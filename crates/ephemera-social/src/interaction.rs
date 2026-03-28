//! Reactions and replies.
//!
//! Reactions are ephemeral feedback on posts (heart, laugh, fire, sad,
//! thinking). Replies are independent posts with a parent link.

use ephemera_types::{ContentId, IdentityKey, Signature, Timestamp};
use serde::{Deserialize, Serialize};

use crate::SocialError;

/// The five allowed reaction emoji types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReactionEmoji {
    /// Heart / love.
    Heart,
    /// Laugh / funny.
    Laugh,
    /// Fire / awesome.
    Fire,
    /// Sad / sympathy.
    Sad,
    /// Thinking / hmm.
    Thinking,
}

impl ReactionEmoji {
    /// All available reaction types.
    pub const ALL: [ReactionEmoji; 5] = [
        Self::Heart,
        Self::Laugh,
        Self::Fire,
        Self::Sad,
        Self::Thinking,
    ];
}

impl std::fmt::Display for ReactionEmoji {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Heart => write!(f, "heart"),
            Self::Laugh => write!(f, "laugh"),
            Self::Fire => write!(f, "fire"),
            Self::Sad => write!(f, "sad"),
            Self::Thinking => write!(f, "thinking"),
        }
    }
}

impl std::str::FromStr for ReactionEmoji {
    type Err = SocialError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "heart" => Ok(Self::Heart),
            "laugh" => Ok(Self::Laugh),
            "fire" => Ok(Self::Fire),
            "sad" => Ok(Self::Sad),
            "thinking" => Ok(Self::Thinking),
            other => Err(SocialError::Validation(format!(
                "unknown emoji type: {other}"
            ))),
        }
    }
}

/// Whether a reaction event adds or removes a reaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReactionAction {
    /// Add the reaction.
    Add,
    /// Remove a previously-added reaction.
    Remove,
}

/// A signed reaction event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reaction {
    /// The post being reacted to.
    pub target: ContentId,
    /// The pseudonym reacting.
    pub reactor: IdentityKey,
    /// Which emoji.
    pub emoji: ReactionEmoji,
    /// Add or remove.
    pub action: ReactionAction,
    /// When the reaction occurred.
    pub timestamp: Timestamp,
    /// Ed25519 signature over the canonical reaction bytes.
    pub signature: Signature,
}

/// Summary of reactions on a post, grouped by emoji type.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReactionSummary {
    /// Count per emoji type.
    pub counts: Vec<(ReactionEmoji, u32)>,
    /// The emoji the querying user has reacted with, if any.
    pub my_emoji: Option<ReactionEmoji>,
}

impl ReactionSummary {
    /// Total number of reactions across all types.
    #[must_use]
    pub fn total(&self) -> u32 {
        self.counts.iter().map(|(_, c)| c).sum()
    }

    /// Count for a specific emoji.
    #[must_use]
    pub fn count_for(&self, emoji: ReactionEmoji) -> u32 {
        self.counts
            .iter()
            .find(|(e, _)| *e == emoji)
            .map_or(0, |(_, c)| *c)
    }
}

/// A reply is just a post with a parent link. This struct captures the
/// metadata needed to display a reply in context without loading the
/// full post.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reply {
    /// Content hash of the reply post.
    pub content_hash: ContentId,
    /// Author of the reply.
    pub author: IdentityKey,
    /// Content hash of the parent post.
    pub parent: ContentId,
    /// When the reply was created.
    pub created_at: Timestamp,
}

/// Service trait for reactions and replies.
#[async_trait::async_trait]
pub trait InteractionService: Send + Sync {
    /// Add or remove a reaction on a post.
    async fn react_to_post(
        &self,
        reactor: &IdentityKey,
        target: &ContentId,
        emoji: ReactionEmoji,
        action: ReactionAction,
    ) -> Result<(), SocialError>;

    /// Get the reaction summary for a post.
    async fn list_reactions(&self, target: &ContentId) -> Result<ReactionSummary, SocialError>;

    /// List replies to a post, ordered by creation time.
    async fn list_replies(&self, parent: &ContentId, limit: u32)
        -> Result<Vec<Reply>, SocialError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reaction_emoji_display() {
        assert_eq!(ReactionEmoji::Heart.to_string(), "heart");
        assert_eq!(ReactionEmoji::Thinking.to_string(), "thinking");
    }

    #[test]
    fn reaction_summary_counts() {
        let summary = ReactionSummary {
            counts: vec![(ReactionEmoji::Heart, 5), (ReactionEmoji::Fire, 3)],
            my_emoji: None,
        };
        assert_eq!(summary.total(), 8);
        assert_eq!(summary.count_for(ReactionEmoji::Heart), 5);
        assert_eq!(summary.count_for(ReactionEmoji::Sad), 0);
    }

    #[test]
    fn all_emoji_types() {
        assert_eq!(ReactionEmoji::ALL.len(), 5);
    }
}
