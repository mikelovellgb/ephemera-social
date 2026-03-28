# Ephemera: Social Features Specification

**Parent document:** [ARCHITECTURE.md](./ARCHITECTURE.md) Section 7
**Version:** 1.0
**Date:** 2026-03-26
**Status:** Implementation-ready specification

---

## 1. Overview

Ephemera's social layer provides posts (text + photos), a mutual-connection social graph, reactions, replies, hashtags, mentions, and end-to-end encrypted 1:1 messaging. All content is ephemeral (1 hour to 30 days). The social graph uses mutual connections (bidirectional, consent-required) as the primary relationship model, with asymmetric follows as a lighter-weight content discovery mechanism.

This document specifies:
- Post content types, creation flow, and validation
- Social graph model (connections, follows, blocks, mutes)
- Interaction mechanics (reactions, replies, mentions, hashtags)
- Messaging protocol (X3DH + Double Ratchet, sealed sender, dead drops)
- Content size and rate limits
- Data structures for all social entities

**Cross-references:**
- Cryptographic signing and encryption: [01_identity_crypto.md](./01_identity_crypto.md)
- Network delivery and gossip: [02_network_protocol.md](./02_network_protocol.md)
- Storage of social data: [03_storage_data.md](./03_storage_data.md)
- Rate limiting and PoW: [05_moderation_safety.md](./05_moderation_safety.md)
- Client UI for social features: [06_client_api.md](./06_client_api.md)

---

## 2. Posts

### 2.1 Supported Content Types (MVP)

