# Ephemera: Rust Workspace Specification

**Parent document:** [ARCHITECTURE.md](./ARCHITECTURE.md) Section 10
**Version:** 1.0
**Date:** 2026-03-26
**Status:** Implementation-ready specification

---

## 1. Overview

The Ephemera project is organized as a single Cargo workspace containing 20 crates. Each crate has a single, well-defined responsibility. The dependency graph is acyclic, with `ephemera-types` as the leaf crate (zero heavy dependencies) and `ephemera-node` as the composition root that wires everything together.

This document specifies:
- Complete workspace directory tree
- Every Cargo.toml (workspace-level and per-crate)
- Crate responsibilities, public API surface, and module layout
- Dependency graph with justification for every external crate
- Feature flags per crate
- CI/CD pipeline (GitHub Actions)
- Testing strategy (unit, integration, simulation)
- Code quality rules and module organization patterns

**Cross-references:**
- Identity and crypto primitives: [01_identity_crypto.md](./01_identity_crypto.md)
- Network protocol and transport: [02_network_protocol.md](./02_network_protocol.md)
- Storage engines and data model: [03_storage_data.md](./03_storage_data.md)
- Social features: [04_social_features.md](./04_social_features.md)
- Moderation and safety: [05_moderation_safety.md](./05_moderation_safety.md)
- Client and API surface: [06_client_api.md](./06_client_api.md)

---

## 2. Workspace Directory Tree

```
ephemera/
├── Cargo.toml                          # Workspace root
├── Cargo.lock                          # Committed to version control
├── rust-toolchain.toml                 # Pin Rust version
├── clippy.toml                         # Workspace-wide Clippy config
├── rustfmt.toml                        # Workspace-wide formatting config
├── deny.toml                           # cargo-deny config (license + advisory audit)
├── .github/
│   └── workflows/
│       ├── ci.yml                      # Build, test, clippy, fmt, audit
│       └── release.yml                 # Tagged release builds
├── proto/
│   ├── envelope.proto                  # Wire protocol definitions
│   ├── gossip.proto                    # Gossip message types
│   ├── dht.proto                       # DHT request/response types
│   └── moderation.proto                # Moderation protocol types
├── crates/
│   ├── ephemera-types/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs                  # Re-exports
│   │       ├── content_hash.rs         # ContentHash newtype
│   │       ├── identity.rs             # IdentityKey, PeerId newtypes
│   │       ├── timestamp.rs            # Timestamp, HLC wrapper
│   │       ├── ttl.rs                  # Ttl newtype, TTL constants
│   │       ├── rich_text.rs            # RichText, constrained Markdown
│   │       ├── tag.rs                  # Tag, MentionTag
│   │       ├── audience.rs             # Audience enum
│   │       ├── sensitivity.rs          # SensitivityLabel
│   │       ├── media_types.rs          # MediaType, Quality, MediaAttachment, MediaVariant
│   │       ├── pow.rs                  # PowStamp type (data only, no computation)
│   │       ├── error.rs               # Common error types
│   │       └── constants.rs            # Protocol constants (MAX_TTL, EPOCH_DURATION, etc.)
│   │
│   ├── ephemera-crypto/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── signing.rs              # Ed25519 sign/verify, batch verification
│   │       ├── encryption.rs           # XChaCha20-Poly1305 encrypt/decrypt
│   │       ├── key_exchange.rs         # X25519 Diffie-Hellman
│   │       ├── key_derivation.rs       # HKDF-SHA256 paths, pseudonym derivation
│   │       ├── hashing.rs             # BLAKE3 content hashing, chunk hashing
│   │       ├── keystore.rs            # Encrypted keystore (Argon2id + XChaCha20)
│   │       ├── mnemonic.rs            # BIP-39 mnemonic generation/recovery
│   │       ├── shamir.rs              # Shamir's Secret Sharing (3-of-5)
│   │       ├── pow.rs                 # Equihash proof generation/verification
│   │       ├── epoch_keys.rs          # Epoch key generation, rotation, deletion
│   │       ├── zeroize_ext.rs         # Zeroize/secrecy wrapper utilities
│   │       └── bech32.rs             # eph1 Bech32m address encoding/decoding
│   │
│   ├── ephemera-protocol/
│   │   ├── Cargo.toml
│   │   ├── build.rs                    # prost-build for .proto compilation
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── envelope.rs             # Envelope type, serialization
│   │       ├── message_types.rs        # All protocol message type enums
│   │       ├── codec.rs               # Length-prefixed framing, LZ4 compression
│   │       ├── version.rs             # Protocol version, capability negotiation
│   │       ├── validation.rs          # Wire-level validation (size, TTL, signature presence)
│   │       ├── cbor.rs               # CBOR serialization for content payloads
│   │       ├── storage_codec.rs       # bincode serialization for on-disk format
│   │       └── generated/             # prost-generated code (build artifact, gitignored)
│   │           └── ephemera.proto.rs
│   │
│   ├── ephemera-transport/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── traits.rs              # AnonTransport trait, ConnectionEvent
│   │       ├── iroh_transport.rs      # Iroh endpoint setup, QUIC connections
│   │       ├── relay.rs               # Relay management, discovery, failover
│   │       ├── nat.rs                 # NAT traversal, hole-punching, DERP fallback
│   │       ├── connection_pool.rs     # Peer connection lifecycle, pooling
│   │       ├── bandwidth.rs           # Bandwidth monitoring, throttling
│   │       ├── tier_t2.rs             # Arti/Tor 3-hop circuit transport
│   │       ├── tier_t3.rs             # Iroh single-hop relay transport
│   │       ├── bootstrap.rs           # Bootstrap node list, initial connection
│   │       └── metrics.rs            # Connection metrics, peer statistics
│   │
│   ├── ephemera-dht/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── routing_table.rs       # Kademlia k-buckets, routing
│   │       ├── rpc.rs                 # FindNode, FindValue, Store, Ping
│   │       ├── ttl_records.rs         # TTL-aware provider records, expiry
│   │       ├── prekeys.rs            # Prekey bundle storage/retrieval
│   │       ├── profile_lookup.rs      # User profile DHT lookups
│   │       ├── iterator.rs           # Kademlia iterative lookups
│   │       └── maintenance.rs        # Bucket refresh, record republishing
│   │
│   ├── ephemera-gossip/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── topic.rs              # Topic management, subscription
│   │       ├── plumtree.rs           # PlumTree protocol, eager/lazy push
│   │       ├── anti_entropy.rs       # Merkle tree sync, state comparison
│   │       ├── peer_sampling.rs      # Peer sampling service
│   │       └── message_dedup.rs      # Message deduplication (seen cache)
│   │
│   ├── ephemera-store/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── traits.rs             # StorageBackend trait
│   │       ├── fjall_backend.rs      # fjall content blob store
│   │       ├── fjall_config.rs       # fjall configuration
│   │       ├── sqlite_meta.rs        # SQLite metadata store
│   │       ├── sqlite_schema.rs      # Table definitions, indexes
│   │       ├── migration.rs          # Schema migration system
│   │       ├── migrations/           # Numbered migration SQL files
│   │       │   ├── 001_initial.sql
│   │       │   └── ...
│   │       ├── gc.rs                 # Garbage collection task
│   │       ├── time_partition.rs     # Day-directory partitioning, bulk delete
│   │       ├── epoch_sweep.rs        # Epoch key deletion sweep
│   │       └── content_address.rs    # ContentHash-based put/get
│   │
│   ├── ephemera-crdt/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── or_set.rs             # OR-Set (add-wins) for follows, reactions
│   │       ├── g_counter.rs          # G-Counter with decay for reputation
│   │       ├── lww_register.rs       # LWW-Register for profiles
│   │       ├── expiring_set.rs       # ExpiringSet (OR-Set + TTL)
│   │       ├── bounded_counter.rs    # BoundedCounter for rate limiting
│   │       └── delta.rs             # Delta-state replication protocol
│   │
│   ├── ephemera-post/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── post.rs               # Post struct, PostType enum
│   │       ├── builder.rs            # PostBuilder (fluent API)
│   │       ├── validation.rs         # Post validation (TTL, size, PoW, signature)
│   │       ├── threading.rs          # Parent/root linkage, causal ordering
│   │       └── serialization.rs      # CBOR encoding/decoding for posts
│   │
│   ├── ephemera-message/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── envelope.rs           # MessageEnvelope struct
│   │       ├── x3dh.rs              # X3DH initial key exchange
│   │       ├── double_ratchet.rs     # Double Ratchet state machine
│   │       ├── dead_drop.rs         # Dead drop address derivation
│   │       ├── sealed_sender.rs      # Sealed sender envelope construction
│   │       ├── padding.rs           # Message padding (256-byte boundary)
│   │       └── validation.rs        # Message validation
│   │
│   ├── ephemera-media/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── pipeline.rs           # Media processing pipeline orchestrator
│   │       ├── exif.rs              # EXIF metadata stripping
│   │       ├── resize.rs            # Image resizing (max 1280px width)
│   │       ├── webp.rs              # WebP compression (quality 80)
│   │       ├── chunking.rs          # 256 KiB chunk splitting, reassembly
│   │       ├── blurhash.rs          # BlurHash generation for placeholders
│   │       ├── csam.rs             # CSAM perceptual hash check
│   │       └── encrypt.rs          # Per-media symmetric encryption + chunk hashing
│   │
│   ├── ephemera-abuse/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── pow_compute.rs        # Equihash proof generation
│   │       ├── pow_verify.rs         # Equihash proof verification
│   │       ├── difficulty.rs         # Adaptive difficulty calculation
│   │       ├── rate_limiter.rs       # Token-bucket rate limiting
│   │       ├── warming.rs           # New-identity warming period logic
│   │       └── reputation.rs        # Reputation scoring, capability gating
│   │
│   ├── ephemera-social/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── feed.rs              # Feed construction, chronological ordering
│   │       ├── connection.rs         # Connection request/accept/reject flow
│   │       ├── follow.rs            # Asymmetric follow management
│   │       ├── reaction.rs          # Reaction processing (OR-Set operations)
│   │       ├── profile.rs           # Profile management (LWW-Register)
│   │       ├── hashtag.rs           # Local hashtag index
│   │       └── mention.rs           # Mention resolution, notification
│   │
│   ├── ephemera-mod/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── bloom_filter.rs       # CSAM bloom filter loading, checking
│   │       ├── bloom_update.rs       # Bloom filter update protocol (3-of-5)
│   │       ├── report.rs            # Content report submission
│   │       ├── quorum.rs            # Quorum voting protocol
│   │       ├── tombstone.rs         # Community tombstone generation/verification
│   │       ├── block_mute.rs        # Local block/mute lists
│   │       └── geofence.rs          # IP geofencing for sanctioned jurisdictions
│   │
│   ├── ephemera-events/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── bus.rs               # Event bus (tokio broadcast channels)
│   │       ├── event_types.rs       # All internal event type definitions
│   │       └── subscriber.rs        # Typed subscriber helpers
│   │
│   ├── ephemera-config/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── config.rs            # NodeConfig struct, ResourceProfile
│   │       ├── loader.rs            # Layered loading: defaults -> file -> env -> CLI
│   │       ├── defaults.rs          # Default configuration values
│   │       └── resource_profile.rs  # Embedded vs Standalone profiles
│   │
│   ├── ephemera-node/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs               # EphemeraNode: composition root
│   │       ├── builder.rs           # NodeBuilder (fluent API for wiring)
│   │       ├── rpc/
│   │       │   ├── mod.rs           # JSON-RPC 2.0 dispatcher
│   │       │   ├── identity.rs      # identity.* handlers
│   │       │   ├── posts.rs         # posts.* handlers
│   │       │   ├── social.rs        # social.* handlers
│   │       │   ├── messages.rs      # messages.* handlers
│   │       │   ├── media.rs         # media.* handlers
│   │       │   ├── moderation.rs    # moderation.* handlers
│   │       │   ├── meta.rs          # meta.* handlers
│   │       │   └── feed.rs          # feed.* handlers
│   │       ├── lifecycle.rs         # Startup, shutdown, signal handling
│   │       ├── tasks.rs             # Background task spawning (GC, sync, etc.)
│   │       └── health.rs            # Health check, peer count, sync status
│   │
│   ├── ephemera-client/
│   │   ├── Cargo.toml
│   │   ├── tauri.conf.json           # Tauri 2.x configuration
│   │   ├── build.rs                  # Tauri build script
│   │   ├── src/
│   │   │   ├── main.rs              # Tauri entry point
│   │   │   ├── commands.rs          # Tauri invoke() command bridge
│   │   │   ├── events.rs           # Backend -> frontend event forwarding
│   │   │   └── state.rs            # Tauri managed state (EphemeraNode handle)
│   │   └── src-ui/                   # SolidJS frontend (separate build)
│   │       ├── package.json
│   │       ├── tsconfig.json
│   │       ├── vite.config.ts
│   │       ├── index.html
│   │       └── src/
│   │           ├── App.tsx
│   │           └── ...
│   │
│   ├── ephemera-cli/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs              # CLI entry point
│   │       └── commands.rs          # Subcommand handlers
│   │
│   └── ephemera-test-utils/
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── mock_network.rs      # In-memory network simulation
│           ├── mock_transport.rs    # Mock AnonTransport implementation
│           ├── mock_store.rs        # In-memory storage backend
│           ├── fixtures.rs          # Test data generators
│           ├── identity.rs         # Test identity/keypair generators
│           ├── assertions.rs       # Custom assertion helpers
│           └── clock.rs            # Controllable mock clock for HLC testing
│
├── tests/
│   ├── integration/
│   │   ├── post_lifecycle.rs        # Post create -> propagate -> expire
│   │   ├── connection_flow.rs       # Request -> accept -> feed visibility
│   │   ├── message_exchange.rs      # X3DH -> ratchet -> send/receive
│   │   ├── media_pipeline.rs        # Image upload -> chunk -> retrieve
│   │   ├── gc_expiry.rs            # TTL enforcement end-to-end
│   │   └── moderation.rs           # Report -> quorum -> tombstone
│   └── simulation/
│       ├── network_sim.rs           # Multi-node network simulation harness
│       ├── gossip_propagation.rs    # Verify gossip reaches all peers
│       ├── partition_recovery.rs    # Network partition -> heal -> resync
│       └── churn.rs                # Node join/leave churn resilience
│
└── xtask/
    ├── Cargo.toml
    └── src/
        └── main.rs                  # Custom build tasks (proto gen, bloom filter pack, etc.)
```

