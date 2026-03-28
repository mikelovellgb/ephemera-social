//! Reputation scoring with time-based decay.
//!
//! Reputation starts at neutral and evolves based on the pseudonym's
//! behavior. Positive actions (posts, connections, accepted reactions)
//! increase it; negative actions (reports, rate limit violations) decrease
//! it. Score naturally decays over time (30-day half-life) unless
//! maintained through continued positive participation.
//!
//! Capabilities are gated by reputation level: new pseudonyms start in a
//! 7-day warming period with reduced privileges.

use serde::{Deserialize, Serialize};

/// An event that modifies reputation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ReputationEvent {
    /// Created a post (+1).
    PostCreated,
    /// Received a reaction on own content (+0.5).
    ReactionReceived,
    /// Formed a mutual connection (+5).
    ConnectionFormed,
    /// Content confirmed as abusive by moderation quorum (-10).
    ContentReported,
    /// Violated a rate limit (-2).
    RateLimitViolation,
    /// Community tombstone issued on own content (-50).
    CommunityTombstone,
}

impl ReputationEvent {
    /// The point value for this event (positive or negative).
    fn points(self) -> f64 {
        match self {
            Self::PostCreated => 1.0,
            Self::ReactionReceived => 0.5,
            Self::ConnectionFormed => 5.0,
            Self::ContentReported => -10.0,
            Self::RateLimitViolation => -2.0,
            Self::CommunityTombstone => -50.0,
        }
    }
}

/// A platform capability that requires a minimum reputation level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    /// Create text-only posts (reputation >= 0).
    CreateTextPost,
    /// Attach photos to posts (reputation >= 5).
    AttachPhotos,
    /// Send DMs to mutual contacts (reputation >= 0).
    DirectMessageMutual,
    /// Send DMs to strangers via message request (reputation >= 10).
    DirectMessageStranger,
    /// Create topic rooms (reputation >= 20).
    CreateTopicRoom,
    /// Participate in moderation votes (reputation >= 50, age >= 30d).
    ModerationVote,
    /// Full posting rate of 10/hour (reputation >= 10).
    FullPostRate,
}

impl Capability {
    /// Minimum reputation score required for this capability.
    fn required_reputation(self) -> f64 {
        match self {
            Self::CreateTextPost | Self::DirectMessageMutual => 0.0,
            Self::AttachPhotos => 5.0,
            Self::DirectMessageStranger | Self::FullPostRate => 10.0,
            Self::CreateTopicRoom => 20.0,
            Self::ModerationVote => 50.0,
        }
    }

    /// Minimum pseudonym age in days (0 means no age requirement).
    fn required_age_days(self) -> u32 {
        match self {
            Self::ModerationVote => 30,
            _ => 0,
        }
    }
}

/// Reputation score for a single pseudonym.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationScore {
    /// Accumulated positive points (before decay).
    positive_points: f64,
    /// Accumulated negative points (before decay, stored as positive value).
    negative_points: f64,
    /// Number of mutual connections.
    connection_count: u32,
    /// Days since the pseudonym was created.
    age_days: u32,
    /// Whether the warming period (7 days) is still active.
    warming_active: bool,
}

impl ReputationScore {
    /// Create a new reputation score for a freshly created pseudonym.
    pub fn new() -> Self {
        Self {
            positive_points: 0.0,
            negative_points: 0.0,
            connection_count: 0,
            age_days: 0,
            warming_active: true,
        }
    }

    /// Compute the current reputation value.
    ///
    /// Formula: `positive + sqrt(age) * 10 + sqrt(connections) * 5 - negative * 3`
    /// Result is clamped to a minimum of 0.
    pub fn value(&self) -> f64 {
        let age_bonus = (self.age_days as f64).sqrt() * 10.0;
        let conn_bonus = (self.connection_count as f64).sqrt() * 5.0;
        let score = self.positive_points + age_bonus + conn_bonus - self.negative_points * 3.0;
        score.max(0.0)
    }

    /// Record a reputation event.
    pub fn record_event(&mut self, event: ReputationEvent) {
        let points = event.points();
        if points >= 0.0 {
            self.positive_points += points;
        } else {
            self.negative_points += points.abs();
        }
    }

