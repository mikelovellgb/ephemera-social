# Ephemera: Network & Protocol Specification

**Parent document:** [ARCHITECTURE.md](./ARCHITECTURE.md) Sections 4-5
**Version:** 1.0
**Date:** 2026-03-26
**Status:** Implementation-ready specification

---

## 1. Overview

Ephemera's network layer is a fully decentralized peer-to-peer system built on Iroh (QUIC transport) with Arti (Tor) layered on top for client privacy. Every client is a contributing node. There are no central servers. Content is disseminated via gossip (PlumTree/iroh-gossip) and retrieved via a custom TTL-aware Kademlia DHT.

This document specifies:
- Iroh integration and transport configuration
- The three privacy tiers (T1/T2/T3)
- Gossip protocol mechanics
- Custom DHT design
- NAT traversal strategy
- Wire protocol and protobuf definitions
- Node discovery and bootstrap
- Bandwidth and resource budgets

**Cross-references:**
- Identity keys used for node authentication: [01_identity_crypto.md](./01_identity_crypto.md) Section 2
- Content storage after receipt: [03_storage_data.md](./03_storage_data.md)
- Content types and validation: [04_social_features.md](./04_social_features.md)
- Rate limiting at network boundary: [05_moderation_safety.md](./05_moderation_safety.md)
- Crate structure: [07_rust_workspace.md](./07_rust_workspace.md)

---

## 2. Transport Foundation: Iroh

### 2.1 Why Iroh

Iroh is chosen over libp2p for these specific reasons:

| Criterion | Iroh | libp2p |
|-----------|------|--------|
| NAT traversal success rate | ~90% direct connections | ~70% direct connections |
| QUIC multipath | `noq` (start on relay, seamless upgrade to direct) | Not native |
| Gossip | Built-in `iroh-gossip` (PlumTree) | Requires external library |
| API complexity | Simple, Rust-native | Complex, multi-language abstractions |
| DHT | No built-in Kademlia (must build custom) | Built-in Kademlia |
| Language | Pure Rust | Rust implementation available, but API designed for multi-language |

**Fallback plan:** If the custom DHT exceeds 4 weeks of engineering effort, fall back to libp2p. All networking is abstracted behind traits to make this swap feasible within a 1-week migration window.

### 2.2 Iroh Configuration

```rust
// ephemera-transport/src/config.rs

pub struct TransportConfig {
    /// Maximum concurrent QUIC connections
    pub max_connections: usize,            // Default: 256
    /// QUIC idle timeout
    pub idle_timeout: Duration,            // Default: 60 seconds
    /// Maximum QUIC stream count per connection
    pub max_streams: u64,                  // Default: 100
    /// UDP socket receive buffer size
    pub recv_buffer_size: usize,           // Default: 2 MiB
    /// UDP socket send buffer size
    pub send_buffer_size: usize,           // Default: 2 MiB
    /// Keep-alive interval for QUIC connections
    pub keep_alive_interval: Duration,     // Default: 15 seconds
    /// Whether to enable QUIC multipath (noq)
    pub enable_multipath: bool,            // Default: true
    /// Community relay nodes (not n0's defaults)
    pub relay_nodes: Vec<RelayNodeConfig>,
    /// Bootstrap node addresses
    pub bootstrap_nodes: Vec<NodeAddr>,
}

pub struct RelayNodeConfig {
    pub node_id: PublicKey,
    pub url: Url,                          // e.g., "https://relay1.ephemera.social"
    pub region: String,                    // Geographic hint for latency optimization
}
```

### 2.3 Connection Lifecycle

```
1. App starts
2. Load or generate node identity (Ed25519 key pair)
3. Initialize Iroh endpoint with node identity
4. Connect to relay nodes (QUIC)
5. Attempt hole-punch to known peers (from cached peer list)
6. Join gossip topics via relay
7. Begin DHT bootstrap (iterative FIND_NODE starting from bootstrap nodes)
8. Background: maintain connection pool, re-connect dropped peers
9. App shutdown: graceful disconnect (QUIC CONNECTION_CLOSE)
```

State machine for individual peer connections:

```
[Disconnected] --> [Connecting] --> [Relayed] --> [Direct]
                       |                |              |
                       v                v              v
                  [Failed]         [Relayed]     [Disconnected]
```

- **Connecting:** QUIC handshake in progress (via relay or direct).
- **Relayed:** Connected through a relay node. Functional but higher latency.
- **Direct:** Hole-punch succeeded. UDP packets flow directly between peers.
- Iroh's `noq` transparently upgrades Relayed to Direct when possible.

---

## 3. Privacy Transport Tiers

All three tiers implement the `AnonTransport` trait:

