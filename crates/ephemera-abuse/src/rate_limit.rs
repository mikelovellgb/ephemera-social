//! Per-identity rate limiting using a token bucket algorithm.
//!
//! Each `(IdentityKey, ActionType)` pair gets its own token bucket.
//! Tokens refill at a configurable rate. An action is allowed if at
//! least one token is available.

use std::collections::HashMap;
use std::time::Instant;

use ephemera_types::IdentityKey;
use serde::{Deserialize, Serialize};

/// The type of action being rate-limited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActionType {
    /// Creating a new post.
    Post,
    /// Replying to a post.
    Reply,
    /// Adding a reaction.
    Reaction,
    /// Sending a DM to a friend.
    DirectMessageFriend,
    /// Sending a DM to a mutual contact.
    DirectMessageMutual,
    /// Sending a message request to a stranger.
    MessageRequest,
    /// Following someone.
    Follow,
    /// Sending a connection request.
    ConnectionRequest,
    /// Creating a new group.
    GroupCreate,
    /// Inviting someone to a group.
    GroupInvite,
    /// Sending a message to a group chat.
    GroupChatMessage,
}

/// Configuration for a single rate limit bucket.
#[derive(Debug, Clone, Copy)]
pub struct RateLimitConfig {
    /// Maximum burst capacity (tokens).
    pub capacity: u32,
    /// Tokens restored per second.
    pub refill_rate: f64,
}

impl RateLimitConfig {
    /// Default configurations per action type, derived from the spec.
    pub fn for_action(action: ActionType) -> Self {
        match action {
            ActionType::Post => Self {
                capacity: 10,
                refill_rate: 10.0 / 3600.0,
            },
            ActionType::Reply => Self {
                capacity: 5,
                refill_rate: 20.0 / 3600.0,
            },
            ActionType::Reaction => Self {
                capacity: 20,
                refill_rate: 100.0 / 3600.0,
            },
            ActionType::DirectMessageFriend => Self {
                capacity: 10,
                refill_rate: 60.0 / 3600.0,
            },
            ActionType::DirectMessageMutual => Self {
                capacity: 5,
                refill_rate: 30.0 / 3600.0,
            },
            ActionType::MessageRequest => Self {
                capacity: 1,
                refill_rate: 5.0 / 3600.0,
            },
            ActionType::Follow => Self {
                capacity: 10,
                refill_rate: 50.0 / 3600.0,
            },
            ActionType::ConnectionRequest => Self {
                capacity: 5,
                refill_rate: 20.0 / 3600.0,
            },
            ActionType::GroupCreate => Self {
                capacity: 3,
                refill_rate: 5.0 / 3600.0, // 5 groups per hour sustained
            },
            ActionType::GroupInvite => Self {
                capacity: 10,
                refill_rate: 20.0 / 3600.0, // 20 invites per hour sustained
            },
            ActionType::GroupChatMessage => Self {
                capacity: 15,
                refill_rate: 60.0 / 3600.0, // 60 messages per hour sustained
            },
        }
    }
}

/// A single token bucket.
struct TokenBucket {
    capacity: u32,
    tokens: f64,
    refill_rate: f64,
    last_refill: Instant,
}

impl TokenBucket {
    fn new(config: &RateLimitConfig) -> Self {
        Self {
            capacity: config.capacity,
            tokens: f64::from(config.capacity),
            refill_rate: config.refill_rate,
            last_refill: Instant::now(),
        }
    }

    /// Refill tokens based on elapsed time.
    fn refill(&mut self) {
        let elapsed = self.last_refill.elapsed().as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(f64::from(self.capacity));
        self.last_refill = Instant::now();
    }

    /// Try to consume one token. Returns `true` if allowed.
    fn try_consume(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Seconds until the next token is available.
    fn retry_after(&mut self) -> u64 {
        self.refill();
        if self.tokens >= 1.0 {
            return 0;
        }
        let deficit = 1.0 - self.tokens;
        (deficit / self.refill_rate).ceil() as u64
    }
}

/// Per-identity rate limiter.
///
/// Maintains a token bucket for each `(IdentityKey, ActionType)` pair.
pub struct RateLimiter {
    buckets: HashMap<(IdentityKey, ActionType), TokenBucket>,
}

impl RateLimiter {
    /// Create a new rate limiter with no active buckets.
    pub fn new() -> Self {
        Self {
            buckets: HashMap::new(),
        }
    }

