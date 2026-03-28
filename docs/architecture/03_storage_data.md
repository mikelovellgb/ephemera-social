# Ephemera: Storage & Data Specification

**Parent document:** [ARCHITECTURE.md](./ARCHITECTURE.md) Section 6
**Version:** 1.0
**Date:** 2026-03-26
**Status:** Implementation-ready specification

---

## 1. Overview

Ephemera uses a dual storage engine architecture: fjall (LSM-tree) for encrypted content blobs and SQLite for relational metadata and indexes. A time-partitioned filesystem layer enables zero-cost bulk expiry of entire day-directories. All content stored by the network is ciphertext -- nodes cannot read what they store.

This document specifies:
- Storage engine configuration and usage
- Complete data model (all content types as Rust structs)
- SQLite schema (all tables, indexes, migrations)
- TTL enforcement (4-layer strategy)
- Garbage collection and compaction
- Content addressing scheme
- Replication and consistency model
- CRDT specifications
- Anti-entropy protocol

**Cross-references:**
- Encryption of stored content: [01_identity_crypto.md](./01_identity_crypto.md) Sections 4.1-4.3
- Gossip for content dissemination: [02_network_protocol.md](./02_network_protocol.md) Section 4
- Content types and social features: [04_social_features.md](./04_social_features.md)
- Crate boundaries (ephemera-store, ephemera-crdt): [07_rust_workspace.md](./07_rust_workspace.md)

---

## 2. Storage Engines

### 2.1 fjall (Content Blob Store)

**Purpose:** Stores encrypted content blobs (posts, media chunks, message ciphertexts). All data is opaque ciphertext. fjall is an LSM-tree key-value store optimized for write-heavy workloads.

**Why fjall:**
- Pure Rust (no FFI, no C dependencies).
- LSM-tree architecture with compaction filters for TTL-based expiry.
- Write-optimized: high throughput for content ingestion from gossip.
- Compaction filters can drop expired entries during background compaction (free GC).

**Configuration:**

```rust
// ephemera-store/src/fjall_config.rs

pub struct FjallConfig {
    /// Base data directory for fjall databases
    pub data_dir: PathBuf,
    /// Write buffer size (memtable)
    pub write_buffer_size: usize,          // Default: 64 MiB
    /// Maximum number of write buffers before stalling
    pub max_write_buffers: usize,          // Default: 3
    /// Target file size for SST files
    pub target_file_size: usize,           // Default: 64 MiB
    /// Block cache size (LRU cache for frequently accessed blocks)
    pub block_cache_size: usize,           // Default: 128 MiB
    /// Enable TTL compaction filter
    pub enable_ttl_compaction: bool,       // Default: true
    /// Compression algorithm for SST files
    pub compression: CompressionType,      // Default: LZ4
}
```

**Key-value layout:**

```
Key:   ContentHash (33 bytes: 1 version + 32 BLAKE3)
Value: EncryptedBlob {
         epoch_number: u64,           // Which epoch key encrypted this
         nonce: [u8; 24],             // XChaCha20 nonce
         ciphertext: Vec<u8>,         // Encrypted content
         stored_at: u64,              // Unix timestamp when stored locally
         expires_at: u64,             // Unix timestamp for expiry
       }
```

**Compaction filter (TTL enforcement):**

```rust
impl CompactionFilter for TtlFilter {
    fn filter(&self, key: &[u8], value: &[u8]) -> CompactionFilterDecision {
        // Parse the expires_at field from the value header
        let expires_at = u64::from_be_bytes(value[32..40].try_into().unwrap());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        if expires_at < now {
            CompactionFilterDecision::Remove
        } else {
            CompactionFilterDecision::Keep
        }
    }
}
```

**Fallback:** If fjall cannot handle the write load (measured by p99 write latency > 100ms at target throughput), the `StorageBackend` trait allows swapping in RocksDB via `rusqlite` FFI. This is a contingency plan, not expected to be needed for the PoC's 100-node target.

### 2.2 SQLite (Metadata and Indexes)

**Purpose:** Stores relational metadata: post metadata, social graph, peer info, routing tables, local hashtag indexes, HLC timestamps, keystore state references.

**Why SQLite:**
- Proven, embedded, zero-configuration.
- Excellent for relational queries (social graph traversal, feed assembly, hashtag lookup).
- WAL mode for concurrent reads during writes.
- Small footprint.

**Configuration:**

```rust
// ephemera-store/src/sqlite_config.rs

pub struct SqliteConfig {
    /// Database file path
    pub db_path: PathBuf,
    /// WAL mode (recommended for concurrent access)
    pub journal_mode: JournalMode,         // Default: WAL
    /// Synchronous mode
    pub synchronous: Synchronous,          // Default: NORMAL
    /// Cache size (pages, negative = KiB)
    pub cache_size: i32,                   // Default: -8000 (8 MiB)
    /// Busy timeout (ms)
    pub busy_timeout: u32,                 // Default: 5000
    /// Enable foreign keys
    pub foreign_keys: bool,                // Default: true
    /// Auto-vacuum mode
    pub auto_vacuum: AutoVacuum,           // Default: INCREMENTAL
}
```

