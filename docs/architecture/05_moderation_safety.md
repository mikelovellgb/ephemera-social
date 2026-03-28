# Ephemera: Moderation & Safety Specification

**Parent document:** [ARCHITECTURE.md](./ARCHITECTURE.md) Section 8
**Version:** 1.0
**Date:** 2026-03-26
**Status:** Implementation-ready specification

---

## 1. Overview

Ephemera provides privacy by default and accountability for extreme abuse. The system is designed so that no single entity can surveil users, but a narrow set of universally condemned content categories receive protocol-level enforcement. Moderation operates without breaking encryption or deanonymizing users.

This document specifies:
- CSAM detection (bloom filter, relay verification, update protocol)
- Distributed community moderation (quorum voting, blinded review)
- Reporting, blocking, and muting mechanics
- Rate limiting and the PoW system (Equihash)
- Sybil resistance (PoW + reputation warming + social trust)
- Reputation system (G-Counter with decay, capability gating)
- Legal compliance features
- What we will NOT build

**Cross-references:**
- Crypto primitives for PoW verification: [01_identity_crypto.md](./01_identity_crypto.md) Section 3
- Network delivery of moderation events: [02_network_protocol.md](./02_network_protocol.md) Section 4
- Storage of moderation data: [03_storage_data.md](./03_storage_data.md)
- Social features being moderated: [04_social_features.md](./04_social_features.md)
- Crate boundary (ephemera-abuse, ephemera-mod): [07_rust_workspace.md](./07_rust_workspace.md)

---

## 2. The Balance

Ephemera's moderation philosophy:

1. **Privacy is the default.** Encryption is not broken for moderation purposes. Ever.
2. **Narrow scope.** Protocol-level enforcement targets only universally condemned categories (CSAM). All other moderation is community-driven.
3. **Honest about limitations.** Encrypted channels provide less moderation coverage than public content. This is the acknowledged cost of not breaking encryption.
4. **No single authority.** Moderation actions require quorum consensus. No single moderator, no single organization, no single government can unilaterally censor content.
5. **Transparency.** All moderation actions are auditable. The bloom filter has a public change log.

---

## 3. CSAM Detection

### 3.1 Layer 1: Client-Side Perceptual Hash Bloom Filter

**Architecture:**

Every official Ephemera client ships with a ~10 MB bloom filter containing perceptual hashes derived from established CSAM databases (NCMEC, IWF, Project Arachnid).

**How it works:**

```rust
pub struct CsamFilter {
    bloom: BloomFilter,          // ~10 MB, FPR ~1e-10
    version: u64,                // Monotonically increasing version number
    merkle_root: [u8; 32],       // BLAKE3 Merkle root of the hash set
    signers: Vec<PublicKey>,     // 5 authorized signers
    threshold: u8,               // 3-of-5 required for updates
}

impl CsamFilter {
    /// Check an image before encryption. Called in the media pipeline.
    pub fn check_image(&self, image_bytes: &[u8]) -> CsamCheckResult {
        // 1. Compute perceptual hash (pHash)
        let phash = perceptual_hash(image_bytes);

        // 2. Also compute DCT hash for robustness
        let dct_hash = dct_hash(image_bytes);

        // 3. Check both against bloom filter
        if self.bloom.contains(&phash) || self.bloom.contains(&dct_hash) {
            CsamCheckResult::Blocked
        } else {
            CsamCheckResult::Clear
        }
    }
}

pub enum CsamCheckResult {
    Clear,
    Blocked,  // Content is silently rejected
}
```

**Integration point in the media pipeline:**

```rust
pub async fn process_image(
    input: &[u8],
    csam_filter: &CsamFilter,
) -> Result<ProcessedImage, MediaError> {
    // 1. Strip EXIF metadata
    let stripped = strip_exif(input)?;

    // 2. CSAM check (BEFORE any further processing)
    match csam_filter.check_image(&stripped) {
        CsamCheckResult::Blocked => {
            // Silent rejection. Generic error message.
            // Do NOT reveal that CSAM was detected (prevents probing).
            return Err(MediaError::UnableToProcess);
        }
        CsamCheckResult::Clear => {}
    }

    // 3. Resize, compress, encrypt, chunk...
    // (continue normal pipeline)
}
```

