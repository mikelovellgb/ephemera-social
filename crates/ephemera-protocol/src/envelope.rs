//! Protocol envelope -- the top-level framing for all wire messages.
//!
//! Every message between Ephemera nodes is wrapped in a [`ProtocolEnvelope`].
//! The envelope carries routing metadata (sender, hop count, TTL), a type
//! discriminator, the serialized payload, and an optional Ed25519 signature.

use crate::messages::MessageType;
use crate::version::ProtocolVersion;
use serde::{Deserialize, Serialize};

/// Top-level wire envelope for all Ephemera protocol messages.
///
/// The envelope is length-prefixed on the wire (u32 big-endian) and then
/// serialized as a single unit. The inner `payload_bytes` contain the
/// type-specific message body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolEnvelope {
    /// Protocol version for compatibility checking.
    pub version: ProtocolVersion,
    /// 32-byte Ed25519 public key of the originating node.
    pub sender_node_id: [u8; 32],
    /// The type of the inner payload.
    pub payload_type: MessageType,
    /// Serialized payload bytes (type-specific content).
    pub payload_bytes: Vec<u8>,
    /// Optional Ed25519 signature over the envelope fields (64 bytes).
    /// Absent for forwarded messages and sealed-sender DMs.
    pub signature: Option<Vec<u8>>,
    /// Number of hops this message has traversed.
    pub hop_count: u8,
    /// Maximum number of hops before the message is dropped.
    pub ttl: u8,
    /// Unix timestamp (seconds) when the message was created.
    pub timestamp: u64,
    /// Whether the payload is LZ4-compressed.
    pub compressed: bool,
    /// Optional 33-byte content hash for deduplication.
    pub content_hash: Option<Vec<u8>>,
}

impl ProtocolEnvelope {
    /// Create a new envelope with sensible defaults.
    pub fn new(
        sender_node_id: [u8; 32],
        payload_type: MessageType,
        payload_bytes: Vec<u8>,
    ) -> Self {
        Self {
            version: ProtocolVersion::current(),
            sender_node_id,
            payload_type,
            payload_bytes,
            signature: None,
            hop_count: 0,
            ttl: Self::DEFAULT_TTL,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before Unix epoch")
                .as_secs(),
            compressed: false,
            content_hash: None,
        }
    }

    /// Default hop TTL for gossip messages.
    pub const DEFAULT_TTL: u8 = 7;

    /// Maximum allowed hop count before a message is discarded.
    pub const MAX_TTL: u8 = 20;

    /// Check whether this envelope has exceeded its hop budget.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        self.hop_count >= self.ttl
    }

    /// Increment the hop count (called when forwarding).
    ///
    /// Returns `false` if the message would exceed its TTL.
    pub fn increment_hop(&mut self) -> bool {
        if self.hop_count >= self.ttl {
            return false;
        }
        self.hop_count += 1;
        true
    }

    /// Attach a signature to this envelope.
    pub fn set_signature(&mut self, sig: Vec<u8>) {
        self.signature = Some(sig);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_envelope() -> ProtocolEnvelope {
        ProtocolEnvelope::new([1u8; 32], MessageType::Ping, vec![0xDE, 0xAD])
    }

    #[test]
    fn new_envelope_defaults() {
        let env = test_envelope();
        assert_eq!(env.hop_count, 0);
        assert_eq!(env.ttl, ProtocolEnvelope::DEFAULT_TTL);
        assert!(!env.is_expired());
        assert!(env.signature.is_none());
        assert!(!env.compressed);
    }

    #[test]
    fn hop_increment_until_expired() {
        let mut env = test_envelope();
        for _ in 0..ProtocolEnvelope::DEFAULT_TTL {
            assert!(env.increment_hop());
        }
        assert!(env.is_expired());
        assert!(!env.increment_hop());
    }
}