**Connection initialization:**

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA cache_size = -8000;
PRAGMA busy_timeout = 5000;
PRAGMA foreign_keys = ON;
PRAGMA auto_vacuum = INCREMENTAL;
PRAGMA temp_store = MEMORY;
PRAGMA mmap_size = 67108864;  -- 64 MiB memory-mapped I/O
```

### 2.3 Filesystem (Time-Partitioned Directories)

**Purpose:** Organizes content blobs into day-based directories for zero-cost bulk expiry.

```
data/
  content/
    2026-03-26/          # All content created on March 26
      <content_hash_hex>.blob
      <content_hash_hex>.blob
    2026-03-27/
      ...
    2026-03-28/
      ...
  media/
    2026-03-26/          # Media chunks created on March 26
      <chunk_hash_hex>.chunk
      ...
```

When all content in a day directory has expired (i.e., `date + 30 days < now`), the entire directory is deleted in one `fs::remove_dir_all()` operation. No per-file scanning required.

### 2.4 StorageBackend Trait

```rust
// ephemera-store/src/traits.rs

#[async_trait]
pub trait StorageBackend: Send + Sync + 'static {
    /// Store an encrypted content blob
    async fn store_content(&self, hash: &ContentHash, blob: &EncryptedBlob) -> Result<()>;

    /// Retrieve an encrypted content blob
    async fn get_content(&self, hash: &ContentHash) -> Result<Option<EncryptedBlob>>;

    /// Check if content exists
    async fn has_content(&self, hash: &ContentHash) -> Result<bool>;

    /// Delete content by hash
    async fn delete_content(&self, hash: &ContentHash) -> Result<()>;

    /// Delete all content for an epoch (cryptographic shredding support)
    async fn delete_epoch(&self, epoch_number: u64) -> Result<u64>;

    /// Get storage usage statistics
    async fn stats(&self) -> Result<StorageStats>;

    /// Run garbage collection
    async fn gc(&self) -> Result<GcReport>;
}

pub struct StorageStats {
    pub total_bytes: u64,
    pub content_count: u64,
    pub oldest_content: Option<u64>,      // Unix timestamp
    pub newest_content: Option<u64>,
    pub expired_pending_gc: u64,          // Count of expired but not yet deleted
}

pub struct GcReport {
    pub items_deleted: u64,
    pub bytes_freed: u64,
    pub duration: Duration,
    pub epochs_shredded: Vec<u64>,
}
```

---

## 3. SQLite Schema

### 3.1 Schema Version Management

```sql
CREATE TABLE schema_version (
    version     INTEGER NOT NULL,
    applied_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    description TEXT
);

-- Initial version
INSERT INTO schema_version (version, description) VALUES (1, 'Initial schema');
```

Migrations are applied sequentially on startup. Each migration is an idempotent SQL script checked against the current `schema_version`.

### 3.2 Posts Table

```sql
CREATE TABLE posts (
    -- Identity & ordering
    content_hash    BLOB NOT NULL PRIMARY KEY,   -- ContentHash (33 bytes)
    author_pubkey   BLOB NOT NULL,               -- Ed25519 public key (32 bytes)
    sequence_number INTEGER NOT NULL,             -- Per-author monotonic counter
    created_at      INTEGER NOT NULL,             -- Unix millis UTC
    expires_at      INTEGER NOT NULL,             -- created_at + ttl_ms
    ttl_seconds     INTEGER NOT NULL,             -- Original TTL

    -- Threading
    parent_hash     BLOB,                         -- Reply-to ContentHash (NULL if top-level)
    root_hash       BLOB,                         -- Thread root ContentHash
    depth           INTEGER NOT NULL DEFAULT 0,   -- 0 = top-level

    -- Content (metadata only -- actual content is in fjall)
    body_preview    TEXT,                          -- First 280 chars of plaintext (for local search)
    media_count     INTEGER NOT NULL DEFAULT 0,    -- Number of attached media items
    has_media       INTEGER NOT NULL DEFAULT 0,    -- Boolean (0/1)

    -- Discovery
    language_hint   TEXT,                          -- ISO 639-1

    -- Abuse prevention
    pow_difficulty  INTEGER NOT NULL,              -- Equihash difficulty used
    identity_age    INTEGER NOT NULL,              -- Author's identity age at creation (seconds)

    -- State
    is_tombstone    INTEGER NOT NULL DEFAULT 0,    -- Marked for deletion
    tombstone_at    INTEGER,                       -- When tombstoned
    received_at     INTEGER NOT NULL,              -- When this node received it
    epoch_number    INTEGER NOT NULL,              -- Which epoch key encrypts content

    -- Signature (for re-verification)
    signature       BLOB NOT NULL                  -- Ed25519 signature (64 bytes)
);

