# Ephemera: Unified Architecture Document

**Version:** 1.0
**Date:** 2026-03-26
**Status:** Authoritative. This is the single source of truth for the Ephemera project.
**Derived from:** 7 specialist group reviews, 3 cross-group synthesis documents, 20 individual agent proposals.

---

## 1. Executive Summary

Ephemera is a decentralized, anonymous, ephemeral social media platform built in Rust. Every piece of content -- posts, messages, media, social graph events -- has a maximum lifetime of 30 days, enforced at the protocol level through both TTL-based garbage collection and cryptographic shredding via epoch key deletion. Users are identified solely by Ed25519 key pairs with no registration, no email, no phone number. Communication is end-to-end encrypted. The network operates as a peer-to-peer system with no central servers, no central authority, and no ability for any single entity to surveil, deanonymize, or decrypt user content.

**Core Principles:**

- **Ephemeral by design.** 30-day maximum TTL is a protocol-level invariant enforced at every layer -- type system, network, storage, and cryptography. Content does not persist.
- **Private by default.** End-to-end encryption, onion-routed transport, sealed-sender messaging, no metadata collection. Privacy is architectural, not policy.
- **Decentralized and resilient.** No central servers, no single points of failure, no kill switch. Every client is a contributing node.
- **Safety without surveillance.** Client-side CSAM hash matching, community moderation quorums, reputation-gated capabilities -- all without breaking encryption or deanonymizing users.
- **Honest about trade-offs.** The system is transparent about what it can and cannot do. It never claims to be "untraceable" or "censorship-proof."

---

## 2. System Overview

### 2.1 High-Level Architecture

```
+-----------------------------------------------------------+
|  User's Device (Tauri 2.x Desktop Application)            |
|                                                           |
|  +---------------------+   +----------------------------+ |
|  |   SolidJS Frontend  |   |   Rust Backend             | |
|  |                     |   |                            | |
|  |  - Feed view        |   |  ephemera-node (embedded)  | |
|  |  - Post composer    |   |  - Identity (Ed25519)      | |
|  |  - Connections mgmt |   |  - Pseudonym manager       | |
|  |  - Messaging        |   |  - Keystore (encrypted)    | |
|  |  - Settings         |   |  - AnonTransport layer     | |
|  |                     |   |    (T1/T2/T3 tiers)        | |
|  |  invoke() <-------->|   |  - Storage (fjall + FS)    | |
|  |  (JSON-RPC 2.0)     |   |  - Gossip + DHT            | |
|  |                     |   |  - Media pipeline           | |
|  |                     |   |  - CSAM hash check          | |
|  |                     |   |  - CRDT state manager       | |
|  +---------------------+   +----------------------------+ |
+----------------------------+------------------------------+
                             |
            +----------------+----------------+
            |                |                |
            v                v                v
    +-------------+  +-------------+  +--------------+
    | T3: Iroh    |  | T2: Arti    |  | T1: Mixnet   |
    | Single-hop  |  | 3-hop Tor   |  | Loopix/      |
    | Relay       |  | Circuit     |  | Sphinx       |
    | (50-200ms)  |  | (200-800ms) |  | (2-30s)      |
    +------+------+  +------+------+  +------+-------+
           |                |                |
           +--------+-------+--------+-------+
                    |                |
                    v                v
    +----------------------------------------------+
    |         Ephemera Node Backbone               |
    |     (Iroh QUIC, node-identity keys)          |
    |                                              |
    |  +----------+ +----------+ +---------------+ |
    |  | Kademlia | | Gossip   | | Content       | |
    |  | DHT      | | (iroh-   | | Storage       | |
    |  | (TTL-    | | gossip/  | | (fjall +      | |
    |  | aware)   | | PlumTree)| | ciphertext)   | |
    |  +----------+ +----------+ +---------------+ |
    |                                              |
    |  +------------------------------------------+|
    |  | Moderation: CRDTs, bloom filter,         ||
    |  | reputation counters, quorum votes        ||
    |  +------------------------------------------+|
    +----------------------------------------------+
```

### 2.2 Component Overview

| Component | Purpose | Key Technology |
|-----------|---------|----------------|
| **Client Frontend** | User interface | SolidJS in Tauri 2.x webview |
| **Client Backend** | Node logic, crypto, storage | Rust, embedded in Tauri process |
| **Identity** | Key generation, pseudonym derivation | Ed25519 + HKDF + BIP-39 |
| **Keystore** | Encrypted storage of all key material | Argon2id + XChaCha20-Poly1305 |
| **AnonTransport** | Privacy-tiered network transport | Arti (T2), Iroh relay (T3), Sphinx mixnet (T1) |
| **Storage** | Content and metadata persistence | fjall 3.1 (content ciphertext), SQLite (metadata) |
| **Gossip** | Broadcast dissemination | iroh-gossip / PlumTree |
| **DHT** | Point lookups (prekeys, profiles, hashtags) | Custom TTL-aware Kademlia over Iroh QUIC |
| **Media Pipeline** | Process, strip, compress, encrypt, chunk | WebP, H.264, Opus, BLAKE3 |
| **CRDT Manager** | Convergent state for reactions, graph, rep | OR-Set, G-Counter, ExpiringSet |
| **Moderation** | CSAM filter, reports, reputation | Perceptual hash bloom filter, quorum votes |

### 2.3 End-to-End Post Flow

**User creates a post:**

1. User types text and optionally attaches a photo in SolidJS UI.
2. Frontend calls `invoke("rpc", { method: "posts.create", params: { text, media, ttl } })`.
3. Rust backend receives JSON-RPC 2.0 request.
4. **Media pipeline** (if image attached): strip EXIF metadata, resize, compress to WebP, run CSAM perceptual hash check (silent reject on match), encrypt with per-post symmetric key, chunk into 256 KiB blocks, compute BLAKE3 hash per chunk.
5. **Post assembly**: sign post with pseudonym's Ed25519 key, assign HLC timestamp, compute BLAKE3 content ID, attach PoW stamp (Equihash, ~100ms base difficulty).
6. **Local storage**: write ciphertext to fjall, write metadata to SQLite, update local hashtag index.
7. **Return response** to frontend immediately (optimistic local-first).
8. **Network publication** (async): wrap post in onion-routed message via AnonTransport (default T2), exit node submits to gossip overlay, post propagates to peers.

**User receives a post from a peer:**

1. Gossip protocol delivers signed post envelope from the backbone.
2. Verify Ed25519 signature (batch verification if multiple posts arrive).
3. Validate TTL (reject if expired or > 30 days + 5 min skew tolerance).
4. If image attached: run CSAM hash check in background.
5. Check if author is in local social graph (connection or public content).
6. If reply: check parent dependency (causal delivery -- buffer up to 30s, then active fetch).
7. Store ciphertext in fjall, metadata in SQLite.
8. Emit event on internal event bus.
9. Tauri forwards event to SolidJS frontend, post appears in feed with animation.