**User-facing behavior on match:**
- The post/message silently fails with a generic error: "Unable to share this image. Please try a different image."
- No indication that CSAM was detected. This prevents attackers from using the error message to probe the filter's contents.
- The failed attempt is logged locally (for the user's own device diagnostics) but NOT reported to the network.

### 3.2 Layer 2: Relay-Side Verification

For unencrypted public content that passes through relay nodes:

```rust
pub async fn relay_verify_content(
    content: &PublicContent,
    csam_filter: &CsamFilter,
) -> RelayDecision {
    // Only applies to public content where media is decryptable
    // (relay has the epoch key for the current epoch)
    if let Some(media) = &content.media {
        for image in media.images() {
            match csam_filter.check_image(&image.decrypted_bytes) {
                CsamCheckResult::Blocked => {
                    // Reject content, do not forward
                    // Generate privacy-preserving report
                    let report = TransparencyReport {
                        content_hash: content.hash.clone(),
                        timestamp: unix_now(),
                        // NO author info, NO IP info
                    };
                    emit_transparency_report(report);
                    return RelayDecision::Reject;
                }
                CsamCheckResult::Clear => {}
            }
        }
    }
    RelayDecision::Forward
}
```

**Limitation:** Relay verification only works for public content where the relay holds the epoch key. Encrypted DMs cannot be verified by relays. For DMs, the system relies on recipient reporting and reputation consequences.

### 3.3 Bloom Filter Update Protocol

The bloom filter is updated via a dedicated gossip message type with multi-signature authorization:

```rust
pub struct BloomFilterUpdate {
    pub version: u64,                   // Must be > current version
    pub delta: BloomFilterDelta,        // Added/removed hashes
    pub merkle_root: [u8; 32],          // New Merkle root after applying delta
    pub merkle_proof: Vec<[u8; 32]>,    // Proof of correct update
    pub signatures: Vec<(PublicKey, Signature)>,  // 3-of-5 multi-sig
    pub timestamp: u64,
    pub changelog_entry: String,        // Human-readable description of change
}

pub struct BloomFilterDelta {
    pub added_hashes: Vec<[u8; 32]>,    // New perceptual hashes to add
    pub removed_hashes: Vec<[u8; 32]>,  // Hashes to remove (false positives corrected)
}
```

**Multi-signature requirement:**
- 5 authorized signers (Foundation board members from different jurisdictions + independent civil liberties organizations).
- 3-of-5 signatures required for any update.
- This prevents unilateral censorship: no single signer can add arbitrary hashes.
- Signers are publicly identified and accountable.

**Verification on receipt:**

```rust
pub fn verify_bloom_update(
    update: &BloomFilterUpdate,
    current_filter: &CsamFilter,
) -> Result<(), BloomUpdateError> {
    // 1. Version must be strictly increasing
    if update.version <= current_filter.version {
        return Err(BloomUpdateError::StaleVersion);
    }

    // 2. Verify at least 3 valid signatures from the 5 authorized signers
    let valid_sigs = update.signatures.iter()
        .filter(|(pubkey, sig)| {
            current_filter.signers.contains(pubkey) &&
            verify_ed25519(pubkey, &update_signed_bytes(update), sig).is_ok()
        })
        .count();

    if valid_sigs < current_filter.threshold as usize {
        return Err(BloomUpdateError::InsufficientSignatures);
    }

    // 3. Verify Merkle proof (ensures the delta correctly transforms the tree)
    verify_merkle_proof(
        &current_filter.merkle_root,
        &update.merkle_root,
        &update.delta,
        &update.merkle_proof,
    )?;

    Ok(())
}
```

**Public audit trail:**
- Every bloom filter update includes a changelog entry.
- The Merkle tree of all hashes is publicly verifiable.
- Anyone can verify that only authorized signers made changes.
- The Merkle root history is a tamper-evident log.

### 3.4 Perceptual Hashing

Two perceptual hash algorithms are used for robustness:

**pHash (Perceptual Hash):**
```
1. Convert image to grayscale
2. Resize to 32x32 (lose high-frequency detail)
3. Apply DCT (discrete cosine transform)
4. Keep top-left 8x8 of DCT coefficients (low-frequency components)
5. Compute median of the 64 coefficients
6. Each bit = 1 if coefficient > median, 0 otherwise
7. Result: 64-bit hash
```

**dHash (Difference Hash):**
```
1. Convert image to grayscale
2. Resize to 9x8 (9 columns, 8 rows)
3. For each row, compare adjacent pixels: bit = 1 if left > right
4. Result: 64-bit hash
```

Both hashes are checked against the bloom filter independently. A match on either triggers the block.

**Hamming distance threshold:** A match is defined as a Hamming distance of <= 5 bits from any hash in the filter. The bloom filter stores all hashes within this threshold (pre-computed during filter generation).

---

## 4. Distributed Moderation

### 4.1 Community Moderation Protocol

**Quorum-based moderation:**
- A moderation action requires 5-of-7 moderators to agree.
- Moderators are elected by the community (reputation-based eligibility).
- Moderation quorums operate within "neighborhoods" -- geographic or topic-based clusters of nodes.

### 4.2 Blinded Review

Moderators see the reported content but NOT the author's identity:

```rust
pub struct BlindedContentReport {
    pub report_id:     ReportId,
    pub content_hash:  ContentHash,
    pub content_snapshot: Vec<u8>,        // Encrypted copy of the content
    pub content_type:  ContentType,       // Text, Image, etc.
    pub report_reason: ReportReason,
    pub reporter_count: u32,              // How many users reported this
    pub reported_at:   Timestamp,
    // NO author_pubkey field -- moderators cannot see who posted
}

pub enum ReportReason {
    Harassment,
    HateSpeech,
    Violence,
    Spam,
    Csam,           // Escalated to automated handling
    Impersonation,
    Other(String),
}
```

### 4.3 Moderation Actions

```rust
pub struct ModerationVote {
    pub report_id:     ReportId,
    pub moderator:     IdentityKey,
    pub action:        ModerationAction,
    pub timestamp:     Timestamp,
    pub signature:     Signature,
}

pub enum ModerationAction {
    /// No action needed (false report)
    Dismiss,
    /// Add content warning label
    LabelSensitive,
    /// Remove content (community tombstone)
    RemoveContent,
    /// Reduce author's reputation
    ReputationPenalty(u32),
}

pub struct ModerationResult {
    pub report_id:     ReportId,
    pub votes:         Vec<ModerationVote>,
    pub outcome:       ModerationAction,
    pub quorum_reached: bool,
    pub threshold_signature: ThresholdSignature,  // 5-of-7 threshold sig
}
```

**Community tombstone:** When a quorum votes for content removal, a community tombstone is generated:

```rust
pub struct CommunityTombstone {
    pub content_hash:       ContentHash,
    pub reason:             ReportReason,
    pub moderator_quorum:   Vec<IdentityKey>,     // 5+ moderators who voted
    pub threshold_signature: ThresholdSignature,   // 5-of-7 threshold signature
    pub timestamp:          Timestamp,
}
```

Nodes that receive a community tombstone verify the threshold signature and, if valid, remove the content. The tombstone propagates via gossip.

### 4.4 Moderator Election

Eligibility criteria:
1. Pseudonym age >= 30 days (survived one full TTL cycle).
2. Reputation score >= threshold (top 10% of active pseudonyms in the neighborhood).
3. No active moderation penalties.
4. Nominated by at least 3 existing moderators or 10 community members.

Election: OR-Set vote among eligible community members. Top 7 vote-getters become the moderation quorum for the neighborhood. Quorum membership expires every 30 days (ephemeral, like everything else).

---

## 5. Reporting, Blocking, and Muting

### 5.1 Report Flow

```
User sees abusive content
  |
  +-> Taps "Report" menu item
  |
  +-> Selects reason (Harassment, Hate Speech, Violence, Spam, CSAM, Impersonation, Other)
  |
  +-> Optional: adds description (max 280 chars)
  |
  +-> Report stored locally in SQLite
  |
  +-> If CSAM selected: content hash added to local CSAM report queue
  |   (post-MVP: forwarded to external reporting pipeline)
  |
  +-> Post-MVP: Report anonymously submitted to moderation quorum
  |   via gossip (moderation topic)
  |
  +-> User sees confirmation: "Report submitted. You can also block this user."
```

### 5.2 Block Flow

```rust
pub async fn block_user(
    blocker: &IdentityKey,
    blocked: &IdentityKey,
    db: &SqlitePool,
) -> Result<()> {
    // 1. Insert into blocks table
    sqlx::query!(
        "INSERT OR REPLACE INTO blocks (blocker_pubkey, blocked_pubkey, created_at) VALUES (?, ?, ?)",
        blocker.as_bytes(), blocked.as_bytes(), unix_now()
    ).execute(db).await?;

    // 2. Remove any existing connection
    sqlx::query!(
        "DELETE FROM connections WHERE local_pubkey = ? AND remote_pubkey = ?",
        blocker.as_bytes(), blocked.as_bytes()
    ).execute(db).await?;

    // 3. Hide all existing content from blocked user in local feed
    // (Content is not deleted -- just hidden from the blocker's feed queries)

    // 4. Reject future content from blocked user in gossip handler
    // (Added to a local blocklist checked during gossip ingestion)

    Ok(())
}
```

- Blocks are local only. The blocked user is NOT notified.
- Blocked users cannot send connection requests or DMs to the blocker.
- Existing content from the blocked user is hidden from the blocker's feed.
- Blocks survive app restarts (persisted in SQLite).

### 5.3 Mute Flow

Same as block but:
- Content is hidden from the feed, but the muted user can still send connection requests and DMs.
- Mutes can be time-limited (e.g., 1 hour, 24 hours, 7 days, permanent).
- Less aggressive than blocking -- suitable for temporarily noisy contacts.

---

## 6. Proof of Work (Equihash)

### 6.1 Algorithm

Equihash is a memory-hard proof-of-work algorithm based on the Generalized Birthday Problem. It requires significant memory (making GPU/ASIC acceleration expensive) and is asymmetrically verifiable (slow to compute, fast to verify).

**Parameters:**
```rust
pub struct EquihashParams {
    pub n: u32,       // 200 (bits)
    pub k: u32,       // 9 (Wagner's algorithm depth)
    // Memory requirement: ~400 MB for n=200, k=9
    // Solution size: ~1344 bytes
}
```

### 6.2 PoW Stamp

```rust
pub struct PowStamp {
    pub algorithm:    PowAlgorithm,      // Equihash(200,9)
    pub difficulty:   u32,               // Target difficulty
    pub nonce:        [u8; 32],          // Random nonce
    pub solution:     Vec<u8>,           // Equihash solution (~1344 bytes)
    pub input_hash:   [u8; 32],          // BLAKE3(challenge_data)
}

pub enum PowAlgorithm {
    Equihash200_9,
}
```

### 6.3 Difficulty Scaling

The PoW difficulty adapts based on context:

```rust
pub fn compute_pow_difficulty(context: &PowContext) -> u32 {
    let base = context.base_difficulty;

    // Activity multiplier: higher difficulty during high activity
    let activity_mult = match context.posts_last_hour {
        0..=5   => 1.0,
        6..=10  => 1.5,
        11..=20 => 2.0,
        _       => 3.0,
    };

    // Content size multiplier: larger content requires more PoW
    let size_mult = match context.content_size_bytes {
        0..=1024        => 1.0,
        1025..=16384    => 1.2,
        16385..=262144  => 1.5,    // Up to 256 KiB
        _               => 2.0,
    };

    // Relationship discount: friends get reduced PoW
    let relationship_disc = match context.relationship {
        Relationship::Self_      => 0.1,
        Relationship::Friend     => 0.0,  // Zero PoW for friends (DMs)
        Relationship::Mutual     => 0.5,
        Relationship::Follower   => 0.8,
        Relationship::Stranger   => 1.0,
    };

    // Reach multiplier: content reaching more people requires more PoW
    let reach_mult = match context.audience {
        Audience::Direct => 0.5,
        Audience::Public => 1.0,
    };

    let difficulty = (base as f64 * activity_mult * size_mult * relationship_disc * reach_mult) as u32;

    // Ceiling: never exceed 60 seconds of computation
    difficulty.min(context.max_difficulty)
}
```

### 6.4 PoW by Action Type

| Action | Base Difficulty | Approximate Time | Notes |
|--------|----------------|------------------|-------|
| Identity creation | Very high | ~30 seconds | Adaptive during surges. One-time cost. |
| Post creation | Medium | ~100ms | Scales with activity and content size |
| Reply creation | Medium | ~100ms | Same as post |
| Reaction | Low | ~10ms | Lightweight interaction |
| DM to friend | None | 0ms | Friends bypass PoW |
| DM to mutual contact | Low | ~50ms | Established relationship |
| DM to stranger (message request) | High | ~10 seconds | Spam deterrent |
| Connection request | Medium | ~500ms | Prevents mass friending |
| Follow | Low | ~50ms | Lightweight action |

### 6.5 PoW Verification

Verification is fast (< 1ms) regardless of difficulty:

```rust
pub fn verify_pow(stamp: &PowStamp, expected_input: &[u8]) -> Result<(), PowError> {
    // 1. Verify the input hash matches
    let input_hash = blake3::hash(expected_input);
    if stamp.input_hash != *input_hash.as_bytes() {
        return Err(PowError::InputMismatch);
    }

    // 2. Verify the Equihash solution
    let challenge = [&stamp.input_hash[..], &stamp.nonce[..]].concat();
    equihash::verify(
        stamp.algorithm.n(),
        stamp.algorithm.k(),
        &challenge,
        &stamp.solution,
    ).map_err(|_| PowError::InvalidSolution)?;

    // 3. Verify difficulty met
    if !meets_difficulty(&stamp.solution, stamp.difficulty) {
        return Err(PowError::InsufficientDifficulty);
    }

    Ok(())
}
```

### 6.6 PoW Ceiling

Maximum PoW computation time is 60 seconds. If the computed difficulty would exceed this, it is clamped:

```rust
const POW_CEILING_SECONDS: u64 = 60;

fn clamp_difficulty(difficulty: u32, hardware_profile: &HardwareProfile) -> u32 {
    let estimated_time = hardware_profile.estimate_pow_time(difficulty);
    if estimated_time > Duration::from_secs(POW_CEILING_SECONDS) {
        hardware_profile.difficulty_for_time(Duration::from_secs(POW_CEILING_SECONDS))
    } else {
        difficulty
    }
}
```

---

## 7. Reputation System

### 7.1 Overview

Reputation is a per-pseudonym score that gates access to platform capabilities. It uses a G-Counter CRDT with time-based decay, meaning reputation naturally degrades if not maintained through positive activity.

### 7.2 Reputation Scoring

```rust
pub struct ReputationScore {
    pub positive_actions: BoundedCounter,    // Posts, replies, connections
    pub negative_actions: BoundedCounter,    // Reports received, spam flags
    pub age_days: u32,                       // Days since pseudonym creation
    pub connection_count: u32,               // Number of mutual connections
}

impl ReputationScore {
    pub fn value(&self) -> f64 {
        let positive = self.positive_actions.value();
        let negative = self.negative_actions.value();
        let age_bonus = (self.age_days as f64).sqrt() * 10.0;
        let connection_bonus = (self.connection_count as f64).sqrt() * 5.0;

        (positive + age_bonus + connection_bonus - negative * 3.0).max(0.0)
    }
}
```

**Positive contributions:**
| Action | Points | Decay Half-Life |
|--------|--------|----------------|
| Create a post | +1 | 30 days |
| Receive a reaction | +0.5 | 30 days |
| Form a mutual connection | +5 | 30 days |
| Successful moderation vote (consensus side) | +3 | 30 days |
| Day of account age | +sqrt(days) | No decay |

**Negative contributions:**
| Action | Points | Decay Half-Life |
|--------|--------|----------------|
| Content reported (confirmed by quorum) | -10 | 30 days |
| Rate limit violation | -2 | 7 days |
| PoW timestamp manipulation detected | -20 | 30 days |
| Community tombstone on your content | -50 | 60 days |

### 7.3 Reputation-Gated Capabilities

| Capability | Reputation Requirement | Warming Period |
|-----------|----------------------|----------------|
| Create text posts | reputation >= 0 | 7-day warming: 1 post/hour max |
| Attach photos | reputation >= 5 | After warming period |
| Send DMs to mutual contacts | reputation >= 0 | Immediate for connected users |
| Send DMs to strangers (requests) | reputation >= 10 | After warming period |
| Create topic rooms | reputation >= 20 | After warming period |
| Vote on moderation | reputation >= 50 AND age >= 30d | Moderator election |
| Full posting rate (10/hr) | reputation >= 10 | After warming period |

### 7.4 Warming Period

New pseudonyms enter a 7-day warming period:

```rust
pub struct WarmingPeriod {
    pub started_at: Timestamp,
    pub duration: Duration,          // 7 days
}

impl WarmingPeriod {
    pub fn is_active(&self) -> bool {
        unix_now() < self.started_at.0 + self.duration.as_secs()
    }

    pub fn posting_limit(&self) -> u32 {
        1  // 1 post per hour during warming
    }

    pub fn allowed_content(&self) -> Vec<ContentType> {
        vec![ContentType::TextOnly]  // No media during warming
    }
}
```

The warming period makes Sybil attacks expensive: an attacker must wait 7 days per pseudonym before gaining full capabilities, and creating each pseudonym costs ~30 seconds of PoW.

### 7.5 EigenTrust-Inspired Local Scoring

Beyond the global reputation counter, each node maintains a local trust graph:

```rust
pub struct LocalTrustScore {
    /// Direct trust: have we had positive interactions?
    pub direct: f64,
    /// Indirect trust: are they trusted by people we trust?
    pub indirect: f64,
    /// Combined score
    pub combined: f64,
}

pub fn compute_local_trust(
    target: &IdentityKey,
    our_connections: &[IdentityKey],
    connection_trust: &HashMap<IdentityKey, f64>,
    their_connections: &HashSet<IdentityKey>,
) -> LocalTrustScore {
    // Direct trust: based on our own interactions
    let direct = connection_trust.get(target).copied().unwrap_or(0.0);

    // Indirect trust: weighted sum of trust from our connections
    let indirect: f64 = our_connections.iter()
        .filter(|conn| their_connections.contains(conn))
        .filter_map(|conn| connection_trust.get(conn))
        .sum::<f64>() / our_connections.len().max(1) as f64;

    let combined = 0.7 * direct + 0.3 * indirect;

    LocalTrustScore { direct, indirect, combined }
}
```

Local trust scores influence content ranking in the feed (posts from highly-trusted connections appear first in the chronological feed if timestamps are identical) and spam filtering (low-trust content from unknown pseudonyms is deprioritized).

---

## 8. Sybil Resistance

### 8.1 Multi-Layer Defense

No single defense suffices against Sybil attacks. Ephemera layers three defenses:

**Layer 1: Identity Creation PoW**
- ~30 seconds of Equihash computation on a modern mid-range phone.
- Adaptive difficulty during surges (if many identities are being created, difficulty increases).
- Memory-hard (resists GPU farms).
- Cost: creating 1,000 Sybil identities takes ~8.3 hours of continuous computation.

**Layer 2: Reputation Warming Period**
- 7-day warming period with severely limited capabilities.
- During warming: 1 post/hour, text only, no DMs to strangers, no moderation votes.
- Cost: each Sybil identity is useless for 7 days after creation.

**Layer 3: Social Trust (EigenTrust)**
- Reputation from mutual connections.
- Content from pseudonyms with zero connections and zero mutual trust is deprioritized.
- A Sybil army of mutually-connected fake identities is detectable (graph analysis: unusually dense, disconnected cluster).

### 8.2 Sybil Attack Economics

| Attack | PoW Cost | Warming Cost | Social Cost | Total |
|--------|----------|-------------|-------------|-------|
| 100 Sybil identities, text spam | 50 min compute | 7 days wait | Zero connections = low reach | High cost, low impact |
| 100 Sybil identities, vote manipulation | 50 min compute | 7 days wait | Need reputation >= 50 per identity | Impractical |
| 10 Sybil identities, sustained abuse | 5 min compute | 7 days wait | Can only post 1/hr each = 10 posts/hr total | Low impact during warming |

### 8.3 Adaptive PoW Difficulty

When the network detects a surge in identity creation:

```rust
pub fn adaptive_identity_pow_difficulty(
    recent_identity_count: u32,    // Identities created in the last hour
    baseline: u32,
) -> u32 {
    let multiplier = match recent_identity_count {
        0..=10   => 1.0,
        11..=50  => 2.0,
        51..=200 => 4.0,
        _        => 8.0,
    };
    ((baseline as f64) * multiplier) as u32
}
```

The identity creation rate is observed locally (how many new identities has this node seen recently?) and shared via gossip (nodes broadcast their local observation). Each node independently computes the adaptive difficulty.

---

## 9. Rate Limiting

### 9.1 Rate Limit Enforcement

Rate limits are enforced locally by the client node using a token bucket algorithm:

```rust
pub struct RateLimiter {
    buckets: HashMap<(IdentityKey, ActionType), TokenBucket>,
}

pub struct TokenBucket {
    capacity: u32,
    tokens: f64,
    refill_rate: f64,     // Tokens per second
    last_refill: Instant,
}

impl TokenBucket {
    pub fn try_consume(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn refill(&mut self) {
        let elapsed = self.last_refill.elapsed().as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.capacity as f64);
        self.last_refill = Instant::now();
    }
}

pub enum ActionType {
    Post,
    Reply,
    Reaction,
    DirectMessage,
    MessageRequest,
    Follow,
    ConnectionRequest,
}
```

### 9.2 Rate Limit Table

| Action | Capacity | Refill Rate | Effective Limit |
|--------|----------|-------------|----------------|
| Post | 10 | 10/hour | 10 posts/hour, burst of 10 |
| Reply (same thread) | 5 | 20/hour | 20 replies/hour, burst of 5 |
| Reaction | 20 | 100/hour | 100 reactions/hour, burst of 20 |
| DM (friend) | 10 | 60/hour | 60 DMs/hour per conversation |
| DM (mutual) | 5 | 30/hour | 30 DMs/hour per conversation |
| DM (stranger/request) | 1 | 5/hour | 5 message requests/hour total |
| Follow | 10 | 50/hour | 50 follows/hour, burst of 10 |
| Connection request | 5 | 20/hour | 20 requests/hour, burst of 5 |

### 9.3 Peer-Side Validation

When receiving content from peers, nodes independently check rate limits:

```rust
pub fn validate_peer_rate(
    author: &IdentityKey,
    action: ActionType,
    rate_tracker: &mut PeerRateTracker,
) -> RateValidation {
    let count = rate_tracker.increment(author, action);
    let limit = rate_limit_for(action);

    if count > limit * 2 {
        // Far over limit: reject and penalize reputation
        RateValidation::RejectWithPenalty
    } else if count > limit {
        // Slightly over: accept but flag
        RateValidation::AcceptWithWarning
    } else {
        RateValidation::Accept
    }
}
```

Peer-side validation is lenient (2x the stated limit) to account for clock skew and network delays. Persistent rate limit violations contribute to reputation penalties.

---

## 10. Spam Detection

### 10.1 SimHash Duplicate Detection

For detecting near-duplicate text spam:

```rust
pub fn simhash(text: &str) -> u64 {
    let mut v = [0i32; 64];
    for token in text.split_whitespace() {
        let hash = hash64(token);
        for i in 0..64 {
            if hash & (1 << i) != 0 {
                v[i] += 1;
            } else {
                v[i] -= 1;
            }
        }
    }
    let mut fingerprint: u64 = 0;
    for i in 0..64 {
        if v[i] > 0 {
            fingerprint |= 1 << i;
        }
    }
    fingerprint
}

pub fn is_near_duplicate(a: u64, b: u64, threshold: u32) -> bool {
    (a ^ b).count_ones() <= threshold  // Hamming distance
}
```

Posts with SimHash Hamming distance <= 3 from a recently seen post are flagged as potential spam. Flagged posts from low-reputation pseudonyms are suppressed from feeds.

---

## 11. Legal Compliance Features

### 11.1 Built Into Official Client

| Feature | Description | Status |
|---------|------------|--------|
| CSAM hash matching | Client-side bloom filter check | MVP must-have |
| Age self-declaration | "By continuing, you confirm you are 16+" | MVP must-have |
| Report button | On every post and message | MVP must-have |
| IP geofencing | Block connections from sanctioned jurisdictions | MVP must-have |
| Terms of service | Community guidelines with legal terms | MVP must-have |

### 11.2 IP Geofencing

The Ephemera client blocks network connections to/from IP addresses in sanctioned jurisdictions (OFAC, EU sanctions list):

```rust
pub struct GeofenceConfig {
    pub sanctioned_countries: HashSet<String>,   // ISO 3166-1 alpha-2
    pub geoip_database: PathBuf,                 // MaxMind GeoLite2 database
    pub enforcement: GeofenceEnforcement,
}

pub enum GeofenceEnforcement {
    /// Block connections from sanctioned IPs
    Block,
    /// Log only (for development/testing)
    LogOnly,
}
```

**Note:** Geofencing is trivially bypassed by VPN/Tor users. It exists as a good-faith compliance measure, not as a reliable enforcement mechanism.

### 11.3 Node Operator Legal Position

- Node operators are "mere conduits" -- they relay encrypted data without inspection.
- This is consistent with EU DSA Article 4, E-Commerce Directive Article 12, and US Section 230.
- Relay nodes MUST NOT log, inspect, or correlate user traffic beyond minimum for delivery.
- Relay retention: undelivered messages purged after 14 days. Delivery metadata purged immediately after successful delivery.

### 11.4 Marketing Language Rules

**DO use:**
- "Privacy-preserving social communication"
- "Encrypted messaging"
- "Ephemeral content"

**NEVER use:**
- "Untraceable"
- "Censorship-proof"
- "Law-enforcement-proof"
- "Anonymous" (use "pseudonymous" instead)

The Grokster inducement theory and Tornado Cash precedent make marketing language a survival issue for the project.

---

## 12. What We Will NOT Build

| Feature | Reason |
|---------|--------|
| Key escrow or ghost protocol participants | Breaks the security model. Non-negotiable. |
| Mandatory identity verification | Defeats the purpose of pseudonymous communication. |
| Server-side scanning of encrypted content | Breaks E2E encryption. Non-negotiable. |
| Permanent bans | Unenforceable in a pseudonymous system. Reputation is the lever. |
| A cryptocurrency token | Regulatory landmine, securities law risk, wrong community signal. |
| Trending or algorithmic amplification | Legal liability target (Pirate Bay, Tornado Cash precedents). Anti-platform philosophy. |
| Backdoors for any government | See key escrow above. |
| Content fingerprinting beyond CSAM | Scope creep risk. CSAM is the narrow, universally-condemned exception. |

---

*This document is part of the Ephemera Architecture series. See [ARCHITECTURE.md](./ARCHITECTURE.md) for the master document.*
