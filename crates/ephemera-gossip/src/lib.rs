//! Topic-based gossip pub/sub for the Ephemera P2P network.
//!
//! Implements a PlumTree-style hybrid push/pull gossip protocol. Messages
//! are eagerly pushed to a subset of peers and lazily announced (via IHave)
//! to the rest. Peers that miss eager pushes can request missing content
//! via IWant.

pub mod fanout;
pub mod plumtree;
pub mod service;
pub mod topic;

pub use service::{
    EagerGossipService, GossipEnvelope, GossipWireMessage, IHaveMessage, IWantMessage,
};
pub use topic::{GossipTopic, TopicSubscription};

use async_trait::async_trait;

/// Trait for the gossip service, implemented by the PlumTree layer.
#[async_trait]
pub trait GossipService: Send + Sync {
    /// Subscribe to a gossip topic.
    async fn subscribe(&self, topic: &GossipTopic) -> Result<TopicSubscription, GossipError>;

    /// Unsubscribe from a gossip topic.
    async fn unsubscribe(&self, topic: &GossipTopic) -> Result<(), GossipError>;

    /// Publish a message to a gossip topic.
    async fn publish(&self, topic: &GossipTopic, payload: Vec<u8>) -> Result<(), GossipError>;

    /// Return the list of currently subscribed topics.
    fn subscriptions(&self) -> Vec<GossipTopic>;
}

/// Errors from the gossip subsystem.
#[derive(Debug, thiserror::Error)]
pub enum GossipError {
    /// The topic subscription limit has been reached.
    #[error("subscription limit reached: {current}/{max}")]
    SubscriptionLimitReached {
        /// Current number of subscriptions.
        current: usize,
        /// Maximum allowed.
        max: usize,
    },

    /// Not subscribed to the requested topic.
    #[error("not subscribed to topic {topic}")]
    NotSubscribed {
        /// The topic in question.
        topic: String,
    },

    /// A message exceeded the maximum gossip message size.
    #[error("message too large: {size} bytes (max {max})")]
    MessageTooLarge {
        /// Actual size.
        size: usize,
        /// Maximum allowed.
        max: usize,
    },

    /// Transport-level error during gossip.
    #[error("transport error: {0}")]
    Transport(#[from] ephemera_transport::TransportError),

    /// Internal channel closed.
    #[error("internal channel closed")]
    ChannelClosed,
}