**Content expiry:**

1. Background GC task runs every 60 seconds.
2. Scans SQLite for posts where `created_at + ttl < now`.
3. Deletes content from fjall, marks metadata as tombstone (retained 3x TTL for propagation).
4. Propagates deletion tombstone via gossip (high priority).
5. When epoch key is deleted (30 days after epoch end), enqueues sweep for any remaining ciphertext from that epoch.
6. Time-partitioned storage directories: entire day-directories deleted when all content expired.

---

## 3. Identity & Cryptography

### 3.1 Identity Model

Ephemera uses a **two-identity architecture** with strict separation:

**Node identity** (network layer):
- Per-installation Ed25519 key pair.
- Used for DHT participation, gossip membership, peer-to-peer authentication.
- Publicly visible on the network. Analogous to an IP address.
- NOT derived from user key material.

**Pseudonym identity** (user layer):
- Ed25519 key pairs derived via HKDF from a master key.
- Used for content signing, DM encryption, prekey publication, social graph operations.
- Multiple unlinkable pseudonyms per user, each derived deterministically: `pseudonym_key = HKDF-SHA256(master_key, "ephemera-pseudonym" || index)`.
- Published to the network only through anonymous transport circuits.
- Human-readable address: Bech32m encoding with `eph1` prefix.

**Key hierarchy:**
```
Master Key (Ed25519, cold storage)
  |
  +-- Device Key 1 (Ed25519, derived via HKDF)
  |     +-- Session Key (rotated per session)
  |     +-- Signing Subkey (for content)
  |
  +-- Device Key 2 ...
  |
  +-- Pseudonym A (Ed25519, derived via HKDF, index=0)
  +-- Pseudonym B (Ed25519, derived via HKDF, index=1)
  +-- ...
```

**Backup and recovery:**
- BIP-39 mnemonic (12 or 24 words) for human-memorizable master key backup.
- Shamir's Secret Sharing (3-of-5) for social recovery.
- Recovery phrase prompted after user's first post (not during onboarding, to reduce friction).

### 3.2 Cryptographic Primitives

| Primitive | Algorithm | Crate | Notes |
|-----------|-----------|-------|-------|
| Identity signatures | Ed25519 | `ed25519-dalek` | Public key IS the identity |
| Key exchange | X25519 | `x25519-dalek` | Diffie-Hellman for session keys |
| Symmetric encryption | XChaCha20-Poly1305 | `chacha20poly1305` | 192-bit nonce, safe for random generation |
| Key derivation | HKDF-SHA256 | `hkdf` | Pseudonym derivation, session keys |
| Password-based KDF | Argon2id | `argon2` | Keystore encryption, PoW validation |
| Content hashing | BLAKE3 | `blake3` | Content addressing, chunk verification |
| Secure erasure | zeroize + secrecy | `zeroize`, `secrecy` | Mandatory on all key types |
| Memory locking | mlock | OS-native | Best-effort (Linux/macOS native, Windows best-effort) |
| Post-quantum (future) | X25519 + ML-KEM-768 hybrid | TBD | Hybrid only, not standalone |

### 3.3 Encryption Schemes

**Public posts:** Encrypted under rotating epoch keys.
- Epoch key `EK_N` generated every 24 hours.
- Content encrypted: `XChaCha20-Poly1305(EK_N, nonce, plaintext)`.
- Epoch key retained for exactly 30 days after its epoch ends, then deleted (cryptographic shredding).
- Any node with the epoch key can decrypt public content. Epoch keys distributed via the social graph.

**Direct messages:** Double Ratchet (Signal Protocol variant).
- Initial key exchange: X3DH using prekey bundles published to DHT (via anonymous circuit).
- Each message encrypted under a unique message key derived from the ratchet state.
- Forward secrecy: sender deletes message key immediately after encryption; recipient deletes after decryption.
- Sealed sender: the wire envelope carries no author identity for DMs. The sender's pseudonym is inside the encrypted payload, readable only by the recipient.
- Deniability: DMs use MAC-based authentication (not signatures) so messages are deniable.

**Local keystore:**
- Format: `[4-byte version][12-byte Argon2id salt][24-byte nonce][ciphertext][16-byte Poly1305 tag]`.
- Passphrase-derived key via Argon2id (memory: 256 MiB, iterations: 3, parallelism: 4).
- Stores: master key, device keys, pseudonym keys, epoch keys, ratchet state.

### 3.4 Threat Model

**What Ephemera protects against:**
- Network observers cannot determine who is communicating with whom (onion routing).
- Storage nodes cannot read content (all stored data is ciphertext).
- Compromised long-term keys do not reveal past messages (forward secrecy via Double Ratchet + epoch key deletion).
- A single compromised node cannot deanonymize users (no node holds identity-to-IP mappings).
- Content is unrecoverable after 30 days (TTL enforcement + cryptographic shredding).