**Text:**
- Constrained Markdown subset.
- Allowed formatting: `**bold**`, `*italic*`, `~~strikethrough~~`, `` `code` ``, `> blockquote` (single level), `[text](url)`, `#hashtag`, `@mention`.
- Disallowed: headers (`#`, `##`), images in markup (`![]()`), tables, HTML, nested blockquotes, ordered/unordered lists.
- Maximum: 2,000 grapheme clusters AND 16 KB wire size (whichever is hit first).
- Grapheme cluster counting uses the Unicode Text Segmentation algorithm (UAX #29).

**Photo:**
- WebP format (single quality tier for MVP).
- Pipeline: strip EXIF metadata -> resize (max 1280px wide, maintain aspect ratio) -> compress to WebP (quality 80) -> encrypt with per-post symmetric key -> chunk into 256 KiB blocks -> BLAKE3 hash per chunk.
- Input limit: 10 MB per photo (pre-processing).
- Output limit: 5 MiB per photo (post-processing, after WebP compression).
- Up to 4 photos per post.

**Text + Photo:**
- A post contains text plus one media type batch (up to 4 photos).
- Mixed media (e.g., photos + video) is deferred.

### 2.2 Deferred Content Types

| Type | Target Phase | Notes |
|------|-------------|-------|
| Video | Phase 2 | H.264, max 720p, max 3 min, max 50 MiB. Client-side transcoding is heavyweight. |
| Audio / voice notes | Phase 2 | Opus in OGG container. Max 5 min, max 10 MiB. |
| Polls | Post-PoC | Structured data type, no media pipeline. |
| Link previews | Post-PoC | Privacy problem unsolved (IP leak when fetching OG metadata). |
| Composite mixed-media | Post-PoC | Photos + video in a single post. |
| Quote reposts | Post-PoC | Embeds signed snapshot of original's first 280 chars (text only). |

### 2.3 Post Creation Flow

```
User action: types text, attaches photos, sets TTL, taps "Post"

1. Frontend validation:
   - Text length <= 2,000 grapheme clusters AND <= 16 KB
   - Photo count <= 4
   - Each photo <= 10 MB (pre-processing)
   - TTL in range [1 hour, 30 days]

2. Frontend calls:
   invoke("rpc", {
     method: "posts.create",
     params: {
       body: "Hello #ephemera @quiet-fox-42",
       media: [{ path: "/tmp/photo1.jpg" }, { path: "/tmp/photo2.jpg" }],
       ttl_seconds: 86400,
       sensitivity: null,
       parent: null,       // null for top-level, ContentHash for reply
     }
   })

3. Rust backend processing:
   a. Parse RichText: extract hashtags, resolve mentions to pubkeys
   b. Media pipeline (for each photo):
      - Read file from temp path
      - Strip EXIF metadata (exif crate)
      - Resize to max 1280px wide
      - Compress to WebP (quality 80)
      - CSAM perceptual hash check (silent reject on match, return generic error)
      - Generate blurhash (~28 chars) for instant placeholder
      - Encrypt with per-post symmetric key (XChaCha20-Poly1305)
      - Chunk into 256 KiB blocks
      - BLAKE3 hash each chunk
      - Build MediaManifest
   c. Assemble Post struct:
      - Generate content_nonce (16 random bytes)
      - Assign sequence_number (per-author monotonic counter from SQLite)
      - Assign HLC timestamp
      - Compute Equihash PoW stamp (~100ms)
      - Sign with pseudonym's Ed25519 key
      - Compute content ID: BLAKE3(CBOR(post_without_signature))
   d. Local storage:
      - Write encrypted content to fjall
      - Write metadata to SQLite (posts table)
      - Write tags to post_tags table
      - Write mentions to post_mentions table
      - Write media metadata to media_attachments and media_variants tables
      - Write media chunks to fjall + media_chunks table

4. Return response to frontend immediately (optimistic local-first):
   {
     "jsonrpc": "2.0",
     "result": {
       "content_hash": "01abcdef...",
       "created_at": 1711411200000,
       "sequence_number": 42
     },
     "id": 1
   }

5. Network publication (async, after response):
   - Wrap post in Envelope protobuf
   - Route through AnonTransport (default T2)
   - Exit node submits to gossip overlay (public_feed + author_feed topics)
   - Post propagates to peers via PlumTree gossip
   - Media chunks submitted to content-addressed store for swarming
```

### 2.4 Post Validation (Receiving Node)

When a post arrives via gossip, the receiving node validates:

```rust
pub fn validate_incoming_post(envelope: &Envelope, post: &Post) -> Result<(), ValidationError> {
    // 1. Signature verification
    verify_ed25519(&post.author, &signed_bytes(envelope), &post.signature)?;

    // 2. Content hash verification
    let expected_hash = ContentHash::compute(&cbor_encode(&post)?);
    if expected_hash != post.id {
        return Err(ValidationError::ContentHashMismatch);
    }

    // 3. TTL validation
    if post.ttl_seconds.as_secs() > Ttl::MAX {
        return Err(ValidationError::TtlTooLong);
    }

    // 4. Expiry check
    let now = unix_now_millis();
    if post.expires_at.0 + CLOCK_SKEW_TOLERANCE_MS < now {
        return Err(ValidationError::Expired);
    }

    // 5. Future timestamp check
    if post.created_at.0 > now + CLOCK_SKEW_TOLERANCE_MS {
        return Err(ValidationError::FutureTimestamp);
    }

    // 6. PoW validation
    validate_pow(&post.pow_stamp, &post.id)?;

    // 7. Content size validation
    if let Some(ref body) = post.body {
        if body.grapheme_count() > 2000 || body.wire_size() > 16_384 {
            return Err(ValidationError::ContentTooLarge);
        }
    }

    // 8. Media count validation
    if post.media.len() > 4 {
        return Err(ValidationError::TooManyMedia);
    }

    // 9. Threading depth check
    if post.depth > 50 {
        return Err(ValidationError::ThreadTooDeep);
    }

    // 10. Tag/mention limits
    if post.tags.len() > 10 || post.mentions.len() > 20 {
        return Err(ValidationError::TooManyTags);
    }

    Ok(())
}
```

### 2.5 Post TTL

- User-configurable from 1 hour to 30 days.
- Default: 24 hours.
- Protocol maximum: 30 days (hard invariant enforced by the `Ttl` type).
- Presets offered in UI: 1h, 6h, 24h, 7d, 30d.
- Custom duration via slider.

### 2.6 Post Deletion

Users can delete their own posts before TTL expiry:

1. User taps "Delete" on their post.
2. Backend creates a `Tombstone` message signed by the post's author.
3. Tombstone propagated via gossip (high priority).
4. Local content deleted from fjall immediately.
5. SQLite metadata marked as `is_tombstone = 1`.
6. Tombstone retained for 3x the original TTL to ensure propagation.
7. Other nodes that receive the tombstone verify the author signature, then delete the content.

---

## 3. Social Graph

### 3.1 Mutual Connections (Primary Model)

**Model:** Bidirectional, consent-required connections.

**Rationale:**
- Stronger privacy than asymmetric follows (follow graphs leak richer social metadata).
- Aligned with anti-surveillance philosophy.
- Reduces the attack surface for social graph analysis.
- Encourages intentional relationship formation.

**Connection flow:**

```
Alice                                 Bob
  |                                    |
  |--- ConnectionRequest (signed) ---->|
  |    { source: alice_pubkey,         |
  |      target: bob_pubkey,           |
  |      message: "Hey!",             |
  |      timestamp, signature }        |
  |                                    |
  |    (Bob's client shows request)    |
  |                                    |
  |<-- ConnectionAccept (signed) ------|
  |    { source: bob_pubkey,           |
  |      target: alice_pubkey,         |
  |      timestamp, signature }        |
  |                                    |
  | (Both sides now have mutual conn)  |
  | (Exchange epoch keys for content)  |
  | (Can now send DMs without PoW)     |
```

**Connection request delivery:**
- Request deposited in DHT mailbox keyed to the recipient's pubkey.
- Recipient polls their mailbox via anonymous transport.
- If recipient is online, also delivered via gossip.

**In-person pairing (QR code):**
1. Alice opens "Add Connection" screen, which displays a QR code containing her pseudonym's Bech32m address and a session nonce.
2. Bob scans the QR code.
3. Bob's client sends a ConnectionRequest directly (via DHT mailbox).
4. Alice's client auto-accepts (because she initiated the QR display).
5. Connection established in ~2 seconds.

**Remote pairing (invite link):**
```
Format: ephemera://connect/<bech32m_pubkey>
Example: ephemera://connect/eph1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx
```
- User shares the link via any channel (existing messenger, email, etc.).
- Clicking the link opens Ephemera and pre-fills a ConnectionRequest.
- Recipient must explicitly accept.

### 3.2 Data Structures

```rust
pub struct ConnectionRequest {
    pub source:      IdentityKey,
    pub target:      IdentityKey,
    pub message:     Option<String>,     // Max 280 chars
    pub timestamp:   Timestamp,
    pub pow_stamp:   PowStamp,           // Required for stranger requests
    pub signature:   Signature,
}

pub struct ConnectionAccept {
    pub source:      IdentityKey,        // Acceptor
    pub target:      IdentityKey,        // Original requester
    pub timestamp:   Timestamp,
    pub signature:   Signature,
}

pub struct ConnectionReject {
    pub source:      IdentityKey,
    pub target:      IdentityKey,
    pub timestamp:   Timestamp,
    pub signature:   Signature,
}

pub struct Disconnection {
    pub source:      IdentityKey,
    pub target:      IdentityKey,
    pub timestamp:   Timestamp,
    pub signature:   Signature,
}

#[derive(Debug, Clone, Copy)]
pub enum ConnectionStatus {
    PendingOutgoing,     // We sent a request, waiting for response
    PendingIncoming,     // We received a request, haven't responded
    Connected,           // Mutual connection established
    Disconnected,        // Previously connected, now disconnected
}
```

### 3.3 Follows (Asymmetric, Content Discovery)

For MVP, asymmetric follows exist as a lighter-weight mechanism for subscribing to public content:

```rust
pub struct FollowEvent {
    pub follower:    IdentityKey,
    pub followed:    IdentityKey,
    pub action:      FollowAction,
    pub timestamp:   Timestamp,
    pub signature:   Signature,
}

pub enum FollowAction { Follow, Unfollow }
```

- Follows are signed events stored in OR-Set CRDTs.
- Following someone subscribes you to their public posts via gossip.
- The followed user does NOT need to accept. Follows are unilateral.
- Follow counts are NOT publicly displayed (anti-social-proof philosophy).
- Follower lists are NOT publicly queryable (privacy).

### 3.4 Blocks and Mutes

**Blocks (local enforcement):**
- Blocking a pseudonym hides all their content from your feed.
- Blocked users cannot send you connection requests or DMs.
- Blocks are stored locally only (not propagated to the network).
- Blocks are silent -- the blocked user is not notified.

**Mutes (local enforcement):**
- Muting a pseudonym hides their content from your feed.
- Muted users can still send you connection requests and DMs (you just don't see them prominently).
- Mutes can be time-limited (e.g., mute for 24 hours).
- Mutes are stored locally only.

```rust
pub struct Block {
    pub blocker:     IdentityKey,
    pub blocked:     IdentityKey,
    pub created_at:  Timestamp,
    pub reason:      Option<String>,     // Local note, never transmitted
}

pub struct Mute {
    pub muter:       IdentityKey,
    pub muted:       IdentityKey,
    pub created_at:  Timestamp,
    pub expires_at:  Option<Timestamp>,  // None = permanent
}
```

### 3.5 Connection Tiers (Post-MVP)

After MVP, connections can have tiers:

| Tier | Visibility | Messaging |
|------|-----------|-----------|
| Close Friends | See all posts including Close-Friends-only | Unlimited DMs |
| Friends | See all public + Friends posts | Standard DM limits |
| Acquaintances | See public posts only | DM with PoW |

This requires per-audience encryption (different epoch keys per tier). Deferred from PoC.

---

## 4. Interactions

### 4.1 Reactions

**Constraint set:** Five emoji reactions: heart, laugh, fire, sad, thinking.

**Semantics:**
- OR-Set per post per emoji. Each user can add or remove their reaction.
- Exact counting for MVP scale (not HyperLogLog approximation).
- Reactions are **private to the post author** -- no public counts, no social proof manipulation.
- The author sees: "3 people reacted with heart" but NOT who reacted.
- Exception: the author can see the list of reactors if they are mutual connections.

**Data structure:**

```rust
pub struct ReactionEvent {
    pub target:     ContentHash,
    pub reactor:    IdentityKey,
    pub emoji:      ReactionEmoji,
    pub action:     ReactionAction,
    pub timestamp:  Timestamp,
    pub signature:  Signature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReactionEmoji {
    Heart,
    Laugh,
    Fire,
    Sad,
    Thinking,
}

#[derive(Debug, Clone, Copy)]
pub enum ReactionAction { Add, Remove }
```

**Delivery:** Reaction events are delivered via gossip (CRDT sync topic). They propagate to peers who hold the target post.

### 4.2 Replies

**Model:** Flat, single `parent` link. No tree threading for MVP.

- Replies are independent posts with their own TTL.
- A reply can outlive its parent; the client displays "[original post expired]" when the parent is unresolvable.
- Reply depth is tracked (`depth` field) for display purposes but not enforced beyond a max depth of 50.
- Replies carry `parent_hash` (the immediate parent) and `root_hash` (the thread root).

**Causal delivery:**
1. Reply arrives via gossip.
2. Check if parent exists in local storage.
3. If parent exists: display immediately.
4. If parent missing: buffer the reply for up to 30 seconds.
5. After 30 seconds: active fetch via DHT content lookup.
6. After 5 minutes: display with placeholder for parent.

**Reply validation:** Same as post validation, plus:
- `parent_hash` must reference an existing post or be awaiting delivery.
- `root_hash` must be consistent with the thread (if the parent's root differs, reject).
- `depth` must equal `parent.depth + 1`.

### 4.3 Hashtags

**Parsing:** Tags are extracted from the post body during creation.

```rust
pub fn parse_tags(body: &str) -> Vec<Tag> {
    let re = regex::Regex::new(r"#(\w{1,50})").unwrap();
    re.captures_iter(body)
        .map(|cap| Tag::new(cap[1].to_lowercase()))
        .collect::<Vec<_>>()
        .into_iter()
        .take(10)    // Max 10 tags per post
        .collect()
}
```

- Tags are normalized to lowercase.
- Stored as structured metadata in the `post_tags` table.
- **Local index only for MVP.** The client can search posts by tag in its local SQLite database.
- **DHT-based hashtag indexing is deferred to Phase 2.** At PoC scale (100 nodes), local indexing from gossip-received posts is sufficient.

### 4.4 Mentions

**Format:** `@display_name` in the post body, resolved to a pubkey by the client.

```rust
pub struct MentionTag {
    pub pubkey:       IdentityKey,     // Resolved pseudonym pubkey
    pub display_hint: String,          // Display name at time of mention
    pub byte_start:   usize,          // Start byte offset in body
    pub byte_end:     usize,          // End byte offset in body
}
```

**Resolution flow:**
1. User types `@` in the composer.
2. Client shows autocomplete from local connection list + recently seen pseudonyms.
3. User selects a pseudonym.
4. Client inserts `@display_name` in the text and records the MentionTag with the resolved pubkey.
5. On the receiving end, clients render the mention as a tappable link to the mentioned user's profile.

**Notification delivery:** When a user is mentioned, a notification is deposited in their DHT mailbox (keyed to their pubkey). The mentioned user's client polls the mailbox via anonymous transport.

### 4.5 Topic Rooms

Topic rooms are user-created discussion spaces:

```rust
pub struct TopicRoom {
    pub id:          TopicRoomId,        // BLAKE3 of (creator_pubkey || name || creation_timestamp)
    pub name:        String,             // Max 50 chars
    pub description: Option<String>,     // Max 280 chars
    pub creator:     IdentityKey,
    pub created_at:  Timestamp,
    pub ttl_seconds: Ttl,                // Room itself expires (max 30 days)
}

pub struct TopicRoomId([u8; 32]);
```

- Posts can be tagged with a TopicRoomId.
- Subscribing to a room's gossip topic delivers all posts tagged with that room.
- Rooms are ephemeral (they expire like all content).
- Room creation requires PoW (same as post creation).
- No moderation within rooms for MVP (community moderation quorums are post-MVP).

---

## 5. Feed Assembly

### 5.1 Feed Types

**Connections feed (primary):**
- Chronological posts from mutual connections.
- Includes replies from connections to posts the user has seen.
- No algorithmic ranking. No trending. Pure reverse-chronological.

**Discover feed:**
- Public posts from non-connected pseudonyms.
- Received via gossip on the public feed topic.
- Displayed separately from the connections feed.
- No ranking algorithm. Reverse-chronological.

**Profile feed:**
- All posts from a specific pseudonym (local cache).

**Topic room feed:**
- All posts tagged with a specific TopicRoomId.

### 5.2 Feed Assembly Query

```sql
-- Connections feed (primary)
SELECT p.*
FROM posts p
JOIN connections c ON p.author_pubkey = c.remote_pubkey
WHERE c.local_pubkey = ?
  AND c.status = 'connected'
  AND p.is_tombstone = 0
  AND p.expires_at > ?           -- Still alive
  AND p.author_pubkey NOT IN (SELECT blocked_pubkey FROM blocks WHERE blocker_pubkey = ?)
  AND p.author_pubkey NOT IN (SELECT muted_pubkey FROM mutes WHERE muter_pubkey = ? AND (expires_at IS NULL OR expires_at > ?))
ORDER BY p.created_at DESC
LIMIT 50
OFFSET ?;                         -- Cursor-based pagination
```

### 5.3 Cursor-Based Pagination

```rust
pub struct FeedCursor {
    pub created_at: Timestamp,       // Last seen timestamp
    pub content_hash: ContentHash,   // Tiebreaker for same timestamp
}

pub struct FeedPage {
    pub posts: Vec<Post>,
    pub next_cursor: Option<FeedCursor>,
    pub has_more: bool,
}
```

The frontend requests pages of 50 posts. The cursor (timestamp + content hash for deterministic ordering) enables efficient pagination without offset-based queries.

### 5.4 "All Caught Up" State

When the user scrolls past all unread posts since their last session:

```rust
pub struct FeedState {
    pub caught_up: bool,             // True if user has seen all posts since last session
    pub last_read_cursor: FeedCursor,
    pub unread_count: u32,
}
```

The UI displays a "You're all caught up" divider with a gentle animation. Posts below the divider are previously-read posts.

---

## 6. Messaging Protocol

### 6.1 Overview

1:1 encrypted messaging using the Signal Protocol's X3DH + Double Ratchet, adapted for a decentralized network with store-and-forward relay delivery and sealed-sender envelopes.

### 6.2 Prekey Bundle

Each pseudonym publishes a prekey bundle to the DHT for asynchronous key exchange:

```rust
pub struct PrekeyBundle {
    pub identity_key:     X25519PublicKey,    // Long-term X25519 identity key
    pub signed_prekey:    X25519PublicKey,    // Rotated every 7 days
    pub signed_prekey_sig: Signature,         // Ed25519 signature over signed_prekey
    pub signed_prekey_id: u32,
    pub one_time_prekeys: Vec<OneTimePrekey>, // Batch of up to 100
    pub timestamp:        Timestamp,
    pub signature:        Signature,          // Ed25519 signature over entire bundle
}

pub struct OneTimePrekey {
    pub id:        u32,
    pub public_key: X25519PublicKey,
}
```

**Prekey lifecycle:**
- Identity key: derived from pseudonym, long-lived (as long as the pseudonym exists).
- Signed prekey: rotated every 7 days. Old signed prekeys retained for 14 days (to handle in-flight messages).
- One-time prekeys: single-use. Published in batches of 100. Replenished when < 20 remain.
- Bundle published to DHT via anonymous circuit (T2/T3). Key: `DhtKey::prekey(pseudonym)`.

### 6.3 X3DH Key Exchange

**Alice wants to send Bob his first message:**

```
1. Alice fetches Bob's prekey bundle from DHT (via anonymous transport).

2. Alice computes the shared secret:
   EK_a = Ephemeral X25519 key pair (generated fresh)
   IK_a = Alice's X25519 identity key
   IK_b = Bob's X25519 identity key (from bundle)
   SPK_b = Bob's signed prekey (from bundle)
   OPK_b = Bob's one-time prekey (if available, consumed)

   DH1 = X25519(IK_a, SPK_b)
   DH2 = X25519(EK_a, IK_b)
   DH3 = X25519(EK_a, SPK_b)
   DH4 = X25519(EK_a, OPK_b)   -- only if OPK available

   If OPK available:
     SK = HKDF(DH1 || DH2 || DH3 || DH4, salt=0, info="ephemera-x3dh-v1")
   Else:
     SK = HKDF(DH1 || DH2 || DH3, salt=0, info="ephemera-x3dh-v1")

3. Alice initializes the Double Ratchet with SK as the root key.

4. Alice sends the initial message:
   InitialMessage {
     identity_key: IK_a.public,
     ephemeral_key: EK_a.public,
     prekey_id: SPK_b.id,
     one_time_prekey_id: OPK_b.id (or None),
     ciphertext: DoubleRatchet.encrypt(plaintext),
   }

5. Alice wraps in sealed-sender envelope:
   MessageEnvelope {
     recipient: Bob's pseudonym pubkey (for routing),
     sender_sealed: [],             // EMPTY on wire
     ciphertext: encrypt(InitialMessage),
     timestamp, ttl_seconds, pow_stamp,
   }

6. Alice deposits envelope at Bob's dead drop.
```

### 6.4 Double Ratchet

After X3DH establishes the initial shared secret, all subsequent messages use the Double Ratchet:

**Symmetric ratchet step (every message):**
```
chain_key[n+1] = HKDF(chain_key[n], "ephemera-chain-v1")
message_key[n] = HKDF(chain_key[n], "ephemera-msg-v1")
```

**DH ratchet step (every turn change):**
```
When the other party sends a new DH public key:
  new_dh = X25519(our_ratchet_secret, their_new_ratchet_public)
  new_root_key, new_chain_key = HKDF(root_key, new_dh, "ephemera-ratchet-v1")
```

**Forward secrecy:** Each message key is deleted immediately after use (sender deletes after encryption, recipient deletes after decryption). Compromising any single message key does not reveal past or future messages.

**Post-compromise security:** After a compromise, the next DH ratchet step generates a new shared secret that the attacker cannot compute (because they don't have the new ephemeral DH secret).

### 6.5 Dead Drop Delivery

Messages are delivered via store-and-forward dead drops on relay nodes:

```rust
/// Dead drop address derived from shared ratchet state
pub fn dead_drop_address(ratchet_state: &RatchetState) -> DeadDropId {
    let hash = blake3::keyed_hash(
        b"ephemera-dead-drop-v1-000000000",  // 32-byte key
        &ratchet_state.receiving_chain_key,
    );
    DeadDropId(hash.as_bytes()[..16].try_into().unwrap())
}

pub struct DeadDropId([u8; 16]);
```

**Delivery flow:**
1. Sender encrypts message with Double Ratchet.
2. Sender wraps in sealed-sender envelope (recipient pubkey visible, sender hidden).
3. Sender deposits envelope at the dead drop address on a relay node (via anonymous transport).
4. Recipient polls the dead drop address (via anonymous transport) periodically.
5. Relay returns all envelopes at the dead drop address.
6. Recipient decrypts, revealing sender identity inside the ciphertext.
7. Relay purges delivered envelopes.

**Dead drop rotation:** The dead drop address rotates with each DH ratchet step. Old dead drop addresses are polled for a grace period (1 hour) to catch in-flight messages.

**Relay retention:** Undelivered messages held for 14 days, then purged.

### 6.6 Message Requests (Stranger DMs)

When a non-connected user wants to send a message:

1. Sender must complete PoW (full difficulty, ~30 seconds).
2. Message appears in the recipient's "Message Requests" section (separate from main inbox).
3. Recipient can accept (start conversation) or reject (silent discard).
4. Accepting a message request does NOT create a connection -- it only opens the DM channel.

### 6.7 Message Padding

All messages are padded to the nearest 256-byte boundary to prevent size-based content inference:

```rust
pub fn pad_message(plaintext: &[u8]) -> Vec<u8> {
    let content_len = plaintext.len();
    let padded_len = ((content_len + 2) / 256 + 1) * 256; // +2 for u16 length prefix
    let mut padded = Vec::with_capacity(padded_len);
    padded.extend_from_slice(&(content_len as u16).to_be_bytes());
    padded.extend_from_slice(plaintext);
    padded.resize(padded_len, 0x00);
    padded
}

pub fn unpad_message(padded: &[u8]) -> Result<&[u8], PaddingError> {
    if padded.len() < 2 {
        return Err(PaddingError::TooShort);
    }
    let content_len = u16::from_be_bytes([padded[0], padded[1]]) as usize;
    if content_len + 2 > padded.len() {
        return Err(PaddingError::InvalidLength);
    }
    Ok(&padded[2..2 + content_len])
}
```

### 6.8 Deniability

DMs use MAC-based authentication (HMAC derived from the shared ratchet key) instead of Ed25519 signatures. This means:
- Both Alice and Bob can verify that a message came from the other (they share the MAC key).
- Neither can prove to a third party who authored the message (either could have generated the MAC).
- Messages are cryptographically deniable.

### 6.9 Ratchet State Persistence

The Double Ratchet state must be persisted to survive app restarts:

```rust
pub struct RatchetState {
    pub root_key:              Secret<[u8; 32]>,
    pub sending_chain_key:     Secret<[u8; 32]>,
    pub receiving_chain_key:   Secret<[u8; 32]>,
    pub our_ratchet_keypair:   X25519KeyPair,
    pub their_ratchet_public:  Option<X25519PublicKey>,
    pub sending_chain_length:  u32,
    pub receiving_chain_length: u32,
    pub previous_sending_chain_length: u32,
    pub skipped_message_keys:  HashMap<(X25519PublicKey, u32), Secret<[u8; 32]>>,
    pub max_skip: u32,          // Default: 1000
}
```

The ratchet state is stored in the encrypted keystore (not in SQLite). It contains secret key material that must be zeroized on drop.

**Skipped message keys:** When messages arrive out of order, the ratchet must "skip ahead" and store intermediate message keys. These are kept for decrypting late-arriving messages, with a maximum skip of 1000 (to prevent DoS).

---

## 7. Content Size and Rate Limits

### 7.1 Content Size Limits

| Parameter | Value | Enforcement |
|-----------|-------|-------------|
| Text post body | 2,000 grapheme clusters AND 16 KB wire | Protocol (hard reject) |
| Message text | 10,000 grapheme clusters AND 32 KB wire | Protocol (hard reject) |
| Photo input (pre-processing) | 10 MB per photo | Client-enforced |
| Photo output (post-processing) | 5 MiB per photo | Client + peer validation |
| Photos per post | 4 | Protocol |
| Photos per message | 1 | Protocol |
| Video (post, Phase 2) | 50 MiB, 720p max, 3 min max | Client + peer validation |
| Video (message) | 50 MiB | Client + peer validation |
| Audio | 10 MiB / 5 minutes, Opus in OGG | Client + peer validation |
| Total post media | 50 MiB | Protocol |
| Profile display name | 30 characters | Protocol |
| Profile bio | 160 characters | Protocol |
| Profile metadata total | 4 KB | Protocol |
| Connection request message | 280 characters | Protocol |
| Chunk size | 256 KiB | Protocol |
| Alt text | 1,000 characters | Protocol |
| Tags per post | 10 | Protocol |
| Mentions per post | 20 | Protocol |

### 7.2 Rate Limits

| Parameter | Value | Enforcement |
|-----------|-------|-------------|
| Posts per hour | 10 (default, configurable 5-20) | Node-enforced |
| Replies per hour (same thread) | 20 | Node-enforced |
| Interactions per hour (reactions) | 100 | Node-enforced |
| DMs/hr to friends | 60 per conversation | Node-enforced |
| DMs/hr to mutual contacts | 30 per conversation | Node-enforced |
| DMs/hr to strangers | 5 total (message requests) | Node-enforced |
| Follows per hour | 50 | Node-enforced |
| Connection requests per hour | 20 | Node-enforced |
| Storage per identity per node | 500 MB | Node-enforced, priority eviction on overflow |

Rate limits are enforced locally by the client node. Peers validate incoming content against rate limits and may reject content from pseudonyms that exceed them. Rate limit violations do not trigger immediate bans but contribute to reputation penalties.

---

## 8. Notification System

### 8.1 Notification Types

| Event | Delivery Method | Priority |
|-------|----------------|----------|
| New connection request | DHT mailbox + gossip | High |
| Connection accepted | DHT mailbox + gossip | High |
| New message | Dead drop polling | High |
| Mention in a post | DHT mailbox | Medium |
| Reaction on your post | DHT mailbox (batched) | Low |
| Reply to your post | Gossip (author feed subscription) | Medium |

### 8.2 DHT Mailbox

Each pseudonym has a DHT mailbox for asynchronous notifications:

```rust
pub fn mailbox_key(pseudonym: &IdentityKey) -> DhtKey {
    DhtKey::from_bytes(&blake3::hash(&[b"dht-mailbox-v1\x00", pseudonym.as_bytes()].concat()))
}
```

Notifications are encrypted under the recipient's X25519 key (from their prekey bundle). The client polls the mailbox periodically (every 30 seconds when active, every 5 minutes when backgrounded).

### 8.3 Event Bus

Internal events are broadcast via the `ephemera-events` crate (tokio broadcast channel):

```rust
pub enum Event {
    PostReceived(ContentHash),
    PostExpired(ContentHash),
    MessageReceived(ConversationId, ContentHash),
    ConnectionRequestReceived(IdentityKey),
    ConnectionAccepted(IdentityKey),
    ReactionReceived(ContentHash, ReactionEmoji),
    MentionReceived(ContentHash, IdentityKey),
    ProfileUpdated(IdentityKey),
    NetworkStatusChanged(NetworkStatus),
    SyncComplete,
}
```

The Tauri bridge subscribes to this event bus and forwards events to the SolidJS frontend for real-time UI updates.

---

*This document is part of the Ephemera Architecture series. See [ARCHITECTURE.md](./ARCHITECTURE.md) for the master document.*