---

## 3. Workspace-Level Cargo.toml

```toml
[workspace]
resolver = "2"
members = [
    "crates/ephemera-types",
    "crates/ephemera-crypto",
    "crates/ephemera-protocol",
    "crates/ephemera-transport",
    "crates/ephemera-dht",
    "crates/ephemera-gossip",
    "crates/ephemera-store",
    "crates/ephemera-crdt",
    "crates/ephemera-post",
    "crates/ephemera-message",
    "crates/ephemera-media",
    "crates/ephemera-abuse",
    "crates/ephemera-social",
    "crates/ephemera-mod",
    "crates/ephemera-events",
    "crates/ephemera-config",
    "crates/ephemera-node",
    "crates/ephemera-client",
    "crates/ephemera-cli",
    "crates/ephemera-test-utils",
    "xtask",
]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.85"
license = "AGPL-3.0-or-later"
repository = "https://github.com/ephemera-social/ephemera"
authors = ["Ephemera Contributors"]

[workspace.dependencies]
# ── Internal crates ──────────────────────────────────────────
ephemera-types       = { path = "crates/ephemera-types" }
ephemera-crypto      = { path = "crates/ephemera-crypto" }
ephemera-protocol    = { path = "crates/ephemera-protocol" }
ephemera-transport   = { path = "crates/ephemera-transport" }
ephemera-dht         = { path = "crates/ephemera-dht" }
ephemera-gossip      = { path = "crates/ephemera-gossip" }
ephemera-store       = { path = "crates/ephemera-store" }
ephemera-crdt        = { path = "crates/ephemera-crdt" }
ephemera-post        = { path = "crates/ephemera-post" }
ephemera-message     = { path = "crates/ephemera-message" }
ephemera-media       = { path = "crates/ephemera-media" }
ephemera-abuse       = { path = "crates/ephemera-abuse" }
ephemera-social      = { path = "crates/ephemera-social" }
ephemera-mod         = { path = "crates/ephemera-mod" }
ephemera-events      = { path = "crates/ephemera-events" }
ephemera-config      = { path = "crates/ephemera-config" }
ephemera-node        = { path = "crates/ephemera-node" }
ephemera-test-utils  = { path = "crates/ephemera-test-utils" }

# ── Async runtime ────────────────────────────────────────────
tokio = { version = "1.43", features = ["full"] }

# ── Networking ───────────────────────────────────────────────
iroh          = "0.32"
iroh-gossip   = "0.32"
arti-client   = { version = "0.25", optional = true }
arti-hyper    = { version = "0.25", optional = true }

# ── Cryptography ─────────────────────────────────────────────
ed25519-dalek      = { version = "2.1", features = ["serde", "batch", "hazmat"] }
x25519-dalek       = { version = "2.0", features = ["serde", "static_secrets"] }
chacha20poly1305   = "0.10"
blake3             = "1.5"
argon2             = "0.5"
hkdf               = "0.12"
sha2               = "0.10"
zeroize            = { version = "1.8", features = ["derive"] }
secrecy            = "0.10"
rand               = "0.8"
rand_core          = "0.6"
equihash           = "0.2"
bip39              = { version = "2.1", features = ["english"] }
bech32             = "0.11"
ssss               = "0.3"

# ── Serialization ────────────────────────────────────────────
serde       = { version = "1.0", features = ["derive"] }
serde_json  = "1.0"
prost       = "0.13"
prost-build = "0.13"
ciborium    = "0.2"
bincode     = "2.0"
toml        = "0.8"

# ── Storage ──────────────────────────────────────────────────
fjall       = "3.1"
rusqlite    = { version = "0.32", features = ["bundled", "column_decltype"] }

# ── Time ─────────────────────────────────────────────────────
uhlc = "0.8"

# ── CRDTs ────────────────────────────────────────────────────
crdts = "7.1"

# ── Image processing ────────────────────────────────────────
image        = { version = "0.25", default-features = false, features = ["webp", "png", "jpeg"] }
kamadak-exif = "0.5"
blurhash     = "0.2"

# ── Compression ──────────────────────────────────────────────
lz4_flex = "0.11"

# ── Desktop application ─────────────────────────────────────
tauri        = { version = "2.2", features = [] }
tauri-build  = "2.2"

# ── CLI ──────────────────────────────────────────────────────
clap = { version = "4.5", features = ["derive"] }

# ── Observability ────────────────────────────────────────────
tracing            = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }

# ── Error handling ───────────────────────────────────────────
thiserror = "2.0"
anyhow    = "1.0"

# ── Misc ─────────────────────────────────────────────────────
bytes     = "1.9"
hex       = "0.4"
base64    = "0.22"
derive_more = { version = "1.0", features = ["display", "from", "into"] }
cfg-if    = "1.0"
parking_lot = "0.12"

# ── Testing ──────────────────────────────────────────────────
tempfile    = "3.14"
proptest    = "1.5"
test-log    = "0.2"
tokio-test  = "0.4"
criterion   = { version = "0.5", features = ["async_tokio"] }
mockall     = "0.13"
assert_matches = "1.5"

[workspace.lints.rust]
unsafe_code = "deny"

[workspace.lints.clippy]
pedantic = { level = "warn", priority = -1 }
unwrap_used = "warn"
expect_used = "warn"
panic = "warn"
todo = "warn"
dbg_macro = "warn"

[profile.dev]
opt-level = 0
debug = true

[profile.release]
opt-level = 3
lto = "thin"
strip = "symbols"
codegen-units = 1

[profile.release-debug]
inherits = "release"
debug = true
strip = "none"
```

---

## 4. Crate Specifications

### 4.1 ephemera-types

**Purpose:** Shared primitive types used across the entire workspace. This is the leaf crate -- it has zero heavy dependencies. Every other crate depends on it.

**Design rule:** This crate must compile in under 3 seconds. It must not pull in any crypto, networking, storage, or async runtime dependencies. Only `serde`, `derive_more`, `thiserror`, `bytes`, and `hex` are allowed.

```toml
# crates/ephemera-types/Cargo.toml
[package]
name = "ephemera-types"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
serde        = { workspace = true }
derive_more  = { workspace = true }
thiserror    = { workspace = true }
bytes        = { workspace = true }
hex          = { workspace = true }

[dev-dependencies]
proptest     = { workspace = true }
serde_json   = { workspace = true }

[lints]
workspace = true
```

**Public types:**

```rust
// content_hash.rs
/// BLAKE3 content hash with 1-byte version prefix.
/// Format: [0x01][32 bytes BLAKE3]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentHash([u8; 33]);

// identity.rs
/// Ed25519 public key used as a pseudonym identity.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdentityKey([u8; 32]);

/// Network-layer peer identifier (distinct from IdentityKey).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PeerId([u8; 32]);

/// Ed25519 signature.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature([u8; 64]);

// timestamp.rs
/// Unix milliseconds UTC. Wraps u64 for type safety.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Timestamp(u64);

// ttl.rs
/// Validated TTL duration. Construction rejects values > MAX_TTL.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ttl(u32); // seconds

// pow.rs (data only)
/// Equihash proof-of-work stamp. Data representation only;
/// computation and verification live in ephemera-crypto.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowStamp {
    pub nonce: [u8; 32],
    pub solution: Vec<u8>,
    pub difficulty: u32,
}
```

**Constants (constants.rs):**

```rust
use std::time::Duration;

pub const MAX_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);        // 30 days
pub const EPOCH_DURATION: Duration = Duration::from_secs(24 * 60 * 60);      // 24 hours
pub const CLOCK_SKEW_TOLERANCE: Duration = Duration::from_secs(5 * 60);      // 5 minutes
pub const EPOCH_KEY_RETENTION: Duration = MAX_TTL;                            // 30 days
pub const TOMBSTONE_RETENTION_MULTIPLIER: u32 = 3;
pub const GC_INTERVAL: Duration = Duration::from_secs(60);                   // 1 minute

pub const MAX_TEXT_GRAPHEME_CLUSTERS: usize = 2_000;
pub const MAX_TEXT_WIRE_BYTES: usize = 16_384;                               // 16 KB
pub const MAX_MESSAGE_GRAPHEME_CLUSTERS: usize = 10_000;
pub const MAX_MESSAGE_WIRE_BYTES: usize = 32_768;                            // 32 KB
pub const MAX_PHOTOS_PER_POST: usize = 4;
pub const MAX_PHOTO_INPUT_BYTES: usize = 10 * 1024 * 1024;                  // 10 MB
pub const MAX_PHOTO_OUTPUT_BYTES: usize = 5 * 1024 * 1024;                  // 5 MiB
pub const MAX_TOTAL_MEDIA_BYTES: usize = 50 * 1024 * 1024;                  // 50 MiB
pub const CHUNK_SIZE: usize = 256 * 1024;                                    // 256 KiB
pub const MAX_PROFILE_METADATA_BYTES: usize = 4_096;                        // 4 KB
pub const MAX_TTL_SECONDS: u32 = 2_592_000;                                 // 30 days

pub const CONTENT_HASH_VERSION: u8 = 0x01;
```

---

### 4.2 ephemera-crypto

**Purpose:** All cryptographic operations. Signing, encryption, key derivation, content hashing, keystore, proof-of-work, mnemonic backup, Shamir recovery. This crate owns key material lifecycle -- generation, derivation, storage, zeroization.

```toml
# crates/ephemera-crypto/Cargo.toml
[package]
name = "ephemera-crypto"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
ephemera-types   = { workspace = true }
ed25519-dalek    = { workspace = true }
x25519-dalek     = { workspace = true }
chacha20poly1305 = { workspace = true }
blake3           = { workspace = true }
argon2           = { workspace = true }
hkdf             = { workspace = true }
sha2             = { workspace = true }
zeroize          = { workspace = true }
secrecy          = { workspace = true }
rand             = { workspace = true }
rand_core        = { workspace = true }
equihash         = { workspace = true }
bip39            = { workspace = true }
bech32           = { workspace = true }
ssss             = { workspace = true }
serde            = { workspace = true }
thiserror        = { workspace = true }
bytes            = { workspace = true }
cfg-if           = { workspace = true }

[dev-dependencies]
proptest        = { workspace = true }
criterion       = { workspace = true }
tempfile        = { workspace = true }

[[bench]]
name = "crypto_bench"
harness = false

[lints]
workspace = true
```

**Key modules:**

- **signing.rs**: `sign(keypair, message) -> Signature`, `verify(pubkey, message, sig) -> Result`, `batch_verify(pubkeys, messages, sigs) -> Result`. Wraps `ed25519-dalek` with `Zeroize` on key material.
- **encryption.rs**: `encrypt(key, nonce, plaintext) -> ciphertext`, `decrypt(key, nonce, ciphertext) -> plaintext`. XChaCha20-Poly1305. Nonces are 24 bytes, generated randomly per operation.
- **key_derivation.rs**: All HKDF paths from the architecture (master -> device -> session/signing, master -> pseudonym). Domain-separated with version tags. See [01_identity_crypto.md](./01_identity_crypto.md) Section 2.3 for the complete derivation path specification.
- **keystore.rs**: `Keystore::create(path, passphrase)`, `Keystore::open(path, passphrase)`, `Keystore::store_key(label, key_bytes)`, `Keystore::load_key(label) -> SecretKey`. File format: `[4-byte version][12-byte Argon2id salt][24-byte nonce][ciphertext][16-byte Poly1305 tag]`. Argon2id parameters: memory 256 MiB, iterations 3, parallelism 4.
- **pow.rs**: `compute_pow(challenge, difficulty) -> PowStamp`, `verify_pow(challenge, stamp) -> bool`. Equihash (n=144, k=5) for memory-hardness.
- **epoch_keys.rs**: `EpochKeyManager` tracks current epoch, generates keys, enforces 30-day retention, deletes expired keys with zeroization. See [01_identity_crypto.md](./01_identity_crypto.md) Section 4.3 for epoch key lifecycle.
- **bech32.rs**: `encode_address(pubkey) -> String` (eph1...), `decode_address(s) -> IdentityKey`.
- **mnemonic.rs**: `generate_mnemonic(word_count) -> Mnemonic`, `mnemonic_to_master_key(mnemonic, passphrase) -> MasterKey`. BIP-39 with `ephemera-v1` passphrase salt.
- **shamir.rs**: `split_secret(secret, threshold, shares) -> Vec<Share>`, `reconstruct_secret(shares) -> Secret`. 3-of-5 default configuration.
- **zeroize_ext.rs**: `SecretKey` wrapper that implements `Zeroize + Drop`, platform-specific `mlock` best-effort (Linux/macOS native, Windows `VirtualLock`).

---

### 4.3 ephemera-protocol

**Purpose:** Wire protocol definitions and serialization. Compiles protobuf schemas with `prost-build`. Provides the `Envelope` type, all message type enums, length-prefixed framing, LZ4 compression, and the CBOR and bincode codec wrappers.

```toml
# crates/ephemera-protocol/Cargo.toml
[package]
name = "ephemera-protocol"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
ephemera-types   = { workspace = true }
ephemera-crypto  = { workspace = true }
prost            = { workspace = true }
ciborium         = { workspace = true }
bincode          = { workspace = true }
serde            = { workspace = true }
lz4_flex         = { workspace = true }
bytes            = { workspace = true }
thiserror        = { workspace = true }

[build-dependencies]
prost-build = { workspace = true }

[dev-dependencies]
proptest = { workspace = true }

[lints]
workspace = true
```