**What Ephemera does NOT protect against:**
- A compromised client device (if your device is owned, your keys are exposed).
- A modified client that bypasses CSAM checks (relay verification catches public content; encrypted channels rely on recipient reporting).
- Global passive adversaries with the ability to monitor all network links simultaneously (Tor's known limitation).
- Screenshots or photos of the screen (physical capture of displayed content).
- Small anonymity sets at launch (mitigated by using Tor's existing anonymity set via Arti).

---

## 4. Network Protocol

### 4.1 P2P Protocol: Iroh

**Decision: Iroh for the transport substrate, with Arti layered on top for client privacy.**

Iroh is chosen over libp2p for the following reasons:
- Superior NAT traversal (~90% direct connection success rate vs. ~70% for libp2p), critical for residential users.
- QUIC multipath via `noq` (start on relay, seamlessly upgrade to direct).
- Simpler API with built-in gossip (`iroh-gossip`) and content-addressed blob transfer.
- Pure Rust, aligns with the project's Rust-first philosophy.

The trade-off is that Iroh lacks a built-in Kademlia DHT, requiring a custom implementation. If the custom DHT exceeds 4 weeks of engineering effort, the fallback is libp2p. All networking is abstracted behind traits to make this swap feasible.

### 4.2 Transport Layer

Three privacy tiers, all implementing `trait AnonTransport`:

| Tier | Technology | Latency | Privacy Level | Use Case |
|------|-----------|---------|---------------|----------|
| **T1 Stealth** | Loopix-style mixnet, Sphinx packets over Iroh QUIC between mix nodes. Client enters via Arti circuit. | 2-30 seconds | Maximum. Cover traffic, Poisson delay, unlinkable timing. | Whistleblowers, high-risk users. |
| **T2 Private** (default) | Arti/Tor 3-hop onion routing. Exit node connects to Ephemera backbone via Iroh QUIC. | 200-800ms | Strong. IP hidden from all peers. Standard Tor anonymity set. | Default for all users. |
| **T3 Fast** | Iroh single-hop encrypted relay. Relay masks client IP from destination. Relay sees client IP but not content (E2E encrypted). | 50-200ms | Moderate. Single relay trust point. | Low-latency needs, mobile clients (due to bandwidth constraints). |

**Node backbone:** Iroh QUIC with iroh-gossip. DHT nodes, storage nodes, and relays communicate directly using Iroh's transport. This is where NAT traversal and multipath provide the most value.

**Wire protocol stack:**

| Layer | Choice | Format |
|-------|--------|--------|
| Transport | QUIC (via Iroh) | UDP datagrams |
| Encryption | QUIC TLS 1.3 (node-to-node), Tor circuits (client-to-network) | Stream cipher |
| Compression | LZ4 | Per-message |
| Framing | Length-prefixed | u32 big-endian + payload |
| Serialization (wire) | Protobuf via `prost` | Binary, schema-evolvable |
| Serialization (storage) | bincode | Binary, fast |
| Serialization (content) | CBOR via `ciborium` | Binary, self-describing |
| Content addressing | BLAKE3 | 32-byte hash with version prefix byte |
| Signatures | Ed25519 | 64-byte signature |
| Timestamps | HLC via `uhlc` | u64 Unix seconds (wire), HLC internally |

### 4.3 Node Discovery

1. **Bootstrap nodes:** 5 hardcoded, geographically diverse VPS instances operated by the project team. These are the initial entry points for new nodes.
2. **Gossip-based peer exchange:** Once connected, nodes exchange peer lists via gossip. Peer addresses are cached locally, reducing bootstrap dependency over time.
3. **DHT-based discovery:** At 100+ nodes, Kademlia routing enables discovery of any node by ID without knowing all peers.
4. **Relay discovery:** Relay nodes register in the DHT. Clients discover relays through DHT lookups via their anonymous transport.

### 4.4 Content Routing

- **Public posts:** Disseminated via gossip (iroh-gossip / PlumTree). Subscribers to a topic or a pseudonym's feed receive posts through the gossip overlay.
- **DHT lookups:** Used for point queries -- prekey bundles, user profiles, hashtag indexes, specific content by ID.
- **Media chunks:** Content-addressed via BLAKE3 CIDs. Retrieved from any peer that has the chunk. Swarming distribution: downloaders become uploaders. Rarest-first replication to maintain redundancy (target: 5 replicas).

### 4.5 NAT Traversal

- **Iroh's built-in QUIC hole-punching** for relay-to-relay and client-to-relay connections.
- **Community-operated Ephemera relay nodes** (not n0's default relays) for clients that cannot hole-punch.
- **DERP fallback** (Iroh's relay protocol) when direct UDP is blocked.
- Clients behind NAT connect to their entry relay or Tor entry node; the relay handles backbone connectivity.

### 4.6 Protobuf Envelope

```protobuf
message Envelope {
    uint32 version = 1;
    MessageType type = 2;
    bytes author_id = 3;       // Pseudonym pubkey for public posts; EMPTY for sealed-sender DMs
    bytes node_id = 4;         // Forwarding node's network identity (for routing)
    bytes payload = 5;         // Type-specific content (encrypted for DMs)
    bytes signature = 6;       // Ed25519 sig by author (public) or absent (DMs)
    uint64 timestamp = 7;      // Unix seconds (u64)
    uint64 ttl_seconds = 8;    // Content lifetime, max 2,592,000
}
```

---

## 5. Privacy Layer

### 5.1 Anonymity Level and Approach

**Default: T2 (Arti/Tor).** The client's traffic exits through a 3-hop Tor circuit. The destination node never learns the client's real IP address. Ephemera leverages Tor's existing anonymity set (millions of users), which is critical at launch when the Ephemera-specific user base is small.

**Client-to-client connections are forbidden.** All communication flows through relays or onion circuits. Iroh's direct peer-to-peer QUIC connections are used only between backbone infrastructure nodes (relays, DHT nodes, storage nodes), never between end-user clients.

### 5.2 Traffic Protection

- **T1:** Constant-rate traffic padding (2 KB/s cover traffic). All packets normalized to fixed size. Poisson-distributed delays (lambda configurable, default 500ms mean). Cover traffic makes real messages indistinguishable from noise.
- **T2:** Standard Tor traffic patterns. No additional padding (relies on Tor's existing protections). Circuit rotation every 24 hours.
- **T3:** Message padding to nearest 256-byte boundary. No timing obfuscation.
- **Mobile exception:** Constant-rate padding (173 MB/day) is impractical on mobile. Mobile clients default to T3 with the option to enable T2 for specific actions.

### 5.3 Metadata Protection

- **Node identity vs. pseudonym identity:** Strict separation. All user-facing operations (content publication, prekey publication, gossip subscription) are routed through the anonymous transport. No link between originating node and pseudonym.
- **Gossip subscription privacy:** The client does NOT subscribe to interest-based topics from its own node. Subscriptions route through the T2/T3 relay, which acts as a gossip proxy.
- **DM metadata:** Sealed sender. The wire envelope carries no author identity for DMs. Relay nodes MUST NOT log or correlate sender-recipient pairs beyond minimum for delivery.
- **Relay retention:** Undelivered messages purged after 14 days. Delivery metadata purged immediately after successful delivery.

---

## 6. Storage & Data

### 6.1 Storage Engine

**Decision: fjall 3.1 for content blob storage (ciphertext). SQLite for metadata and indexes.**

- **fjall** is chosen over sled (pre-1.0, unstable) and RocksDB (FFI complexity). fjall is pure Rust, has LSM-tree architecture with compaction filters for TTL-based expiry, and handles the write-heavy workload of content ingestion efficiently.
- **SQLite** (via `rusqlite`) handles relational metadata: post metadata, social graph, peer info, routing tables, local hashtag indexes, HLC timestamps.
- **Filesystem** with time-partitioned directories (one per day) for zero-cost bulk expiry: when all content in a day directory has expired, delete the entire directory.
- **RocksDB** is the proven fallback if fjall cannot handle load. The `StorageBackend` trait enables swapping.

**fjall stores only ciphertext.** Content is encrypted by the publishing client before submission to the network. Storage nodes store opaque bytes and cannot read content.

### 6.2 Data Model

**Post:**
```rust
pub struct Post {
    // Identity & ordering
    pub id:                  ContentHash,        // BLAKE3(serialized CBOR payload)
    pub author:              IdentityKey,        // Ed25519 pseudonym pubkey
    pub sequence_number:     u64,                // Per-author monotonic counter
    pub created_at:          Timestamp,          // u64 Unix millis UTC
    pub expires_at:          Timestamp,          // created_at + ttl_seconds * 1000
    pub ttl_seconds:         u32,                // max 2,592,000 (30 days)

    // Abuse prevention
    pub pow_stamp:           PowStamp,           // Equihash proof
    pub identity_created_at: Timestamp,          // For warming period checks

    // Content
    pub body:                Option<RichText>,   // Constrained Markdown
    pub media:               Vec<MediaAttachment>,
    pub sensitivity:         Option<SensitivityLabel>,

    // Threading
    pub parent:              Option<ContentHash>,// Reply-to
    pub root:                Option<ContentHash>,// Thread root
    pub depth:               u8,                 // 0 = top-level

    // Discovery
    pub tags:                Vec<Tag>,           // Normalized lowercase hashtags
    pub mentions:            Vec<MentionTag>,    // Pubkey-anchored
    pub topic_rooms:         Vec<TopicRoomId>,
    pub language_hint:       Option<String>,     // ISO 639-1
    pub audience:            Audience,           // Public for MVP

    // Integrity
    pub content_nonce:       [u8; 16],           // Prevents hash collision attacks
    pub signature:           Signature,          // Ed25519 over all above fields
}
```

**MediaAttachment:**
```rust
pub struct MediaAttachment {
    pub media_type:   MediaType,        // Image, Video, Audio
    pub mime_type:    String,           // "image/webp", "video/mp4", "audio/ogg"
    pub variants:     Vec<MediaVariant>,
    pub blurhash:     Option<String>,   // ~28 chars, instant placeholder
    pub alt_text:     Option<String>,   // Max 1,000 chars, accessibility
    pub dimensions:   Option<(u32, u32)>,
    pub duration_ms:  Option<u64>,      // For video/audio
}

pub struct MediaVariant {
    pub quality:   Quality,        // Original, Display, Thumbnail
    pub cid:       ContentHash,    // BLAKE3 of encrypted+chunked blob
    pub size_bytes: u64,
    pub width:     Option<u32>,
    pub height:    Option<u32>,
}
```

**Social graph events:** Signed CRDT operations.
- Follow: `FollowEvent { source: IdentityKey, target: IdentityKey, action: Follow|Unfollow, timestamp, signature }` -- OR-Set semantics (add-wins).
- Reactions: `ReactionEvent { target: ContentHash, reactor: IdentityKey, emoji: String, action: Add|Remove }` -- OR-Set per post.
- Profiles: `ProfileUpdate { author: IdentityKey, display_name, bio, avatar_cid, timestamp, signature }` -- LWW-Register keyed on HLC timestamp.

**Messages:**
```rust
pub struct MessageEnvelope {
    pub recipient:      IdentityKey,     // Recipient's pseudonym pubkey
    pub sender_sealed:  Vec<u8>,         // Sender identity inside encrypted payload
    pub ciphertext:     Vec<u8>,         // Double Ratchet encrypted content
    pub timestamp:      Timestamp,
    pub ttl_seconds:    u32,             // Max 30 days, default 14 days
    pub pow_stamp:      PowStamp,        // Difficulty varies by relationship
}
```

### 6.3 TTL Enforcement

**Single source of truth for TTL constants** (in `ephemera-types`):

```rust
pub const MAX_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);        // 30 days
pub const EPOCH_DURATION: Duration = Duration::from_secs(24 * 60 * 60);      // 24 hours
pub const CLOCK_SKEW_TOLERANCE: Duration = Duration::from_secs(5 * 60);      // 5 minutes
pub const EPOCH_KEY_RETENTION: Duration = MAX_TTL;                            // 30 days
pub const TOMBSTONE_RETENTION_MULTIPLIER: u32 = 3;                            // 3x original TTL
pub const GC_INTERVAL: Duration = Duration::from_secs(60);                    // 1 minute
```

**Four-layer enforcement:**
1. **Type system:** `Ttl::new()` rejects durations > 30 days at construction time.
2. **Network layer:** Incoming records with TTL > 30 days + clock skew tolerance are rejected.
3. **Storage layer:** fjall compaction filters drop expired entries. Background sweep catches stragglers.
4. **Cryptographic shredding:** Epoch key `EK_N` deleted 30 days after epoch N ends. All content encrypted under `EK_N` becomes permanently unrecoverable.

**Proactive GC:** When a node deletes an epoch key, it enqueues a background sweep to identify and drop all content encrypted under that epoch's key.

### 6.4 Replication and Consistency

**Model: Eventual consistency (AP system).**

**Replication targets:**
- Content blobs: R=5 (5 replicas). Re-replicate below 3.
- Text-only posts: R=10 (cheap to replicate, higher availability).
- DHT routing table: k=20 (standard Kademlia bucket size).

**Consistency tiers:**
- **Causal consistency** for thread-structured content only. Replies carry `parent_hash` dependency. A reply is displayed only after its parent is available. Buffer for 30s, then active fetch, then placeholder after 5 minutes.
- **Eventual consistency with CRDTs** for everything else. Reactions (OR-Set), follows (OR-Set, add-wins), profiles (LWW-Register), reputation counters (G-Counter with decay).
- **Protocol-enforced determinism** for TTL expiration and author deletion tombstones.

**Anti-entropy:** Two Merkle trees (content + expiration tombstones). Sync interval: 120 seconds. Bandwidth budget: 50 KB/s for anti-entropy. Full state transfer as fallback for initial sync or long-partition recovery.

**Clock:** Hybrid Logical Clocks via `uhlc` crate. HLC provides causal ordering without requiring synchronized wall clocks. Clock skew tolerance of 5 minutes for TTL enforcement.

### 6.5 Content Addressing

Content ID = BLAKE3 hash with a 1-byte version prefix, wrapped in a `ContentHash` newtype:

```rust
pub struct ContentHash([u8; 33]); // 1 byte version + 32 bytes BLAKE3

impl ContentHash {
    pub fn compute(payload: &[u8]) -> Self {
        let hash = blake3::hash(payload);
        let mut bytes = [0u8; 33];
        bytes[0] = 0x01; // version 1
        bytes[1..].copy_from_slice(hash.as_bytes());
        Self(bytes)
    }
}
```

Media chunks are 256 KiB, each independently content-addressed. A post with media includes a manifest listing all chunk CIDs, enabling parallel retrieval and integrity verification.

---

## 7. Social Features

### 7.1 Posts

**Supported content types (MVP):**
- **Text:** Constrained Markdown. Allowed: `**bold**`, `*italic*`, `~~strikethrough~~`, `` `code` ``, `> blockquote`, `[text](url)`, `#hashtag`, `@mention`. Disallowed: headers, images in markup, tables, HTML, nested blockquotes, lists.
- **Photo:** WebP, single quality tier for MVP. Pipeline: strip EXIF, resize (max 1280px wide), compress to WebP, encrypt, chunk (256 KiB), BLAKE3 hash. Up to 4 photos per post.
- **Text + Photo:** A post contains text plus one media type batch.

**Deferred content types:** Video (H.264, deferred to Phase 2), audio (Opus/OGG, deferred to Phase 2), polls, check-ins, composite mixed-media posts, link previews.

**Post TTL:** User-configurable from 1 hour to 30 days. Default: 24 hours. Protocol maximum: 30 days (hard invariant).

### 7.2 Social Graph

**Model: Mutual connections (bidirectional, consent-required).**

This is chosen over asymmetric follows because:
- Stronger privacy (asymmetric follow graphs leak richer social metadata).
- Aligned with the anti-surveillance philosophy.
- Reduces the attack surface for social graph analysis.

**Connection flow:** Request -> Accept/Reject. QR code display+scan for in-person pairing. Invite links (`ephemera://connect/<pubkey>`) for remote discovery.

Public posts are viewable by anyone on the network without a connection. The author's profile and non-public posts require a mutual connection.

**Follow (asymmetric) for content discovery:** For MVP, asymmetric follows exist as a lighter-weight mechanism for subscribing to public content from identities you have not exchanged connection requests with. Follows are signed events stored in OR-Set CRDTs.

**Connection tiers (post-MVP):** Close Friends, Friends, Acquaintances -- each with different visibility and messaging permissions.

### 7.3 Interactions

**Reactions:** OR-Set per post. Exact counting (not HLL approximation) for MVP scale. Small emoji set: heart, laugh, fire, sad, thinking. Reactions are private to the post author -- no public counts, no social proof manipulation.

**Replies:** Flat, single `parent` link. No tree threading for MVP. Replies are independent posts with their own TTL. A reply can outlive its parent; the client displays "[original post expired]" when the parent is unresolvable.

**Hashtags:** Parsed from post body and stored as structured metadata (`tags: Vec<Tag>`, normalized lowercase). Local index only for MVP. DHT-based hashtag indexing deferred to Phase 2.

**Mentions:** User types `@alice`, client resolves to pubkey, stores as `MentionTag { pubkey, display_hint, byte_range }`. Notifications delivered via DHT mailbox keyed to the mentioned pubkey.

**Reposts/Quote reposts:** Deferred from MVP. When implemented, quote reposts embed a signed snapshot of the original's first 280 characters (text only, no media snapshot).

### 7.4 Messaging

**1:1 encrypted messaging (MVP):**
- Key exchange: X3DH using prekey bundles published to DHT via anonymous circuit.
- Encryption: Double Ratchet providing forward secrecy and post-compromise security.
- Delivery: Store-and-forward via relay nodes. Sender deposits encrypted message at a dead drop address derived from shared ratchet state. Recipient polls the dead drop via anonymous transport.
- Sealed sender: wire envelope carries no author identity. Sender pseudonym inside encrypted payload.
- Message requests: strangers must complete PoW to send first message. Friends message with zero PoW.
- Offline delivery: relay nodes hold undelivered messages for up to 14 days.
- Message padding: all messages padded to nearest 256-byte boundary to reduce size-based content inference.

**Group messaging:** Deferred to Phase 2. Will use MLS (RFC 9420) for all group encryption (both content sharing and chat), capped at 100 members.

### 7.5 Content Size and Rate Limits

| Parameter | Value | Enforcement |
|-----------|-------|-------------|
| Text post body | 2,000 grapheme clusters AND 16 KB wire | Protocol (hard reject) |
| Message text | 10,000 grapheme clusters AND 32 KB wire | Protocol (hard reject) |
| Photo input (pre-processing) | 10 MB per photo | Client-enforced |
| Photo output (post-processing) | 5 MiB per photo after WebP compression | Client + peer validation |
| Photos per post | 4 | Protocol |
| Photos per message | 1 | Protocol |
| Video (post, MVP) | 50 MiB, 720p max, 3 min max | Client + peer validation |
| Video (message) | 50 MiB | Client + peer validation |
| Audio | 10 MiB / 5 minutes, Opus in OGG | Client + peer validation |
| Total post media | 50 MiB | Protocol |
| Profile metadata | 4 KB | Protocol |
| Chunk size | 256 KiB | Protocol |
| Posts per hour | 10 (default, configurable 5-20) | Node-enforced |
| Replies per hour (same thread) | 20 | Node-enforced |
| Interactions per hour | 100 | Node-enforced |
| DMs/hr to friends | 60 per conversation | Node-enforced |
| DMs/hr to mutual contacts | 30 per conversation | Node-enforced |
| DMs/hr to strangers | 5 total (message requests) | Node-enforced |
| Follows per hour | 50 | Node-enforced |
| Storage per identity per node | 500 MB | Node-enforced, priority eviction on overflow |
| Content TTL max | 30 days (2,592,000 seconds) | Protocol (hard invariant) |
| Relay retention (undelivered) | 14 days | Relay-enforced |
| Minimum replication | 5 peers (re-replicate below 3) | Network-enforced |

---

## 8. Content Moderation & Safety

### 8.1 The Balance

Ephemera provides **privacy by default and accountability for extreme abuse.** The system is designed so that no single entity can surveil users, but a narrow set of universally condemned content categories receive protocol-level enforcement. The system is honest about what it can and cannot do. Encrypted channels provide less moderation coverage than public content -- this is the cost of not breaking encryption.

### 8.2 CSAM Detection

**Layer 1 -- Client-side perceptual hash bloom filter:**
- Ships with every official client. ~10 MB bloom filter of perceptual hashes derived from established databases (NCMEC, IWF, Project Arachnid).
- Runs silently in the media pipeline before encryption. If a match is detected, the post is silently blocked with a generic error ("Unable to share this image").
- Updated via a dedicated gossip message type (`BloomFilterUpdate`) with 3-of-5 multi-signature from independent signers (Foundation board members from different jurisdictions + independent civil liberties organizations).
- Public audit trail of filter changes (Merkle-tree-based).

**Layer 2 -- Relay-side verification:**
- For unencrypted public content, relay nodes independently compute perceptual hashes and check the bloom filter.
- Content matching the filter is rejected and not forwarded.
- Privacy-preserving report (content hash + timestamp, no author info) for transparency aggregation.

**Limitation:** A modified client can bypass client-side checks. Relay verification only works for public content. For encrypted DMs, the system relies on recipient reporting and reputation consequences.

### 8.3 Distributed Moderation

**Community moderation protocol:**
- Quorum-based: 5-of-7 moderators agree to take action.
- Blinded review: moderators see content but not author identity.
- Protobuf message types: `ContentReport`, `ModerationVote`, `ModerationAction`.
- CRDT-based moderator sets (OR-Set) and reputation counters (G-Counter with decay) propagated via gossip within neighborhoods.
- Content removal via community tombstones signed by quorum (threshold signature from 5-of-7 moderators).

### 8.4 Reporting

**MVP:** "Report" menu item on posts (alongside "Mute" and "Block"). Reports stored locally. The recipient of an abusive DM can block the sender (local, silent).

**Post-MVP:** Trusted flagger integration, external reporting pipeline (NCMEC/IWF), distributed moderation quorums for message abuse.

### 8.5 Abuse Prevention

**Proof-of-Work (Equihash):**
- Identity creation: ~30 seconds on a modern mid-range phone. Adaptive difficulty during surges.
- Post creation: ~100ms base, scaling with activity and content size.
- Message to stranger: full PoW. Message to friend: zero PoW.
- PoW formula: `difficulty = base * activity_multiplier * content_size_multiplier * relationship_discount * reach_multiplier`.
- PoW ceiling: 60 seconds maximum.
- Equihash chosen over Hashcash for memory-hardness (resists GPU/ASIC acceleration).

**Reputation-gated capabilities:**
- New pseudonym (reputation = 0): 1 post/hour (7-day warming period), text only, no moderation votes, no DMs to strangers.
- Established pseudonym: full posting, media, DMs, voting eligibility.
- Moderator-eligible: high reputation + community election.
- Creating a new pseudonym to evade reputation penalties starts from zero and must rebuild -- making sustained abuse expensive.

**Sybil resistance:** Identity creation PoW + reputation warming period + social trust (EigenTrust-inspired local scoring). No single defense suffices; layering is essential.

### 8.6 What We Will Not Build

- Key escrow or ghost protocol participants.
- Mandatory identity verification.
- Server-side scanning of encrypted content.
- Permanent bans (unenforceable in a pseudonymous system; reputation is the lever).
- A cryptocurrency token (regulatory landmine, wrong community signal).
- Trending or algorithmic amplification (legal liability target, anti-platform philosophy).

---

## 9. Client Architecture

### 9.1 Framework: Tauri 2.x + SolidJS

**Decision: Tauri 2.x with SolidJS frontend.**

- Tauri provides a native webview shell with direct Rust backend access via `invoke()`.
- SolidJS is chosen for fine-grained reactivity, small bundle size, and excellent performance for feed-style UIs.
- The Rust backend embeds `ephemera-node` as a library (embedded mode).
- Single binary distribution, no separate daemon process for the PoC.

### 9.2 Client-Node API

**JSON-RPC 2.0** is the formal API boundary between the SolidJS frontend and the Rust backend.

In embedded mode, JSON-RPC messages are passed in-process through Rust channels (zero serialization overhead for the common case). The protocol is designed so that in daemon mode (post-PoC), the same messages serialize over Unix domain socket or WebSocket on localhost.

**API namespaces:**
- `identity.*` -- keypair management, pseudonym creation, profile updates.
- `posts.*` -- create, list, get, delete. Cursor-based pagination.
- `social.*` -- connect, disconnect, request, accept, reject. Connection list.
- `messages.*` -- send, list conversations, get thread.
- `media.*` -- attach, upload progress, retrieve.
- `moderation.*` -- report, block, mute, unblock.
- `meta.*` -- capabilities handshake, node status, peer count, network health.
- `feed.*` -- chronological feed from connections, discover (public posts, no ranking).

**Event streaming:** The backend pushes events to the frontend via Tauri's event system for real-time updates (new post, new connection request, new message, expiry notification).

### 9.3 Offline Support

- **Local SQLite is the source of truth.** The network is a sync mechanism.
- The app launches and renders the feed from local cache in < 1 second.
- Posts composed offline are queued locally and published on reconnect.
- Network bootstrap (3-10 seconds) and full sync (30-120 seconds) happen asynchronously in the background.
- A three-dot connectivity indicator shows offline/connecting/connected state.
- Expired content fades out with a dissolve animation during the final 24 hours.

### 9.4 Onboarding

Flow: **Welcome -> Age Confirmation + Terms (one tap) -> Name Selection -> Avatar (generative grid) -> Feed.** Total: ~17 seconds.

- Age confirmation: "By continuing, you confirm you are 16 or older and agree to our community guidelines." Single tap. Link to full text available but not required reading.
- Name: generated adjective-animal pattern (e.g., "quiet-fox-42") with random suggestions. Editable.
- Avatar: identicon/generative art derived from public key. No photo upload required.
- Recovery phrase: prompted after user's first post or first connection request, not during onboarding.

### 9.5 Performance Targets

| Metric | Target | Notes |
|--------|--------|-------|
| UI ready (feed rendered) | < 1 second | From local SQLite cache |
| Network ready | 3-10 seconds | Peer connections established, invisible to user |
| Fully synced | 30-120 seconds | Anti-entropy complete, missed content backfilled |
| Total app memory | ~350 MB | 150 MB app + 200 MB node (L1 cache only) |
| Feed scroll | 60 FPS | SolidJS + virtual scrolling |
| Post creation | < 30ms local | Optimistic update; network propagation async |
| Post propagation (P95) | 2 seconds | To 95% of connected peers |

### 9.6 PoC Scope

The PoC ships as a **single Tauri 2.x desktop binary** (Windows, macOS, Linux) with the node embedded. The client operates as a **light node by default**: 500 MB storage cap, bandwidth-limited relay, no DHT routing. Users can opt into full-node mode via settings.

---

## 10. Rust Workspace Structure

### 10.1 Crate Layout

```
ephemera/
  crates/
    ephemera-types/        # Shared primitives: IdentityKey, Signature, ContentHash,
                           # Timestamp, RichText, Tag, MentionTag, Audience,
                           # SensitivityLabel, TTL constants. Zero heavy deps.

    ephemera-crypto/       # Signing (Ed25519), encryption (XChaCha20-Poly1305),
                           # key derivation (HKDF, Argon2id), content hashing
                           # (BLAKE3), keystore, zeroize/secrecy wrappers.

    ephemera-protocol/     # Wire protocol: Protobuf definitions (prost), Envelope,
                           # message types, versioning, capability negotiation.

    ephemera-transport/    # QUIC connections (iroh), NAT traversal, relay management,
                           # peer connection lifecycle. AnonTransport trait + T2/T3
                           # implementations.

    ephemera-dht/          # TTL-aware Kademlia: routing table, provider records,
                           # content lookup, prekey storage. Built on iroh QUIC.

    ephemera-gossip/       # Topic-based pub/sub (wraps iroh-gossip), PlumTree,
                           # anti-entropy sync, Merkle tree comparison.

    ephemera-store/        # Storage engine abstraction (StorageBackend trait),
                           # fjall backend for content, SQLite for metadata,
                           # time-partitioned filesystem, GC/compaction, epoch
                           # key management.

    ephemera-crdt/         # CRDT implementations: ExpiringSet, BoundedCounter,
                           # OR-Set, G-Counter, LWW-Register. Delta-state
                           # replication. Built on `crdts` crate.

    ephemera-post/         # Post data model: Post, PostType, MediaAttachment,
                           # MediaVariant, PowStamp. Validation logic.
                           # Depends on ephemera-types.

    ephemera-message/      # Message data model: MessageEnvelope, X3DH, Double
                           # Ratchet state, dead drop addressing. Separate from
                           # posts (different encryption, delivery, lifecycle).

    ephemera-media/        # Media processing: EXIF strip, resize, WebP compress,
                           # H.264 transcode (Phase 2), Opus encode (Phase 2),
                           # CSAM hash check, encrypt, chunk (256 KiB), BLAKE3.

    ephemera-abuse/        # PoW computation/validation (Equihash), rate-limit
                           # checking, warming period logic, reputation scoring.
                           # Shared by post and message subsystems.

    ephemera-social/       # Domain logic: feed assembly, connection management,
                           # reaction processing, hashtag indexing, profile
                           # management.

    ephemera-mod/          # Moderation: perceptual hash bloom filter, community
                           # flags, report handling, quorum voting, tombstone
                           # generation.

    ephemera-events/       # Internal event bus (tokio broadcast channel). Decouples
                           # subsystems.

    ephemera-config/       # Layered config loading: defaults, file, env vars,
                           # CLI args. ResourceProfile (Embedded/Standalone).

    ephemera-node/         # Composition root: wires all crates together. Embeddable
                           # as library (for Tauri) or standalone binary (daemon).
                           # JSON-RPC 2.0 API handler. NodeConfig with
                           # ResourceProfile.

    ephemera-client/       # Tauri 2.x app: SolidJS frontend (src-ui/), Rust
                           # backend embedding ephemera-node. invoke() bridge.

    ephemera-cli/          # CLI tool: connects to daemon via JSON-RPC over Unix
                           # domain socket. Post-PoC.
```

### 10.2 Dependency Graph

```
ephemera-types (leaf, zero heavy deps)
    |
    +---> ephemera-crypto (types + blake3, ed25519-dalek, chacha20poly1305, argon2, zeroize)
    |         |
    |         +---> ephemera-protocol (crypto + prost)
    |         |         |
    |         |         +---> ephemera-transport (protocol + iroh, arti)
    |         |         |         |
    |         |         |         +---> ephemera-dht (transport)
    |         |         |         +---> ephemera-gossip (transport)
    |         |         |
    |         |         +---> ephemera-store (protocol + fjall, rusqlite)
    |         |
    |         +---> ephemera-post (crypto + types)
    |         +---> ephemera-message (crypto + types)
    |         +---> ephemera-media (crypto + image, webp libs)
    |         +---> ephemera-abuse (crypto + equihash)
    |
    +---> ephemera-crdt (types + crdts)
    +---> ephemera-events (types + tokio)
    +---> ephemera-config (types + serde, toml)

ephemera-social (post + message + crdt + store + events)

ephemera-mod (post + abuse + crypto + store + gossip)

ephemera-node (ALL crates above)
    |
    +---> ephemera-client (node + tauri + solidjs frontend)
    +---> ephemera-cli (node + clap)
```

### 10.3 Key Rust Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio` | 1.x (full features) | Async runtime |
| `iroh` | latest stable | QUIC transport, NAT traversal, relay |
| `iroh-gossip` | latest stable | Pub/sub gossip overlay |
| `arti-client` | latest stable | Tor onion routing (T2) |
| `ed25519-dalek` | 2.x | Ed25519 signatures |
| `x25519-dalek` | 2.x | X25519 Diffie-Hellman |
| `chacha20poly1305` | 0.10 | XChaCha20-Poly1305 AEAD |
| `blake3` | 1.x | Content hashing |
| `argon2` | 0.5 | Password KDF, PoW validation |
| `hkdf` | 0.12 | Key derivation |
| `zeroize` | 1.x | Secure memory erasure |
| `secrecy` | 0.8 | Secret-wrapping types |
| `prost` | 0.12 | Protobuf serialization (wire) |
| `ciborium` | 0.2 | CBOR serialization (content) |
| `bincode` | 2.x | Binary serialization (storage) |
| `fjall` | 3.1 | LSM-tree KV store for content |
| `rusqlite` | 0.31 | SQLite for metadata |
| `serde` | 1.x | Serialization framework |
| `uhlc` | 0.7 | Hybrid Logical Clocks |
| `crdts` | 7.x | CRDT primitives |
| `equihash` | latest | Memory-hard PoW |
| `tauri` | 2.x | Desktop application shell |
| `lz4_flex` | 0.11 | Wire compression |
| `tracing` | 0.1 | Structured logging |

---

## 11. PoC Roadmap

### Phase 1: Core Loop (Weeks 1-4)

**Goal:** "A user can create an identity, post text+photo, connect with someone, and see their posts."

| Week | Deliverable |
|------|------------|
| **1-2** | Crate scaffolding (`ephemera-types`, `ephemera-crypto`, `ephemera-store`, `ephemera-transport`, `ephemera-node`). Ed25519 identity generation with OS secure storage. SQLite schema with versioning. Transport spike: evaluate iroh (decision by end of Week 2). Tauri 2.x shell with SolidJS skeleton and dark theme. |
| **3-4** | Text post creation (Ed25519 signed, HLC timestamped, TTL 1h-30d, Equihash PoW). Gossip protocol for post propagation. Chronological feed from local SQLite with cursor-based pagination. Content expiry GC task + time-partitioned filesystem deletion + tombstone propagation. Mutual connection flow (request/accept/reject, QR code). Onboarding (Welcome -> Age confirmation -> Name -> Avatar -> Feed). Deploy 5 bootstrap nodes + 3 relay nodes on VPS. |

### Phase 2: Interaction & Communication (Weeks 5-8)

**Goal:** "Users can reply, react, discover content by hashtag, and message each other privately."

| Week | Deliverable |
|------|------------|
| **5-6** | Image attachments (EXIF strip, resize, WebP, BLAKE3, CSAM hash check, content-addressed storage). Flat replies with causal ordering (parent_hash dependency). Reactions (OR-Set, private to author). Local hashtag index. Report/block/mute (local enforcement). IP geofencing for sanctioned jurisdictions. Recovery phrase (BIP-39 mnemonic, prompted after first post). |
| **7-8** | 1:1 encrypted messaging (X3DH + Double Ratchet over dead drop mailboxes). Store-and-forward relay for offline delivery. Message requests with PoW gating. Topic rooms (basic: subscribe to topic ID, posts tagged with room appear in room). Blurhash rendering for instant media placeholders. Network status indicator (three-dot system). "All caught up" end-of-feed state. Offline compose queue with "pending" indicator. |

### Phase 3: Polish & Hardening (Weeks 9-12)

**Goal:** "The platform feels usable for daily interaction."

| Week | Deliverable |
|------|------------|
| **9-10** | User profiles (display name, bio, avatar, published to DHT). Notification delivery (DHT mailbox + relay). Basic spam fingerprinting (SimHash duplicate detection). Improved feed sync efficiency. Invite links (`ephemera://connect/`). Settings (display name edit, node status, resource profile). |
| **11-12** | Performance optimization (batch Ed25519 verification, LZ4 wire compression, TinyLFU L1 cache). Security hardening (fuzz testing on protocol parsing, key handling audit). Integration testing at 100-node scale. Accessibility pass (ARIA labels, keyboard nav, screen reader, high contrast, font scaling). Bug fixes and stabilization. |

### Explicitly NOT in PoC

| Feature | Reason |
|---------|--------|
| Video posts | H.264 client-side transcoding is heavyweight. Photo is sufficient for PoC. |
| Audio posts / voice notes | Requires audio pipeline. Not blocking. |
| Group messaging (MLS) | MLS is complex. 1:1 messaging is the MVP. |
| Groups / communities | Requires group key management and moderation tools. |
| Friends-only / followers-only posts | Requires per-audience encryption infrastructure. Public only for MVP. |
| Multi-device sync | Hard problem (sender-side fan-out). Single device for PoC. |
| Trending / algorithmic ranking | Anti-platform philosophy. Legal liability target. |
| Content decay (quality degrades over TTL) | Novel but adds complexity to storage and rendering. |
| Link previews | Privacy problem unsolved (IP leak when fetching OG metadata). |
| Polls, check-ins, composite posts | Niche content types. Text + photo covers 80% of use cases. |
| DHT hashtag indexing | Local index sufficient for small PoC network. |
| T1 mixnet | Requires sufficient user base for anonymity set. Launch with T2 only. |
| Mobile builds | Tauri mobile is post-PoC. Desktop only. |
| Daemon mode / CLI tools | Embedded mode only for PoC. |
| Swiss foundation incorporation | Year 2-4 milestone. |
| Managed-node hosting service | Validated as feasible. Year 2+ business development. |
| Contribution tracking / QoS tiers | Requires privacy-preserving aggregation. Phase 2+. |
| NCMEC/IWF external reporting | Post-PoC compliance milestone. |
| Community blocklists | Local blocking sufficient for MVP. |
| Erasure coding (Reed-Solomon) | Reliability optimization for larger networks. |
| Auto-updates | Post-PoC distribution concern. |

---

## Appendix A: Infrastructure

**Project-operated infrastructure for PoC:**
- 5 bootstrap nodes on VPS (~$50-100/month each, geographically diverse).
- 3 relay nodes on VPS (~$20-50/month each, public IPs).
- Total baseline: ~$400-750/month, funded by grants.

**Default-client-as-node:** Every Tauri desktop client contributes as a light node (500 MB storage, 10% bandwidth, 5% CPU). Users can opt into full-node mode. This is the foundation of network sustainability.

**Design target:** 100 nodes. Architecture must not preclude 10,000. Build for 100, test at 100, optimize for 1,000 only after stability at 100 is proven.

## Appendix B: Legal & Sustainability

**Foundation:** Swiss Stiftung (non-profit), incorporated at Year 2-4. Until then, BDFL governance with fiscal sponsorship (Software Freedom Conservancy or Open Collective Foundation).

**Funding model (three pillars):**
1. Self-sustaining network (default-client-as-node, contribution-gated QoS tiers post-MVP).
2. Grant-funded development (NLnet, OTF, European Commission NGI programs, $100K-500K/year target).
3. Diversified institutional funding at maturity (corporate sponsors, managed-node hosting, consulting).

**No cryptocurrency token. Ever.** Both legal and sustainability analysis independently reject tokens from different angles. This is settled.

**Marketing language:** "Privacy-preserving social communication." Never: "untraceable," "censorship-proof," "law-enforcement-proof." The Grokster inducement theory and Tornado Cash precedent make language a survival issue.

**Compliance features in official client:**
- Client-side CSAM hash matching (default-on, silent, PoC must-have).
- Age self-declaration (16+, single tap during onboarding).
- Report button on posts (local enforcement for PoC).
- IP geofencing for sanctioned jurisdictions (networking layer).
- Node operators are "mere conduits" -- relay encrypted data without inspection (EU DSA Article 4, E-Commerce Directive Article 12).

## Appendix C: Decisions Register

Every major architectural decision, for reference:

| # | Decision | Rationale |
|---|----------|-----------|
| 1 | Iroh over libp2p for transport | 90% NAT traversal success, QUIC multipath, simpler API. Custom DHT required; libp2p is fallback. |
| 2 | Arti/Tor as default privacy transport (T2) | Leverages existing Tor anonymity set. No custom onion routing for v1. |
| 3 | fjall 3.1 over sled/RocksDB for content storage | Pure Rust, compaction filters for TTL, write-optimized LSM. sled is unstable (pre-1.0). RocksDB is fallback. |
| 4 | SQLite for metadata | Proven, embedded, relational queries for social graph and indexes. |
| 5 | Two-identity architecture (node key + pseudonym key) | Prevents linking network participation to user identity. |
| 6 | Mutual connections over asymmetric follows | Stronger privacy, less metadata exposure, anti-surveillance aligned. |
| 7 | XChaCha20-Poly1305 over AES-256-GCM | No AES-NI dependency (ARM support), 192-bit nonce safe for random generation. |
| 8 | BLAKE3 over SHA-256 | 3.5x faster, tree-hashing for chunk verification, pure Rust. |
| 9 | Equihash over Hashcash for PoW | Memory-hard, resists GPU/ASIC acceleration. Critical for Sybil resistance on mobile. |
| 10 | CBOR for content serialization | Self-describing binary format, schema-flexible, used in other decentralized protocols. |
| 11 | Protobuf for wire protocol | Schema evolution across protocol versions, cross-language compatibility. |
| 12 | bincode for on-disk storage | Fast, compact, both sides are Rust. |
| 13 | HLC via uhlc for timestamps | Causal ordering without synchronized wall clocks. Bounded size (unlike vector clocks). |
| 14 | Tauri 2.x + SolidJS over Electron | Smaller footprint, native Rust backend, fine-grained reactivity. |
| 15 | Embedded node for PoC over daemon mode | Simpler deployment (single binary). JSON-RPC 2.0 boundary enables future extraction to daemon. |
| 16 | No trending / no algorithmic ranking | Anti-manipulation philosophy. Legal liability target (Pirate Bay precedent). |
| 17 | No cryptocurrency token | Regulatory risk, securities law, wrong community signal. Independently rejected by legal and sustainability analysis. |
| 18 | Switzerland for foundation jurisdiction | Avoids EU DSA exposure. Favorable privacy laws, political neutrality, Ethereum Foundation precedent. |
| 19 | 30-second identity creation PoW | Compromise between 10s (too weak) and 60s (too slow on phones). Adaptive difficulty during surges. |
| 20 | Double Ratchet for DMs | Forward secrecy + post-compromise security. Signal Protocol precedent. |
| 21 | Epoch key rotation (24h) + 30-day retention | Cryptographic shredding. Defense-in-depth with TTL-based GC. |
| 22 | CSAM bloom filter with 3-of-5 multi-sig updates | Prevents unilateral censorship. Public audit trail. Derived from established databases (NCMEC, IWF). |
| 23 | 100-node PoC design target | Realistic starting point. Architecture must not preclude 10,000. |
| 24 | Dark theme as default | Brand identity ("night sky" aesthetic). Dawn (light) available for accessibility. |

---

*Ephemera Unified Architecture Document v1.0 -- 2026-03-26*
*Synthesized from 7 group reviews and 3 cross-group integration documents.*
*This document is authoritative. Conflicts with earlier brainstorm documents are resolved in favor of this document.*