```rust
// ephemera-transport/src/traits.rs

#[async_trait]
pub trait AnonTransport: Send + Sync + 'static {
    /// Send a message to a destination node via anonymous routing
    async fn send(&self, destination: &NodeAddr, payload: &[u8]) -> Result<(), TransportError>;

    /// Receive messages addressed to us
    async fn recv(&self) -> Result<(Vec<u8>, NodeAddr), TransportError>;

    /// Get the current privacy tier
    fn tier(&self) -> PrivacyTier;

    /// Estimated round-trip latency for this tier
    fn estimated_latency(&self) -> Duration;

    /// Whether this tier is currently available/connected
    fn is_available(&self) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyTier {
    /// T1: Loopix-style mixnet (maximum privacy, 2-30s latency)
    Stealth,
    /// T2: Arti/Tor 3-hop onion routing (strong privacy, 200-800ms latency)
    Private,
    /// T3: Iroh single-hop relay (moderate privacy, 50-200ms latency)
    Fast,
}
```

### 3.1 T3: Fast (Iroh Single-Hop Relay)

**Architecture:**
```
[Client] --QUIC--> [Ephemera Relay] --QUIC--> [Destination Node]
```

- Client connects to one of the community-operated Ephemera relay nodes.
- Relay masks the client's IP from the destination. The destination sees only the relay's IP.
- Relay sees the client's IP but NOT the content (end-to-end encrypted at the application layer).
- Single trust point: the relay operator knows the client's IP and the destination node.

**When to use:**
- Default for low-latency needs (real-time feed updates, typing indicators).
- Mobile clients (bandwidth constraints make T2 impractical).
- Explicitly opted-in by users who prioritize speed over maximum privacy.

**Message padding:** All T3 messages padded to the nearest 256-byte boundary:
```rust
fn pad_message(msg: &[u8]) -> Vec<u8> {
    let padded_len = ((msg.len() + 2) / 256 + 1) * 256;  // +2 for length prefix
    let mut padded = Vec::with_capacity(padded_len);
    padded.extend_from_slice(&(msg.len() as u16).to_be_bytes());
    padded.extend_from_slice(msg);
    padded.resize(padded_len, 0x00);  // Zero padding
    padded
}
```

### 3.2 T2: Private (Arti/Tor 3-Hop)

**Architecture:**
```
[Client] --Tor Circuit (3 hops)--> [Exit Node] --QUIC--> [Ephemera Backbone]
```

- Default privacy tier for all users.
- Uses `arti-client` to establish 3-hop Tor circuits.
- The exit node connects to the Ephemera backbone via Iroh QUIC.
- Client's IP is hidden from all Ephemera nodes. Tor entry guard knows the client's IP but not the destination.
- Leverages Tor's existing anonymity set (millions of users), which is critical at launch.

**Configuration:**
```rust
// ephemera-transport/src/t2_private.rs

pub struct T2Config {
    /// Circuit rotation interval
    pub circuit_rotation: Duration,      // Default: 24 hours
    /// Number of pre-built circuits to maintain
    pub circuit_pool_size: usize,        // Default: 3
    /// Arti state directory (cached relay descriptors, etc.)
    pub arti_state_dir: PathBuf,
    /// Whether to use bridges (for censored networks)
    pub use_bridges: bool,               // Default: false
    /// Bridge addresses (if use_bridges is true)
    pub bridges: Vec<String>,
}
```

**Circuit management:**
1. On startup, build 3 Tor circuits in parallel (takes 5-15 seconds).
2. Route all user-facing operations through Tor circuits (round-robin for load distribution).
3. Rotate circuits every 24 hours (Tor best practice).
4. If a circuit fails, rebuild immediately from the pool.
5. Pre-fetch Tor relay descriptors during idle time.

**Client-to-client connections are FORBIDDEN:**
- All communication flows through relays or onion circuits.
- Iroh's direct peer-to-peer QUIC connections are used ONLY between backbone infrastructure nodes (relays, DHT nodes, storage nodes).
- A client NEVER reveals its IP to another client.

### 3.3 T1: Stealth (Mixnet) -- Deferred from PoC

**Architecture (design only, not implemented in PoC):**
```
[Client] --Tor--> [Mix 1] --> [Mix 2] --> [Mix 3] --> [Ephemera Backbone]
```

- Loopix-style mixnet with Sphinx packet format.
- Constant-rate cover traffic (2 KB/s padding).
- Poisson-distributed delays (lambda=500ms mean) at each mix node.
- All packets normalized to fixed 2 KB size.
- Real messages are indistinguishable from cover traffic.
- Requires sufficient user base for a meaningful anonymity set. Deferred until post-PoC.