**build.rs:**

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_files = [
        "../../proto/envelope.proto",
        "../../proto/gossip.proto",
        "../../proto/dht.proto",
        "../../proto/moderation.proto",
    ];
    let includes = ["../../proto"];

    prost_build::Config::new()
        .out_dir("src/generated")
        .compile_protos(&proto_files, &includes)?;

    // Rerun if any proto file changes
    for proto in &proto_files {
        println!("cargo:rerun-if-changed={proto}");
    }

    Ok(())
}
```

**Key types:**

```rust
// envelope.rs
pub struct Envelope {
    pub version: u32,
    pub message_type: MessageType,
    pub author_id: Option<IdentityKey>,   // None for sealed-sender DMs
    pub node_id: PeerId,
    pub payload: Bytes,
    pub signature: Option<Signature>,
    pub timestamp: Timestamp,
    pub ttl_seconds: u32,
}

// message_types.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum MessageType {
    // Content
    Post = 0x0001,
    PostTombstone = 0x0002,
    MediaChunk = 0x0003,

    // Social
    ConnectionRequest = 0x0100,
    ConnectionAccept = 0x0101,
    ConnectionReject = 0x0102,
    FollowEvent = 0x0103,
    ReactionEvent = 0x0104,
    ProfileUpdate = 0x0105,

    // Messaging
    DirectMessage = 0x0200,
    PrekeyBundle = 0x0201,

    // DHT
    DhtFindNode = 0x0300,
    DhtFindValue = 0x0301,
    DhtStore = 0x0302,
    DhtPing = 0x0303,

    // Gossip
    GossipData = 0x0400,
    GossipHave = 0x0401,
    GossipWant = 0x0402,
    AntiEntropySync = 0x0403,

    // Moderation
    ContentReport = 0x0500,
    ModerationVote = 0x0501,
    ModerationAction = 0x0502,
    BloomFilterUpdate = 0x0503,

    // System
    CapabilityHandshake = 0xFF00,
    Keepalive = 0xFF01,
}

// codec.rs
/// Length-prefixed frame: u32 big-endian length + LZ4-compressed payload.
pub struct LengthDelimitedCodec;

impl LengthDelimitedCodec {
    pub fn encode(message: &[u8], dst: &mut BytesMut) -> Result<(), CodecError>;
    pub fn decode(src: &mut BytesMut) -> Result<Option<Bytes>, CodecError>;
}
```

**Serialization strategy (three codecs for three contexts):**

| Context | Codec | Crate | Rationale |
|---------|-------|-------|-----------|
| Wire (peer-to-peer) | Protobuf | `prost` | Schema evolution, cross-language compatibility |
| Content payload | CBOR | `ciborium` | Self-describing binary, schema-flexible, decentralized protocol standard |
| On-disk storage | bincode | `bincode` | Fastest encode/decode, compact, both sides are Rust |

---

### 4.4 ephemera-transport

**Purpose:** Network connectivity. Wraps Iroh for QUIC connections, NAT traversal, and relay management. Defines the `AnonTransport` trait and provides T2 (Arti/Tor) and T3 (Iroh relay) implementations. Manages the peer connection pool and bandwidth monitoring.

```toml
# crates/ephemera-transport/Cargo.toml
[package]
name = "ephemera-transport"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
ephemera-types    = { workspace = true }
ephemera-crypto   = { workspace = true }
ephemera-protocol = { workspace = true }
iroh              = { workspace = true }
tokio             = { workspace = true }
tracing           = { workspace = true }
thiserror         = { workspace = true }
bytes             = { workspace = true }
parking_lot       = { workspace = true }

# T2 (Tor) is behind a feature flag
arti-client = { workspace = true, optional = true }
arti-hyper  = { workspace = true, optional = true }

[features]
default = ["tier-t3"]
tier-t2 = ["dep:arti-client", "dep:arti-hyper"]
tier-t3 = []

[dev-dependencies]
ephemera-test-utils = { workspace = true }
tokio-test          = { workspace = true }

[lints]
workspace = true
```

**Core trait:**

```rust
// traits.rs
use async_trait::async_trait;

#[async_trait]
pub trait AnonTransport: Send + Sync + 'static {
    /// Send a message to a destination, routed through the anonymity layer.
    async fn send(&self, dest: PeerId, message: Bytes) -> Result<(), TransportError>;

    /// Receive the next inbound message.
    async fn recv(&self) -> Result<(PeerId, Bytes), TransportError>;

    /// The privacy tier of this transport.
    fn tier(&self) -> PrivacyTier;

    /// Estimated round-trip latency for this transport tier.
    fn estimated_latency(&self) -> Duration;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrivacyTier {
    T1Stealth,   // Mixnet (future, not implemented in PoC)
    T2Private,   // Arti/Tor 3-hop circuits
    T3Fast,      // Iroh single-hop encrypted relay
}
```

**Key modules:**

- **iroh_transport.rs**: Creates and manages the `iroh::Endpoint`. Handles node identity binding, QUIC connection establishment, and the Iroh relay lifecycle.
- **tier_t3.rs**: Implements `AnonTransport` using Iroh's single-hop relay. Messages are end-to-end encrypted; the relay sees the client IP but not the content. Latency: 50-200ms.
- **tier_t2.rs**: (Feature-gated behind `tier-t2`.) Implements `AnonTransport` using Arti for 3-hop Tor circuits. The exit node connects to the Ephemera backbone. Latency: 200-800ms. Circuit rotation every 24 hours.
- **connection_pool.rs**: Manages a pool of active peer connections. Handles connection deduplication, liveness checks (keepalive), and graceful disconnection.
- **bootstrap.rs**: Hardcoded list of 5 geographically diverse bootstrap nodes. Attempts parallel connections on startup. Caches discovered peers locally for subsequent launches.
- **bandwidth.rs**: Monitors bytes sent/received per peer and globally. Enforces bandwidth limits per `ResourceProfile`.

---

### 4.5 ephemera-dht

**Purpose:** Custom TTL-aware Kademlia DHT built on Iroh QUIC. Handles routing table maintenance, iterative lookups, provider records with TTL, and specialized storage for prekey bundles and user profiles.

```toml
# crates/ephemera-dht/Cargo.toml
[package]
name = "ephemera-dht"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
ephemera-types     = { workspace = true }
ephemera-crypto    = { workspace = true }
ephemera-protocol  = { workspace = true }
ephemera-transport = { workspace = true }
tokio              = { workspace = true }
tracing            = { workspace = true }
thiserror          = { workspace = true }
parking_lot        = { workspace = true }
rand               = { workspace = true }

[dev-dependencies]
ephemera-test-utils = { workspace = true }
tokio-test          = { workspace = true }

[lints]
workspace = true
```

**Key parameters:**

```rust
pub const K_BUCKET_SIZE: usize = 20;        // Standard Kademlia k
pub const ALPHA_CONCURRENCY: usize = 3;     // Parallel lookups
pub const RECORD_TTL_MAX: Duration = Duration::from_secs(30 * 24 * 60 * 60);
pub const BUCKET_REFRESH_INTERVAL: Duration = Duration::from_secs(3600);
pub const REPUBLISH_INTERVAL: Duration = Duration::from_secs(3600);
```

**Key modules:**

- **routing_table.rs**: Kademlia k-bucket routing table. XOR-based distance metric. Each bucket holds up to `K_BUCKET_SIZE` (20) entries. Entries are evicted LRU-style when buckets are full and the least-recently-seen entry fails a liveness check.
- **rpc.rs**: Four DHT RPC operations: `FindNode(target) -> Vec<PeerInfo>`, `FindValue(key) -> Value | Vec<PeerInfo>`, `Store(key, value, ttl)`, `Ping -> Pong`. All operations are signed by the node identity.
- **ttl_records.rs**: Provider records include a TTL field. Records are automatically dropped on expiry. The Store RPC rejects records with TTL > `RECORD_TTL_MAX`.
- **iterator.rs**: Kademlia iterative closest-node lookup. Contacts `ALPHA_CONCURRENCY` (3) nodes in parallel. Converges on the k closest nodes to the target key.
- **maintenance.rs**: Background tasks for bucket refresh (random lookup in each bucket's range) and record republishing (re-store local records before they expire).

---

### 4.6 ephemera-gossip

**Purpose:** Topic-based publish/subscribe wrapping `iroh-gossip`. Implements PlumTree eager/lazy push, anti-entropy Merkle tree synchronization, and message deduplication.

```toml
# crates/ephemera-gossip/Cargo.toml
[package]
name = "ephemera-gossip"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
ephemera-types     = { workspace = true }
ephemera-crypto    = { workspace = true }
ephemera-protocol  = { workspace = true }
ephemera-transport = { workspace = true }
iroh-gossip        = { workspace = true }
tokio              = { workspace = true }
tracing            = { workspace = true }
thiserror          = { workspace = true }
blake3             = { workspace = true }
parking_lot        = { workspace = true }

[dev-dependencies]
ephemera-test-utils = { workspace = true }

[lints]
workspace = true
```

**Anti-entropy parameters:**

```rust
pub const SYNC_INTERVAL: Duration = Duration::from_secs(120);       // 2 minutes
pub const ANTI_ENTROPY_BANDWIDTH_BUDGET: usize = 50 * 1024;         // 50 KB/s
pub const SEEN_CACHE_SIZE: usize = 100_000;                          // Message ID dedup cache
pub const EAGER_PUSH_FANOUT: usize = 3;                              // PlumTree eager push to 3 peers
pub const LAZY_PUSH_INTERVAL: Duration = Duration::from_millis(500); // Lazy push IHAVE interval
```

**Key modules:**

- **topic.rs**: Topic creation, subscription, unsubscription. A topic is identified by a BLAKE3 hash of its name. Clients subscribe to pseudonym feeds and hashtag topics through their anonymous transport (never from the node identity directly).
- **plumtree.rs**: PlumTree protocol implementation. Eager push to `EAGER_PUSH_FANOUT` peers on first receipt. Lazy push (IHAVE/IWANT) to remaining peers. Adaptive tree optimization based on delivery latency.
- **anti_entropy.rs**: Two Merkle trees -- one for content, one for tombstones. Exchanged every `SYNC_INTERVAL` (120s). Diff-based synchronization: only missing entries are transferred. Bandwidth-limited to `ANTI_ENTROPY_BANDWIDTH_BUDGET`.
- **message_dedup.rs**: LRU cache of `SEEN_CACHE_SIZE` (100,000) recently-seen message IDs. Messages already in the cache are silently dropped. Prevents duplicate processing and infinite gossip loops.

---

### 4.7 ephemera-store

**Purpose:** Dual storage engine: fjall for encrypted content blobs, SQLite for relational metadata. Owns the `StorageBackend` trait, the migration system, garbage collection, time-partitioned day-directories, and epoch key sweep.

```toml
# crates/ephemera-store/Cargo.toml
[package]
name = "ephemera-store"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
ephemera-types    = { workspace = true }
ephemera-crypto   = { workspace = true }
ephemera-protocol = { workspace = true }
fjall             = { workspace = true }
rusqlite          = { workspace = true }
tokio             = { workspace = true }
tracing           = { workspace = true }
thiserror         = { workspace = true }
serde             = { workspace = true }
bincode           = { workspace = true }

[dev-dependencies]
ephemera-test-utils = { workspace = true }
tempfile            = { workspace = true }

[lints]
workspace = true
```

**StorageBackend trait:**

```rust
// traits.rs
use async_trait::async_trait;

#[async_trait]
pub trait StorageBackend: Send + Sync + 'static {
    /// Store an encrypted content blob, keyed by ContentHash.
    async fn put_content(&self, hash: &ContentHash, blob: &EncryptedBlob) -> Result<(), StoreError>;

    /// Retrieve an encrypted content blob by ContentHash.
    async fn get_content(&self, hash: &ContentHash) -> Result<Option<EncryptedBlob>, StoreError>;

    /// Delete a content blob. Returns true if the key existed.
    async fn delete_content(&self, hash: &ContentHash) -> Result<bool, StoreError>;

    /// Scan for all content expired before the given timestamp.
    async fn scan_expired(&self, before: Timestamp) -> Result<Vec<ContentHash>, StoreError>;

    /// Bulk delete all content for a given epoch.
    async fn sweep_epoch(&self, epoch_number: u64) -> Result<u64, StoreError>;
}
```

**Migration system (migration.rs):**

```rust
/// Reads SQL files from the migrations/ directory in numeric order.
/// Tracks applied migrations in a `_migrations` table within SQLite.
/// Each migration runs inside a transaction. Partially-applied migrations
/// are rolled back. Forward-only -- no down migrations.
pub struct Migrator {
    conn: rusqlite::Connection,
}

impl Migrator {
    pub fn run_pending(&mut self) -> Result<Vec<String>, MigrationError>;
    pub fn current_version(&self) -> Result<u64, MigrationError>;
}
```

**Initial migration (migrations/001_initial.sql):**

See [03_storage_data.md](./03_storage_data.md) Section 3 for the complete SQLite schema specification. The initial migration creates:

```sql
-- Posts metadata
CREATE TABLE IF NOT EXISTS posts (
    content_hash  BLOB PRIMARY KEY,   -- 33 bytes (ContentHash)
    author        BLOB NOT NULL,      -- 32 bytes (IdentityKey)
    sequence_num  INTEGER NOT NULL,
    created_at    INTEGER NOT NULL,   -- Unix millis
    expires_at    INTEGER NOT NULL,   -- Unix millis
    ttl_seconds   INTEGER NOT NULL,
    parent_hash   BLOB,               -- Reply parent
    root_hash     BLOB,               -- Thread root
    depth         INTEGER NOT NULL DEFAULT 0,
    language_hint TEXT,
    is_tombstone  INTEGER NOT NULL DEFAULT 0,
    tombstone_expires_at INTEGER      -- 3x original TTL
);

