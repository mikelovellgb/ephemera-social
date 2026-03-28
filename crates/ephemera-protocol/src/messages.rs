//! Message types carried inside a [`ProtocolEnvelope`](crate::ProtocolEnvelope).

use serde::{Deserialize, Serialize};

/// Discriminator for the payload inside a protocol envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u16)]
pub enum MessageType {
    /// A public post or reply.
    Post = 0,
    /// An end-to-end encrypted direct message.
    DirectMessage = 1,
    /// A social-graph event (follow, unfollow, block).
    SocialEvent = 2,
    /// A DHT request (find_node, find_value, store).
    DhtRequest = 3,
    /// A DHT response.
    DhtResponse = 4,
    /// Profile update announcement.
    ProfileUpdate = 5,
    /// Bloom filter update (CSAM hash database).
    BloomFilterUpdate = 6,
    /// Community moderation vote.
    ModerationVote = 7,
    /// Content report.
    ContentReport = 8,
    /// Tombstone (content deletion marker).
    Tombstone = 9,
    /// Prekey bundle publication.
    PrekeyBundle = 10,
    /// Peer exchange (share known peers).
    PeerExchange = 11,
    /// Media chunk request.
    ChunkRequest = 12,
    /// Media chunk response.
    ChunkResponse = 13,
    /// Capability handshake (first message after QUIC connect).
    CapabilityHandshake = 14,
    /// Gossip announcement (IHave / IWant / Graft / Prune).
    GossipControl = 15,
    /// Ping request.
    Ping = 100,
    /// Pong response.
    Pong = 101,
}

/// Role a node plays on the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NodeRole {
    /// Default client: limited storage and relay capacity.
    LightNode,
    /// Opt-in full participation: complete DHT routing and storage.
    FullNode,
    /// Infrastructure: relays client traffic.
    RelayNode,
    /// Bootstrap entry point.
    BootstrapNode,
}

/// Capability handshake -- first message after a QUIC connection opens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityHandshake {
    /// Protocol version supported by this node.
    pub protocol_version: u32,
    /// List of protocol capabilities (e.g. "gossip-v1", "dht-v1").
    pub capabilities: Vec<String>,
    /// Client software identifier (e.g. "ephemera-desktop/0.1.0").
    pub client_name: String,
    /// How long this node has been running, in seconds.
    pub uptime_seconds: u64,
    /// The role this node assumes on the network.
    pub role: NodeRole,
}

/// A connection request from one pseudonym to another.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionRequest {
    /// Pseudonym public key of the requester.
    pub from_pseudonym: [u8; 32],
    /// Pseudonym public key of the target.
    pub to_pseudonym: [u8; 32],
    /// Optional greeting message (plaintext, max 280 chars).
    pub greeting: Option<String>,
    /// Unix timestamp.
    pub timestamp: u64,
    /// Signature over the request fields (64 bytes).
    pub signature: Vec<u8>,
}

/// Response to a connection request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionResponse {
    /// The original request's content hash (33 bytes).
    pub request_hash: Vec<u8>,
    /// Whether the connection was accepted.
    pub accepted: bool,
    /// Unix timestamp.
    pub timestamp: u64,
    /// Signature over the response fields (64 bytes).
    pub signature: Vec<u8>,
}

/// Gossip announcement for PlumTree protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GossipAnnounce {
    /// Eager push: full message content.
    EagerPush {
        topic: [u8; 32],
        payload: Vec<u8>,
        content_hash: [u8; 32],
    },
    /// Lazy push: just the content hash (IHave).
    IHave {
        topic: [u8; 32],
        hashes: Vec<[u8; 32]>,
    },
    /// Request for content we learned about via IHave.
    IWant { hashes: Vec<[u8; 32]> },
    /// Graft: promote a lazy link to eager.
    Graft { topic: [u8; 32] },
    /// Prune: demote an eager link to lazy.
    Prune { topic: [u8; 32] },
}

/// DHT put request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DhtPut {
    /// 32-byte DHT key.
    pub key: [u8; 32],
    /// Record type discriminator.
    pub record_type: u32,
    /// Serialized record value (max 8 KiB).
    pub value: Vec<u8>,
    /// Publisher's public key.
    pub publisher: [u8; 32],
    /// HLC timestamp.
    pub timestamp: u64,
    /// TTL in seconds (max 30 days).
    pub ttl_seconds: u32,
    /// Signature over all fields above (64 bytes).
    pub signature: Vec<u8>,
}