**Cover traffic budget:** 2 KB/s * 86400 s/day = ~173 MB/day. Impractical on mobile. T1 is desktop-only.

---

## 4. Gossip Protocol

### 4.1 Overview

Content dissemination uses `iroh-gossip`, which implements the PlumTree protocol (Push-Lazy-Push Multicast Tree). This is a hybrid push/pull epidemic broadcast protocol optimized for low-latency delivery with logarithmic message complexity.

### 4.2 Topic Structure

Gossip is organized into topics. Each topic is identified by a 32-byte BLAKE3 hash.

```rust
pub struct GossipTopic([u8; 32]);

impl GossipTopic {
    /// Global public feed (all public posts)
    pub fn public_feed() -> Self {
        Self(blake3::hash(b"ephemera-topic-public-feed-v1").into())
    }

    /// Per-pseudonym feed (posts from a specific author)
    pub fn author_feed(author: &IdentityKey) -> Self {
        Self(blake3::hash(&[b"ephemera-topic-author-v1", author.as_bytes()].concat()).into())
    }

    /// Topic room (user-created topic)
    pub fn topic_room(room_id: &TopicRoomId) -> Self {
        Self(blake3::hash(&[b"ephemera-topic-room-v1", room_id.as_bytes()].concat()).into())
    }

    /// Moderation events (reports, votes, tombstones)
    pub fn moderation() -> Self {
        Self(blake3::hash(b"ephemera-topic-moderation-v1").into())
    }

    /// CRDT sync for social graph events in a neighborhood
    pub fn crdt_sync(neighborhood: &NeighborhoodId) -> Self {
        Self(blake3::hash(&[b"ephemera-topic-crdt-v1", neighborhood.as_bytes()].concat()).into())
    }

    /// Bloom filter updates (CSAM hash database)
    pub fn bloom_updates() -> Self {
        Self(blake3::hash(b"ephemera-topic-bloom-v1").into())
    }
}
```

### 4.3 Gossip Subscription Privacy

**Critical invariant:** A client MUST NOT subscribe to gossip topics from its own node identity. This would link the node identity to the user's interests.

**Solution:** Gossip subscriptions are routed through the anonymous transport tier:
1. Client connects to a relay/exit node via T2 or T3.
2. The relay acts as a gossip proxy: it subscribes to the requested topics on behalf of the client.
3. Messages matching the subscribed topics are forwarded to the client through the anonymous circuit.
4. The relay sees which topics are subscribed but cannot link them to the client's real IP (in T2 mode).

### 4.4 Message Deduplication

Gossip protocols naturally produce duplicate messages. Deduplication uses a Bloom filter with time-based rotation:

```rust
pub struct MessageDedup {
    current: BloomFilter,      // Current epoch (5-minute windows)
    previous: BloomFilter,     // Previous epoch (for overlap)
    epoch_start: Instant,
    epoch_duration: Duration,  // 5 minutes
}

impl MessageDedup {
    pub fn is_duplicate(&mut self, content_hash: &ContentHash) -> bool {
        if self.current.contains(content_hash) || self.previous.contains(content_hash) {
            return true;
        }
        self.current.insert(content_hash);
        self.maybe_rotate();
        false
    }

    fn maybe_rotate(&mut self) {
        if self.epoch_start.elapsed() >= self.epoch_duration {
            self.previous = std::mem::replace(&mut self.current, BloomFilter::new());
            self.epoch_start = Instant::now();
        }
    }
}
```

### 4.5 Gossip Configuration

```rust
pub struct GossipConfig {
    /// Maximum message size in gossip (larger content uses chunked transfer)
    pub max_message_size: usize,           // Default: 64 KiB
    /// Fanout for eager push (PlumTree parameter)
    pub eager_push_peers: usize,           // Default: 3
    /// Fanout for lazy push (PlumTree parameter)
    pub lazy_push_peers: usize,            // Default: 6
    /// Interval for lazy push IHave messages
    pub ihave_interval: Duration,          // Default: 500ms
    /// Timeout before switching from lazy to eager for a message
    pub lazy_timeout: Duration,            // Default: 2 seconds
    /// Maximum topics a node can subscribe to
    pub max_subscriptions: usize,          // Default: 100
    /// Deduplication bloom filter expected insertions
    pub dedup_bloom_capacity: usize,       // Default: 100_000
    /// Deduplication bloom false positive rate
    pub dedup_bloom_fp_rate: f64,          // Default: 0.001
}
```

---

## 5. Custom TTL-Aware Kademlia DHT

### 5.1 Design Rationale

Iroh does not include a Kademlia DHT. A custom implementation is required for point lookups (prekey bundles, user profiles, hashtag indexes, specific content by ID). The DHT is TTL-aware: records expire and are not re-replicated after their TTL elapses.