CREATE INDEX idx_posts_expires_at ON posts(expires_at);
CREATE INDEX idx_posts_author ON posts(author, created_at DESC);
CREATE INDEX idx_posts_parent ON posts(parent_hash) WHERE parent_hash IS NOT NULL;
CREATE INDEX idx_posts_root ON posts(root_hash) WHERE root_hash IS NOT NULL;

-- Social graph
CREATE TABLE IF NOT EXISTS connections (
    source        BLOB NOT NULL,      -- IdentityKey
    target        BLOB NOT NULL,      -- IdentityKey
    status        TEXT NOT NULL,      -- 'pending', 'accepted', 'rejected'
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    PRIMARY KEY (source, target)
);

CREATE TABLE IF NOT EXISTS follows (
    follower      BLOB NOT NULL,
    followee      BLOB NOT NULL,
    created_at    INTEGER NOT NULL,
    PRIMARY KEY (follower, followee)
);

-- Reactions (OR-Set style: store adds and removes)
CREATE TABLE IF NOT EXISTS reactions (
    content_hash  BLOB NOT NULL,
    reactor       BLOB NOT NULL,
    emoji         TEXT NOT NULL,
    unique_tag    BLOB NOT NULL,      -- OR-Set unique tag
    is_removed    INTEGER NOT NULL DEFAULT 0,
    timestamp     INTEGER NOT NULL,
    PRIMARY KEY (content_hash, reactor, emoji, unique_tag)
);

-- Profiles (LWW-Register: latest timestamp wins)
CREATE TABLE IF NOT EXISTS profiles (
    identity      BLOB PRIMARY KEY,
    display_name  TEXT,
    bio           TEXT,
    avatar_cid    BLOB,
    updated_at    INTEGER NOT NULL   -- HLC timestamp for LWW
);

-- Hashtag index
CREATE TABLE IF NOT EXISTS hashtag_index (
    tag           TEXT NOT NULL,
    content_hash  BLOB NOT NULL,
    created_at    INTEGER NOT NULL,
    PRIMARY KEY (tag, content_hash)
);

CREATE INDEX idx_hashtag_tag ON hashtag_index(tag, created_at DESC);

-- Peer info
CREATE TABLE IF NOT EXISTS peers (
    peer_id       BLOB PRIMARY KEY,
    addresses     TEXT NOT NULL,      -- JSON array of multiaddrs
    last_seen     INTEGER NOT NULL,
    reputation    INTEGER NOT NULL DEFAULT 0
);

-- Epoch keys
CREATE TABLE IF NOT EXISTS epoch_keys (
    epoch_number  INTEGER PRIMARY KEY,
    created_at    INTEGER NOT NULL,
    expires_at    INTEGER NOT NULL,   -- epoch end + 30 days
    is_deleted    INTEGER NOT NULL DEFAULT 0
);

-- Messages metadata
CREATE TABLE IF NOT EXISTS messages (
    message_id    BLOB PRIMARY KEY,
    recipient     BLOB NOT NULL,
    created_at    INTEGER NOT NULL,
    expires_at    INTEGER NOT NULL,
    is_delivered  INTEGER NOT NULL DEFAULT 0,
    conversation_id BLOB NOT NULL
);

CREATE INDEX idx_messages_recipient ON messages(recipient, created_at DESC);
CREATE INDEX idx_messages_conversation ON messages(conversation_id, created_at ASC);
CREATE INDEX idx_messages_expires ON messages(expires_at);

-- Migration tracking
CREATE TABLE IF NOT EXISTS _migrations (
    version       INTEGER PRIMARY KEY,
    name          TEXT NOT NULL,
    applied_at    INTEGER NOT NULL
);
```

**Key modules:**

- **fjall_backend.rs**: Implements `StorageBackend` using fjall. Key is `ContentHash` (33 bytes), value is `EncryptedBlob` serialized via bincode. Compaction filter drops entries where `expires_at < now`.
- **fjall_config.rs**: Configuration struct for fjall: write buffer size (default 64 MiB), max write buffers (3), target SST file size (64 MiB), block cache size (128 MiB), LZ4 compression.
- **gc.rs**: Background garbage collection task. Runs every `GC_INTERVAL` (60 seconds). Scans SQLite for expired posts, deletes from fjall, writes tombstones (retained 3x original TTL for propagation).
- **time_partition.rs**: Organizes fjall data directories by date (`YYYY-MM-DD/`). When all content in a day-directory has expired, the entire directory is deleted (zero-cost bulk expiry).
- **epoch_sweep.rs**: When an epoch key is deleted (30 days after epoch end), scans fjall for all content encrypted under that epoch. Deletes any remaining ciphertext. This is the cryptographic shredding enforcement layer.

---

### 4.8 ephemera-crdt

**Purpose:** CRDT implementations for convergent state: OR-Set (follows, reactions), G-Counter with decay (reputation), LWW-Register (profiles), ExpiringSet (OR-Set + TTL), BoundedCounter (rate limiting). Delta-state replication protocol for efficient sync.

```toml
# crates/ephemera-crdt/Cargo.toml
[package]
name = "ephemera-crdt"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
ephemera-types = { workspace = true }
crdts          = { workspace = true }
serde          = { workspace = true }
thiserror      = { workspace = true }

[dev-dependencies]
proptest = { workspace = true }

[lints]
workspace = true
```

**Key types:**

```rust
// expiring_set.rs
/// An OR-Set where each element has a TTL. Elements are automatically
/// considered removed after their TTL expires. Used for ephemeral
/// social graph events and time-limited reactions.
pub struct ExpiringSet<T: Ord + Clone> {
    inner: OrSet<(T, Timestamp)>,  // Element + expiry timestamp
}

impl<T: Ord + Clone> ExpiringSet<T> {
    pub fn add(&mut self, element: T, ttl: Ttl, actor: ActorId) -> Delta;
    pub fn remove(&mut self, element: &T, actor: ActorId) -> Delta;
    pub fn contains(&self, element: &T, now: Timestamp) -> bool;
    pub fn gc_expired(&mut self, now: Timestamp) -> usize;
    pub fn merge(&mut self, delta: Delta);
}

// g_counter.rs
/// G-Counter with time-based decay for reputation scoring.
/// Values decay by 50% every `decay_interval` (default 30 days).
/// This prevents reputation from accumulating indefinitely and
/// ensures fresh behavior has more weight than historical behavior.
pub struct DecayingCounter {
    counts: BTreeMap<ActorId, (u64, Timestamp)>,  // (count, last_updated)
    decay_interval: Duration,
    decay_factor: f64,  // 0.5 = halve every interval
}

// delta.rs
/// Delta-state replication: instead of sending the full CRDT state,
/// send only the operations since the last sync. Dramatically reduces
/// anti-entropy bandwidth.
pub struct DeltaBuffer {
    deltas: Vec<(Timestamp, CrdtDelta)>,
    max_buffer_size: usize,  // Discard oldest if exceeded
}
```

---

### 4.9 ephemera-post

**Purpose:** Post data model, builder, validation, threading logic, and CBOR serialization. This crate defines the `Post` struct and its associated types. Validation enforces all protocol constraints (TTL, size, PoW, signature).

```toml
# crates/ephemera-post/Cargo.toml
[package]
name = "ephemera-post"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
ephemera-types  = { workspace = true }
ephemera-crypto = { workspace = true }
serde           = { workspace = true }
ciborium        = { workspace = true }
thiserror       = { workspace = true }

[dev-dependencies]
ephemera-test-utils = { workspace = true }

[lints]
workspace = true
```

**Post struct (defined here, mirrors [ARCHITECTURE.md](./ARCHITECTURE.md) Section 6.2):**

```rust
// post.rs
pub struct Post {
    // Identity & ordering
    pub id:                  ContentHash,
    pub author:              IdentityKey,
    pub sequence_number:     u64,
    pub created_at:          Timestamp,
    pub expires_at:          Timestamp,
    pub ttl_seconds:         u32,

    // Abuse prevention
    pub pow_stamp:           PowStamp,
    pub identity_created_at: Timestamp,

    // Content
    pub body:                Option<RichText>,
    pub media:               Vec<MediaAttachment>,
    pub sensitivity:         Option<SensitivityLabel>,

    // Threading
    pub parent:              Option<ContentHash>,
    pub root:                Option<ContentHash>,
    pub depth:               u8,

    // Discovery
    pub tags:                Vec<Tag>,
    pub mentions:            Vec<MentionTag>,
    pub topic_rooms:         Vec<TopicRoomId>,
    pub language_hint:       Option<String>,
    pub audience:            Audience,

    // Integrity
    pub content_nonce:       [u8; 16],
    pub signature:           Signature,
}
```

**PostBuilder:**

```rust
// builder.rs
pub struct PostBuilder {
    body: Option<RichText>,
    media: Vec<MediaAttachment>,
    ttl: Ttl,
    parent: Option<ContentHash>,
    root: Option<ContentHash>,
    tags: Vec<Tag>,
    mentions: Vec<MentionTag>,
    sensitivity: Option<SensitivityLabel>,
    language_hint: Option<String>,
}

impl PostBuilder {
    pub fn new(ttl: Ttl) -> Self;
    pub fn body(self, text: RichText) -> Self;
    pub fn media(self, attachment: MediaAttachment) -> Self;
    pub fn reply_to(self, parent: ContentHash, root: ContentHash) -> Self;
    pub fn tag(self, tag: Tag) -> Self;
    pub fn mention(self, mention: MentionTag) -> Self;
    pub fn sensitivity(self, label: SensitivityLabel) -> Self;
    pub fn language(self, hint: String) -> Self;

    /// Build, sign, compute content hash, and attach PoW stamp.
    /// Returns a fully-formed Post ready for storage and network publication.
    pub fn build(
        self,
        keypair: &SigningKeypair,
        pow_difficulty: u32,
        clock: &HlcClock,
    ) -> Result<Post, PostBuildError>;
}
```

**Validation (validation.rs):**

```rust
/// Validates an incoming post from the network. Checks:
/// 1. TTL within bounds (1 second to MAX_TTL_SECONDS)
/// 2. TTL + created_at does not exceed now + CLOCK_SKEW_TOLERANCE
/// 3. Text body within size limits
/// 4. Media count within limits
/// 5. Content hash matches computed BLAKE3 hash
/// 6. Ed25519 signature verifies against author public key
/// 7. PoW stamp verifies at the required difficulty
/// 8. Thread depth within bounds (max 255)
pub fn validate_post(post: &Post, now: Timestamp) -> Result<(), ValidationError>;
```

---

### 4.10 ephemera-message

**Purpose:** Encrypted messaging: X3DH initial key exchange, Double Ratchet state machine, sealed-sender envelope construction, dead-drop address derivation, message padding. Separate from posts because messages have fundamentally different encryption, delivery, and lifecycle models.

```toml
# crates/ephemera-message/Cargo.toml
[package]
name = "ephemera-message"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
ephemera-types  = { workspace = true }
ephemera-crypto = { workspace = true }
serde           = { workspace = true }
thiserror       = { workspace = true }
zeroize         = { workspace = true }
bytes           = { workspace = true }

[dev-dependencies]
ephemera-test-utils = { workspace = true }

[lints]
workspace = true
```

**Double Ratchet state:**

```rust
// double_ratchet.rs
/// The Double Ratchet state for a single conversation.
/// All key material implements Zeroize and is dropped securely.
pub struct RatchetState {
    root_key: SecretKey,           // Zeroize on drop
    sending_chain_key: SecretKey,
    receiving_chain_key: SecretKey,
    sending_ratchet_key: X25519KeyPair,
    receiving_ratchet_pubkey: Option<X25519PublicKey>,
    send_count: u32,
    recv_count: u32,
    previous_send_count: u32,
    skipped_message_keys: BTreeMap<(X25519PublicKey, u32), SecretKey>,
}

impl RatchetState {
    pub fn init_sender(
        shared_secret: &SharedSecret,
        remote_ratchet_key: &X25519PublicKey,
    ) -> Self;

    pub fn init_receiver(
        shared_secret: &SharedSecret,
        our_ratchet_key: X25519KeyPair,
    ) -> Self;

    /// Encrypt a plaintext message. Advances the sending chain.
    /// The message key is zeroized immediately after encryption.
    pub fn encrypt(
        &mut self,
        plaintext: &[u8],
    ) -> Result<(RatchetHeader, Vec<u8>), RatchetError>;