-- Indexes for common queries
CREATE INDEX idx_posts_author ON posts(author_pubkey, created_at DESC);
CREATE INDEX idx_posts_expires ON posts(expires_at) WHERE is_tombstone = 0;
CREATE INDEX idx_posts_parent ON posts(parent_hash) WHERE parent_hash IS NOT NULL;
CREATE INDEX idx_posts_root ON posts(root_hash) WHERE root_hash IS NOT NULL;
CREATE INDEX idx_posts_created ON posts(created_at DESC);
CREATE INDEX idx_posts_epoch ON posts(epoch_number);
CREATE INDEX idx_posts_tombstone ON posts(is_tombstone, tombstone_at)
    WHERE is_tombstone = 1;
```

### 3.3 Tags Table

```sql
CREATE TABLE post_tags (
    content_hash  BLOB NOT NULL REFERENCES posts(content_hash) ON DELETE CASCADE,
    tag           TEXT NOT NULL,            -- Normalized lowercase
    PRIMARY KEY (content_hash, tag)
);

CREATE INDEX idx_tags_lookup ON post_tags(tag, content_hash);
```

### 3.4 Mentions Table

```sql
CREATE TABLE post_mentions (
    content_hash   BLOB NOT NULL REFERENCES posts(content_hash) ON DELETE CASCADE,
    mentioned_key  BLOB NOT NULL,           -- Mentioned pseudonym's pubkey
    display_hint   TEXT,                    -- Display name hint
    byte_start     INTEGER NOT NULL,        -- Start position in body
    byte_end       INTEGER NOT NULL,        -- End position in body
    PRIMARY KEY (content_hash, mentioned_key)
);

CREATE INDEX idx_mentions_key ON post_mentions(mentioned_key, content_hash);
```

### 3.5 Media Attachments Table

```sql
CREATE TABLE media_attachments (
    content_hash   BLOB NOT NULL REFERENCES posts(content_hash) ON DELETE CASCADE,
    media_index    INTEGER NOT NULL,        -- Order within the post (0-3)
    media_type     TEXT NOT NULL,            -- 'image', 'video', 'audio'
    mime_type      TEXT NOT NULL,            -- 'image/webp', etc.
    blurhash       TEXT,                     -- ~28 char placeholder
    alt_text       TEXT,                     -- Accessibility text
    width          INTEGER,
    height         INTEGER,
    duration_ms    INTEGER,
    PRIMARY KEY (content_hash, media_index)
);

CREATE TABLE media_variants (
    content_hash   BLOB NOT NULL,
    media_index    INTEGER NOT NULL,
    quality        TEXT NOT NULL,            -- 'original', 'display', 'thumbnail'
    chunk_manifest BLOB NOT NULL,            -- BLAKE3 of manifest, references chunk hashes
    size_bytes     INTEGER NOT NULL,
    width          INTEGER,
    height         INTEGER,
    PRIMARY KEY (content_hash, media_index, quality),
    FOREIGN KEY (content_hash, media_index) REFERENCES media_attachments(content_hash, media_index) ON DELETE CASCADE
);
```

### 3.6 Media Chunks Table

```sql
CREATE TABLE media_chunks (
    chunk_hash     BLOB NOT NULL PRIMARY KEY,  -- ContentHash (33 bytes)
    manifest_hash  BLOB NOT NULL,              -- Which manifest this belongs to
    chunk_index    INTEGER NOT NULL,            -- Order in manifest
    size_bytes     INTEGER NOT NULL,            -- Chunk size (max 256 KiB)
    stored_locally INTEGER NOT NULL DEFAULT 1,  -- Whether we have the chunk in fjall
    provider_count INTEGER NOT NULL DEFAULT 0,  -- Known providers on the network
    expires_at     INTEGER NOT NULL
);

CREATE INDEX idx_chunks_manifest ON media_chunks(manifest_hash, chunk_index);
CREATE INDEX idx_chunks_expires ON media_chunks(expires_at) WHERE stored_locally = 1;
```

### 3.7 Social Graph Tables

```sql
-- Connections (mutual, bidirectional)
CREATE TABLE connections (
    local_pubkey    BLOB NOT NULL,          -- Our pseudonym
    remote_pubkey   BLOB NOT NULL,          -- Their pseudonym
    status          TEXT NOT NULL,           -- 'pending_outgoing', 'pending_incoming', 'connected', 'disconnected'
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    display_name    TEXT,                    -- Cached display name
    PRIMARY KEY (local_pubkey, remote_pubkey)
);

CREATE INDEX idx_connections_status ON connections(local_pubkey, status);

-- Follows (asymmetric, for content discovery)
CREATE TABLE follows (
    follower_pubkey  BLOB NOT NULL,
    followed_pubkey  BLOB NOT NULL,
    created_at       INTEGER NOT NULL,
    PRIMARY KEY (follower_pubkey, followed_pubkey)
);

CREATE INDEX idx_follows_followed ON follows(followed_pubkey, created_at DESC);

-- Blocks (local, not propagated)
CREATE TABLE blocks (
    blocker_pubkey  BLOB NOT NULL,
    blocked_pubkey  BLOB NOT NULL,
    created_at      INTEGER NOT NULL,
    reason          TEXT,
    PRIMARY KEY (blocker_pubkey, blocked_pubkey)
);