### 5.2 DHT Parameters

```rust
pub struct DhtConfig {
    /// Kademlia k parameter (bucket size)
    pub k: usize,                          // Default: 20
    /// Kademlia alpha parameter (parallel lookups)
    pub alpha: usize,                      // Default: 3
    /// Replication factor for stored records
    pub replication: usize,                // Default: 5
    /// Routing table refresh interval
    pub refresh_interval: Duration,        // Default: 60 seconds
    /// Record republish interval
    pub republish_interval: Duration,      // Default: 1 hour
    /// Maximum records stored per node
    pub max_records: usize,                // Default: 100_000
    /// Maximum record value size
    pub max_record_size: usize,            // Default: 8 KiB
    /// Stale contact timeout
    pub stale_timeout: Duration,           // Default: 5 minutes
}
```

### 5.3 Record Types

```rust
pub enum DhtRecordType {
    /// Prekey bundle for X3DH key exchange
    PrekeyBundle,
    /// User profile (display name, bio, avatar CID)
    Profile,
    /// Hashtag index entry (maps tag -> list of content hashes)
    HashtagIndex,
    /// Content lookup (maps content hash -> list of nodes holding it)
    ContentProvider,
    /// Relay node advertisement
    RelayAdvertisement,
}

pub struct DhtRecord {
    pub key: DhtKey,                     // BLAKE3 hash (32 bytes)
    pub record_type: DhtRecordType,
    pub value: Vec<u8>,                  // Type-specific payload (max 8 KiB)
    pub publisher: IdentityKey,          // Pseudonym that published (for prekeys/profiles)
                                         // or node ID (for provider records)
    pub timestamp: u64,                  // HLC timestamp
    pub ttl_seconds: u32,               // Max 30 days
    pub signature: Signature,            // Ed25519 signature over all above fields
}
```

### 5.4 DHT Key Derivation

```rust
impl DhtKey {
    /// Key for a pseudonym's prekey bundle
    pub fn prekey(pseudonym: &IdentityKey) -> Self {
        Self(blake3::hash(&[b"dht-prekey-v1\x00", pseudonym.as_bytes()].concat()).into())
    }

    /// Key for a pseudonym's profile
    pub fn profile(pseudonym: &IdentityKey) -> Self {
        Self(blake3::hash(&[b"dht-profile-v1\x00", pseudonym.as_bytes()].concat()).into())
    }

    /// Key for a hashtag index
    pub fn hashtag(tag: &str) -> Self {
        let normalized = tag.to_lowercase();
        Self(blake3::hash(&[b"dht-hashtag-v1\x00", normalized.as_bytes()].concat()).into())
    }

    /// Key for content provider records
    pub fn content(content_hash: &ContentHash) -> Self {
        Self(blake3::hash(&[b"dht-content-v1\x00", content_hash.as_bytes()].concat()).into())
    }

    /// Key for relay node advertisements
    pub fn relay(node_id: &PublicKey) -> Self {
        Self(blake3::hash(&[b"dht-relay-v1\x00", node_id.as_bytes()].concat()).into())
    }
}
```

### 5.5 DHT Operations

**FIND_NODE(target_id):**
1. Select alpha (3) closest nodes from the routing table.
2. Send FIND_NODE RPC to each in parallel.
3. Each node responds with up to k (20) closest nodes it knows.
4. Add new nodes to the candidate set, select next alpha closest unqueried nodes.
5. Repeat until no closer nodes are discovered.
6. Return the k closest nodes to the target.

**FIND_VALUE(key):**
1. Same iterative process as FIND_NODE.
2. If a node holds the record for the key, it returns the record instead of closer nodes.
3. On success, cache the record at the closest node that did NOT have it (Kademlia caching).
4. Validate TTL: reject records where `timestamp + ttl_seconds < now`.

**STORE(key, record):**
1. Find the k closest nodes to the key via FIND_NODE.
2. Send STORE RPC to all k nodes.
3. Each node validates: signature, TTL, record size, publisher authority.
4. Nodes store the record and set a local expiry timer.
5. Re-publish every `republish_interval` (1 hour) until the record's TTL expires.

**TTL enforcement in the DHT:**
- Records carry `ttl_seconds` and `timestamp`.
- Nodes reject records where TTL > 30 days.
- Nodes reject records where `timestamp + ttl_seconds < now` (already expired).
- Background sweep every 60 seconds drops expired records.
- Records are NOT re-replicated after their TTL expires.

### 5.6 DHT Protobuf Messages