    /// Decrypt an incoming message. May perform a DH ratchet step.
    /// The message key is zeroized immediately after decryption.
    pub fn decrypt(
        &mut self,
        header: &RatchetHeader,
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, RatchetError>;
}

impl Drop for RatchetState {
    fn drop(&mut self) {
        self.root_key.zeroize();
        self.sending_chain_key.zeroize();
        self.receiving_chain_key.zeroize();
        // skipped_message_keys zeroized via SecretKey::Drop
    }
}
```

**X3DH (x3dh.rs):**

```rust
/// X3DH initial key exchange. See 01_identity_crypto.md Section 5
/// for the complete protocol specification.
pub struct X3dhInitiator;

impl X3dhInitiator {
    /// Perform the initiator side of X3DH using the recipient's prekey bundle.
    /// Returns: shared secret + initial message (encrypted with the shared secret).
    pub fn initiate(
        our_identity: &IdentityKeyPair,
        their_prekey_bundle: &PrekeyBundle,
    ) -> Result<(SharedSecret, X3dhMessage), X3dhError>;
}

pub struct X3dhResponder;

impl X3dhResponder {
    /// Perform the responder side of X3DH.
    /// Returns: shared secret derived from the initial message.
    pub fn respond(
        our_identity: &IdentityKeyPair,
        our_signed_prekey: &SignedPrekeyPair,
        our_one_time_prekey: Option<&OneTimePrekeyPair>,
        message: &X3dhMessage,
    ) -> Result<SharedSecret, X3dhError>;
}
```

**Dead drop addressing (dead_drop.rs):**

```rust
/// Dead drop address derivation. The address is deterministic given the
/// shared ratchet state, so both sender and recipient can compute it
/// without coordination.
pub fn derive_dead_drop_address(
    root_key: &[u8; 32],
    epoch: u64,
    sequence: u32,
) -> ContentHash;
```

---

### 4.11 ephemera-media

**Purpose:** Media processing pipeline: EXIF metadata stripping, image resizing, WebP compression, CSAM perceptual hash checking, per-media symmetric encryption, 256 KiB chunking, BlurHash generation, BLAKE3 hash per chunk.

```toml
# crates/ephemera-media/Cargo.toml
[package]
name = "ephemera-media"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
ephemera-types  = { workspace = true }
ephemera-crypto = { workspace = true }
image           = { workspace = true }
kamadak-exif    = { workspace = true }
blurhash        = { workspace = true }
blake3          = { workspace = true }
tokio           = { workspace = true }
tracing         = { workspace = true }
thiserror       = { workspace = true }
bytes           = { workspace = true }

[features]
default = ["csam"]
csam = []  # CSAM bloom filter checking (always on for official builds)

[dev-dependencies]
ephemera-test-utils = { workspace = true }
tempfile            = { workspace = true }

[lints]
workspace = true
```

**Pipeline orchestrator:**

```rust
// pipeline.rs
pub struct MediaPipeline {
    csam_filter: Option<CsamFilter>,  // None only if "csam" feature disabled
    max_width: u32,                   // Default: 1280
    webp_quality: f32,                // Default: 80.0
    chunk_size: usize,                // Default: CHUNK_SIZE (256 KiB)
}

impl MediaPipeline {
    /// Process an image through the full pipeline.
    ///
    /// Steps:
    /// 1. Validate input size (reject > MAX_PHOTO_INPUT_BYTES)
    /// 2. Strip EXIF metadata (kamadak-exif)
    /// 3. CSAM check (BEFORE resize, on stripped but full-res image)
    ///    - If match: return MediaError::ContentBlocked (generic to user)
    /// 4. Resize (max_width, maintain aspect ratio)
    /// 5. Compress to WebP (quality 80)
    /// 6. Validate output size (<= MAX_PHOTO_OUTPUT_BYTES)
    /// 7. Generate BlurHash
    /// 8. Generate per-media symmetric key
    /// 9. Encrypt with XChaCha20-Poly1305
    /// 10. Split into CHUNK_SIZE chunks
    /// 11. BLAKE3 hash per chunk
    /// 12. Return ProcessedMedia with all chunk CIDs + metadata
    ///
    /// Image processing (steps 2-7) runs on spawn_blocking to avoid
    /// blocking the async runtime.
    pub async fn process_image(
        &self,
        input: &[u8],
    ) -> Result<ProcessedMedia, MediaError>;
}

pub struct ProcessedMedia {
    pub chunks: Vec<MediaChunk>,
    pub manifest: MediaManifest,
    pub blurhash: String,
    pub dimensions: (u32, u32),
    pub total_size: u64,
    pub encryption_key: SecretKey,  // Caller must distribute this to recipients
}

pub struct MediaChunk {
    pub cid: ContentHash,          // BLAKE3 hash of encrypted chunk data
    pub data: Bytes,               // Encrypted chunk bytes
    pub index: u32,                // Chunk sequence number
}

pub struct MediaManifest {
    pub chunk_cids: Vec<ContentHash>,
    pub total_chunks: u32,
    pub original_size: u64,
    pub mime_type: String,         // "image/webp"
}
```

---

### 4.12 ephemera-abuse

**Purpose:** Proof-of-work computation and verification (Equihash), adaptive difficulty calculation, token-bucket rate limiting, new-identity warming period enforcement, reputation scoring, and capability gating.

```toml
# crates/ephemera-abuse/Cargo.toml
[package]
name = "ephemera-abuse"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
ephemera-types  = { workspace = true }
ephemera-crypto = { workspace = true }
serde           = { workspace = true }
thiserror       = { workspace = true }
parking_lot     = { workspace = true }

[dev-dependencies]
proptest = { workspace = true }

[lints]
workspace = true
```

**Difficulty formula:**

```rust
// difficulty.rs

/// Compute the PoW difficulty for a given action.
///
/// Formula: difficulty = base * activity * content_size * relationship * reach
/// Clamped to [MIN_POW_DIFFICULTY, MAX_POW_DIFFICULTY].
pub fn compute_difficulty(params: &DifficultyParams) -> u32 {
    let base = params.base_difficulty;
    let activity = params.activity_multiplier;      // Higher if posting rapidly
    let content_size = params.content_size_factor;  // Larger content = more work
    let relationship = params.relationship_discount; // 0.0 for friends, 1.0 for strangers
    let reach = params.reach_multiplier;            // Higher for wider-audience content

    let raw = (base as f64) * activity * content_size * relationship * reach;
    raw.min(MAX_POW_DIFFICULTY as f64).max(MIN_POW_DIFFICULTY as f64) as u32
}

pub const MAX_POW_DIFFICULTY: u32 = 24;      // ~60 seconds ceiling
pub const MIN_POW_DIFFICULTY: u32 = 8;       // ~100ms floor for posts
pub const IDENTITY_POW_DIFFICULTY: u32 = 20; // ~30 seconds for identity creation
```

**Warming period:**

```rust
// warming.rs

pub struct WarmingPolicy {
    pub warming_duration: Duration,              // 7 days
    pub max_posts_per_hour_warming: u32,         // 1
    pub media_allowed_during_warming: bool,      // false
    pub dm_strangers_during_warming: bool,       // false
    pub moderation_votes_during_warming: bool,   // false
}

/// Capabilities available to an identity based on its age and reputation.
pub struct Capabilities {
    pub max_posts_per_hour: u32,
    pub media_allowed: bool,
    pub dm_strangers: bool,
    pub moderation_voting: bool,
    pub max_connections_per_hour: u32,
}

impl WarmingPolicy {
    pub fn capabilities(&self, identity_age: Duration) -> Capabilities {
        if identity_age < self.warming_duration {
            Capabilities::restricted()
        } else {
            Capabilities::full()
        }
    }
}
```

**Rate limiter (rate_limiter.rs):**

```rust
/// Token-bucket rate limiter. Each identity has a bucket per action type.
/// Tokens are replenished at a fixed rate. Actions consume tokens.
/// When the bucket is empty, the action is rejected.
pub struct RateLimiter {
    buckets: HashMap<(IdentityKey, ActionType), TokenBucket>,
}

pub enum ActionType {
    Post,
    Reply,
    Reaction,
    DirectMessage,
    Follow,
    ConnectionRequest,
}
```

---

### 4.13 ephemera-social

**Purpose:** High-level domain logic that combines posts, messages, CRDTs, storage, and events into user-facing social features: feed construction, connection management, reaction processing, hashtag indexing, profile management.

```toml
# crates/ephemera-social/Cargo.toml
[package]
name = "ephemera-social"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
ephemera-types   = { workspace = true }
ephemera-crypto  = { workspace = true }
ephemera-post    = { workspace = true }
ephemera-message = { workspace = true }
ephemera-crdt    = { workspace = true }
ephemera-store   = { workspace = true }
ephemera-events  = { workspace = true }
tokio            = { workspace = true }
tracing          = { workspace = true }
thiserror        = { workspace = true }

[dev-dependencies]
ephemera-test-utils = { workspace = true }

[lints]
workspace = true
```

**Feed construction:**

```rust
// feed.rs
pub struct FeedService {
    store: Arc<dyn StorageBackend>,
    meta: Arc<SqliteMetaStore>,
}

impl FeedService {
    /// Chronological feed from connections + own posts.
    /// Cursor-based pagination. No algorithmic ranking.
    pub async fn get_feed(
        &self,
        identity: &IdentityKey,
        cursor: Option<Timestamp>,
        limit: usize,
    ) -> Result<FeedPage, FeedError>;

    /// Discover feed: public posts from the wider network.
    /// No ranking, chronological order only.
    pub async fn get_discover(
        &self,
        cursor: Option<Timestamp>,
        limit: usize,
    ) -> Result<FeedPage, FeedError>;
}

pub struct FeedPage {
    pub posts: Vec<Post>,
    pub next_cursor: Option<Timestamp>,
    pub has_more: bool,
}
```

**Connection management (connection.rs):**

```rust
pub struct ConnectionService {
    meta: Arc<SqliteMetaStore>,
    events: Arc<EventBus>,
}

impl ConnectionService {
    /// Send a connection request. Creates a signed ConnectionRequest
    /// event and publishes it to the network.
    pub async fn request_connection(
        &self,
        our_identity: &IdentityKey,
        their_identity: &IdentityKey,
        keypair: &SigningKeypair,
    ) -> Result<(), ConnectionError>;

    /// Accept a pending connection request. Creates a signed
    /// ConnectionAccept event.
    pub async fn accept_connection(
        &self,
        our_identity: &IdentityKey,
        their_identity: &IdentityKey,
        keypair: &SigningKeypair,
    ) -> Result<(), ConnectionError>;

    /// Reject a pending connection request.
    pub async fn reject_connection(
        &self,
        our_identity: &IdentityKey,
        their_identity: &IdentityKey,
    ) -> Result<(), ConnectionError>;

    /// List all accepted connections for an identity.
    pub async fn list_connections(
        &self,
        identity: &IdentityKey,
    ) -> Result<Vec<ConnectionInfo>, ConnectionError>;
}
```

---

### 4.14 ephemera-mod

**Purpose:** Moderation subsystem: CSAM bloom filter management (loading, checking, multi-sig update protocol), content reporting, quorum voting for community moderation, community tombstone generation and verification, local block/mute lists, and IP geofencing.

```toml
# crates/ephemera-mod/Cargo.toml
[package]
name = "ephemera-mod"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
ephemera-types    = { workspace = true }
ephemera-crypto   = { workspace = true }
ephemera-post     = { workspace = true }
ephemera-abuse    = { workspace = true }
ephemera-store    = { workspace = true }
ephemera-gossip   = { workspace = true }
serde             = { workspace = true }
tracing           = { workspace = true }
thiserror         = { workspace = true }
blake3            = { workspace = true }

[features]
default = ["csam-filter"]
csam-filter = []  # Bloom filter loading + checking
geofence = []     # IP geofencing for sanctioned jurisdictions

[dev-dependencies]
ephemera-test-utils = { workspace = true }

[lints]
workspace = true
```

**Key modules:**

- **bloom_filter.rs**: Loads the ~10 MB CSAM bloom filter from a bundled binary file. Provides `check(phash) -> bool`. False positive rate: ~1e-10.
- **bloom_update.rs**: Handles `BloomFilterUpdate` messages from gossip. Verifies 3-of-5 multi-signature from authorized signers. Updates the local bloom filter atomically. Maintains a Merkle-tree-based public audit trail of all changes.
- **quorum.rs**: Implements 5-of-7 quorum voting for community moderation actions. Moderators see content but not author identity (blinded review). Votes are CRDT-based (OR-Set of moderator votes per content hash).
- **tombstone.rs**: Community tombstone generation. When a quorum is reached, generates a signed tombstone. Tombstones propagate via gossip and cause receiving nodes to delete the content.
- **block_mute.rs**: Local block and mute lists. Blocked identities: all content hidden, connection severed. Muted identities: content hidden but connection maintained.
- **geofence.rs**: (Feature-gated behind `geofence`.) IP-based geofencing for sanctioned jurisdictions. Checked at the transport layer during connection establishment.

---

### 4.15 ephemera-events

**Purpose:** Internal event bus using tokio broadcast channels. Decouples subsystems -- the transport layer emits events when content arrives, the social layer emits events when the feed changes, and the client layer subscribes for UI updates.

```toml
# crates/ephemera-events/Cargo.toml
[package]
name = "ephemera-events"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
ephemera-types = { workspace = true }
tokio          = { workspace = true }
tracing        = { workspace = true }
serde          = { workspace = true }

[lints]
workspace = true
```

**Event types:**

```rust
// event_types.rs
#[derive(Clone, Debug, Serialize)]
pub enum EphemeraEvent {
    // Content events
    PostReceived { hash: ContentHash, author: IdentityKey },
    PostExpired { hash: ContentHash },
    PostDeleted { hash: ContentHash },
    MediaChunkReceived { cid: ContentHash, index: u32 },

    // Social events
    ConnectionRequest { from: IdentityKey },
    ConnectionAccepted { peer: IdentityKey },
    ConnectionRejected { peer: IdentityKey },
    NewFollower { follower: IdentityKey },

    // Message events
    MessageReceived { conversation_id: ContentHash },
    MessageDelivered { message_id: ContentHash },

    // Moderation events
    ContentReported { hash: ContentHash },
    ContentTombstoned { hash: ContentHash },

    // Network events
    PeerConnected { peer: PeerId },
    PeerDisconnected { peer: PeerId },
    SyncComplete,