-- Mutes (local, not propagated)
CREATE TABLE mutes (
    muter_pubkey    BLOB NOT NULL,
    muted_pubkey    BLOB NOT NULL,
    created_at      INTEGER NOT NULL,
    expires_at      INTEGER,                -- NULL = permanent
    PRIMARY KEY (muter_pubkey, muted_pubkey)
);
```

### 3.8 Reactions Table

```sql
CREATE TABLE reactions (
    content_hash   BLOB NOT NULL,
    reactor_pubkey BLOB NOT NULL,
    emoji          TEXT NOT NULL,            -- One of: 'heart', 'laugh', 'fire', 'sad', 'thinking'
    action         TEXT NOT NULL,            -- 'add' or 'remove'
    timestamp      INTEGER NOT NULL,         -- HLC timestamp for CRDT ordering
    PRIMARY KEY (content_hash, reactor_pubkey, emoji)
);

CREATE INDEX idx_reactions_post ON reactions(content_hash);
```

### 3.9 Profiles Table

```sql
CREATE TABLE profiles (
    pubkey         BLOB NOT NULL PRIMARY KEY,
    display_name   TEXT,
    bio            TEXT,                     -- Max 160 chars
    avatar_cid     BLOB,                    -- ContentHash of avatar image
    updated_at     INTEGER NOT NULL,         -- HLC timestamp (LWW-Register)
    signature      BLOB NOT NULL,
    received_at    INTEGER NOT NULL
);
```

### 3.10 Messages Table

```sql
CREATE TABLE conversations (
    conversation_id  BLOB NOT NULL PRIMARY KEY,  -- BLAKE3(sort(our_pubkey, their_pubkey))
    our_pubkey       BLOB NOT NULL,
    their_pubkey     BLOB NOT NULL,
    last_message_at  INTEGER,
    unread_count     INTEGER NOT NULL DEFAULT 0,
    is_request       INTEGER NOT NULL DEFAULT 0,  -- Message request from stranger
    created_at       INTEGER NOT NULL
);

CREATE INDEX idx_conversations_last ON conversations(last_message_at DESC);