```protobuf
message DhtRequest {
    oneof request {
        FindNodeRequest find_node = 1;
        FindValueRequest find_value = 2;
        StoreRequest store = 3;
        PingRequest ping = 4;
    }
}

message DhtResponse {
    oneof response {
        FindNodeResponse find_node = 1;
        FindValueResponse find_value = 2;
        StoreResponse store = 3;
        PingResponse ping = 4;
    }
}

message FindNodeRequest {
    bytes target_id = 1;       // 32-byte node ID to find
    bytes requester_id = 2;    // Requester's node ID
}

message FindNodeResponse {
    repeated NodeInfo closer_nodes = 1;
}

message FindValueRequest {
    bytes key = 1;             // 32-byte DHT key
    bytes requester_id = 2;
}

message FindValueResponse {
    oneof result {
        DhtRecord record = 1;             // Found the record
        FindNodeResponse closer_nodes = 2; // Didn't find it, here are closer nodes
    }
}

message StoreRequest {
    DhtRecord record = 1;
}

message StoreResponse {
    bool success = 1;
    string error = 2;
}

message PingRequest {
    bytes sender_id = 1;
}

message PingResponse {
    bytes responder_id = 1;
}

message NodeInfo {
    bytes node_id = 1;        // 32-byte Ed25519 public key
    repeated bytes addrs = 2; // Multiaddr-encoded addresses
    uint64 last_seen = 3;     // Unix timestamp
}

message DhtRecord {
    bytes key = 1;
    uint32 record_type = 2;
    bytes value = 3;
    bytes publisher = 4;
    uint64 timestamp = 5;
    uint32 ttl_seconds = 6;
    bytes signature = 7;
}
```

### 5.7 Routing Table

The routing table uses standard Kademlia k-buckets:

```rust
pub struct RoutingTable {
    local_id: NodeId,                      // This node's ID
    buckets: [KBucket; 256],               // One bucket per XOR distance prefix bit
}

pub struct KBucket {
    nodes: BoundedVec<NodeEntry, K>,       // K=20, LRU eviction
    replacement_cache: BoundedVec<NodeEntry, K>,
}

pub struct NodeEntry {
    pub id: NodeId,
    pub addrs: Vec<SocketAddr>,
    pub last_seen: Instant,
    pub last_ping_rtt: Option<Duration>,
    pub failures: u32,
}
```

Eviction policy:
1. If the bucket is not full, insert the new node.
2. If the bucket is full, ping the least-recently-seen node.
3. If it responds, move it to the tail (most-recently-seen) and add the new node to the replacement cache.
4. If it does not respond within 5 seconds, evict it and insert the new node.

---

## 6. Node Discovery and Bootstrap

### 6.1 Bootstrap Nodes

5 hardcoded, geographically diverse VPS instances operated by the project team:

```rust
pub const BOOTSTRAP_NODES: &[&str] = &[
    // Format: "<node_id_hex>@<ip>:<port>"
    // Actual values set at deployment time
    "bootstrap-eu-west.ephemera.social:4433",
    "bootstrap-us-east.ephemera.social:4433",
    "bootstrap-us-west.ephemera.social:4433",
    "bootstrap-ap-southeast.ephemera.social:4433",
    "bootstrap-eu-east.ephemera.social:4433",
];
```

Bootstrap nodes are full DHT participants and relay nodes. They do NOT have special privileges beyond being the initial entry points.

### 6.2 Discovery Phases

**Phase 1: Bootstrap (seconds 0-10)**
1. Connect to bootstrap nodes via Iroh QUIC (through T2/T3 for client nodes).
2. Perform iterative FIND_NODE for own node ID (populates routing table with diverse nodes).
3. Cache all discovered peers locally in SQLite.

**Phase 2: Gossip-based peer exchange (seconds 10-30)**
1. Join the public feed gossip topic.
2. Receive peer announcements through gossip.
3. Connect to announced peers that are geographically close (latency-based selection).

**Phase 3: DHT-ready (seconds 30+)**
1. Routing table has enough entries for functional Kademlia lookups.
2. Begin serving DHT requests from other nodes.
3. Publish own records (prekey bundles, profiles) to the DHT.

**Phase 4: Steady state (minutes 2+)**
1. Maintain connection pool of 20-50 active peers.
2. Refresh routing table buckets every 60 seconds.
3. Re-publish records every hour.
4. Periodic peer exchange to discover new nodes.

### 6.3 Relay Discovery

Relay nodes register themselves in the DHT under a well-known key prefix:

```rust
let relay_key = DhtKey::relay(&relay_node_id);
let relay_record = DhtRecord {
    key: relay_key,
    record_type: DhtRecordType::RelayAdvertisement,
    value: bincode::serialize(&RelayAdvertisement {
        url: "https://relay1.ephemera.social".to_string(),
        region: "eu-west".to_string(),
        capacity: 1000,           // Max concurrent clients
        current_load: 150,        // Current connected clients
        bandwidth_mbps: 100,      // Available bandwidth
    })?,
    ttl_seconds: 3600,            // Re-publish hourly
    ..
};
```