    /// Update the pseudonym age. Called daily or on access.
    pub fn set_age_days(&mut self, days: u32) {
        self.age_days = days;
        if days >= 7 {
            self.warming_active = false;
        }
    }

    /// Update the connection count.
    pub fn set_connection_count(&mut self, count: u32) {
        self.connection_count = count;
    }

    /// Apply time-based decay (30-day half-life).
    ///
    /// Call this periodically (e.g., daily) to decay accumulated points.
    /// The decay factor for `n` days is `0.5^(n/30)`.
    pub fn apply_decay(&mut self, elapsed_days: f64) {
        let factor = 0.5_f64.powf(elapsed_days / 30.0);
        self.positive_points *= factor;
        self.negative_points *= factor;
    }

    /// Check whether the pseudonym has a specific capability.
    pub fn has_capability(&self, capability: Capability) -> bool {
        if self.warming_active {
            return matches!(
                capability,
                Capability::CreateTextPost | Capability::DirectMessageMutual
            );
        }
        self.value() >= capability.required_reputation()
            && self.age_days >= capability.required_age_days()
    }

    /// Whether the 7-day warming period is still active.
    pub fn is_warming(&self) -> bool {
        self.warming_active
    }

    /// The current positive points (before decay).
    pub fn positive_points(&self) -> f64 {
        self.positive_points
    }

    /// The current negative points (before decay, as positive value).
    pub fn negative_points(&self) -> f64 {
        self.negative_points
    }

    /// Posts allowed per hour during the current state.
    pub fn posts_per_hour(&self) -> u32 {
        if self.warming_active {
            1
        } else if self.has_capability(Capability::FullPostRate) {
            10
        } else {
            5
        }
    }
}

impl Default for ReputationScore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_score_is_zero() {
        let score = ReputationScore::new();
        assert_eq!(score.value(), 0.0);
        assert!(score.is_warming());
    }

    #[test]
    fn positive_events_increase_score() {
        let mut score = ReputationScore::new();
        score.record_event(ReputationEvent::ConnectionFormed);
        assert!(score.value() > 0.0);
    }

    #[test]
    fn negative_events_decrease_score() {
        let mut score = ReputationScore::new();
        for _ in 0..20 {
            score.record_event(ReputationEvent::ConnectionFormed);
        }
        let before = score.value();
        score.record_event(ReputationEvent::ContentReported);
        assert!(score.value() < before);
    }

    #[test]
    fn score_never_goes_below_zero() {
        let mut score = ReputationScore::new();
        score.record_event(ReputationEvent::CommunityTombstone);
        assert_eq!(score.value(), 0.0);
    }

    #[test]
    fn age_increases_score() {
        let mut score = ReputationScore::new();
        score.set_age_days(100);
        assert!((score.value() - 100.0).abs() < 0.01);
    }

    #[test]
    fn warming_limits_capabilities() {
        let score = ReputationScore::new();
        assert!(score.has_capability(Capability::CreateTextPost));
        assert!(!score.has_capability(Capability::AttachPhotos));
        assert!(!score.has_capability(Capability::CreateTopicRoom));
    }

    #[test]
    fn warming_ends_after_seven_days() {
        let mut score = ReputationScore::new();
        score.set_age_days(7);
        assert!(!score.is_warming());
    }

    #[test]
    fn moderation_vote_requires_age() {
        let mut score = ReputationScore::new();
        score.set_age_days(10); // Past warming, but not 30 days
                                // Build enough reputation
        for _ in 0..20 {
            score.record_event(ReputationEvent::ConnectionFormed);
        }
        // Has reputation but not enough age
        assert!(!score.has_capability(Capability::ModerationVote));

        // Now meet the age requirement
        score.set_age_days(30);
        assert!(score.has_capability(Capability::ModerationVote));
    }

    #[test]
    fn decay_reduces_points() {
        let mut score = ReputationScore::new();
        for _ in 0..10 {
            score.record_event(ReputationEvent::ConnectionFormed);
        }
        let before = score.positive_points();
        score.apply_decay(30.0);
        let after = score.positive_points();
        assert!((after - before / 2.0).abs() < 0.01);
    }

    #[test]
    fn posts_per_hour_warming() {
        let score = ReputationScore::new();
        assert_eq!(score.posts_per_hour(), 1);
    }
}