    // System events
    EpochKeyRotated { epoch: u64 },
    EpochKeyDeleted { epoch: u64 },
    GcCycleComplete { deleted_count: u64 },
}

// bus.rs
pub struct EventBus {
    sender: tokio::sync::broadcast::Sender<EphemeraEvent>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self;
    pub fn publish(&self, event: EphemeraEvent);
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<EphemeraEvent>;
}
```

---

### 4.16 ephemera-config

**Purpose:** Layered configuration loading. Defaults are compiled in. Overridden by TOML config file, then environment variables (prefixed `EPHEMERA_`), then CLI arguments. Defines `NodeConfig` and `ResourceProfile` (Embedded for Tauri, Standalone for daemon mode).

```toml
# crates/ephemera-config/Cargo.toml
[package]
name = "ephemera-config"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
ephemera-types = { workspace = true }
serde          = { workspace = true }
toml           = { workspace = true }
thiserror      = { workspace = true }

[lints]
workspace = true
```

**Resource profiles:**

```rust
// resource_profile.rs
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ResourceProfile {
    /// Embedded in Tauri. Conservative resource usage.
    Embedded {
        max_storage_bytes: u64,        // 500 MB
        max_bandwidth_kbps: u32,       // 1000 KB/s
        max_connections: usize,        // 50
        dht_participation: bool,       // false (light node)
    },
    /// Standalone daemon. Full node.
    Standalone {
        max_storage_bytes: u64,        // 10 GB
        max_bandwidth_kbps: u32,       // 10000 KB/s
        max_connections: usize,        // 500
        dht_participation: bool,       // true
    },
}
```

**Configuration loading order (loader.rs):**

```rust
/// Load configuration with layered overrides:
/// 1. Compiled-in defaults (defaults.rs)
/// 2. TOML config file (e.g., ~/.config/ephemera/config.toml)
/// 3. Environment variables (EPHEMERA_DATA_DIR, EPHEMERA_LOG_LEVEL, etc.)
/// 4. CLI arguments (only in daemon mode)
///
/// Later layers override earlier layers. Missing values fall through
/// to the previous layer.
pub fn load_config(
    config_path: Option<&Path>,
    env_prefix: &str,
) -> Result<NodeConfig, ConfigError>;
```

---

### 4.17 ephemera-node

**Purpose:** Composition root. Wires all crates together into an `EphemeraNode` that can be embedded as a library (Tauri) or run as a standalone daemon. Exposes the JSON-RPC 2.0 API surface. Owns the application lifecycle (startup, shutdown, background tasks).

```toml
# crates/ephemera-node/Cargo.toml
[package]
name = "ephemera-node"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
ephemera-types     = { workspace = true }
ephemera-crypto    = { workspace = true }
ephemera-protocol  = { workspace = true }
ephemera-transport = { workspace = true }
ephemera-dht       = { workspace = true }
ephemera-gossip    = { workspace = true }
ephemera-store     = { workspace = true }
ephemera-crdt      = { workspace = true }
ephemera-post      = { workspace = true }
ephemera-message   = { workspace = true }
ephemera-media     = { workspace = true }
ephemera-abuse     = { workspace = true }
ephemera-social    = { workspace = true }
ephemera-mod       = { workspace = true }
ephemera-events    = { workspace = true }
ephemera-config    = { workspace = true }
tokio              = { workspace = true }
serde              = { workspace = true }
serde_json         = { workspace = true }
tracing            = { workspace = true }
tracing-subscriber = { workspace = true }
thiserror          = { workspace = true }
anyhow             = { workspace = true }

[features]
default = ["embedded"]
embedded = []      # Library mode for Tauri embedding
standalone = []    # Binary daemon mode with socket listener (post-PoC)

[dev-dependencies]
ephemera-test-utils = { workspace = true }
tokio-test          = { workspace = true }

[lints]
workspace = true
```

**Node builder:**

```rust
// builder.rs
pub struct NodeBuilder {
    config: NodeConfig,
    resource_profile: ResourceProfile,
    data_dir: PathBuf,
}

impl NodeBuilder {
    pub fn new(config: NodeConfig) -> Self;
    pub fn data_dir(self, path: PathBuf) -> Self;
    pub fn resource_profile(self, profile: ResourceProfile) -> Self;

    /// Build and start the node. Returns a handle for API calls
    /// and a shutdown signal sender.
    ///
    /// Startup sequence:
    /// 1. Initialize keystore (open or create)
    /// 2. Load or generate node identity
    /// 3. Open storage (fjall + SQLite, run migrations)
    /// 4. Initialize transport (Iroh endpoint)
    /// 5. Start DHT (bootstrap)
    /// 6. Join gossip topics
    /// 7. Start background tasks (GC, anti-entropy, epoch rotation)
    /// 8. Start event bus
    /// 9. Return node handle
    pub async fn build(self) -> Result<(EphemeraNode, ShutdownHandle), NodeError>;
}
```

**EphemeraNode (lib.rs):**

```rust
pub struct EphemeraNode {
    store: Arc<dyn StorageBackend>,
    meta: Arc<SqliteMetaStore>,
    transport: Arc<dyn AnonTransport>,
    gossip: Arc<GossipService>,
    dht: Arc<DhtService>,
    social: Arc<SocialService>,
    media: Arc<MediaPipeline>,
    abuse: Arc<AbuseService>,
    moderation: Arc<ModerationService>,
    events: Arc<EventBus>,
    config: NodeConfig,
}

impl EphemeraNode {
    /// Handle a JSON-RPC 2.0 request. Returns a JSON-RPC 2.0 response.
    /// This is the primary API entry point for Tauri invoke() calls.
    ///
    /// Dispatches to the appropriate RPC handler based on the method name:
    /// - identity.* -> rpc/identity.rs
    /// - posts.*    -> rpc/posts.rs
    /// - social.*   -> rpc/social.rs
    /// - messages.* -> rpc/messages.rs
    /// - media.*    -> rpc/media.rs
    /// - moderation.* -> rpc/moderation.rs
    /// - meta.*     -> rpc/meta.rs
    /// - feed.*     -> rpc/feed.rs
    pub async fn handle_rpc(&self, request: &str) -> String;

    /// Subscribe to the event stream (for Tauri event forwarding).
    pub fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<EphemeraEvent>;
}

/// Handle for graceful shutdown.
pub struct ShutdownHandle {
    sender: tokio::sync::oneshot::Sender<()>,
}

impl ShutdownHandle {
    /// Signal the node to shut down gracefully.
    /// Waits for background tasks to complete (with a 10-second timeout).
    pub fn shutdown(self);
}
```

---

### 4.18 ephemera-client

**Purpose:** Tauri 2.x desktop application shell. Embeds `ephemera-node` as a library. Bridges the SolidJS frontend to the Rust backend via Tauri's `invoke()` mechanism. Forwards backend events to the frontend via Tauri's event system.

```toml
# crates/ephemera-client/Cargo.toml
[package]
name = "ephemera-client"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
ephemera-node   = { workspace = true, features = ["embedded"] }
ephemera-events = { workspace = true }
tauri           = { workspace = true, features = ["devtools"] }
serde           = { workspace = true }
serde_json      = { workspace = true }
tokio           = { workspace = true }
tracing         = { workspace = true }

[build-dependencies]
tauri-build = { workspace = true }

[lints]
workspace = true
```

**Tauri bridge:**

```rust
// commands.rs
/// The single Tauri command that bridges all JSON-RPC 2.0 calls.
/// The SolidJS frontend calls invoke("rpc", { request: "..." })
/// where the request is a JSON-RPC 2.0 string.
#[tauri::command]
async fn rpc(
    state: tauri::State<'_, AppState>,
    request: String,
) -> Result<String, String> {
    Ok(state.node.handle_rpc(&request).await)
}

// events.rs
/// Background task that forwards EphemeraEvent to the Tauri frontend.
pub async fn event_forwarder(
    app_handle: tauri::AppHandle,
    mut receiver: tokio::sync::broadcast::Receiver<EphemeraEvent>,
) {
    while let Ok(event) = receiver.recv().await {
        // Tauri event name matches the event variant for frontend filtering
        let _ = app_handle.emit("ephemera-event", &event);
    }
}

// state.rs
pub struct AppState {
    pub node: EphemeraNode,
    pub shutdown: Option<ShutdownHandle>,
}
```

---

### 4.19 ephemera-cli

**Purpose:** Command-line interface for interacting with a running ephemera-node daemon. Connects via JSON-RPC 2.0 over Unix domain socket (Linux/macOS) or named pipe (Windows). Post-PoC deliverable.

```toml
# crates/ephemera-cli/Cargo.toml
[package]
name = "ephemera-cli"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
ephemera-node = { workspace = true, features = ["standalone"] }
clap          = { workspace = true }
serde_json    = { workspace = true }
tokio         = { workspace = true }
tracing       = { workspace = true }
anyhow        = { workspace = true }

[lints]
workspace = true
```

---

### 4.20 ephemera-test-utils

**Purpose:** Shared test infrastructure used by all crates. Provides mock implementations of key traits, test data generators, a controllable clock, and a multi-node network simulator. This crate is never included in production builds.

```toml
# crates/ephemera-test-utils/Cargo.toml
[package]
name = "ephemera-test-utils"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
ephemera-types     = { workspace = true }
ephemera-crypto    = { workspace = true }
ephemera-protocol  = { workspace = true }
ephemera-transport = { workspace = true }
ephemera-store     = { workspace = true }
ephemera-events    = { workspace = true }
tokio              = { workspace = true }
rand               = { workspace = true }
tempfile           = { workspace = true }
parking_lot        = { workspace = true }

[lints]
workspace = true
```

**Key test utilities:**

```rust
// mock_transport.rs
/// In-memory transport that delivers messages directly between nodes
/// without real network I/O. Supports configurable latency and packet loss.
pub struct MockTransport {
    peers: Arc<DashMap<PeerId, mpsc::Sender<(PeerId, Bytes)>>>,
    latency: Duration,
    packet_loss_rate: f64,  // 0.0 = no loss, 1.0 = total loss
}

impl AnonTransport for MockTransport { /* ... */ }

// mock_store.rs
/// In-memory storage backend for fast tests.
pub struct MockStore {
    content: Arc<RwLock<HashMap<ContentHash, EncryptedBlob>>>,
}

impl StorageBackend for MockStore { /* ... */ }

// clock.rs
/// Controllable clock for testing time-dependent behavior.
/// Allows advancing time without real waiting. Thread-safe.
pub struct MockClock {
    now: Arc<AtomicU64>,  // Unix millis
}

impl MockClock {
    pub fn new(initial: Timestamp) -> Self;
    pub fn advance(&self, duration: Duration);
    pub fn set(&self, timestamp: Timestamp);
    pub fn now(&self) -> Timestamp;
}

// fixtures.rs
/// Generate a random test identity (keypair + pseudonym).
pub fn test_identity() -> (SigningKeypair, IdentityKey);

/// Generate a random test post with valid signature and PoW.
pub fn test_post(author: &SigningKeypair, ttl: Ttl) -> Post;

/// Generate a random text body within size limits.
pub fn random_text(max_graphemes: usize) -> RichText;

/// Generate N connected nodes for integration testing.
pub async fn test_network(n: usize) -> Vec<TestNode>;

// mock_network.rs
/// Connects multiple MockTransport instances into a simulated network.
/// Supports partitioning, latency injection, and message interception.
pub struct MockNetworkFabric {
    transports: Vec<Arc<MockTransport>>,
    partitions: Vec<(Vec<usize>, Vec<usize>)>,
}

impl MockNetworkFabric {
    pub fn new(node_count: usize) -> Self;
    pub fn transport_for(&self, node_index: usize) -> Arc<MockTransport>;
    pub fn partition(&mut self, group_a: &[usize], group_b: &[usize]);
    pub fn heal_all(&mut self);
    pub fn set_latency(&mut self, from: usize, to: usize, latency: Duration);
}
```

---

## 5. Dependency Graph

```
                                  ephemera-types
                                       |
                     +-----------------+--+-------------------+
                     |                 |  |                   |
                     v                 |  v                   v
              ephemera-crypto          | ephemera-crdt  ephemera-events
                     |                 |       |              |
        +-----+-----+-----+-----+     |       |              |
        |     |     |     |     |      |       |         ephemera-config
        v     v     v     v     v      |       |
      post  msg  media abuse protocol  |       |
        |     |     |     |     |      |       |
        |     |     |     |   +-+------+-------+
        |     |     |     |   |
        |     |     |     |   v
        |     |     |     | ephemera-transport
        |     |     |     |   |
        |     |     |     |   +-------+-------+
        |     |     |     |   |               |
        |     |     |     |   v               v
        |     |     |     | ephemera-dht  ephemera-gossip
        |     |     |     |   |               |
        |     |     |     |   |               |
        |     |     +-----+---+-------+-------+
        |     |           |           |
        v     v           v           v
      ephemera-store  ephemera-social  ephemera-mod
        |                 |               |
        +-----------------+-------+-------+
                                  |
                                  v
                          ephemera-node
                            |       |
                            v       v
                    ephemera-client  ephemera-cli
```

**Detailed dependency table:**

| Crate | Internal Dependencies |
|-------|----------------------|
| `ephemera-types` | (none -- leaf crate) |
| `ephemera-crypto` | types |
| `ephemera-protocol` | types, crypto |
| `ephemera-transport` | types, crypto, protocol |
| `ephemera-dht` | types, crypto, protocol, transport |
| `ephemera-gossip` | types, crypto, protocol, transport |
| `ephemera-store` | types, crypto, protocol |
| `ephemera-crdt` | types |
| `ephemera-post` | types, crypto |
| `ephemera-message` | types, crypto |
| `ephemera-media` | types, crypto |
| `ephemera-abuse` | types, crypto |
| `ephemera-social` | types, crypto, post, message, crdt, store, events |
| `ephemera-mod` | types, crypto, post, abuse, store, gossip |
| `ephemera-events` | types |
| `ephemera-config` | types |
| `ephemera-node` | ALL internal crates |
| `ephemera-client` | node, events |
| `ephemera-cli` | node |
| `ephemera-test-utils` | types, crypto, protocol, transport, store, events |

---

## 6. External Dependencies

### 6.1 Complete Dependency Justification

| Crate | Version | Used By | Purpose | Justification |
|-------|---------|---------|---------|---------------|
| `tokio` | 1.43 | All async crates | Async runtime | Industry standard, required by iroh |
| `iroh` | 0.32 | transport, dht, gossip | QUIC transport, NAT traversal | Architecture Decision #1. 90% NAT traversal, QUIC multipath, pure Rust |
| `iroh-gossip` | 0.32 | gossip | PlumTree pub/sub | Built-in gossip for iroh ecosystem |
| `arti-client` | 0.25 | transport (feature) | Tor onion routing (T2) | Architecture Decision #2. Leverages Tor anonymity set |
| `arti-hyper` | 0.25 | transport (feature) | HTTP client over Tor | Required for Arti integration |
| `ed25519-dalek` | 2.1 | crypto | Ed25519 signatures | Pure Rust, batch verification, well-audited |
| `x25519-dalek` | 2.0 | crypto | X25519 DH key exchange | Same dalek ecosystem, pure Rust |
| `chacha20poly1305` | 0.10 | crypto | XChaCha20-Poly1305 AEAD | Architecture Decision #7. No AES-NI dependency, 192-bit nonce |
| `blake3` | 1.5 | crypto, gossip, media | Content hashing | Architecture Decision #8. 3.5x faster than SHA-256, tree-hashing |
| `argon2` | 0.5 | crypto | Password KDF for keystore | Memory-hard KDF, standard for passphrase-derived keys |
| `hkdf` | 0.12 | crypto | Key derivation | HKDF-SHA256 for pseudonym derivation, session keys |
| `sha2` | 0.10 | crypto | SHA-256 (HKDF internal) | Required by HKDF |
| `zeroize` | 1.8 | crypto, message | Secure memory erasure | Mandatory for all key material. Architecture requirement |
| `secrecy` | 0.10 | crypto | Secret-wrapping types | Prevents accidental logging/display of secrets |
| `rand` | 0.8 | crypto, test-utils | CSPRNG | Standard randomness source |
| `rand_core` | 0.6 | crypto | RNG traits | Core traits for `rand` |
| `equihash` | 0.2 | crypto, abuse | Memory-hard PoW | Architecture Decision #9. Resists GPU/ASIC acceleration |
| `bip39` | 2.1 | crypto | Mnemonic backup | BIP-39 standard for human-readable key backup |
| `bech32` | 0.11 | crypto | Address encoding | `eph1` prefix addresses (Bech32m) |
| `ssss` | 0.3 | crypto | Shamir's Secret Sharing | 3-of-5 social recovery |
| `prost` | 0.13 | protocol | Protobuf serialization | Architecture Decision #11. Schema evolution for wire protocol |
| `prost-build` | 0.13 | protocol (build) | Proto compilation | Generates Rust code from .proto files |
| `ciborium` | 0.2 | protocol, post | CBOR serialization | Architecture Decision #10. Self-describing binary for content |
| `bincode` | 2.0 | protocol, store | Binary serialization | Architecture Decision #12. Fast compact on-disk format |
| `fjall` | 3.1 | store | LSM-tree KV store | Architecture Decision #3. Pure Rust, compaction filters for TTL |
| `rusqlite` | 0.32 | store | SQLite metadata store | Architecture Decision #4. Bundled SQLite, no external dependency |
| `uhlc` | 0.8 | types (via crypto) | Hybrid Logical Clocks | Architecture Decision #13. Causal ordering without wall clock sync |
| `crdts` | 7.1 | crdt | CRDT primitives | Established library for OR-Set, G-Counter, LWW-Register |
| `image` | 0.25 | media | Image decoding/encoding | Multi-format image processing. WebP, PNG, JPEG support |
| `kamadak-exif` | 0.5 | media | EXIF metadata reading | Read EXIF to strip it. Privacy requirement |
| `blurhash` | 0.2 | media | BlurHash generation | Instant image placeholders (28 chars) |
| `lz4_flex` | 0.11 | protocol | Wire compression | Fast compression for network messages. Pure Rust |
| `tauri` | 2.2 | client | Desktop app shell | Architecture Decision #14. Native webview + Rust backend |
| `tauri-build` | 2.2 | client (build) | Tauri build script | Tauri build system integration |
| `clap` | 4.5 | cli | CLI argument parsing | Standard CLI framework |
| `serde` | 1.0 | All crates | Serialization framework | Universal serialization traits |
| `serde_json` | 1.0 | node, client | JSON-RPC 2.0 | JSON serialization for API boundary |
| `toml` | 0.8 | config | TOML config parsing | Human-readable config format |
| `tracing` | 0.1 | All crates | Structured logging | Async-aware structured logging |
| `tracing-subscriber` | 0.3 | node | Log output | Console and file log formatting |
| `thiserror` | 2.0 | All crates | Error types | Derive macro for custom error enums |
| `anyhow` | 1.0 | node, cli | Error propagation | Application-level error handling |
| `bytes` | 1.9 | protocol, transport | Byte buffers | Efficient reference-counted byte buffers |
| `hex` | 0.4 | types | Hex encoding | Display formatting for hashes and keys |
| `base64` | 0.22 | crypto | Base64 encoding | Keystore export/import |
| `derive_more` | 1.0 | types | Derive macros | Display, From, Into derives for newtypes |
| `cfg-if` | 1.0 | crypto | Conditional compilation | Platform-specific code (mlock on Linux vs Windows) |
| `parking_lot` | 0.12 | transport, abuse, dht | Synchronization | Faster Mutex/RwLock than std for non-async contexts |

### 6.2 Dev-Only Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tempfile` | 3.14 | Temporary directories for storage tests |
| `proptest` | 1.5 | Property-based testing (fuzzing-like) |
| `test-log` | 0.2 | Enable tracing in tests via `#[test_log::test]` |
| `tokio-test` | 0.4 | Tokio test utilities (mock time, etc.) |
| `criterion` | 0.5 | Benchmarking framework |
| `mockall` | 0.13 | Mock trait implementations |
| `assert_matches` | 1.5 | Pattern matching assertions |

---

## 7. Feature Flags

### 7.1 Per-Crate Feature Matrix

| Crate | Feature | Default | Description |
|-------|---------|---------|-------------|
| `ephemera-transport` | `tier-t3` | yes | Iroh single-hop relay transport |
| `ephemera-transport` | `tier-t2` | no | Arti/Tor 3-hop circuit transport. Pulls in `arti-client` + `arti-hyper` (~20 MB compile-time deps) |
| `ephemera-media` | `csam` | yes | CSAM perceptual hash bloom filter checking. Always on for official builds. Can be disabled for pure library usage (no user-facing media) |
| `ephemera-mod` | `csam-filter` | yes | Bloom filter loading and checking. Mirrors `ephemera-media/csam` for the moderation layer |
| `ephemera-mod` | `geofence` | no | IP geofencing for sanctioned jurisdictions. Requires a GeoIP database |
| `ephemera-node` | `embedded` | yes | Library mode for Tauri embedding |
| `ephemera-node` | `standalone` | no | Binary daemon mode with socket listener (post-PoC) |

### 7.2 Feature Flag Rules

1. **Features must be additive.** Enabling a feature must never remove functionality. A crate compiled with all features enabled must be a superset of a crate compiled with default features.
2. **Heavy optional dependencies use feature gates.** Arti adds significant compile time and binary size. It is gated behind `tier-t2` and only pulled in when needed.
3. **The `csam` feature is always on for official distributed builds.** It can only be disabled for library consumers who do not handle user-generated media (e.g., a headless relay node). The CI pipeline tests with `csam` enabled.
4. **No `nightly` features.** The workspace compiles on stable Rust.

---

## 8. Toolchain Configuration

### 8.1 rust-toolchain.toml

```toml
[toolchain]
channel = "1.85"
components = ["rustfmt", "clippy", "rust-src"]
targets = [
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "aarch64-apple-darwin",
]
```

### 8.2 rustfmt.toml

```toml
edition = "2024"
max_width = 100
tab_spaces = 4
use_field_init_shorthand = true
use_try_shorthand = true
imports_granularity = "Crate"
group_imports = "StdExternalCrate"
```

### 8.3 clippy.toml

```toml
max-fn-params = 7
too-many-lines-threshold = 300
```

### 8.4 deny.toml (cargo-deny)

```toml
[advisories]
vulnerability = "deny"
unmaintained = "warn"
yanked = "deny"
notice = "warn"

[licenses]
unlicensed = "deny"
allow = [
    "MIT",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Zlib",
    "Unicode-3.0",
    "Unicode-DFS-2016",
    "BSL-1.0",
    "CC0-1.0",
]
copyleft = "deny"
# Exception: our own AGPL crates
[[licenses.exceptions]]
allow = ["AGPL-3.0-or-later"]
crate = "ephemera-*"

[bans]
multiple-versions = "warn"
wildcards = "deny"
deny = [
    # No OpenSSL. We use pure-Rust crypto.
    { crate = "openssl-sys", use-instead = "rustls" },
]

[sources]
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
```

---

## 9. CI/CD Pipeline

### 9.1 GitHub Actions: ci.yml

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

env:
  CARGO_TERM_COLOR: always
  RUSTFLAGS: "-Dwarnings"
  RUST_BACKTRACE: 1

jobs:
  check:
    name: Check
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: "1.85"
      - uses: Swatinem/rust-cache@v2
      - run: cargo check --workspace --all-features

