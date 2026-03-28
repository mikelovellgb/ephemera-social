//! Block and mute management.
//!
//! Blocks and mutes are local-only operations — they are never propagated
//! to the network. Blocking hides all content and prevents connection
//! requests and DMs. Muting hides content from feeds but still allows
//! interactions.

use ephemera_types::{IdentityKey, Timestamp};
use serde::{Deserialize, Serialize};

use crate::SocialError;

/// A local block record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    /// The pseudonym doing the blocking.
    pub blocker: IdentityKey,
    /// The pseudonym being blocked.
    pub blocked: IdentityKey,
    /// When the block was created.
    pub created_at: Timestamp,
    /// Local-only note explaining the block (never transmitted).
    pub reason: Option<String>,
}

/// A local mute record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mute {
    /// The pseudonym doing the muting.
    pub muter: IdentityKey,
    /// The pseudonym being muted.
    pub muted: IdentityKey,
    /// When the mute was created.
    pub created_at: Timestamp,
    /// When the mute expires, or `None` for permanent.
    pub expires_at: Option<Timestamp>,
}

impl Mute {
    /// Whether this mute has expired at the given time.
    #[must_use]
    pub fn is_expired_at(&self, now: Timestamp) -> bool {
        match self.expires_at {
            Some(exp) => now.as_secs() > exp.as_secs(),
            None => false,
        }
    }

    /// Whether this mute is currently expired.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        self.is_expired_at(Timestamp::now())
    }
}

/// Service trait for block and mute operations.
///
/// All operations are local-only and synchronous in nature, but the
/// trait is async to match the rest of the service layer.
#[async_trait::async_trait]
pub trait BlockService: Send + Sync {
    /// Block a pseudonym. Silently no-ops if already blocked.
    async fn block(
        &self,
        blocker: &IdentityKey,
        blocked: &IdentityKey,
        reason: Option<&str>,
    ) -> Result<(), SocialError>;

    /// Unblock a pseudonym.
    async fn unblock(
        &self,
        blocker: &IdentityKey,
        blocked: &IdentityKey,
    ) -> Result<(), SocialError>;

    /// Mute a pseudonym, optionally with an expiry.
    async fn mute(
        &self,
        muter: &IdentityKey,
        muted: &IdentityKey,
        expires_at: Option<Timestamp>,
    ) -> Result<(), SocialError>;

    /// Unmute a pseudonym.
    async fn unmute(&self, muter: &IdentityKey, muted: &IdentityKey) -> Result<(), SocialError>;

    /// Check whether `target` is blocked by `checker`.
    async fn is_blocked(
        &self,
        checker: &IdentityKey,
        target: &IdentityKey,
    ) -> Result<bool, SocialError>;

    /// Check whether `target` is muted by `checker`.
    async fn is_muted(
        &self,
        checker: &IdentityKey,
        target: &IdentityKey,
    ) -> Result<bool, SocialError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alice() -> IdentityKey {
        IdentityKey::from_bytes([0x01; 32])
    }

    fn bob() -> IdentityKey {
        IdentityKey::from_bytes([0x02; 32])
    }

    #[test]
    fn permanent_mute_never_expires() {
        let mute = Mute {
            muter: alice(),
            muted: bob(),
            created_at: Timestamp::from_secs(1_000_000),
            expires_at: None,
        };
        assert!(!mute.is_expired_at(Timestamp::from_secs(u64::MAX - 1)));
    }

    #[test]
    fn temporary_mute_expires() {
        let mute = Mute {
            muter: alice(),
            muted: bob(),
            created_at: Timestamp::from_secs(1_000_000),
            expires_at: Some(Timestamp::from_secs(1_000_100)),
        };
        assert!(!mute.is_expired_at(Timestamp::from_secs(1_000_050)));
        assert!(mute.is_expired_at(Timestamp::from_secs(1_000_101)));
    }

    #[test]
    fn block_reason_is_optional() {
        let block = Block {
            blocker: alice(),
            blocked: bob(),
            created_at: Timestamp::now(),
            reason: None,
        };
        assert!(block.reason.is_none());

        let block_with_reason = Block {
            blocker: alice(),
            blocked: bob(),
            created_at: Timestamp::now(),
            reason: Some("spam".into()),
        };
        assert_eq!(block_with_reason.reason.as_deref(), Some("spam"));
    }
}