Clients discover relays by querying the DHT for relay advertisements and selecting based on latency and load.

---

## 7. Wire Protocol Stack

### 7.1 Protocol Layers

```
+--------------------------------------------------+
| Application (Post, DM, SocialEvent, DhtOp, ...)  |
+--------------------------------------------------+
| Serialization: CBOR (content), Protobuf (wire)    |
+--------------------------------------------------+
| Compression: LZ4 (per-message, if > 128 bytes)   |
+--------------------------------------------------+
| Framing: u32 big-endian length prefix + payload   |
+--------------------------------------------------+
| Encryption: QUIC TLS 1.3 (node-to-node)          |
|             + Tor circuits (client-to-network)     |
+--------------------------------------------------+
| Transport: QUIC (via Iroh) over UDP               |
+--------------------------------------------------+
```

### 7.2 Framing

Every message on the wire is length-prefixed:

```
+------------------+------------------+
| Length (4 bytes)  | Payload          |
| u32 big-endian   | (Protobuf bytes) |
+------------------+------------------+
```

Maximum frame size: 1 MiB (1,048,576 bytes). Messages exceeding this are chunked at the application layer (media chunks are 256 KiB each).

### 7.3 Compression

LZ4 compression is applied per-message for payloads > 128 bytes:

```rust
pub fn compress_if_beneficial(payload: &[u8]) -> (bool, Vec<u8>) {
    if payload.len() <= 128 {
        return (false, payload.to_vec());
    }
    let compressed = lz4_flex::compress_prepend_size(payload);
    if compressed.len() < payload.len() {
        (true, compressed)
    } else {
        (false, payload.to_vec())  // Compression didn't help
    }
}
```

The `compressed` flag is carried in the Envelope header.

### 7.4 Protobuf Message Definitions

```protobuf
syntax = "proto3";
package ephemera.protocol.v1;

// Top-level envelope for all wire messages
message Envelope {
    uint32 version = 1;        // Protocol version (currently 1)
    MessageType type = 2;
    bytes author_id = 3;       // 32-byte pseudonym pubkey (public) or EMPTY (DMs)
    bytes node_id = 4;         // 32-byte forwarding node's network identity
    bytes payload = 5;         // Type-specific content
    bytes signature = 6;       // 64-byte Ed25519 sig (public) or ABSENT (DMs)
    uint64 timestamp = 7;      // Unix seconds (u64)
    uint64 ttl_seconds = 8;    // Content lifetime, max 2,592,000
    bool compressed = 9;       // Whether payload is LZ4 compressed
    bytes content_hash = 10;   // 33-byte ContentHash for deduplication
}

enum MessageType {
    POST = 0;
    DIRECT_MESSAGE = 1;
    SOCIAL_EVENT = 2;
    DHT_REQUEST = 3;
    DHT_RESPONSE = 4;
    PROFILE_UPDATE = 5;
    BLOOM_FILTER_UPDATE = 6;
    MODERATION_VOTE = 7;
    CONTENT_REPORT = 8;
    TOMBSTONE = 9;
    PREKEY_BUNDLE = 10;
    PEER_EXCHANGE = 11;
    CHUNK_REQUEST = 12;
    CHUNK_RESPONSE = 13;
    CAPABILITY_HANDSHAKE = 14;
}

// Capability negotiation (first message after QUIC connection)
message CapabilityHandshake {
    uint32 protocol_version = 1;
    repeated string capabilities = 2;   // e.g., ["gossip-v1", "dht-v1", "relay-v1"]
    string client_name = 3;             // e.g., "ephemera-desktop/0.1.0"
    uint64 uptime_seconds = 4;
    NodeRole role = 5;
}

enum NodeRole {
    LIGHT_NODE = 0;       // Default client: 500 MB storage, limited relay
    FULL_NODE = 1;        // User opt-in: full DHT routing, full storage
    RELAY_NODE = 2;       // Infrastructure: relays client traffic
    BOOTSTRAP_NODE = 3;   // Initial entry point
}

// Peer exchange message
message PeerExchange {
    repeated NodeInfo peers = 1;
    uint64 timestamp = 2;
}

// Tombstone for content deletion
message Tombstone {
    bytes content_hash = 1;    // ContentHash of the deleted item
    bytes author_id = 2;       // Pseudonym that authored the original content
    uint64 original_ttl = 3;   // Original TTL of the deleted content
    uint64 deleted_at = 4;     // Timestamp of deletion
    bytes signature = 5;       // Author's signature authorizing deletion
}

// Media chunk request/response
message ChunkRequest {
    bytes content_hash = 1;    // BLAKE3 hash of the requested chunk
    uint32 chunk_index = 2;    // Index within the media manifest
}

message ChunkResponse {
    bytes content_hash = 1;
    uint32 chunk_index = 2;
    bytes data = 3;            // Chunk payload (up to 256 KiB)
    uint64 total_chunks = 4;
}
```