CREATE TABLE messages (
    message_id       BLOB NOT NULL PRIMARY KEY,   -- ContentHash of encrypted payload
    conversation_id  BLOB NOT NULL REFERENCES conversations(conversation_id),
    sender_pubkey    BLOB NOT NULL,
    received_at      INTEGER NOT NULL,
    expires_at       INTEGER NOT NULL,
    is_read          INTEGER NOT NULL DEFAULT 0,
    -- Plaintext is NOT stored in SQLite. Only in fjall as ciphertext.
    -- This table stores metadata for list/sort operations.
    body_preview     TEXT,                         -- First 100 chars (decrypted locally)
    has_media        INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_messages_conversation ON messages(conversation_id, received_at DESC);
CREATE INDEX idx_messages_expires ON messages(expires_at);
```

### 3.11 Peer and Network Tables

```sql
CREATE TABLE known_peers (
    node_id        BLOB NOT NULL PRIMARY KEY,   -- Ed25519 public key
    addrs          TEXT NOT NULL,                -- JSON array of addresses
    last_seen      INTEGER NOT NULL,
    last_latency   INTEGER,                      -- RTT in ms
    role           TEXT NOT NULL DEFAULT 'light', -- 'light', 'full', 'relay', 'bootstrap'
    failures       INTEGER NOT NULL DEFAULT 0,
    is_banned      INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_peers_last_seen ON known_peers(last_seen DESC) WHERE is_banned = 0;

CREATE TABLE dht_routing (
    node_id        BLOB NOT NULL PRIMARY KEY,
    bucket_index   INTEGER NOT NULL,             -- XOR distance prefix bit
    addrs          TEXT NOT NULL,
    last_seen      INTEGER NOT NULL,
    last_rtt       INTEGER
);

CREATE INDEX idx_dht_bucket ON dht_routing(bucket_index, last_seen DESC);
```

### 3.12 Epoch Keys Table

```sql
CREATE TABLE epoch_keys (
    epoch_number   INTEGER NOT NULL PRIMARY KEY,
    -- Key material is in the encrypted keystore, NOT in SQLite.
    -- This table tracks metadata for GC coordination.
    created_at     INTEGER NOT NULL,
    expires_at     INTEGER NOT NULL,            -- epoch_end + 30 days
    is_deleted     INTEGER NOT NULL DEFAULT 0,
    deleted_at     INTEGER,
    content_count  INTEGER NOT NULL DEFAULT 0   -- Approximate count of content using this epoch
);

CREATE INDEX idx_epoch_expires ON epoch_keys(expires_at) WHERE is_deleted = 0;
```

### 3.13 Local State Table

```sql
CREATE TABLE local_state (
    key    TEXT NOT NULL PRIMARY KEY,
    value  TEXT NOT NULL
);

-- Expected entries:
-- 'active_pseudonym_index' -> '0'
-- 'recovery_backed_up' -> 'true'/'false'
-- 'last_gc_run' -> Unix timestamp
-- 'last_anti_entropy' -> Unix timestamp
-- 'onboarding_complete' -> 'true'/'false'
-- 'transport_tier' -> 'T2'
```

---

## 4. TTL Enforcement (Four Layers)

### 4.1 Layer 1: Type System

The `Ttl` type rejects invalid durations at construction time:

```rust
// ephemera-types/src/ttl.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ttl(u32); // seconds

impl Ttl {
    pub const MAX: u32 = 30 * 24 * 60 * 60; // 2,592,000 seconds = 30 days
    pub const MIN: u32 = 60 * 60;            // 3,600 seconds = 1 hour
    pub const DEFAULT: u32 = 24 * 60 * 60;   // 86,400 seconds = 24 hours

    pub fn new(seconds: u32) -> Result<Self, TtlError> {
        if seconds > Self::MAX {
            return Err(TtlError::ExceedsMaximum { requested: seconds, max: Self::MAX });
        }
        if seconds < Self::MIN {
            return Err(TtlError::BelowMinimum { requested: seconds, min: Self::MIN });
        }
        Ok(Self(seconds))
    }

    pub fn as_secs(&self) -> u32 { self.0 }
    pub fn as_duration(&self) -> Duration { Duration::from_secs(self.0 as u64) }
}
```

Any code path that creates a TTL goes through this constructor. There is no way to create a `Ttl` value > 30 days without triggering a compilation error or runtime error.

### 4.2 Layer 2: Network Validation

Incoming messages have their TTL validated at the network boundary:

```rust
pub fn validate_incoming_ttl(envelope: &Envelope) -> Result<(), ValidationError> {
    let max_ttl = Ttl::MAX as u64;
    let clock_skew = CLOCK_SKEW_TOLERANCE.as_secs();  // 300 seconds = 5 minutes

    // Reject TTL > 30 days
    if envelope.ttl_seconds > max_ttl {
        return Err(ValidationError::TtlTooLong);
    }

    // Reject already-expired content
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let expires_at = envelope.timestamp + envelope.ttl_seconds;
    if expires_at + clock_skew < now {
        return Err(ValidationError::Expired);
    }

    // Reject content with timestamp too far in the future
    if envelope.timestamp > now + clock_skew {
        return Err(ValidationError::FutureTimestamp);
    }

    Ok(())
}
```

### 4.3 Layer 3: Storage Compaction

fjall's compaction filter drops expired entries during background compaction. This is "free" GC -- no separate scan required. Additionally, a background sweep task runs every 60 seconds:

```rust
pub async fn gc_sweep(store: &impl StorageBackend, db: &SqlitePool) -> GcReport {
    let now = unix_now();
    let mut report = GcReport::default();

    // 1. Find expired posts in SQLite
    let expired_posts = sqlx::query!(
        "SELECT content_hash, epoch_number FROM posts WHERE expires_at < ?1 AND is_tombstone = 0",
        now
    )
    .fetch_all(db)
    .await?;

    for post in &expired_posts {
        // Delete content blob from fjall
        store.delete_content(&ContentHash::from_bytes(&post.content_hash)).await?;
        report.items_deleted += 1;
    }

    // 2. Mark as tombstones (retained for 3x TTL for propagation)
    sqlx::query!(
        "UPDATE posts SET is_tombstone = 1, tombstone_at = ?1 WHERE expires_at < ?1 AND is_tombstone = 0",
        now
    )
    .execute(db)
    .await?;

    // 3. Delete old tombstones (retained 3x original TTL)
    let old_tombstones = sqlx::query!(
        "DELETE FROM posts WHERE is_tombstone = 1 AND tombstone_at + (ttl_seconds * 3) < ?1",
        now
    )
    .execute(db)
    .await?;
    report.items_deleted += old_tombstones.rows_affected();

    // 4. Delete expired messages
    let expired_messages = sqlx::query!(
        "DELETE FROM messages WHERE expires_at < ?1",
        now
    )
    .execute(db)
    .await?;
    report.items_deleted += expired_messages.rows_affected();

    // 5. Delete expired media chunks
    let expired_chunks = sqlx::query!(
        "SELECT chunk_hash FROM media_chunks WHERE expires_at < ?1 AND stored_locally = 1",
        now
    )
    .fetch_all(db)
    .await?;

    for chunk in &expired_chunks {
        store.delete_content(&ContentHash::from_bytes(&chunk.chunk_hash)).await?;
    }

    sqlx::query!("DELETE FROM media_chunks WHERE expires_at < ?1", now)
        .execute(db)
        .await?;

    // 6. Delete empty day directories
    delete_expired_day_dirs(&store.data_dir()).await?;

    report
}
```

**Day directory cleanup:**

```rust
async fn delete_expired_day_dirs(data_dir: &Path) -> Result<()> {
    let now = Utc::now().date_naive();
    let cutoff = now - chrono::Duration::days(31); // 30 days + 1 day buffer

    for entry in std::fs::read_dir(data_dir.join("content"))? {
        let entry = entry?;
        if let Some(dir_name) = entry.file_name().to_str() {
            if let Ok(dir_date) = NaiveDate::parse_from_str(dir_name, "%Y-%m-%d") {
                if dir_date < cutoff {
                    std::fs::remove_dir_all(entry.path())?;
                }
            }
        }
    }
    Ok(())
}
```

### 4.4 Layer 4: Cryptographic Shredding

When an epoch key is deleted (30 days after the epoch ends), all content encrypted under that epoch becomes permanently unrecoverable:

```rust
pub async fn shred_epoch(
    epoch_number: u64,
    store: &impl StorageBackend,
    db: &SqlitePool,
    keystore: &mut Keystore,
) -> Result<u64> {
    // 1. Delete the epoch key from the keystore
    keystore.delete_epoch_key(epoch_number)?;

    // 2. Sweep for any remaining content from this epoch
    let deleted = store.delete_epoch(epoch_number).await?;

    // 3. Update metadata
    sqlx::query!(
        "UPDATE epoch_keys SET is_deleted = 1, deleted_at = ?1 WHERE epoch_number = ?2",
        unix_now(), epoch_number
    )
    .execute(db)
    .await?;

    // 4. Clean up post metadata for this epoch
    sqlx::query!(
        "DELETE FROM posts WHERE epoch_number = ?1",
        epoch_number as i64
    )
    .execute(db)
    .await?;

    Ok(deleted)
}
```

---

## 5. Data Model (Rust Structs)

### 5.1 Post

```rust
// ephemera-post/src/lib.rs

pub struct Post {
    // Identity & ordering
    pub id:                  ContentHash,        // BLAKE3(serialized CBOR payload)
    pub author:              IdentityKey,        // Ed25519 pseudonym pubkey
    pub sequence_number:     u64,                // Per-author monotonic counter
    pub created_at:          Timestamp,          // u64 Unix millis UTC
    pub expires_at:          Timestamp,          // created_at + ttl_seconds * 1000
    pub ttl_seconds:         Ttl,                // Validated: max 30 days

    // Abuse prevention
    pub pow_stamp:           PowStamp,           // Equihash proof
    pub identity_created_at: Timestamp,          // For warming period checks

    // Content
    pub body:                Option<RichText>,   // Constrained Markdown (max 2000 grapheme clusters, 16 KB)
    pub media:               Vec<MediaAttachment>, // Max 4 photos

    // Sensitivity
    pub sensitivity:         Option<SensitivityLabel>,

    // Threading
    pub parent:              Option<ContentHash>, // Reply-to
    pub root:                Option<ContentHash>, // Thread root
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

### 5.2 Media Types

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
    pub quality:    Quality,        // Original, Display, Thumbnail
    pub cid:        ContentHash,    // BLAKE3 of encrypted+chunked blob
    pub size_bytes: u64,
    pub width:      Option<u32>,
    pub height:     Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub enum MediaType { Image, Video, Audio }

#[derive(Debug, Clone, Copy)]
pub enum Quality { Original, Display, Thumbnail }
```

### 5.3 Social Graph Events

```rust
pub struct FollowEvent {
    pub source:     IdentityKey,
    pub target:     IdentityKey,
    pub action:     FollowAction,        // Follow or Unfollow
    pub timestamp:  Timestamp,           // HLC timestamp
    pub signature:  Signature,
}

pub enum FollowAction { Follow, Unfollow }

pub struct ReactionEvent {
    pub target:     ContentHash,         // Post being reacted to
    pub reactor:    IdentityKey,
    pub emoji:      ReactionEmoji,       // Constrained set
    pub action:     ReactionAction,      // Add or Remove
    pub timestamp:  Timestamp,
    pub signature:  Signature,
}

pub enum ReactionEmoji { Heart, Laugh, Fire, Sad, Thinking }
pub enum ReactionAction { Add, Remove }

pub struct ProfileUpdate {
    pub author:       IdentityKey,
    pub display_name: Option<String>,    // Max 30 chars
    pub bio:          Option<String>,    // Max 160 chars
    pub avatar_cid:   Option<ContentHash>,
    pub timestamp:    Timestamp,         // HLC (LWW-Register keyed on this)
    pub signature:    Signature,
}
```

### 5.4 Message Types

```rust
pub struct MessageEnvelope {
    pub recipient:      IdentityKey,     // Recipient's pseudonym pubkey
    pub sender_sealed:  Vec<u8>,         // EMPTY on wire -- sender inside ciphertext
    pub ciphertext:     Vec<u8>,         // Double Ratchet encrypted content
    pub timestamp:      Timestamp,
    pub ttl_seconds:    Ttl,             // Max 30 days, default 14 days
    pub pow_stamp:      PowStamp,        // Difficulty varies by relationship
}

/// Plaintext message content (inside the encrypted payload)
pub struct MessageContent {
    pub sender:        IdentityKey,      // Revealed only to recipient
    pub body:          Option<String>,   // Max 10,000 grapheme clusters, 32 KB
    pub media:         Option<MediaAttachment>, // Max 1 photo or 1 video
    pub reply_to:      Option<ContentHash>,     // In-conversation threading
}
```

---

## 6. Content Addressing

### 6.1 ContentHash

```rust
pub struct ContentHash([u8; 33]); // 1 byte version + 32 bytes BLAKE3

impl ContentHash {
    pub const VERSION_1: u8 = 0x01;
    pub const NULL: Self = Self([0u8; 33]);  // Version 0x00 = null

    pub fn compute(payload: &[u8]) -> Self {
        let hash = blake3::hash(payload);
        let mut bytes = [0u8; 33];
        bytes[0] = Self::VERSION_1;
        bytes[1..].copy_from_slice(hash.as_bytes());
        Self(bytes)
    }

    pub fn version(&self) -> u8 { self.0[0] }
    pub fn hash_bytes(&self) -> &[u8; 32] { self.0[1..33].try_into().unwrap() }
    pub fn is_null(&self) -> bool { self.0[0] == 0x00 }
    pub fn as_bytes(&self) -> &[u8; 33] { &self.0 }

    pub fn to_hex(&self) -> String {
        hex::encode(&self.0)
    }
}
```

### 6.2 What Gets Hashed

- **Posts:** BLAKE3 of the CBOR-serialized post payload (including content_nonce, excluding signature).
- **Media chunks:** BLAKE3 of the raw encrypted chunk bytes.
- **Media manifests:** BLAKE3 of the serialized manifest (list of chunk hashes).
- **Profiles:** BLAKE3 of the CBOR-serialized profile (excluding signature).

The `content_nonce` field in posts prevents hash collision attacks where an attacker crafts a different post with the same hash.

---

## 7. Replication and Consistency

### 7.1 Consistency Model

Ephemera is an AP system (available + partition-tolerant, eventually consistent).

### 7.2 Replication Targets

| Content Type | Replicas (R) | Re-replicate Threshold | Notes |
|-------------|-------------|----------------------|-------|
| Text-only posts | 10 | Below 5 | Cheap to replicate, high availability |
| Media chunks | 5 | Below 3 | Rarest-first replication strategy |
| DHT records | k=20 | Standard Kademlia | Stored on k closest nodes |
| Tombstones | Flood | N/A | Propagated to all connected peers via gossip |

### 7.3 Consistency Tiers

**Causal consistency (thread-structured content only):**
- Replies carry a `parent_hash` dependency.
- A reply is displayed only after its parent is available.
- If the parent is missing: buffer for 30 seconds (it may arrive via gossip).
- After 30 seconds: active fetch from peers (DHT content lookup).
- After 5 minutes: display placeholder ("[original post expired]").

**Eventual consistency with CRDTs (everything else):**
- Reactions: OR-Set per post per emoji.
- Follows: OR-Set (add-wins semantics).
- Profiles: LWW-Register keyed on HLC timestamp.
- Reputation counters: G-Counter with time-based decay.

**Protocol-enforced determinism:**
- TTL expiration: deterministic based on `created_at + ttl_seconds`.
- Tombstones: signed by the original author, deterministic application.

---

## 8. CRDT Specifications

### 8.1 ExpiringSet (Custom CRDT)

An OR-Set where every element has a TTL. Elements are automatically removed when their TTL expires.

```rust
pub struct ExpiringSet<T: Ord + Clone> {
    elements: BTreeMap<T, ExpiringEntry>,
    tombstones: BTreeMap<T, Timestamp>,   // For remove-wins on concurrent add/remove
}

struct ExpiringEntry {
    added_at: Timestamp,           // HLC timestamp
    expires_at: Timestamp,         // added_at + ttl
    unique_tag: u128,              // Unique tag for OR-Set semantics
}

impl<T: Ord + Clone> ExpiringSet<T> {
    pub fn add(&mut self, element: T, ttl: Duration, clock: &HLC) -> Delta<T> {
        let now = clock.new_timestamp();
        let entry = ExpiringEntry {
            added_at: now,
            expires_at: now + ttl,
            unique_tag: rand::random(),
        };
        self.elements.insert(element.clone(), entry);
        self.tombstones.remove(&element);
        Delta::Add(element, now, ttl)
    }

    pub fn remove(&mut self, element: &T, clock: &HLC) -> Delta<T> {
        let now = clock.new_timestamp();
        self.elements.remove(element);
        self.tombstones.insert(element.clone(), now);
        Delta::Remove(element.clone(), now)
    }

    pub fn gc_expired(&mut self) {
        let now = SystemTime::now();
        self.elements.retain(|_, entry| entry.expires_at > now);
    }

    pub fn merge(&mut self, other: &Self) {
        for (element, entry) in &other.elements {
            match self.elements.get(element) {
                Some(existing) if existing.added_at >= entry.added_at => {},  // Ours is newer
                _ => {
                    // Check if we have a newer tombstone
                    if let Some(&tombstone_ts) = self.tombstones.get(element) {
                        if tombstone_ts > entry.added_at {
                            continue;  // Tombstone wins
                        }
                    }
                    self.elements.insert(element.clone(), entry.clone());
                }
            }
        }
    }
}
```

### 8.2 BoundedCounter (Custom CRDT)

A G-Counter with time-based decay, used for reputation scores.

```rust
pub struct BoundedCounter {
    counts: HashMap<IdentityKey, Vec<CountEntry>>,
    decay_half_life: Duration,      // Default: 30 days
}

struct CountEntry {
    value: u64,
    timestamp: Timestamp,
}

impl BoundedCounter {
    pub fn increment(&mut self, node: &IdentityKey, clock: &HLC) -> Delta {
        let entry = CountEntry {
            value: 1,
            timestamp: clock.new_timestamp(),
        };
        self.counts.entry(node.clone()).or_default().push(entry);
        Delta::Increment(node.clone(), entry.timestamp)
    }

    pub fn value(&self) -> f64 {
        let now = SystemTime::now();
        self.counts.values().flatten().map(|entry| {
            let age = now.duration_since(entry.timestamp).unwrap_or_default();
            let decay = 0.5f64.powf(age.as_secs_f64() / self.decay_half_life.as_secs_f64());
            entry.value as f64 * decay
        }).sum()
    }

    pub fn merge(&mut self, other: &Self) {
        for (node, entries) in &other.counts {
            let local = self.counts.entry(node.clone()).or_default();
            for entry in entries {
                if !local.iter().any(|e| e.timestamp == entry.timestamp) {
                    local.push(entry.clone());
                }
            }
        }
    }
}
```

### 8.3 Standard CRDTs (from `crdts` crate)

| CRDT | Use Case | Crate Type |
|------|----------|------------|
| OR-Set | Reactions per post, follow sets | `crdts::orswot::Orswot` |
| G-Counter | Reputation (with custom decay wrapper) | `crdts::gcounter::GCounter` |
| LWW-Register | Profile fields (display_name, bio, avatar) | `crdts::lwwreg::LWWReg` |

---

## 9. Anti-Entropy Protocol

### 9.1 Overview

Anti-entropy runs every 120 seconds between connected peers. It uses Merkle trees to efficiently identify divergence.

### 9.2 Merkle Trees

Two separate Merkle trees are maintained:

1. **Content tree:** Covers all non-tombstoned content hashes. Leaf = ContentHash. Tree is keyed by content hash prefix for efficient range queries.
2. **Tombstone tree:** Covers all active tombstones. Leaf = tombstone ContentHash + deletion timestamp.

```rust
pub struct MerkleTree {
    root: MerkleNode,
    depth: u8,           // Default: 16 levels (65536 leaf buckets)
}

pub struct MerkleNode {
    hash: [u8; 32],      // BLAKE3 of children
    left: Option<Box<MerkleNode>>,
    right: Option<Box<MerkleNode>>,
}
```

### 9.3 Sync Protocol

```
Peer A                              Peer B
  |                                    |
  |------- MerkleRoot(content) ------->|
  |<------ MerkleRoot(content) --------|
  |                                    |
  | (if roots differ)                  |
  |------- MerkleLevel(depth=1) ----->|
  |<------ MerkleLevel(depth=1) ------|
  |                                    |
  | (recurse on differing subtrees)    |
  |------- MerkleLevel(depth=N) ----->|
  |<------ MerkleLevel(depth=N) ------|
  |                                    |
  | (exchange missing content hashes)  |
  |------- ContentRequest([hashes]) -->|
  |<------ ContentResponse([blobs]) ---|
  |                                    |
```

**Bandwidth budget:** 50 KB/s for anti-entropy (full node), 25 KB/s (light node). The Merkle tree comparison typically requires only a few KB. Content transfer is the main cost and is rate-limited to stay within budget.

### 9.4 Full State Transfer

For initial sync or recovery after a long partition:

1. Request full Merkle tree (all levels).
2. Identify all missing content hashes.
3. Request missing content in batches of 100.
4. Apply in chronological order.

This is a fallback; normal operation uses incremental Merkle sync.

---

## 10. Storage Quotas and Priority Eviction

### 10.1 Quotas

| Node Type | Storage Cap | Action on Overflow |
|-----------|------------|-------------------|
| Light node | 500 MB | Priority eviction |
| Full node | 10 GB (configurable) | Priority eviction |
| Per-identity limit | 500 MB per pseudonym per node | Reject new content from that pseudonym |

### 10.2 Priority Eviction Order

When storage exceeds the cap, content is evicted in this order:

1. Expired content not yet GC'd (should be first to go).
2. Content with the shortest remaining TTL.
3. Content from pseudonyms the user is not connected to.
4. Media chunks with high provider count (available elsewhere on the network).
5. Content from pseudonyms with low reputation.
6. Oldest content.

User's own content and content from mutual connections is NEVER evicted (protected).

---

*This document is part of the Ephemera Architecture series. See [ARCHITECTURE.md](./ARCHITECTURE.md) for the master document.*