/// DHT get request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DhtGet {
    /// 32-byte DHT key to look up.
    pub key: [u8; 32],
    /// Requester's node ID.
    pub requester_id: [u8; 32],
}

/// A direct message envelope (sealed-sender, E2E encrypted).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectMessage {
    /// Dead-drop identifier (derived from shared secret).
    pub dead_drop_id: [u8; 32],
    /// Encrypted payload (Double Ratchet ciphertext).
    pub ciphertext: Vec<u8>,
    /// Ephemeral key for the current ratchet step.
    pub ephemeral_key: [u8; 32],
    /// Message counter for ordering.
    pub counter: u64,
    /// Previous chain length (for skipped messages).
    pub prev_chain_length: u64,
}

/// Content report from a user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportMessage {
    /// Content hash of the reported item (33 bytes).
    pub content_hash: Vec<u8>,
    /// Reason category.
    pub reason: ReportReason,
    /// Optional free-text description (max 500 chars).
    pub description: Option<String>,
    /// Reporter's pseudonym public key.
    pub reporter: [u8; 32],
    /// Unix timestamp.
    pub timestamp: u64,
    /// Signature over the report fields (64 bytes).
    pub signature: Vec<u8>,
}

/// Reason categories for content reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReportReason {
    /// Illegal content (CSAM, terrorism, etc.).
    Illegal,
    /// Spam or commercial abuse.
    Spam,
    /// Harassment or targeted abuse.
    Harassment,
    /// Misleading content.
    Misinformation,
    /// Other.
    Other,
}

/// Ping message for liveness checking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingMessage {
    /// Sender's node ID.
    pub sender_id: [u8; 32],
    /// Nonce for matching request to response.
    pub nonce: u64,
}

/// Pong response to a ping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PongMessage {
    /// Responder's node ID.
    pub responder_id: [u8; 32],
    /// Echoed nonce from the ping.
    pub nonce: u64,
}

/// Peer exchange message -- share known peers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerExchange {
    /// List of known peers with their addresses.
    pub peers: Vec<PeerInfo>,
    /// Unix timestamp.
    pub timestamp: u64,
}

/// Information about a peer for exchange purposes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    /// 32-byte node ID.
    pub node_id: [u8; 32],
    /// Socket addresses (serialized).
    pub addresses: Vec<String>,
    /// When this peer was last seen (Unix seconds).
    pub last_seen: u64,
}

/// Tombstone for content deletion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TombstoneMessage {
    /// Content hash of the deleted item (33 bytes).
    pub content_hash: Vec<u8>,
    /// Author's pseudonym public key.
    pub author_id: [u8; 32],
    /// Original TTL of the deleted content.
    pub original_ttl: u64,
    /// When the deletion occurred.
    pub deleted_at: u64,
    /// Author's signature authorizing deletion (64 bytes).
    pub signature: Vec<u8>,
}

/// Post message carried on the gossip network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostMessage {
    /// Author's pseudonym public key.
    pub author_id: [u8; 32],
    /// CBOR-encoded content body.
    pub content: Vec<u8>,
    /// Content hash (33 bytes).
    pub content_hash: Vec<u8>,
    /// Unix timestamp.
    pub timestamp: u64,
    /// TTL in seconds.
    pub ttl_seconds: u32,
    /// Audience visibility.
    pub audience: u8,
    /// Signature over all fields (64 bytes).
    pub signature: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_type_discriminator() {
        assert_eq!(MessageType::Post as u16, 0);
        assert_eq!(MessageType::Ping as u16, 100);
        assert_ne!(MessageType::DhtRequest, MessageType::DhtResponse);
    }

    #[test]
    fn capability_handshake_serde() {
        let hs = CapabilityHandshake {
            protocol_version: 1,
            capabilities: vec!["gossip-v1".into(), "dht-v1".into()],
            client_name: "ephemera-desktop/0.1.0".into(),
            uptime_seconds: 3600,
            role: NodeRole::LightNode,
        };
        let json = serde_json::to_string(&hs).unwrap();
        let recovered: CapabilityHandshake = serde_json::from_str(&json).unwrap();
        assert_eq!(recovered.protocol_version, 1);
        assert_eq!(recovered.capabilities.len(), 2);
    }
}