### 7.5 Serialization Strategy

| Context | Format | Crate | Rationale |
|---------|--------|-------|-----------|
| Wire protocol (peer-to-peer) | Protobuf | `prost` | Schema evolution across protocol versions. Cross-language compatibility. |
| Content payloads (posts, profiles) | CBOR | `ciborium` | Self-describing binary. Schema-flexible. Used in other decentralized protocols (IPLD, WebAuthn). |
| On-disk storage | bincode | `bincode` 2.x | Fastest Rust-to-Rust serialization. Both writer and reader are Rust. |
| Client API (frontend <-> backend) | JSON | `serde_json` | JSON-RPC 2.0 standard. Human-readable for debugging. |

### 7.6 Timestamps

**Wire format:** Unix seconds as `u64`. This is the canonical representation on the wire and in the DHT.

**Internal format:** Hybrid Logical Clocks (HLC) via the `uhlc` crate. HLC provides causal ordering without requiring synchronized wall clocks.

```rust
use uhlc::HLC;

pub struct TimestampService {
    hlc: HLC,
}

impl TimestampService {
    pub fn now(&self) -> Timestamp {
        Timestamp(self.hlc.new_timestamp().as_u64())
    }

    pub fn update(&self, received: Timestamp) -> Timestamp {
        let received_hlc = uhlc::Timestamp::from_u64(received.0);
        self.hlc.update_with_timestamp(&received_hlc).ok();
        self.now()
    }
}
```

**Clock skew tolerance:** 5 minutes. Messages with timestamps more than 5 minutes in the future are rejected. Messages with timestamps in the past are accepted (they may be delayed messages from a slow network path).

---

## 8. Content Routing

### 8.1 Routing Strategy by Content Type

| Content Type | Dissemination Method | Retrieval Method |
|-------------|---------------------|-----------------|
| Public posts (text) | Gossip (public feed + author feed topics) | Local cache, gossip replay, DHT content lookup |
| Public posts (media) | Gossip (metadata only), then chunk transfer | Chunk request/response to provider nodes |
| Direct messages | Store-and-forward via relay dead drops | Recipient polls dead drop via anonymous transport |
| Profiles | DHT STORE | DHT FIND_VALUE |
| Prekey bundles | DHT STORE (via anonymous circuit) | DHT FIND_VALUE (via anonymous circuit) |
| Social graph events | Gossip (CRDT sync topic per neighborhood) | Anti-entropy Merkle tree sync |
| Tombstones | Gossip (high priority, public feed topic) | Anti-entropy sync |

### 8.2 Media Distribution (Swarming)

Large media is distributed using a BitTorrent-inspired swarming protocol:

1. Author publishes post with a media manifest (list of chunk CIDs and total size).
2. Post metadata propagates via gossip. Media chunks propagate separately.
3. Nodes that want the media request chunks from peers that have them.
4. **Rarest-first replication:** Nodes prefer to fetch and store the chunk with the fewest known providers.
5. **Target replication:** Each chunk should exist on at least 5 nodes. Re-replicate below 3.
6. **Downloaders become uploaders:** As soon as a node has a chunk, it can serve it to others.

```rust
pub struct MediaManifest {
    pub media_hash: ContentHash,        // BLAKE3 of the entire encrypted media blob
    pub chunks: Vec<ChunkInfo>,
    pub total_size: u64,
    pub encryption_nonce: [u8; 24],     // Nonce for the per-post symmetric key
}

pub struct ChunkInfo {
    pub index: u32,
    pub hash: ContentHash,              // BLAKE3 of this chunk
    pub size: u32,                      // Size in bytes (max 256 KiB)
}
```

---

## 9. NAT Traversal

### 9.1 Strategy

NAT traversal is handled by Iroh's built-in mechanisms. The priority order is:

1. **Direct UDP:** If both peers have public IPs, connect directly via QUIC.
2. **QUIC hole-punching:** Iroh coordinates hole-punch via relay signaling. ~90% success rate.
3. **Relay fallback (DERP):** If hole-punching fails, traffic flows through an Ephemera relay. The relay forwards encrypted QUIC packets without inspecting content.
4. **Multipath (noq):** Start on relay, transparently upgrade to direct when hole-punch succeeds. No application-layer interruption.