  fmt:
    name: Format
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: "1.85"
          components: rustfmt
      - run: cargo fmt --all -- --check

  clippy:
    name: Clippy
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: "1.85"
          components: clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo clippy --workspace --all-features --all-targets -- -D warnings

  test:
    name: Test (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: "1.85"
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --workspace --all-features
        env:
          RUST_LOG: "ephemera=debug"

  test-no-default-features:
    name: Test (no default features)
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: "1.85"
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --workspace --no-default-features

  security-audit:
    name: Security Audit
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: rustsec/audit-check@v2
        with:
          token: ${{ secrets.GITHUB_TOKEN }}

  cargo-deny:
    name: Dependency Check
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: EmbarkStudios/cargo-deny-action@v2

  doc:
    name: Documentation
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: "1.85"
      - uses: Swatinem/rust-cache@v2
      - run: cargo doc --workspace --all-features --no-deps
        env:
          RUSTDOCFLAGS: "-D warnings"

  file-length:
    name: File Length Check
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Check no .rs file exceeds 300 lines
        run: |
          violations=0
          while IFS= read -r f; do
            lines=$(wc -l < "$f")
            if [ "$lines" -gt 300 ]; then
              echo "ERROR: $f has $lines lines (max 300)"
              violations=$((violations + 1))
            fi
          done < <(find crates/ -name '*.rs' -not -path '*/generated/*')
          if [ "$violations" -gt 0 ]; then
            echo "$violations file(s) exceed the 300-line limit"
            exit 1
          fi

  coverage:
    name: Code Coverage
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: "1.85"
          components: llvm-tools-preview
      - uses: Swatinem/rust-cache@v2
      - uses: taiki-e/install-action@cargo-llvm-cov
      - run: cargo llvm-cov --workspace --all-features --lcov --output-path lcov.info
      - uses: codecov/codecov-action@v4
        with:
          files: lcov.info

  bench:
    name: Benchmarks (compile only)
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: "1.85"
      - uses: Swatinem/rust-cache@v2
      - run: cargo bench --workspace --no-run
```

### 9.2 GitHub Actions: release.yml

```yaml
name: Release

on:
  push:
    tags: ["v*"]

jobs:
  build:
    name: Build (${{ matrix.target }})
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        include:
          - target: x86_64-unknown-linux-gnu
            os: ubuntu-latest
          - target: x86_64-pc-windows-msvc
            os: windows-latest
          - target: aarch64-apple-darwin
            os: macos-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: "1.85"
          targets: ${{ matrix.target }}
      - uses: Swatinem/rust-cache@v2

      - name: Install system deps (Linux)
        if: matrix.os == 'ubuntu-latest'
        run: |
          sudo apt-get update
          sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev

      - run: cargo install tauri-cli --version "^2"
      - run: cargo tauri build --target ${{ matrix.target }}
        working-directory: crates/ephemera-client

      - uses: actions/upload-artifact@v4
        with:
          name: ephemera-${{ matrix.target }}
          path: crates/ephemera-client/target/${{ matrix.target }}/release/bundle/
```

---

## 10. Testing Strategy

### 10.1 Test Categories

| Category | Location | Scope | Runs In CI |
|----------|----------|-------|------------|
| **Unit tests** | `crates/*/src/**/*.rs` (inline `#[cfg(test)]` modules) | Single function or module | Yes |
| **Crate integration tests** | `crates/*/tests/*.rs` | Single crate, real dependencies | Yes |
| **Workspace integration tests** | `tests/integration/*.rs` | Multiple crates wired together | Yes |
| **Simulation tests** | `tests/simulation/*.rs` | Multi-node network behavior | Yes (reduced scale) |
| **Property tests** | Inline via `proptest!` | Invariant verification | Yes |
| **Benchmarks** | `crates/*/benches/*.rs` | Performance regression detection | Compile-only in CI |

### 10.2 Unit Tests

Every public function must have at least one test. Every error path must have a test. Tests live in inline `#[cfg(test)]` modules at the bottom of each file.

```rust
// Example: crates/ephemera-types/src/ttl.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttl_rejects_over_30_days() {
        let result = Ttl::new(MAX_TTL_SECONDS + 1);
        assert!(result.is_err());
    }

    #[test]
    fn ttl_accepts_30_days() {
        let ttl = Ttl::new(MAX_TTL_SECONDS).unwrap();
        assert_eq!(ttl.as_secs(), MAX_TTL_SECONDS);
    }

    #[test]
    fn ttl_rejects_zero() {
        let result = Ttl::new(0);
        assert!(result.is_err());
    }
}
```

### 10.3 Integration Tests

Integration tests exercise complete user flows across multiple crates. They use `ephemera-test-utils` for mock infrastructure.

```rust
// tests/integration/post_lifecycle.rs
//
// Tests the complete lifecycle of a post:
// 1. Create identity
// 2. Create post (sign, PoW, hash)
// 3. Store locally (fjall + SQLite)
// 4. Publish via mock gossip
// 5. Receive on another mock node
// 6. Verify signature, validate TTL
// 7. Store on receiving node
// 8. Advance clock past TTL
// 9. GC deletes post
// 10. Verify post is gone from both fjall and SQLite

// tests/integration/connection_flow.rs
//
// Tests the mutual connection flow:
// 1. Alice creates identity
// 2. Bob creates identity
// 3. Alice sends connection request to Bob
// 4. Bob receives the request
// 5. Bob accepts
// 6. Alice's feed now includes Bob's posts
// 7. Bob's feed now includes Alice's posts

// tests/integration/message_exchange.rs
//
// Tests encrypted messaging end-to-end:
// 1. Alice publishes prekey bundle to mock DHT
// 2. Bob fetches Alice's prekey bundle
// 3. Bob initiates X3DH, sends first message
// 4. Alice receives, completes X3DH, decrypts
// 5. Alice replies (Double Ratchet)
// 6. Verify forward secrecy: old message keys are zeroed

// tests/integration/media_pipeline.rs
//
// Tests the image processing pipeline:
// 1. Load a test JPEG with EXIF data
// 2. Process through MediaPipeline
// 3. Verify EXIF is stripped
// 4. Verify output is WebP
// 5. Verify output dimensions <= 1280px wide
// 6. Verify chunks are 256 KiB each
// 7. Verify BLAKE3 hash per chunk is correct
// 8. Decrypt chunks and verify integrity

// tests/integration/gc_expiry.rs
//
// Tests TTL enforcement end-to-end:
// 1. Create posts with various TTLs (1h, 24h, 7d)
// 2. Advance MockClock past 1h TTL
// 3. Run GC
// 4. Verify 1h post is gone, others remain
// 5. Advance past 24h
// 6. Run GC
// 7. Verify 24h post is gone, 7d remains
// 8. Verify tombstones exist (3x original TTL)
```

### 10.4 Simulation Tests

Multi-node network simulations using `MockTransport` and `MockClock`. These test emergent network behavior.

```rust
// tests/simulation/network_sim.rs

/// Harness that creates N in-memory nodes connected via MockTransport.
pub struct NetworkSim {
    nodes: Vec<TestNode>,
    clock: MockClock,
    fabric: MockNetworkFabric,
}

impl NetworkSim {
    /// Create a network of N fully-connected nodes.
    pub async fn new(n: usize) -> Self;

    /// Advance simulated time by the given duration.
    pub fn advance_time(&self, duration: Duration);

    /// Wait until all pending gossip messages are delivered.
    pub async fn drain_gossip(&mut self);

    /// Partition the network: nodes in group A cannot reach group B.
    pub fn partition(&mut self, group_a: &[usize], group_b: &[usize]);

    /// Heal a partition, restoring connectivity.
    pub fn heal_partition(&mut self);
}

// tests/simulation/gossip_propagation.rs
//
// 1. Create 20-node network
// 2. Node 0 publishes a post
// 3. drain_gossip()
// 4. Assert all 20 nodes have the post

// tests/simulation/partition_recovery.rs
//
// 1. Create 10-node network
// 2. Partition into [0..5] and [5..10]
// 3. Node 0 publishes post A; Node 5 publishes post B
// 4. Verify: group A has A but not B; group B has B but not A
// 5. Heal partition
// 6. Trigger anti-entropy sync
// 7. Assert all 10 nodes have both A and B

// tests/simulation/churn.rs
//
// 1. Create 20-node network
// 2. Publish 50 posts from random nodes
// 3. Remove 5 random nodes (simulate crash)
// 4. Add 5 new nodes
// 5. Wait for sync
// 6. Assert new nodes have all 50 posts (that haven't expired)
```

### 10.5 Property-Based Tests

Property tests using `proptest` to verify invariants hold across random inputs.

```rust
// In ephemera-crypto
proptest! {
    #[test]
    fn sign_verify_roundtrip(message in any::<Vec<u8>>()) {
        let keypair = SigningKeypair::generate();
        let sig = keypair.sign(&message);
        prop_assert!(keypair.public_key().verify(&message, &sig).is_ok());
    }

    #[test]
    fn encrypt_decrypt_roundtrip(
        plaintext in any::<Vec<u8>>(),
        key in any::<[u8; 32]>(),
    ) {
        let nonce = generate_nonce();
        let ciphertext = encrypt(&key, &nonce, &plaintext).unwrap();
        let decrypted = decrypt(&key, &nonce, &ciphertext).unwrap();
        prop_assert_eq!(plaintext, decrypted);
    }
}

// In ephemera-types
proptest! {
    #[test]
    fn content_hash_deterministic(data in any::<Vec<u8>>()) {
        let hash1 = ContentHash::compute(&data);
        let hash2 = ContentHash::compute(&data);
        prop_assert_eq!(hash1, hash2);
    }

    #[test]
    fn ttl_always_within_bounds(secs in 1u32..=MAX_TTL_SECONDS) {
        let ttl = Ttl::new(secs).unwrap();
        prop_assert!(ttl.as_secs() <= MAX_TTL_SECONDS);
        prop_assert!(ttl.as_secs() >= 1);
    }
}

// In ephemera-protocol
proptest! {
    #[test]
    fn cbor_roundtrip(text in ".{0,2000}") {
        let post = test_post_with_text(&text);
        let encoded = cbor_encode(&post).unwrap();
        let decoded: Post = cbor_decode(&encoded).unwrap();
        prop_assert_eq!(post, decoded);
    }
}
```

### 10.6 Benchmarks

Performance-critical paths have Criterion benchmarks. These are compiled but not run in CI to avoid noise.

```rust
// crates/ephemera-crypto/benches/crypto_bench.rs
fn bench_ed25519_sign(c: &mut Criterion) {
    let keypair = SigningKeypair::generate();
    let message = vec![0u8; 1024];
    c.bench_function("ed25519_sign_1kb", |b| {
        b.iter(|| keypair.sign(black_box(&message)))
    });
}

fn bench_ed25519_batch_verify(c: &mut Criterion) {
    let messages: Vec<_> = (0..100).map(|_| vec![0u8; 256]).collect();
    let keypairs: Vec<_> = (0..100).map(|_| SigningKeypair::generate()).collect();
    let sigs: Vec<_> = keypairs.iter().zip(&messages)
        .map(|(kp, m)| kp.sign(m)).collect();
    let pubkeys: Vec<_> = keypairs.iter().map(|kp| kp.public_key()).collect();

    c.bench_function("ed25519_batch_verify_100", |b| {
        b.iter(|| batch_verify(
            black_box(&pubkeys),
            black_box(&messages),
            black_box(&sigs),
        ))
    });
}

fn bench_blake3_hash(c: &mut Criterion) {
    let data = vec![0u8; 256 * 1024]; // 256 KiB chunk
    c.bench_function("blake3_hash_256kib", |b| {
        b.iter(|| ContentHash::compute(black_box(&data)))
    });
}

fn bench_pow_compute(c: &mut Criterion) {
    let challenge = [0u8; 32];
    c.bench_function("equihash_pow_base_difficulty", |b| {
        b.iter(|| compute_pow(black_box(&challenge), MIN_POW_DIFFICULTY))
    });
}

criterion_group!(
    benches,
    bench_ed25519_sign,
    bench_ed25519_batch_verify,
    bench_blake3_hash,
    bench_pow_compute,
);
criterion_main!(benches);
```

---

## 11. Code Quality Rules

### 11.1 File Size Limit

**Maximum 300 lines per `.rs` file.** This is enforced by the `file-length` CI job (see Section 9.1).

When a file approaches 300 lines, extract a submodule. Prefer many small focused files over few large files.

**Exceptions:**
- Generated code (`*/generated/*.rs`) is exempt.
- Workspace-level integration test files (`tests/integration/*.rs`, `tests/simulation/*.rs`) are exempt.

### 11.2 Module Organization Pattern

Each crate follows this structure:

```
crate-name/
  src/
    lib.rs          # Only re-exports and module declarations. No logic.
    error.rs        # Crate-specific error types (thiserror).
    traits.rs       # Public trait definitions (if any).
    <feature>.rs    # One file per logical feature.
```

**lib.rs must contain only:**

```rust
//! Crate-level documentation.

mod error;
mod feature_a;
mod feature_b;

pub use error::*;
pub use feature_a::{PublicType, public_function};
pub use feature_b::AnotherType;
```

No logic, no trait implementations, no struct definitions in `lib.rs`. The only exception is `ephemera-node/src/lib.rs`, which contains the `EphemeraNode` struct definition (the composition root).

### 11.3 Error Handling Pattern

Every crate defines its own error enum using `thiserror`. Errors are propagated with `?`. No `unwrap()` or `expect()` in library code (enforced by Clippy lints `clippy::unwrap_used` and `clippy::expect_used`).

```rust
// crate-name/src/error.rs
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CrateNameError {
    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("storage error: {0}")]
    Storage(#[from] ephemera_store::StoreError),

    #[error("crypto error: {0}")]
    Crypto(#[from] ephemera_crypto::CryptoError),

    #[error("internal error: {0}")]
    Internal(String),
}
```

**anyhow vs thiserror:** Libraries (all crates except `ephemera-node` and `ephemera-cli`) use `thiserror` for structured error types. Applications (`ephemera-node`, `ephemera-cli`) may use `anyhow` for top-level error handling at the RPC boundary.

### 11.4 Public API Rules

1. **Minimal public surface.** Default to `pub(crate)`. Only types and functions used by other crates are `pub`.
2. **Documented.** Every `pub` item has a doc comment (`///`). Clippy `missing_docs` is a warning.
3. **No `pub` fields on structs.** Use builder patterns or constructor functions. Exception: simple data-only structs in `ephemera-types` where fields are part of the stable API.
4. **All public traits are object-safe** where possible (for `Arc<dyn Trait>` usage in the composition root). If a trait method requires `Sized`, document why.

### 11.5 Naming Conventions

| Entity | Convention | Example |
|--------|-----------|---------|
| Crate | `ephemera-{name}` (kebab-case) | `ephemera-crypto` |
| Module | `snake_case` | `key_derivation` |
| Type | `PascalCase` | `ContentHash`, `IdentityKey` |
| Trait | `PascalCase` | `StorageBackend`, `AnonTransport` |
| Function | `snake_case` | `compute_pow`, `verify_signature` |
| Constant | `UPPER_SNAKE_CASE` | `MAX_TTL`, `CHUNK_SIZE` |
| Feature flag | `kebab-case` | `tier-t2`, `csam-filter` |

### 11.6 Async Conventions

1. **Functions that do I/O are async.** Functions that do CPU-only work are sync.
2. **CPU-intensive work runs on `tokio::task::spawn_blocking`.** This includes: PoW computation (Equihash), image processing (resize, WebP encode), Argon2id key derivation, CSAM hash computation.
3. **All async functions return `Result`.** No panicking in async code.
4. **Cancellation safety.** All async functions document whether they are cancellation-safe. Functions that are not cancellation-safe are wrapped in `tokio::task::spawn` (not used in `select!` branches directly).

### 11.7 Unsafe Code

**`unsafe` is denied at the workspace level** (see `workspace.lints.rust.unsafe_code = "deny"`). If an unsafe block is ever needed (for FFI or performance-critical paths), it requires:

1. A `// SAFETY:` comment explaining why it is sound.
2. A crate-level `#![allow(unsafe_code)]` with a comment explaining why the crate needs it.
3. Review by at least two maintainers.

For the PoC, no `unsafe` code is expected. All dependencies are pure Rust.

---

## 12. Build and Development

### 12.1 Initial Workspace Setup

```bash
# Clone the repo
git clone https://github.com/ephemera-social/ephemera.git
cd ephemera

# Install Rust toolchain (from rust-toolchain.toml)
rustup show

# Install development tools
cargo install cargo-deny cargo-llvm-cov cargo-watch

# For Tauri development (Linux)
sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev

# Build everything
cargo build --workspace

# Run all tests
cargo test --workspace --all-features

# Run Clippy
cargo clippy --workspace --all-features --all-targets

# Run the Tauri desktop app (development)
cd crates/ephemera-client
cargo tauri dev
```

### 12.2 xtask Commands

The `xtask` crate provides custom build commands:

```bash
# Generate protobuf Rust code from .proto files
cargo xtask proto

# Pack the CSAM bloom filter from source hashes
cargo xtask bloom-pack --input hashes.txt --output bloom.bin

# Run the full CI suite locally
cargo xtask ci

# Generate dependency graph visualization (DOT format)
cargo xtask dep-graph
```

### 12.3 Development Workflow

```bash
# Watch mode: recompile and run tests on file change
cargo watch -x "test --workspace"

# Run tests for a single crate
cargo test -p ephemera-crypto

# Run a specific test
cargo test -p ephemera-crypto -- test_sign_verify

# Run benchmarks
cargo bench -p ephemera-crypto

# Check documentation builds without warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps

# Run cargo-deny locally
cargo deny check
```

---

## 13. Compilation Order and Build Times

### 13.1 Expected Build DAG

The workspace resolver builds crates in dependency order. The critical path (longest chain) is:

```
types -> crypto -> protocol -> transport -> dht/gossip -> [store, social, mod] -> node -> client
```

Crates at the same level in the dependency graph compile in parallel.

**Target build times (release, clean build, 8-core desktop):**

| Crate | Est. Compile Time | Notes |
|-------|------------------|-------|
| `ephemera-types` | ~3s | Minimal deps |
| `ephemera-crypto` | ~15s | Crypto crates are heavy |
| `ephemera-protocol` | ~10s | prost codegen |
| `ephemera-transport` | ~20s | iroh is large |
| `ephemera-store` | ~12s | fjall + rusqlite (bundled C compile) |
| `ephemera-node` | ~8s | Wiring only, most deps already compiled |
| `ephemera-client` | ~25s | Tauri bindings |
| **Total workspace** | **~90s** | Clean build. With warm cache: ~15s |

### 13.2 Build Optimization

1. **Workspace-level `[workspace.dependencies]`** ensures all crates use the same version of shared dependencies, preventing duplicate compilation.
2. **`resolver = "2"`** enables the v2 feature resolver, which correctly handles platform-specific and dev-only features.
3. **`lto = "thin"` in release** provides most of LTO's benefit with faster link times than `lto = "fat"`.
4. **`codegen-units = 1` in release** for maximum optimization (slower compile, faster runtime).
5. **Sccache or mold linker** recommended for development (not configured in workspace -- developer's choice via environment variables).

---

*Ephemera Rust Workspace Specification v1.0 -- 2026-03-26*
*Part of the Ephemera Unified Architecture Document series.*