    /// Check whether the given action is allowed for the identity.
    ///
    /// Consumes a token if allowed. Returns `Ok(())` on success, or an
    /// error with the retry-after duration if rate-limited.
    pub fn check(
        &mut self,
        identity: &IdentityKey,
        action: ActionType,
    ) -> Result<(), crate::AbuseError> {
        let config = RateLimitConfig::for_action(action);
        let bucket = self
            .buckets
            .entry((*identity, action))
            .or_insert_with(|| TokenBucket::new(&config));

        if bucket.try_consume() {
            Ok(())
        } else {
            let retry_after = bucket.retry_after();
            Err(crate::AbuseError::RateLimited {
                action: format!("{action:?}"),
                retry_after_secs: retry_after,
            })
        }
    }

    /// Peek at whether an action would be allowed without consuming a token.
    pub fn would_allow(&mut self, identity: &IdentityKey, action: ActionType) -> bool {
        let config = RateLimitConfig::for_action(action);
        let bucket = self
            .buckets
            .entry((*identity, action))
            .or_insert_with(|| TokenBucket::new(&config));

        bucket.refill();
        bucket.tokens >= 1.0
    }

    /// Remove all buckets for an identity (e.g., when a peer disconnects).
    pub fn clear_identity(&mut self, identity: &IdentityKey) {
        self.buckets.retain(|(id, _), _| id != identity);
    }

    /// Remove all buckets (reset).
    pub fn clear_all(&mut self) {
        self.buckets.clear();
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_identity() -> IdentityKey {
        IdentityKey::from_bytes([42; 32])
    }

    #[test]
    fn allows_within_burst() {
        let mut limiter = RateLimiter::new();
        let id = test_identity();

        // Message requests have capacity 1
        assert!(limiter.check(&id, ActionType::MessageRequest).is_ok());
        // Second one should be denied (capacity 1)
        assert!(limiter.check(&id, ActionType::MessageRequest).is_err());
    }

    #[test]
    fn allows_burst_up_to_capacity() {
        let mut limiter = RateLimiter::new();
        let id = test_identity();

        // Posts have capacity 10
        for _ in 0..10 {
            assert!(limiter.check(&id, ActionType::Post).is_ok());
        }
        // 11th should fail
        assert!(limiter.check(&id, ActionType::Post).is_err());
    }

    #[test]
    fn different_actions_have_separate_buckets() {
        let mut limiter = RateLimiter::new();
        let id = test_identity();

        // Exhaust message requests (capacity 1)
        assert!(limiter.check(&id, ActionType::MessageRequest).is_ok());
        assert!(limiter.check(&id, ActionType::MessageRequest).is_err());

        // Posts should still work (different bucket)
        assert!(limiter.check(&id, ActionType::Post).is_ok());
    }

    #[test]
    fn different_identities_have_separate_buckets() {
        let mut limiter = RateLimiter::new();
        let alice = IdentityKey::from_bytes([1; 32]);
        let bob = IdentityKey::from_bytes([2; 32]);

        // Exhaust Alice's message requests
        assert!(limiter.check(&alice, ActionType::MessageRequest).is_ok());
        assert!(limiter.check(&alice, ActionType::MessageRequest).is_err());

        // Bob should still be allowed
        assert!(limiter.check(&bob, ActionType::MessageRequest).is_ok());
    }

    #[test]
    fn would_allow_does_not_consume() {
        let mut limiter = RateLimiter::new();
        let id = test_identity();

        assert!(limiter.would_allow(&id, ActionType::MessageRequest));
        assert!(limiter.would_allow(&id, ActionType::MessageRequest));
        // Still allowed because we only peeked
        assert!(limiter.check(&id, ActionType::MessageRequest).is_ok());
    }

    #[test]
    fn clear_identity_removes_buckets() {
        let mut limiter = RateLimiter::new();
        let id = test_identity();

        // Exhaust message requests
        let _ = limiter.check(&id, ActionType::MessageRequest);
        let _ = limiter.check(&id, ActionType::MessageRequest);
        assert!(limiter.check(&id, ActionType::MessageRequest).is_err());

        // Clear and try again
        limiter.clear_identity(&id);
        assert!(limiter.check(&id, ActionType::MessageRequest).is_ok());
    }
}