### 9.2 Ephemera Relay Nodes

Community-operated relay nodes (NOT n0's default relays):

```rust
pub struct RelayNode {
    pub node_id: PublicKey,
    pub public_url: Url,               // HTTPS endpoint for relay discovery
    pub quic_addr: SocketAddr,         // UDP address for QUIC relay traffic
    pub region: String,                // Geographic region
    pub max_clients: u32,              // Capacity limit
    pub bandwidth_limit_mbps: u32,     // Per-client bandwidth cap
}
```

Relay nodes:
- Do NOT inspect or log content (all traffic is E2E encrypted).
- Do NOT log client IP addresses beyond what's needed for active connections.
- Purge connection metadata on disconnect.
- Are registered in the DHT for discovery.
- Are run by the project team for the PoC. Community operation post-PoC.

### 9.3 Client Connectivity

Clients behind NAT connect as follows:

```
[Client behind NAT] --QUIC--> [Relay/Tor Entry] --QUIC--> [Backbone]
```

- The client maintains a persistent QUIC connection to its relay (T3) or Tor entry guard (T2).
- The relay/entry node handles backbone connectivity on behalf of the client.
- The client NEVER exposes a listening port or accepts inbound connections.

---

## 10. Bandwidth and Resource Budgets

### 10.1 Light Node (Default Client)

| Resource | Budget |
|----------|--------|
| Storage | 500 MB (content cache, local posts, keystore) |
| Bandwidth (upload) | 10% of available, max 1 Mbps |
| Bandwidth (download) | Unlimited (user's content) |
| CPU | 5% background (GC, gossip, DHT) |
| Connections | Max 50 active peers |
| DHT routing | NOT a DHT router (queries only) |
| Gossip fanout | Reduced: eager=2, lazy=3 |
| Anti-entropy | 25 KB/s budget |

### 10.2 Full Node (User Opt-In)

| Resource | Budget |
|----------|--------|
| Storage | Configurable, default 10 GB |
| Bandwidth (upload) | 50% of available, max 10 Mbps |
| Bandwidth (download) | Unlimited |
| CPU | 15% background |
| Connections | Max 256 active peers |
| DHT routing | Full Kademlia routing |
| Gossip fanout | Full: eager=3, lazy=6 |
| Anti-entropy | 50 KB/s budget |

### 10.3 Relay Node (Infrastructure)

| Resource | Budget |
|----------|--------|
| Storage | 50 GB (message relay buffer) |
| Bandwidth | Dedicated, 100 Mbps+ |
| CPU | Dedicated |
| Connections | Max 1000 clients |
| DHT routing | Full |
| Gossip | Full + proxy subscriptions for clients |

---

## 11. Error Handling and Resilience

### 11.1 Connection Failures

- **Relay unreachable:** Try next relay in the list (round-robin with latency-weighted selection).
- **Tor circuit broken:** Rebuild from the circuit pool (3 pre-built circuits).
- **All relays down:** Queue messages locally. Retry with exponential backoff (1s, 2s, 4s, ..., 60s max).
- **Bootstrap nodes unreachable:** Use cached peer list from SQLite. If empty, wait and retry.

### 11.2 Message Delivery Guarantees

- **Gossip (public posts):** Best-effort with anti-entropy repair. Messages may be delayed but are eventually delivered if the network is connected. Anti-entropy Merkle tree sync every 120 seconds fills gaps.
- **DHT (point lookups):** At-least-once delivery. Queries retry up to 3 times with 5-second timeout per attempt.
- **DMs (store-and-forward):** Relay holds messages for 14 days. Recipient polls via anonymous transport. If the relay goes down, the sender retries on a different relay.

### 11.3 Protocol Versioning

The `Envelope.version` field enables protocol evolution:

```rust
const CURRENT_PROTOCOL_VERSION: u32 = 1;
const MIN_SUPPORTED_VERSION: u32 = 1;

fn validate_version(version: u32) -> Result<(), ProtocolError> {
    if version < MIN_SUPPORTED_VERSION {
        return Err(ProtocolError::VersionTooOld(version));
    }
    if version > CURRENT_PROTOCOL_VERSION {
        // Forward compatibility: accept messages with unknown fields (protobuf default)
        // But warn if the version is significantly ahead
        if version > CURRENT_PROTOCOL_VERSION + 10 {
            return Err(ProtocolError::VersionTooNew(version));
        }
    }
    Ok(())
}
```

The `CapabilityHandshake` message (sent after QUIC connection establishment) negotiates supported features. Nodes only use features both sides support.

---

*This document is part of the Ephemera Architecture series. See [ARCHITECTURE.md](./ARCHITECTURE.md) for the master document.*
